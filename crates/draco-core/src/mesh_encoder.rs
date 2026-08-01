use crate::attribute_quantization_transform::AttributeQuantizationTransform;
use crate::attribute_transform::AttributeTransform;
use crate::compression_config::EncodedGeometryType;
use crate::compression_config::MeshEncodingMethod;
use crate::corner_table::CornerTable;
use crate::draco_types::DataType;
use crate::encoder_buffer::EncoderBuffer;
use crate::encoder_options::EncoderOptions;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{FaceIndex, PointIndex, INVALID_ATTRIBUTE_VALUE_INDEX};
use crate::mesh::Mesh;
use crate::mesh_edgebreaker_encoder::{EdgebreakerAttributeConnectivity, MeshEdgebreakerEncoder};
use crate::metadata::METADATA_FLAG_MASK;
use crate::point_cloud::PointCloud;
use crate::point_cloud_encoder::GeometryEncoder;
use crate::sequential_attribute_encoder::{select_sequential_encoder, SequentialAttributeEncoder};
use crate::sequential_integer_attribute_encoder::SequentialIntegerAttributeEncoder;
use crate::sequential_normal_attribute_encoder::SequentialNormalAttributeEncoder;
use crate::status::{DracoError, Status};
use crate::version::{
    has_header_flags, uses_varint_encoding, uses_varint_unique_id, DEFAULT_MESH_VERSION,
};

/// `(min, max)` per-component position bounds, each present when computable.
type PositionBounds = (Option<Vec<f64>>, Option<Vec<f64>>);

/// Encoder for Draco triangle mesh bitstreams.
///
/// A `MeshEncoder` takes a [`Mesh`] plus [`EncoderOptions`] and writes a
/// self-contained `.drc` bitstream (header, optional metadata, connectivity,
/// and attributes) into an [`EncoderBuffer`]. The encoding method (EdgeBreaker or
/// sequential), prediction schemes, and quantization are selected from the
/// options, mirroring the C++ `MeshEncoder`/`ExpertEncoder` configuration.
///
/// After a successful [`encode`](MeshEncoder::encode), per-attribute and
/// per-face details are available via
/// [`encoded_mesh_info`](MeshEncoder::encoded_mesh_info).
///
/// # Examples
///
/// Build a single-triangle mesh, encode it, and decode it back:
///
/// ```
/// use draco_core::{
///     DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, FaceIndex,
///     GeometryAttributeType, Mesh, MeshDecoder, MeshEncoder, PointAttribute,
/// };
///
/// // One triangle with a float32 position attribute (3 vertices).
/// let mut mesh = Mesh::new();
/// let mut position = PointAttribute::new();
/// position.init(GeometryAttributeType::Position, 3, DataType::Float32, false, 3);
/// let coords: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
/// for (i, value) in coords.iter().enumerate() {
///     position.buffer_mut().write(i * 4, &value.to_le_bytes());
/// }
/// mesh.add_attribute(position);
/// mesh.set_num_faces(1);
/// mesh.set_face(FaceIndex(0), [0u32.into(), 1u32.into(), 2u32.into()]);
///
/// // Encode to a Draco bitstream.
/// let mut encoder = MeshEncoder::new();
/// encoder.set_mesh(mesh);
/// let mut buffer = EncoderBuffer::new();
/// encoder.encode(&EncoderOptions::new(), &mut buffer)?;
///
/// // Decode it back.
/// let mut decoded = Mesh::new();
/// MeshDecoder::new().decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)?;
/// assert_eq!(decoded.num_faces(), 1);
/// # Ok::<(), draco_core::DracoError>(())
/// ```
pub struct MeshEncoder {
    mesh: Option<Mesh>,
    options: EncoderOptions,
    num_encoded_faces: usize,
    corner_table: Option<CornerTable>,
    point_ids: Vec<PointIndex>,
    data_to_corner_map: Option<Vec<u32>>,
    vertex_to_data_map: Option<Vec<i32>>,
    edgebreaker_attribute_connectivity: Vec<EdgebreakerAttributeConnectivity>,
    active_corner_table: Option<CornerTable>,
    active_data_to_corner_map: Option<Vec<u32>>,
    active_vertex_to_data_map: Option<Vec<i32>>,
    /// Depth-first order for the non-position attribute groups, present only
    /// when the position group uses a different one (speed 0).
    #[allow(clippy::type_complexity)]
    attribute_traversal: Option<(Vec<PointIndex>, Vec<u32>, Vec<i32>)>,
    /// Attributes in their portable (quantized) form, for the current group.
    /// Prediction schemes read their parent attribute from here.
    portable_attributes: Vec<(i32, PointAttribute)>,
    /// Kept past `encode_edgebreaker_connectivity` for its corner order, which
    /// an attribute with interior seams needs to walk its own corner table.
    edgebreaker_encoder: Option<MeshEdgebreakerEncoder>,
    method: i32,
    /// Maps point indices to vertex indices in the corner table.
    /// Used when position-based deduplication is enabled.
    point_to_vertex_map: Option<Vec<u32>>,
    /// Whether we're using single connectivity (all attributes share same corner table).
    use_single_connectivity: bool,
    encoded_mesh_info: Option<EncodedMeshInfo>,
}

/// Geometry shape and attribute metadata produced by a successful mesh encode.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedMeshInfo {
    /// Numeric Draco mesh encoding method used for the output.
    pub encoding_method: i32,
    /// Number of faces encoded into the bitstream.
    pub num_encoded_faces: usize,
    /// Number of points encoded into the bitstream.
    pub num_encoded_points: usize,
    /// Per-attribute information captured during encoding.
    pub attributes: Vec<EncodedAttributeInfo>,
}

/// Attribute metadata produced by a successful mesh encode.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedAttributeInfo {
    /// Source attribute id in the input mesh.
    pub source_attribute_id: i32,
    /// Semantic type of the encoded attribute.
    pub attribute_type: GeometryAttributeType,
    /// Scalar data type of the encoded attribute.
    pub data_type: DataType,
    /// Number of scalar components per encoded value.
    pub num_components: u8,
    /// Whether integer values are normalized.
    pub normalized: bool,
    /// Draco unique id assigned to the attribute.
    pub unique_id: u32,
    /// Number of unique values encoded for the attribute.
    pub num_encoded_values: usize,
    /// Minimum position components when known for position attributes.
    pub position_min: Option<Vec<f64>>,
    /// Maximum position components when known for position attributes.
    pub position_max: Option<Vec<f64>>,
}

impl GeometryEncoder for MeshEncoder {
    fn point_cloud(&self) -> Option<&PointCloud> {
        self.mesh.as_ref().map(|m| m as &PointCloud)
    }

    fn mesh(&self) -> Option<&Mesh> {
        self.mesh.as_ref()
    }

    fn corner_table(&self) -> Option<&CornerTable> {
        self.active_corner_table
            .as_ref()
            .or(self.corner_table.as_ref())
    }

    fn options(&self) -> &EncoderOptions {
        &self.options
    }

    fn get_geometry_type(&self) -> EncodedGeometryType {
        EncodedGeometryType::TriangularMesh
    }

    fn get_encoding_method(&self) -> Option<i32> {
        Some(self.method)
    }

    fn get_data_to_corner_map(&self) -> Option<&[u32]> {
        self.active_data_to_corner_map
            .as_deref()
            .or(self.data_to_corner_map.as_deref())
    }

    fn get_vertex_to_data_map(&self) -> Option<&[i32]> {
        self.active_vertex_to_data_map
            .as_deref()
            .or(self.vertex_to_data_map.as_deref())
    }

    fn get_portable_attribute(&self, att_id: i32) -> Option<&PointAttribute> {
        // Falls back to the attribute itself when it has no portable form, as
        // SequentialAttributeEncoder::GetPortableAttribute does. An attribute
        // that needs no transform -- an already-integral one, say -- is its own
        // portable form upstream, and a predictor asking for it must get it
        // rather than a null it would treat as a failure.
        self.portable_attributes
            .iter()
            .find(|(id, _)| *id == att_id)
            .map(|(_, att)| att)
            .or_else(|| {
                self.mesh
                    .as_ref()
                    .and_then(|mesh| mesh.try_attribute(att_id).ok())
            })
    }
}

impl MeshEncoder {
    /// Creates an encoder without an assigned mesh.
    pub fn new() -> Self {
        Self {
            mesh: None,
            options: EncoderOptions::default(),
            num_encoded_faces: 0,
            corner_table: None,
            point_ids: Vec::new(),
            data_to_corner_map: None,
            vertex_to_data_map: None,
            edgebreaker_attribute_connectivity: Vec::new(),
            active_corner_table: None,
            active_data_to_corner_map: None,
            active_vertex_to_data_map: None,
            attribute_traversal: None,
            portable_attributes: Vec::new(),
            edgebreaker_encoder: None,
            method: 0,
            point_to_vertex_map: None,
            use_single_connectivity: false,
            encoded_mesh_info: None,
        }
    }

    /// Assigns the mesh to encode.
    pub fn set_mesh(&mut self, mesh: Mesh) {
        self.mesh = Some(mesh);
    }

    /// Drops everything the previous encode derived from its mesh.
    ///
    /// An encoder is reusable - `set_mesh` then `encode`, twice - and each
    /// encode caches connectivity for the attribute stage to read back:
    /// a corner table, a point order, corner and vertex maps, per-attribute
    /// seam connectivity. Only some of that is rewritten by every path. The
    /// sequential connectivity branch does not build a corner table, so after
    /// an EdgeBreaker encode it inherited the previous mesh's one and wrote
    /// attributes against topology the stream does not describe: encoding an
    /// attributed mesh with EdgeBreaker and then a plain mesh sequentially
    /// with the same encoder produced a stream this crate's own decoder
    /// rejects. Resetting in one place is the fix that does not depend on
    /// every future path remembering to.
    fn reset_derived_state(&mut self) {
        self.encoded_mesh_info = None;
        self.portable_attributes.clear();
        self.edgebreaker_encoder = None;
        self.num_encoded_faces = 0;
        self.corner_table = None;
        self.point_ids.clear();
        self.data_to_corner_map = None;
        self.vertex_to_data_map = None;
        self.edgebreaker_attribute_connectivity.clear();
        self.active_corner_table = None;
        self.active_data_to_corner_map = None;
        self.active_vertex_to_data_map = None;
        self.attribute_traversal = None;
        self.method = 0;
        self.point_to_vertex_map = None;
        self.use_single_connectivity = false;
    }

    /// Returns the assigned mesh, if any.
    pub fn mesh(&self) -> Option<&Mesh> {
        self.mesh.as_ref()
    }

    /// Returns the number of faces encoded by the last successful encode.
    pub fn num_encoded_faces(&self) -> usize {
        self.num_encoded_faces
    }

    /// Returns the corner table built during the last mesh encode, if any.
    pub fn corner_table(&self) -> Option<&CornerTable> {
        self.corner_table.as_ref()
    }

    /// Returns information captured during the last successful mesh encode.
    pub fn encoded_mesh_info(&self) -> Option<&EncodedMeshInfo> {
        self.encoded_mesh_info.as_ref()
    }

    /// Encodes the assigned mesh into an output buffer.
    ///
    /// A mesh must have been provided with [`set_mesh`](MeshEncoder::set_mesh)
    /// first. On success the bitstream is appended to `out_buffer` and
    /// [`encoded_mesh_info`](MeshEncoder::encoded_mesh_info) is populated.
    ///
    /// # Errors
    ///
    /// Returns an error if no mesh was set, if the requested encoding method or
    /// options are unsupported, or if attribute encoding fails.
    pub fn encode(&mut self, options: &EncoderOptions, out_buffer: &mut EncoderBuffer) -> Status {
        self.options = options.clone();
        self.reset_derived_state();

        if self.mesh.is_none() {
            return Err(DracoError::DracoError("Mesh not set".to_string()));
        }
        crate::point_cloud_encoder::validate_encodable_attributes(self.mesh.as_ref().unwrap())?;
        let (major, minor) = self.options.get_version();
        crate::version::validate_encodable_version(major, minor, DEFAULT_MESH_VERSION)?;
        Self::validate_face_indices(self.mesh.as_ref().unwrap())?;
        self.validate_predictive_traversal()?;
        self.validate_prediction_schemes(self.mesh.as_ref().unwrap())?;

        // 1. Encode Header
        self.encode_header(out_buffer)?;
        self.encode_metadata(out_buffer)?;

        // 2. Encode geometry data (connectivity + attributes)
        self.encode_geometry_data(out_buffer)?;

        Ok(())
    }

    fn encode_metadata(&self, buffer: &mut EncoderBuffer) -> Status {
        if let Some(metadata) = self
            .mesh
            .as_ref()
            .and_then(|mesh| mesh.metadata())
            .filter(|metadata| !metadata.is_empty())
        {
            metadata.encode(buffer)?;
        }
        Ok(())
    }

    /// Rejects a tex-coord prediction scheme forced onto an attribute that is
    /// not a texture coordinate.
    ///
    /// Both tex-coord predictors work on two components and predict from the
    /// position, so the encoder builds one for any attribute that presents two
    /// components. A normal does, once the octahedron transform has folded it
    /// from three - so a scheme meant for UVs was accepted for normals and
    /// wrote values the normal decoder cannot read back. Three-component
    /// attributes were already refused, which is why only normals slipped
    /// through.
    fn validate_prediction_schemes(&self, mesh: &Mesh) -> Status {
        const TEX_COORDS_DEPRECATED: i32 = 3;
        const TEX_COORDS_PORTABLE: i32 = 5;

        for att_id in 0..mesh.num_attributes() {
            let scheme = self.options.get_attribute_prediction_scheme(att_id);
            if !matches!(scheme, TEX_COORDS_DEPRECATED | TEX_COORDS_PORTABLE) {
                continue;
            }
            let attribute_type = mesh.attribute(att_id).attribute_type();
            if attribute_type != GeometryAttributeType::TexCoord {
                return Err(DracoError::DracoError(format!(
                    "Prediction scheme {scheme} predicts texture coordinates and cannot be used \
                     for attribute {att_id}, which is a {attribute_type:?}"
                )));
            }
        }
        Ok(())
    }

    /// Rejects the legacy predictive traversal on a version that cannot carry
    /// it.
    ///
    /// `force_predictive_traversal` round-trips pre-0.10.0 connectivity and
    /// only belongs in a target version below 2.0, which the encoder's own
    /// comment said and nothing checked. Set on a current-version encode, it
    /// produced a type-1 traversal inside a 2.x stream - which the decoder
    /// refuses, since 2.x connectivity has no predictive traversal to read.
    fn validate_predictive_traversal(&self) -> Status {
        if self.options.get_global_int("force_predictive_traversal", 0) == 0 {
            return Ok(());
        }
        let (mut major, mut minor) = self.options.get_version();
        if major == 0 && minor == 0 {
            (major, minor) = DEFAULT_MESH_VERSION;
        }
        if !crate::version::version_less_than(major, minor, (2, 0)) {
            return Err(DracoError::UnsupportedFeature(format!(
                "force_predictive_traversal requires a target bitstream version below 2.0, \
                 not {major}.{minor}"
            )));
        }
        Ok(())
    }

    /// Rejects a face that references a point the mesh does not have.
    ///
    /// The index buffer is the other half of caller-supplied geometry, and
    /// nothing between a file and this encoder re-checks it against the point
    /// count. Both connectivity paths then use face indices to index
    /// point-sized arrays directly - `point_to_vertex[face[j]]` in the corner
    /// table build is the shortest route to it - so an out-of-range index is a
    /// panic rather than an encode failure. One pass here answers for every
    /// such use.
    fn validate_face_indices(mesh: &Mesh) -> Status {
        let num_points = mesh.num_points();
        for face_id in 0..mesh.num_faces() {
            let face = mesh.face(FaceIndex(face_id as u32));
            for index in face {
                if index.0 as usize >= num_points {
                    return Err(DracoError::DracoError(format!(
                        "Face {face_id} references point {} but the mesh has {num_points} points",
                        index.0
                    )));
                }
            }
        }
        Ok(())
    }

    fn encode_header(&self, buffer: &mut EncoderBuffer) -> Status {
        let (mut major, mut minor) = self.options.get_version();
        if major == 0 && minor == 0 {
            // Default to latest mesh version
            (major, minor) = DEFAULT_MESH_VERSION;
        }
        let has_metadata = self
            .mesh
            .as_ref()
            .and_then(|mesh| mesh.metadata())
            .is_some_and(|metadata| !metadata.is_empty());

        if has_metadata && !has_header_flags(major, minor) {
            return Err(DracoError::UnsupportedVersion(
                "Metadata requires Draco bitstream version 1.3 or newer".to_string(),
            ));
        }

        // C++ default behavior: Edgebreaker if speed != 10, Sequential if speed == 10
        let method_int = self.options.get_global_int("encoding_method", -1);
        let method = if method_int == -1 {
            if self.options.get_speed() == 10 {
                0
            } else {
                1
            }
        } else if method_int == 1 {
            1
        } else {
            0
        };

        #[cfg(not(feature = "legacy_bitstream_encode"))]
        if method == 1 {
            let bitstream_version = crate::version::bitstream_version(major, minor);
            if bitstream_version < 0x0202 {
                return Err(DracoError::UnsupportedVersion(
                    "EdgeBreaker mesh encoding before bitstream 2.2 requires the \
                     legacy_bitstream_encode feature"
                        .to_string(),
                ));
            }
            if self.options.get_global_int("force_predictive_traversal", 0) != 0 {
                return Err(DracoError::UnsupportedFeature(
                    "force_predictive_traversal requires the legacy_bitstream_encode feature"
                        .to_string(),
                ));
            }
        }
        #[cfg(not(feature = "legacy_bitstream_encode"))]
        match self.options.get_prediction_scheme() {
            2 | 3 => {
                return Err(DracoError::UnsupportedFeature(
                    "legacy prediction schemes require the legacy_bitstream_encode feature"
                        .to_string(),
                ));
            }
            _ => {}
        }

        buffer.encode_data(b"DRACO");

        buffer.encode_u8(major);
        buffer.encode_u8(minor);
        buffer.set_version(major, minor);
        buffer.encode_u8(self.get_geometry_type() as u8);
        buffer.encode_u8(method);

        // The flags field is always present in the binary header (the decoder reads
        // it unconditionally); only the metadata bit gains meaning at v1.3+, which
        // is guarded by the metadata check above. Emitting it only for >= 1.3 left
        // pre-1.3 streams two bytes short, misaligning the rest of the stream.
        let flags = if has_metadata { METADATA_FLAG_MASK } else { 0 };
        buffer.encode_u16(flags);
        Ok(())
    }

    fn encode_geometry_data(&mut self, out_buffer: &mut EncoderBuffer) -> Status {
        // First encode connectivity
        self.encode_connectivity(out_buffer)?;

        // Check if we should store the number of encoded faces
        if self
            .options
            .get_global_int("store_number_of_encoded_faces", 0)
            != 0
        {
            self.compute_number_of_encoded_faces();
        }

        // Then encode attributes
        self.encode_attributes(out_buffer)?;
        self.build_encoded_mesh_info()?;

        Ok(())
    }

    fn encode_connectivity(&mut self, out_buffer: &mut EncoderBuffer) -> Status {
        let mesh = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding");

        // Determine encoding method FIRST (before building corner table)
        let method_int = self.options.get_global_int("encoding_method", -1);
        let method = if method_int == -1 {
            if self.options.get_speed() == 10 {
                MeshEncodingMethod::MeshSequentialEncoding
            } else {
                MeshEncodingMethod::MeshEdgebreakerEncoding
            }
        } else if method_int == 1 {
            MeshEncodingMethod::MeshEdgebreakerEncoding
        } else {
            MeshEncodingMethod::MeshSequentialEncoding
        };
        self.method = if method == MeshEncodingMethod::MeshEdgebreakerEncoding {
            1
        } else {
            0
        };

        // C++ behavior: use_single_connectivity_ when speed >= 6
        // When false (speed < 6), use position attribute to deduplicate vertices
        let speed = self.options.get_speed();
        // Check if split_mesh_on_seams is explicitly set, otherwise use speed-based default
        let split_on_seams_explicit = self.options.get_global_int("split_mesh_on_seams", -1);
        let use_single_connectivity = if split_on_seams_explicit >= 0 {
            split_on_seams_explicit != 0
        } else {
            speed >= 6
        };

        // Only build corner table if needed (not for sequential encoding)
        if method == MeshEncodingMethod::MeshEdgebreakerEncoding {
            let (faces, point_to_vertex_map) = if use_single_connectivity {
                // CreateCornerTableFromAllAttributes: use point indices directly
                let faces: Vec<[crate::geometry_indices::VertexIndex; 3]> = (0..mesh.num_faces())
                    .map(|i| {
                        let face = mesh.face(FaceIndex(i as u32));
                        [
                            crate::geometry_indices::VertexIndex(face[0].0),
                            crate::geometry_indices::VertexIndex(face[1].0),
                            crate::geometry_indices::VertexIndex(face[2].0),
                        ]
                    })
                    .collect();
                // Identity mapping
                let point_to_vertex: Vec<u32> = (0..mesh.num_points() as u32).collect();
                (faces, point_to_vertex)
            } else {
                // CreateCornerTableFromPositionAttribute: use position attribute to deduplicate
                self.create_corner_table_from_position_attribute(mesh)
            };

            // Initialize corner table for the mesh
            let mut corner_table = CornerTable::new(0);
            corner_table.init(&faces);

            // A mesh whose every face is degenerate has no connectivity to
            // traverse: `point_ids` comes back empty, and everything downstream
            // that assumes at least one encoded point panics rather than
            // failing cleanly. C++ rejects the same input outright --
            // `MeshEdgebreakerEncoderImpl::Init` checks
            // `num_faces() == NumDegeneratedFaces()` before doing anything else.
            if corner_table.num_faces() > 0
                && corner_table.num_faces() == corner_table.num_degenerated_faces()
            {
                return Err(DracoError::DracoError(
                    "All triangles are degenerate.".to_string(),
                ));
            }

            self.corner_table = Some(corner_table);
            self.point_to_vertex_map = Some(point_to_vertex_map);
            self.edgebreaker_attribute_connectivity.clear();
            if !use_single_connectivity {
                if let Some(ref ct) = self.corner_table {
                    for i in 0..mesh.num_attributes() {
                        let att = mesh.attribute(i);
                        if att.attribute_type() != GeometryAttributeType::Position {
                            self.edgebreaker_attribute_connectivity
                                .push(EdgebreakerAttributeConnectivity::build(mesh, ct, i));
                        }
                    }
                }
            }
        } else {
            // Sequential encoding: no corner table needed, use identity mapping
            let point_to_vertex: Vec<u32> = (0..mesh.num_points() as u32).collect();
            self.point_to_vertex_map = Some(point_to_vertex);
            self.edgebreaker_attribute_connectivity.clear();
        }
        self.use_single_connectivity = use_single_connectivity;

        match method {
            MeshEncodingMethod::MeshSequentialEncoding => {
                self.encode_sequential_connectivity(out_buffer)
            }
            MeshEncodingMethod::MeshEdgebreakerEncoding => {
                self.encode_edgebreaker_connectivity(out_buffer)
            }
        }
    }

    fn encode_edgebreaker_connectivity(&mut self, out_buffer: &mut EncoderBuffer) -> Status {
        let mesh = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding");
        let corner_table = self
            .corner_table
            .as_ref()
            .expect("corner_table must be set before edgebreaker encoding");

        let mut encoder = MeshEdgebreakerEncoder::new(mesh.num_faces(), mesh.num_points());
        // Opt-in legacy predictive (type-1) traversal, for round-tripping the
        // pre-0.10.0 connectivity. Requires a < 2.0 target version.
        #[cfg(feature = "legacy_bitstream_encode")]
        encoder.set_force_predictive(
            self.options.get_global_int("force_predictive_traversal", 0) == 1,
        );
        let (point_ids, data_to_corner_map, vertex_to_data_map) = encoder.encode_connectivity(
            mesh,
            corner_table,
            &self.edgebreaker_attribute_connectivity,
            out_buffer,
            self.options.get_speed() as usize,
            self.use_single_connectivity,
        )?;
        #[cfg(feature = "debug_logs")]
        {
            debug_log!("DEBUG: encode_edgebreaker_connectivity: point_ids.len()={}, data_to_corner_map.len()={}, vertex_to_data_map.len()={}",
                 point_ids.len(), data_to_corner_map.len(), vertex_to_data_map.len());
        }
        // At speed 0 the position walks the mesh by max prediction degree while
        // every other attribute stays depth first, so the two orders part ways
        // and the non-position groups need their own. At any other speed the
        // position order already is the depth-first one.
        self.attribute_traversal = if self.options.get_speed() == 0 && mesh.num_attributes() > 1 {
            Some(encoder.generate_depth_first_traversal(mesh, corner_table))
        } else {
            None
        };

        self.point_ids = point_ids;

        // Draco stores corner mapping in attribute (data) order.
        self.data_to_corner_map = Some(data_to_corner_map);
        self.vertex_to_data_map = Some(vertex_to_data_map);

        // Held for the corner order it carries: an attribute with interior
        // seams walks its own corner table seeded from that order, and this is
        // the last point at which it exists.
        self.edgebreaker_encoder = Some(encoder);

        Ok(())
    }

    /// Creates faces array using position attribute to deduplicate vertices.
    /// This mimics C++ CreateCornerTableFromPositionAttribute.
    /// Returns (faces, point_to_vertex_map) where:
    /// - faces: vertex indices (deduplicated based on position values)
    /// - point_to_vertex_map: maps each point index to its vertex index in the corner table
    fn create_corner_table_from_position_attribute(
        &self,
        mesh: &Mesh,
    ) -> (Vec<[crate::geometry_indices::VertexIndex; 3]>, Vec<u32>) {
        use crate::geometry_attribute::GeometryAttributeType;

        let pos_att_id = mesh.named_attribute_id(GeometryAttributeType::Position);
        if pos_att_id < 0 {
            // No position attribute, fall back to identity mapping
            let faces: Vec<[crate::geometry_indices::VertexIndex; 3]> = (0..mesh.num_faces())
                .map(|i| {
                    let face = mesh.face(FaceIndex(i as u32));
                    [
                        crate::geometry_indices::VertexIndex(face[0].0),
                        crate::geometry_indices::VertexIndex(face[1].0),
                        crate::geometry_indices::VertexIndex(face[2].0),
                    ]
                })
                .collect();
            let point_to_vertex: Vec<u32> = (0..mesh.num_points() as u32).collect();
            return (faces, point_to_vertex);
        }

        let pos_att = mesh.attribute(pos_att_id);
        let _buffer = pos_att.buffer();
        let num_components = pos_att.num_components() as usize;
        let _byte_stride = match pos_att.data_type() {
            crate::draco_types::DataType::Float32 => num_components * 4,
            crate::draco_types::DataType::Float64 => num_components * 8,
            crate::draco_types::DataType::Int8 | crate::draco_types::DataType::Uint8 => {
                num_components
            }
            crate::draco_types::DataType::Int16 | crate::draco_types::DataType::Uint16 => {
                num_components * 2
            }
            crate::draco_types::DataType::Int32 | crate::draco_types::DataType::Uint32 => {
                num_components * 4
            }
            crate::draco_types::DataType::Int64 | crate::draco_types::DataType::Uint64 => {
                num_components * 8
            }
            _ => num_components * 4, // Default to 4 bytes per component
        };

        // Use attribute mapped indices directly to build point->vertex map. This mirrors
        // C++ CreateCornerTableFromAttribute which uses att->mapped_index(face[j]).
        let mut point_to_vertex: Vec<u32> = vec![0; mesh.num_points()];
        for i in 0..mesh.num_points() {
            let pt = PointIndex(i as u32);
            let val_idx = pos_att.mapped_index(pt);
            point_to_vertex[i] = val_idx.0;
        }

        // Build faces using attribute mapped indices (exact same mapping as C++).
        let faces: Vec<[crate::geometry_indices::VertexIndex; 3]> = (0..mesh.num_faces())
            .map(|i| {
                let face = mesh.face(FaceIndex(i as u32));
                [
                    crate::geometry_indices::VertexIndex(point_to_vertex[face[0].0 as usize]),
                    crate::geometry_indices::VertexIndex(point_to_vertex[face[1].0 as usize]),
                    crate::geometry_indices::VertexIndex(point_to_vertex[face[2].0 as usize]),
                ]
            })
            .collect();

        #[cfg(feature = "debug_logs")]
        {
            debug_log!(
                "Rust created faces (first 12): {:?}",
                faces
                    .iter()
                    .take(12)
                    .map(|f| [f[0].0, f[1].0, f[2].0])
                    .collect::<Vec<_>>()
            );
            debug_log!(
                "Rust point_to_vertex (first 25): {:?}",
                point_to_vertex.iter().take(25).cloned().collect::<Vec<_>>()
            );
        }
        (faces, point_to_vertex)
    }

    fn encode_sequential_connectivity(&mut self, out_buffer: &mut EncoderBuffer) -> Status {
        let mesh = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding");

        // Encode the number of faces and points
        // Use the buffer's version (set in encode_header) for version checks
        let major = out_buffer.version_major();
        let minor = out_buffer.version_minor();
        if !uses_varint_encoding(major, minor) {
            out_buffer.encode_u32(mesh.num_faces() as u32);
            out_buffer.encode_u32(mesh.num_points() as u32);
        } else {
            out_buffer.encode_varint(mesh.num_faces() as u64);
            out_buffer.encode_varint(mesh.num_points() as u64);
        }

        if mesh.num_faces() > 0 && mesh.num_points() > 0 {
            out_buffer.encode_u8(1); // Raw connectivity
            if mesh.num_points() < 256 {
                for face_id in 0..mesh.num_faces() {
                    let face = mesh.face(FaceIndex(face_id as u32));
                    for i in 0..3 {
                        out_buffer.encode_u8(face[i].0 as u8);
                    }
                }
            } else if mesh.num_points() < 65536 {
                for face_id in 0..mesh.num_faces() {
                    let face = mesh.face(FaceIndex(face_id as u32));
                    for i in 0..3 {
                        out_buffer.encode_u16(face[i].0 as u16);
                    }
                }
            } else if mesh.num_points() < (1 << 21) {
                // Use varint encoding for indices when points fit in 21 bits
                // This matches C++ behavior for better compression
                for face_id in 0..mesh.num_faces() {
                    let face = mesh.face(FaceIndex(face_id as u32));
                    for i in 0..3 {
                        out_buffer.encode_varint(face[i].0 as u64);
                    }
                }
            } else {
                // Default: use u32 for very large meshes
                for face_id in 0..mesh.num_faces() {
                    let face = mesh.face(FaceIndex(face_id as u32));
                    for i in 0..3 {
                        out_buffer.encode_u32(face[i].0);
                    }
                }
            }
        }

        // Identity permutation for sequential encoding
        self.point_ids = (0..mesh.num_points())
            .map(|i| PointIndex(i as u32))
            .collect();

        Ok(())
    }

    fn encode_attributes(&mut self, out_buffer: &mut EncoderBuffer) -> Status {
        // NOTE: Unlike the decoder, the encoder does NOT need to apply UpdatePointToAttributeIndexMapping
        // because the attribute still has identity mapping. The encoder uses the point_ids array
        // (from edgebreaker traversal) to determine the order in which to process points, and
        // mapped_index with identity mapping just returns the point index directly.

        let mesh = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding");

        let method_int = self.options.get_global_int("encoding_method", -1);
        // Match C++ behavior: if encoding_method is not set (-1),
        // use Edgebreaker for all options except speed == 10
        let is_edgebreaker = if method_int == -1 {
            self.options.get_speed() != 10
        } else {
            method_int == 1
        };

        if is_edgebreaker && !self.use_single_connectivity {
            return self.encode_edgebreaker_attributes_split(out_buffer);
        }

        // Encode number of attribute decoders (u8).
        // For both sequential and edgebreaker with single-connectivity mode:
        // there's only ONE attribute encoder containing ALL attributes.
        // This matches C++ behavior when use_single_connectivity_ = true (speed >= 6).
        let num_attributes = mesh.num_attributes();
        let num_encoders = if num_attributes > 0 { 1 } else { 0 };
        // Use the buffer's version (set in encode_header) for version checks.
        let major = out_buffer.version_major();
        let minor = out_buffer.version_minor();

        out_buffer.encode_u8(num_encoders as u8);

        // Phase 1: attributes decoder identifiers.
        // For single-encoder mode: one encoder with att_data_id = -1 (uses position connectivity)
        if num_encoders > 0 && is_edgebreaker {
            // att_data_id (i8), encoder_type (u8), traversal_method (u8)
            // -1 means use position connectivity (single connectivity mode)
            out_buffer.encode_u8((-1i8) as u8); // att_data_id = -1
            out_buffer.encode_u8(0); // element_type = MESH_VERTEX_ATTRIBUTE

            // Traversal method was added in bitstream 1.2. Older streams
            // default to DEPTH_FIRST on decode and must not carry the byte.
            if crate::version::bitstream_version(major, minor) >= 0x0102 {
                // PREDICTION_DEGREE (1) for speed 0, DEPTH_FIRST (0) otherwise.
                // This must match the traversal used in MeshEdgebreakerEncoder.
                let encoding_speed = self.options.get_speed();
                let traversal_method: u8 = if encoding_speed == 0 { 1 } else { 0 };
                out_buffer.encode_u8(traversal_method);
            }
        }
        // For sequential, nothing is written in phase 1 (EncodeAttributesEncoderIdentifier does nothing)

        let mut decoder_types: Vec<u8> = Vec::with_capacity(mesh.num_attributes() as usize);

        // Phase 2: Encode attribute encoder data
        // Both sequential and edgebreaker now use single-encoder mode:
        //   - Write num_attrs = total attributes
        //   - Write all attribute metadata
        //   - Write all decoder types

        if num_encoders > 0 {
            // Single encoder with all attributes (single-connectivity mode for edgebreaker)
            // Write num_attrs = total number of attributes
            if !uses_varint_encoding(major, minor) {
                out_buffer.encode_u32(mesh.num_attributes() as u32);
            } else {
                out_buffer.encode_varint(mesh.num_attributes() as u64);
            }

            // Write all attribute metadata first
            for i in 0..mesh.num_attributes() {
                let att = mesh.attribute(i);

                #[cfg(feature = "debug_logs")]
                {
                    debug_log!("DEBUG: Encoder encoding attribute {} metadata. Type: {:?}, Components: {}, Data: {:?}", i, att.attribute_type(), att.num_components(), att.data_type());
                }
                out_buffer.encode_u8(att.attribute_type() as u8);
                out_buffer.encode_u8(att.data_type() as u8);
                out_buffer.encode_u8(att.num_components());
                out_buffer.encode_u8(if att.normalized() { 1 } else { 0 });

                if !uses_varint_unique_id(major, minor) {
                    out_buffer.encode_u16(att.unique_id() as u16);
                } else {
                    out_buffer.encode_varint(att.unique_id() as u64);
                }
            }

            // Write all decoder types after all metadata (SequentialAttributeEncodersController pattern)
            for i in 0..mesh.num_attributes() {
                let att = mesh.attribute(i);
                let quantization_bits = self.options.get_attribute_int(i, "quantization_bits", -1);
                let decoder_type = select_sequential_encoder(att, quantization_bits) as u8;
                out_buffer.encode_u8(decoder_type);
                decoder_types.push(decoder_type);
            }
        }

        // Phase 3: Encode attribute values (all attributes first)
        // C++ order: all EncodePortableAttribute calls, then all EncodeDataNeededByPortableTransform calls

        // Store transforms and encoders for later use in transform data encoding
        let mut quantization_transforms: Vec<Option<AttributeQuantizationTransform>> = Vec::new();
        let mut portable_attributes: Vec<Option<PointAttribute>> = Vec::new();
        let mut normal_encoders: Vec<Option<SequentialNormalAttributeEncoder>> = Vec::new();

        // First pass: encode all attribute VALUES
        for i in 0..mesh.num_attributes() {
            let att = mesh.attribute(i);
            let decoder_type = decoder_types[i as usize];
            let quantization_bits = self.options.get_attribute_int(i, "quantization_bits", -1);

            match decoder_type {
                3 => {
                    // Normal attribute with octahedral encoding
                    let mut encoder = SequentialNormalAttributeEncoder::new();
                    if !encoder.init(
                        self.point_cloud().expect("point_cloud set"),
                        i,
                        &self.options,
                    ) {
                        return Err(DracoError::DracoError(
                            "Failed to init normal encoder".to_string(),
                        ));
                    }
                    if !encoder.encode_values(
                        self.point_cloud().expect("point_cloud set"),
                        &self.point_ids,
                        out_buffer,
                        &self.options,
                        self,
                    ) {
                        return Err(DracoError::DracoError(
                            "Failed to encode normal values".to_string(),
                        ));
                    }
                    normal_encoders.push(Some(encoder));
                    quantization_transforms.push(None);
                    portable_attributes.push(None);
                }
                2 => {
                    // Quantized attribute (mapping already applied at start of encode_attributes)
                    let mut q_transform = AttributeQuantizationTransform::new();
                    if !q_transform.compute_parameters(att, quantization_bits) {
                        return Err(DracoError::DracoError(
                            "Failed to compute quantization parameters".to_string(),
                        ));
                    }
                    let mut portable = PointAttribute::default();
                    if !q_transform.transform_attribute(att, &self.point_ids, &mut portable) {
                        return Err(DracoError::DracoError(
                            "Failed to quantize attribute".to_string(),
                        ));
                    }

                    let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                    att_encoder.init(i);
                    if !att_encoder.encode_values(
                        mesh as &PointCloud,
                        &self.point_ids,
                        out_buffer,
                        &self.options,
                        self,
                        Some(&portable),
                        true,
                    ) {
                        return Err(DracoError::DracoError(format!(
                            "Failed to encode attribute {}",
                            i
                        )));
                    }

                    quantization_transforms.push(Some(q_transform));
                    portable_attributes.push(Some(portable));
                    normal_encoders.push(None);
                }
                1 => {
                    // Integer attribute
                    let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                    att_encoder.init(i);
                    if !att_encoder.encode_values(
                        mesh as &PointCloud,
                        &self.point_ids,
                        out_buffer,
                        &self.options,
                        self,
                        None,
                        true,
                    ) {
                        return Err(DracoError::DracoError(format!(
                            "Failed to encode attribute {}",
                            i
                        )));
                    }
                    quantization_transforms.push(None);
                    portable_attributes.push(None);
                    normal_encoders.push(None);
                }
                0 => {
                    // Generic/float attribute
                    let mut att_encoder = SequentialAttributeEncoder::new();
                    att_encoder.init(i);
                    if !att_encoder.encode_values(mesh as &PointCloud, &self.point_ids, out_buffer)
                    {
                        return Err(DracoError::DracoError(format!(
                            "Failed to encode attribute {}",
                            i
                        )));
                    }
                    quantization_transforms.push(None);
                    portable_attributes.push(None);
                    normal_encoders.push(None);
                }
                _ => {
                    return Err(DracoError::DracoError(format!(
                        "Unsupported encoder type {}",
                        decoder_type
                    )));
                }
            }
        }

        // Second pass: encode all TRANSFORM DATA
        for i in 0..mesh.num_attributes() {
            let decoder_type = decoder_types[i as usize];

            match decoder_type {
                3 => {
                    // Normal attribute - encode octahedral transform data
                    let bitstream_version = crate::version::bitstream_version(major, minor);
                    if bitstream_version != 0 && bitstream_version < 0x0200 {
                        continue;
                    }
                    if let Some(ref encoder) = normal_encoders[i as usize] {
                        if !encoder.encode_data_needed_by_portable_transform(out_buffer) {
                            return Err(DracoError::DracoError(
                                "Failed to encode normal transform data".to_string(),
                            ));
                        }
                    }
                }
                2 => {
                    // Quantized attribute - encode quantization parameters
                    if let Some(ref q_transform) = quantization_transforms[i as usize] {
                        if !q_transform.encode_parameters(out_buffer) {
                            return Err(DracoError::DracoError(
                                "Failed to encode quantization parameters".to_string(),
                            ));
                        }
                    }
                }
                1 | 0 => {
                    // No transform data for integer/generic attributes
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn encode_edgebreaker_attributes_split(&mut self, out_buffer: &mut EncoderBuffer) -> Status {
        let mesh = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding");
        let mut groups: Vec<(i8, Vec<i32>)> = Vec::new();
        let mut position_attrs = Vec::new();
        for i in 0..mesh.num_attributes() {
            if mesh.attribute(i).attribute_type() == GeometryAttributeType::Position {
                position_attrs.push(i);
            }
        }
        if !position_attrs.is_empty() {
            groups.push((-1, position_attrs));
        }
        for (data_id, attr_conn) in self.edgebreaker_attribute_connectivity.iter().enumerate() {
            groups.push((data_id as i8, vec![attr_conn.attribute_id]));
        }

        // The group count is one byte in the bitstream, so a mesh needing more
        // groups than that cannot be described. Truncating wrote a stream that
        // decodes as a different mesh: 261 groups became 5, and the decoder
        // read the sixth group's bytes as attribute data. Measured at the
        // boundary rather than assumed - 256 groups is the first count that
        // breaks, and everything below it round-trips, including the counts
        // where the per-group `i8` data id goes negative.
        if groups.len() > u8::MAX as usize {
            return Err(DracoError::DracoError(format!(
                "Mesh needs {} attribute groups but the bitstream field holds {}",
                groups.len(),
                u8::MAX
            )));
        }
        out_buffer.encode_u8(groups.len() as u8);

        let major = out_buffer.version_major();
        let minor = out_buffer.version_minor();
        let writes_traversal_method = crate::version::bitstream_version(major, minor) >= 0x0102;
        // Prediction degree is the position group's traversal alone, and only at
        // speed 0. Every other group is walked depth first, whatever the speed --
        // upstream guards on the attribute being POSITION, and the groups here
        // carry att_data_id -1 for exactly that one. Declaring it for the rest
        // mislabels a stream whose values were written in depth-first order.
        let position_prediction_degree = self.options.get_speed() == 0
            && !(self.use_single_connectivity && mesh.num_attributes() > 1);
        for (att_data_id, _) in &groups {
            out_buffer.encode_u8(*att_data_id as u8);
            let element_type = if *att_data_id >= 0
                && !self.edgebreaker_attribute_connectivity[*att_data_id as usize].no_interior_seams
            {
                1 // MESH_CORNER_ATTRIBUTE
            } else {
                0 // MESH_VERTEX_ATTRIBUTE
            };
            out_buffer.encode_u8(element_type);
            if writes_traversal_method {
                let is_position_group = *att_data_id < 0;
                let traversal_method: u8 = if position_prediction_degree && is_position_group {
                    1
                } else {
                    0
                };
                out_buffer.encode_u8(traversal_method);
            }
        }

        let mut decoder_types_by_group: Vec<Vec<u8>> = Vec::with_capacity(groups.len());

        for (_, attr_ids) in &groups {
            if !uses_varint_encoding(major, minor) {
                out_buffer.encode_u32(attr_ids.len() as u32);
            } else {
                out_buffer.encode_varint(attr_ids.len() as u64);
            }

            for &att_id in attr_ids {
                let att = mesh.attribute(att_id);
                out_buffer.encode_u8(att.attribute_type() as u8);
                out_buffer.encode_u8(att.data_type() as u8);
                out_buffer.encode_u8(att.num_components());
                out_buffer.encode_u8(if att.normalized() { 1 } else { 0 });
                if !uses_varint_unique_id(major, minor) {
                    out_buffer.encode_u16(att.unique_id() as u16);
                } else {
                    out_buffer.encode_varint(att.unique_id() as u64);
                }
            }

            let mut decoder_types = Vec::with_capacity(attr_ids.len());
            for &att_id in attr_ids {
                let decoder_type = self.decoder_type_for_attribute(att_id);
                out_buffer.encode_u8(decoder_type);
                decoder_types.push(decoder_type);
            }
            decoder_types_by_group.push(decoder_types);
        }

        for (group_i, (att_data_id, attr_ids)) in groups.iter().enumerate() {
            let point_ids = if *att_data_id >= 0 {
                self.prepare_active_attribute_connectivity(*att_data_id as usize)?
            } else {
                self.active_corner_table = None;
                self.active_data_to_corner_map = None;
                self.active_vertex_to_data_map = None;
                self.point_ids.clone()
            };

            self.encode_attribute_group_values(
                attr_ids,
                &decoder_types_by_group[group_i],
                &point_ids,
                out_buffer,
            )?;
        }

        self.active_corner_table = None;
        self.active_data_to_corner_map = None;
        self.active_vertex_to_data_map = None;
        Ok(())
    }

    fn decoder_type_for_attribute(&self, att_id: i32) -> u8 {
        let mesh = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding");
        let att = mesh.attribute(att_id);
        let quantization_bits = self
            .options
            .get_attribute_int(att_id, "quantization_bits", -1);
        select_sequential_encoder(att, quantization_bits) as u8
    }

    fn prepare_active_attribute_connectivity(
        &mut self,
        data_id: usize,
    ) -> Result<Vec<PointIndex>, DracoError> {
        let mesh = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding");
        let base_ct = self
            .corner_table
            .as_ref()
            .ok_or_else(|| DracoError::DracoError("corner_table must be set".to_string()))?;
        let attr_conn = self
            .edgebreaker_attribute_connectivity
            .get(data_id)
            .ok_or_else(|| {
                DracoError::DracoError("Invalid attribute connectivity id".to_string())
            })?;

        if attr_conn.no_interior_seams {
            // Same corner table as the position, but not necessarily the same
            // walk over it: `attribute_traversal` is set when the position took
            // the max-prediction-degree order and this attribute must not.
            self.active_corner_table = None;
            if let Some((point_ids, data_to_corner_map, vertex_to_data_map)) =
                self.attribute_traversal.clone()
            {
                self.active_data_to_corner_map = Some(data_to_corner_map);
                self.active_vertex_to_data_map = Some(vertex_to_data_map);
                return Ok(point_ids);
            }
            self.active_data_to_corner_map = None;
            self.active_vertex_to_data_map = None;
            return Ok(self.point_ids.clone());
        }

        let mut attr_ct = base_ct.clone();
        for c_idx in 0..attr_conn.seam_edges.len() {
            if !attr_conn.seam_edges[c_idx] {
                continue;
            }
            let c = crate::geometry_indices::CornerIndex(c_idx as u32);
            let opp = attr_ct.opposite(c);
            if opp != crate::geometry_indices::INVALID_CORNER_INDEX {
                attr_ct.set_opposite(c, crate::geometry_indices::INVALID_CORNER_INDEX);
                attr_ct.set_opposite(opp, crate::geometry_indices::INVALID_CORNER_INDEX);
            }
        }
        let base_num_vertices = attr_ct.num_vertices();
        if !attr_ct.compute_vertex_corners(base_num_vertices) {
            return Err(DracoError::DracoError(
                "Failed to compute attribute seam corner table".to_string(),
            ));
        }

        // Walk the attribute's own table depth first, seeded by the edgebreaker
        // corner order, as upstream does with
        // `DepthFirstTraverser<MeshAttributeCornerTable>` and
        // `SetCornerOrder(processed_connectivity_corners_)`.
        //
        // Enumerating `vertex_corners` instead, as this used to, yields the
        // identity permutation of attribute-vertex indices -- `vertex_corners[v]`
        // has vertex `v` by construction -- which is not an encoding order at
        // all. The decoder walks the table it rebuilds from the seam bits, so
        // the values came back attached to the wrong points.
        let Some(encoder) = self.edgebreaker_encoder.as_ref() else {
            return Err(DracoError::DracoError(
                "Attribute seams need the edgebreaker corner order".to_string(),
            ));
        };
        let (point_ids, data_to_corner_map, vertex_to_data_map) =
            encoder.generate_depth_first_traversal(mesh, &attr_ct);

        self.active_corner_table = Some(attr_ct);
        self.active_data_to_corner_map = Some(data_to_corner_map);
        self.active_vertex_to_data_map = Some(vertex_to_data_map);
        Ok(point_ids)
    }

    fn encode_attribute_group_values(
        &mut self,
        attr_ids: &[i32],
        decoder_types: &[u8],
        point_ids: &[PointIndex],
        out_buffer: &mut EncoderBuffer,
    ) -> Status {
        // Three passes over the group, one per step of C++
        // SequentialAttributeEncodersController: transform every attribute to its
        // portable form, encode them all, then encode the data their transforms
        // need. Each pass is marked below.
        //
        // Pass one, TransformAttributesToPortableFormat. It has to finish before
        // any attribute is encoded: a prediction scheme that reads a parent needs
        // the parent's portable values, and in a single pass the parent would not
        // exist yet for anything encoded ahead of it.
        let mut quantization_transforms: Vec<Option<AttributeQuantizationTransform>> = Vec::new();
        {
            let mesh = self
                .mesh
                .as_ref()
                .expect("mesh must be set before encoding");
            let mut portables: Vec<(i32, PointAttribute)> = Vec::new();
            for (local_i, &att_id) in attr_ids.iter().enumerate() {
                if decoder_types[local_i] != 2 {
                    quantization_transforms.push(None);
                    continue;
                }
                let att = mesh.attribute(att_id);
                let is_parent_attribute = att.attribute_type() == GeometryAttributeType::Position
                    && self.options.get_speed() < 4;
                let quantization_bits =
                    self.options
                        .get_attribute_int(att_id, "quantization_bits", -1);
                let mut q_transform = AttributeQuantizationTransform::new();
                if !q_transform.compute_parameters(att, quantization_bits) {
                    return Err(DracoError::DracoError(
                        "Failed to compute quantization parameters".to_string(),
                    ));
                }
                let mut portable = PointAttribute::default();
                if !q_transform.transform_attribute(att, point_ids, &mut portable) {
                    return Err(DracoError::DracoError(
                        "Failed to quantize attribute".to_string(),
                    ));
                }

                // Rebuild the portable attribute's point map, as upstream does
                // in SequentialIntegerAttributeEncoder::TransformAttributeToPortableFormat.
                // The values were written in encoding order, but a prediction
                // scheme reads its parent as `mapped_index(point_id)`; without
                // this the lookup returns whichever vertex happens to sit at
                // that index in the traversal, and encoder and decoder predict
                // from different positions.
                //
                // Upstream guards this with `is_parent_encoder()`. The two
                // schemes that declare a parent -- tex coords portable and
                // geometric normal -- both name the position and are both
                // selected only below speed 4; every other scheme declares
                // none. So this is the same guard, decided up front rather than
                // by a flag set during scheme construction.
                if is_parent_attribute {
                    let num_points = mesh.num_points();
                    let mut value_to_value = vec![0u32; att.size().max(1)];
                    for (entry, &point_id) in point_ids.iter().enumerate() {
                        let src = att.mapped_index(point_id);
                        if (src.0 as usize) < value_to_value.len() {
                            value_to_value[src.0 as usize] = entry as u32;
                        }
                    }
                    portable.set_explicit_mapping(num_points);
                    for point in 0..num_points {
                        let src = att.mapped_index(PointIndex(point as u32));
                        let entry = value_to_value
                            .get(src.0 as usize)
                            .copied()
                            .unwrap_or_default();
                        portable.try_set_point_map_entry(
                            PointIndex(point as u32),
                            crate::geometry_indices::AttributeValueIndex(entry),
                        )?;
                    }
                }

                portables.push((att_id, portable));
                quantization_transforms.push(Some(q_transform));
            }
            // Accumulated across groups, not replaced: attributes are encoded one
            // group at a time and the position lives in its own, so replacing
            // here would take the position's portable values away from every
            // later group's predictors.
            for (att_id, portable) in portables {
                match self
                    .portable_attributes
                    .iter_mut()
                    .find(|(id, _)| *id == att_id)
                {
                    Some((_, existing)) => *existing = portable,
                    None => self.portable_attributes.push((att_id, portable)),
                }
            }
        }

        // Pass two, EncodePortableAttributes: the values themselves, in attribute
        // order.
        let mesh = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding");
        let mut normal_encoders: Vec<Option<SequentialNormalAttributeEncoder>> = Vec::new();

        for (local_i, &att_id) in attr_ids.iter().enumerate() {
            let att = mesh.attribute(att_id);
            let decoder_type = decoder_types[local_i];
            let _ = att;

            match decoder_type {
                3 => {
                    let mut encoder = SequentialNormalAttributeEncoder::new();
                    if !encoder.init(
                        self.point_cloud().expect("point_cloud set"),
                        att_id,
                        &self.options,
                    ) {
                        return Err(DracoError::DracoError(
                            "Failed to init normal encoder".to_string(),
                        ));
                    }
                    if !encoder.encode_values(
                        self.point_cloud().expect("point_cloud set"),
                        point_ids,
                        out_buffer,
                        &self.options,
                        self,
                    ) {
                        return Err(DracoError::DracoError(
                            "Failed to encode normal values".to_string(),
                        ));
                    }
                    normal_encoders.push(Some(encoder));
                }
                2 => {
                    let portable = self
                        .portable_attributes
                        .iter()
                        .find(|(id, _)| *id == att_id)
                        .map(|(_, att)| att)
                        .ok_or_else(|| {
                            DracoError::DracoError(format!(
                                "Missing portable attribute for {att_id}"
                            ))
                        })?;

                    let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                    att_encoder.init(att_id);
                    if !att_encoder.encode_values(
                        mesh as &PointCloud,
                        point_ids,
                        out_buffer,
                        &self.options,
                        self,
                        Some(portable),
                        true,
                    ) {
                        return Err(DracoError::DracoError(format!(
                            "Failed to encode attribute {}",
                            att_id
                        )));
                    }
                    normal_encoders.push(None);
                }
                1 => {
                    let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                    att_encoder.init(att_id);
                    if !att_encoder.encode_values(
                        mesh as &PointCloud,
                        point_ids,
                        out_buffer,
                        &self.options,
                        self,
                        None,
                        true,
                    ) {
                        return Err(DracoError::DracoError(format!(
                            "Failed to encode attribute {}",
                            att_id
                        )));
                    }
                    normal_encoders.push(None);
                }
                0 => {
                    let mut att_encoder = SequentialAttributeEncoder::new();
                    att_encoder.init(att_id);
                    if !att_encoder.encode_values(mesh as &PointCloud, point_ids, out_buffer) {
                        return Err(DracoError::DracoError(format!(
                            "Failed to encode attribute {}",
                            att_id
                        )));
                    }
                    normal_encoders.push(None);
                }
                _ => {
                    return Err(DracoError::DracoError(format!(
                        "Unsupported encoder type {}",
                        decoder_type
                    )));
                }
            }
        }

        // Pass three, EncodeDataNeededByPortableTransforms: the parameters a
        // decoder needs to undo each transform -- quantization ranges, and the
        // octahedron's bit count. Separate from pass two because upstream emits
        // every attribute's values first and only then every attribute's
        // transform data, so the two cannot be interleaved.
        for (local_i, &decoder_type) in decoder_types.iter().enumerate() {
            match decoder_type {
                3 => {
                    let major = out_buffer.version_major();
                    let minor = out_buffer.version_minor();
                    let bitstream_version = crate::version::bitstream_version(major, minor);
                    if bitstream_version != 0 && bitstream_version < 0x0200 {
                        continue;
                    }
                    if let Some(ref encoder) = normal_encoders[local_i] {
                        if !encoder.encode_data_needed_by_portable_transform(out_buffer) {
                            return Err(DracoError::DracoError(
                                "Failed to encode normal transform data".to_string(),
                            ));
                        }
                    }
                }
                2 => {
                    if let Some(ref q_transform) = quantization_transforms[local_i] {
                        if !q_transform.encode_parameters(out_buffer) {
                            return Err(DracoError::DracoError(
                                "Failed to encode quantization parameters".to_string(),
                            ));
                        }
                    }
                }
                1 | 0 => {}
                _ => {}
            }
        }

        Ok(())
    }

    fn compute_number_of_encoded_faces(&mut self) {
        if let Some(ref mesh) = self.mesh {
            self.num_encoded_faces = mesh.num_faces();
        }
    }

    fn build_encoded_mesh_info(&mut self) -> Status {
        let num_attributes = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding")
            .num_attributes();
        let mut attributes = Vec::with_capacity(num_attributes as usize);
        let mut encoded_num_points = self.point_ids.len();

        for att_id in 0..num_attributes {
            let point_ids = self.encoded_point_ids_for_attribute(att_id)?;
            let num_encoded_values = point_ids.len();
            encoded_num_points = encoded_num_points.max(num_encoded_values);

            let (position_min, position_max) =
                self.position_bounds_for_attribute(att_id, &point_ids)?;
            let att = self
                .mesh
                .as_ref()
                .expect("mesh must be set before encoding")
                .attribute(att_id);
            attributes.push(EncodedAttributeInfo {
                source_attribute_id: att_id,
                attribute_type: att.attribute_type(),
                data_type: att.data_type(),
                num_components: att.num_components(),
                normalized: att.normalized(),
                unique_id: att.unique_id(),
                num_encoded_values,
                position_min,
                position_max,
            });
        }

        let (source_num_points, num_faces) = self
            .mesh
            .as_ref()
            .map(|mesh| (mesh.num_points(), mesh.num_faces()))
            .expect("mesh must be set before encoding");
        if self.method == 0 {
            encoded_num_points = source_num_points;
        } else {
            encoded_num_points = self.encoded_num_points_for_mesh(encoded_num_points)?;
        }

        self.active_corner_table = None;
        self.active_data_to_corner_map = None;
        self.active_vertex_to_data_map = None;
        self.encoded_mesh_info = Some(EncodedMeshInfo {
            encoding_method: self.method,
            num_encoded_faces: num_faces,
            num_encoded_points: encoded_num_points,
            attributes,
        });
        Ok(())
    }

    fn encoded_point_ids_for_attribute(
        &mut self,
        att_id: i32,
    ) -> Result<Vec<PointIndex>, DracoError> {
        if self.method == 0 || self.use_single_connectivity {
            return Ok(self.point_ids.clone());
        }

        if let Some(data_id) = self
            .edgebreaker_attribute_connectivity
            .iter()
            .position(|connectivity| connectivity.attribute_id == att_id)
        {
            return self.prepare_active_attribute_connectivity(data_id);
        }

        Ok(self.point_ids.clone())
    }

    fn encoded_num_points_for_mesh(&mut self, base_num_points: usize) -> Result<usize, DracoError> {
        if self.method == 0 || self.use_single_connectivity {
            return Ok(base_num_points);
        }

        let mut num_points = base_num_points;
        for data_id in 0..self.edgebreaker_attribute_connectivity.len() {
            if self.edgebreaker_attribute_connectivity[data_id].no_interior_seams {
                continue;
            }
            let point_ids = self.prepare_active_attribute_connectivity(data_id)?;
            num_points = num_points.max(point_ids.len());
        }
        self.active_corner_table = None;
        self.active_data_to_corner_map = None;
        self.active_vertex_to_data_map = None;
        Ok(num_points)
    }

    fn position_bounds_for_attribute(
        &self,
        att_id: i32,
        point_ids: &[PointIndex],
    ) -> Result<PositionBounds, DracoError> {
        let mesh = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding");
        let att = mesh.attribute(att_id);
        if att.attribute_type() != GeometryAttributeType::Position {
            return Ok((None, None));
        }
        if att.num_components() != 3 || att.data_type() != DataType::Float32 {
            return Ok((None, None));
        }

        if self.decoder_type_for_attribute(att_id) == 2 {
            let quantization_bits = self
                .options
                .get_attribute_int(att_id, "quantization_bits", -1);
            let mut q_transform = AttributeQuantizationTransform::new();
            if !q_transform.compute_parameters(att, quantization_bits) {
                return Err(DracoError::DracoError(
                    "Failed to compute position quantization parameters".to_string(),
                ));
            }

            let mut portable = PointAttribute::default();
            if !q_transform.transform_attribute(att, point_ids, &mut portable) {
                return Err(DracoError::DracoError(
                    "Failed to quantize position attribute for encoded mesh info".to_string(),
                ));
            }

            let mut dequantized = PointAttribute::new();
            dequantized.try_init(
                GeometryAttributeType::Position,
                3,
                DataType::Float32,
                false,
                portable.size(),
            )?;
            if !q_transform.inverse_transform_attribute(&portable, &mut dequantized) {
                return Err(DracoError::DracoError(
                    "Failed to dequantize position attribute for encoded mesh info".to_string(),
                ));
            }

            return Self::position_bounds_from_attribute(&dequantized, &[]);
        }

        Self::position_bounds_from_attribute(att, point_ids)
    }

    fn position_bounds_from_attribute(
        att: &PointAttribute,
        point_ids: &[PointIndex],
    ) -> Result<PositionBounds, DracoError> {
        let count = if point_ids.is_empty() {
            att.size()
        } else {
            point_ids.len()
        };
        if count == 0 {
            return Ok((None, None));
        }

        let stride = usize::try_from(att.byte_stride()).map_err(|_| {
            DracoError::DracoError("Position attribute has invalid byte stride".to_string())
        })?;
        let bytes = att.buffer().data();
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];

        for i in 0..count {
            let point = if point_ids.is_empty() {
                PointIndex(i as u32)
            } else {
                point_ids[i]
            };
            let value_index = att.mapped_index(point);
            if value_index == INVALID_ATTRIBUTE_VALUE_INDEX {
                return Err(DracoError::DracoError(
                    "Position attribute point map contains an invalid entry".to_string(),
                ));
            }

            let value_offset = (value_index.0 as usize)
                .checked_mul(stride)
                .ok_or_else(|| {
                    DracoError::DracoError("Position attribute offset overflow".to_string())
                })?;
            for component in 0..3 {
                let offset = value_offset
                    .checked_add(component * DataType::Float32.byte_length())
                    .ok_or_else(|| {
                        DracoError::DracoError("Position attribute offset overflow".to_string())
                    })?;
                let end = offset
                    .checked_add(DataType::Float32.byte_length())
                    .ok_or_else(|| {
                        DracoError::DracoError("Position attribute offset overflow".to_string())
                    })?;
                let Some(component_bytes) = bytes.get(offset..end) else {
                    return Err(DracoError::DracoError(
                        "Position attribute buffer is shorter than metadata".to_string(),
                    ));
                };
                let value = f32::from_le_bytes([
                    component_bytes[0],
                    component_bytes[1],
                    component_bytes[2],
                    component_bytes[3],
                ]);
                min[component] = min[component].min(value);
                max[component] = max[component].max(value);
            }
        }

        Ok((
            Some(min.into_iter().map(f64::from).collect()),
            Some(max.into_iter().map(f64::from).collect()),
        ))
    }
}

impl Default for MeshEncoder {
    fn default() -> Self {
        Self::new()
    }
}
