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
use crate::mesh_edgebreaker_encoder::{
    select_edgebreaker_traversal, EdgebreakerAttributeConnectivity, EdgebreakerTraversal,
    MeshEdgebreakerEncoder,
};
use crate::metadata::METADATA_FLAG_MASK;
use crate::point_cloud::PointCloud;
use crate::point_cloud_encoder::GeometryEncoder;
use crate::prediction_scheme::{
    EntryToPointIdMap, PredictionSchemeMethod, PredictionSchemeTransformType,
};
use crate::sequential_attribute_encoder::{
    select_sequential_encoder, SequentialAttributeEncoder, SequentialAttributeEncoderType,
};
use crate::sequential_integer_attribute_encoder::SequentialIntegerAttributeEncoder;
use crate::sequential_normal_attribute_encoder::SequentialNormalAttributeEncoder;
use crate::status::{DracoError, Status};
use crate::version::{
    has_header_flags, uses_varint_encoding, uses_varint_unique_id, DEFAULT_MESH_VERSION,
};

/// Picks EdgeBreaker or sequential connectivity, as C++ `ExpertEncoder` does.
///
/// Shared by `encode_header` and by the version validation that runs before it,
/// so the version a stream is checked against is the one it is written with.
/// The two used to derive it separately, which is how a check can pass for a
/// coder the encoder then does not use.
fn select_mesh_encoding_method(options: &EncoderOptions) -> i32 {
    // C++ default: EdgeBreaker unless speed is 10, which asks for sequential.
    match options.get_global_int("encoding_method", -1) {
        -1 if options.get_speed() == 10 => 0,
        -1 => 1,
        1 => 1,
        _ => 0,
    }
}

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
/// [`encode`](MeshEncoder::encode) produces the bitstream and nothing else.
/// A caller who also wants per-attribute and per-face details of what the
/// encode did uses [`encode_with_info`](MeshEncoder::encode_with_info), which
/// derives them; they are not a byproduct of encoding and are not computed
/// for callers who do not ask.
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
    /// Whether we're using single connectivity (all attributes share same corner table).
    use_single_connectivity: bool,
    /// Prediction choices made by the attribute encoders, keyed by attribute
    /// id. Collected as encoding runs because the encoders are built at their
    /// use site and dropped there, and only they know what they settled on.
    attribute_predictions: Vec<(i32, PredictionSchemeMethod, PredictionSchemeTransformType)>,
    /// The quantization parameters each attribute was encoded with, keyed by
    /// attribute id. Kept for the same reason as `attribute_predictions`: the
    /// encoded-mesh-info pass runs after the attribute encoders are gone and
    /// would otherwise recompute these, and recomputing means a second full
    /// min/max sweep of the attribute -- a pass the reference never makes.
    attribute_quantization: Vec<(i32, AttributeQuantizationTransform)>,
}

/// Geometry shape, encoder choices and attribute metadata produced by a
/// successful mesh encode.
///
/// The encoder decides several things the caller does not state: the
/// connectivity coder, the EdgeBreaker traversal, whether attributes share one
/// connectivity, and a prediction scheme per attribute. Everything it resolved
/// is reported here, so "what did this encode actually do" is answerable
/// without re-deriving the selection rules or parsing the stream back.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EncodedMeshInfo {
    /// Numeric Draco mesh encoding method used for the output.
    pub encoding_method: i32,
    /// Bitstream version written, after the default was substituted for an
    /// unset one.
    pub bitstream_version: (u8, u8),
    /// EdgeBreaker traversal written, or `None` for sequential connectivity.
    pub traversal: Option<EdgebreakerTraversal>,
    /// Speed the choices above were made at, after `encoding_speed` and
    /// `decoding_speed` were resolved into one value.
    pub speed: i32,
    /// Whether every attribute shared the position's connectivity. When false,
    /// attributes with seams were encoded against their own corner tables.
    pub single_connectivity: bool,
    /// Number of faces encoded into the bitstream.
    pub num_encoded_faces: usize,
    /// Number of points encoded into the bitstream.
    pub num_encoded_points: usize,
    /// Per-attribute information captured during encoding.
    pub attributes: Vec<EncodedAttributeInfo>,
}

/// Attribute metadata produced by a successful mesh encode.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
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
    /// Per-attribute encoder the values went through, which is what decides
    /// whether the two fields below are populated.
    pub encoder_type: SequentialAttributeEncoderType,
    /// Quantization bits applied, or `None` when the attribute was not
    /// quantized. A `quantization_bits` option set on an integer or generic
    /// attribute is ignored by the encoder and reported as `None` here.
    pub quantization_bits: Option<i32>,
    /// Prediction scheme and transform the encoder settled on, or `None` when
    /// the attribute never reached the integer path. This is the resolved
    /// choice, not the request: several schemes fall back to `Difference` when
    /// the attribute or the mesh cannot support them.
    pub prediction: Option<(PredictionSchemeMethod, PredictionSchemeTransformType)>,
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

/// Rebuilds a portable attribute's point map, as upstream does in
/// `SequentialIntegerAttributeEncoder::TransformAttributeToPortableFormat`.
///
/// The values were written in encoding order, but a prediction scheme reads its
/// parent as `mapped_index(point_id)`; without this the lookup returns whichever
/// vertex happens to sit at that index in the traversal, and encoder and decoder
/// predict from different positions.
#[cfg(feature = "encoder")]
fn rebuild_parent_point_map(
    attribute: &PointAttribute,
    portable: &mut PointAttribute,
    point_ids: &[PointIndex],
    num_points: usize,
) -> Status {
    let mut value_to_value = vec![0u32; attribute.size().max(1)];
    for (entry, &point_id) in point_ids.iter().enumerate() {
        let src = attribute.mapped_index(point_id);
        if (src.0 as usize) < value_to_value.len() {
            value_to_value[src.0 as usize] = entry as u32;
        }
    }
    portable.set_explicit_mapping(num_points);
    for point in 0..num_points {
        let src = attribute.mapped_index(PointIndex(point as u32));
        let entry = value_to_value
            .get(src.0 as usize)
            .copied()
            .unwrap_or_default();
        portable.try_set_point_map_entry(
            PointIndex(point as u32),
            crate::geometry_indices::AttributeValueIndex(entry),
        )?;
    }
    Ok(())
}

/// Whether a prediction scheme that reads the position as its parent will be
/// used, so the position needs a portable form for the predictor to read.
///
/// Speed is upstream's own condition -- the tex-coords-portable and
/// geometric-normal schemes are the two that declare a parent, and
/// `SelectPredictionMethod` picks neither at speed 4 or above. An explicit
/// `prediction_scheme` option does not go through that selection, though, so
/// asking for one by number at any speed has to count as well: otherwise the
/// scheme is built and the parent it reads is the original attribute, whose
/// value order and point map are not what the decoder reconstructs.
#[cfg(feature = "encoder")]
fn position_is_a_prediction_parent(point_cloud: &PointCloud, options: &EncoderOptions) -> bool {
    if options.get_speed() < 4 {
        return true;
    }
    (0..point_cloud.num_attributes())
        .any(|att_id| matches!(options.get_attribute_prediction_scheme(att_id), 3 | 5 | 6))
}

/// The portable form of an already-integral attribute: its values converted to
/// `i32` in encoding order, which is the shape the decoder reconstructs.
///
/// Upstream builds one for *every* integer attribute --
/// `SequentialIntegerAttributeEncoder::PrepareValues` calls
/// `PreparePortableAttribute` before it looks at quantization at all -- and a
/// prediction scheme reaching for a parent gets that, never the original. This
/// port only built one where a quantization transform produced it, so a scheme
/// predicting from an integer position read the original instead: its own value
/// count and its own point map, both of which a deduplicated or seamed mesh
/// makes different from what the decoder will hold. Encoder and decoder then
/// predicted from different positions and disagreed about which entries carry
/// an orientation bit, which the decoder reports as running out of them.
#[cfg(feature = "encoder")]
fn integral_portable_attribute(
    attribute: &PointAttribute,
    point_ids: &[PointIndex],
) -> Result<PointAttribute, DracoError> {
    let num_components = attribute.num_components();
    let data_type = attribute.data_type();
    let byte_stride = attribute.byte_stride() as usize;
    let component_size = data_type.byte_length();

    let mut portable = PointAttribute::default();
    portable.try_init(
        attribute.attribute_type(),
        num_components,
        crate::draco_types::DataType::Int32,
        false,
        point_ids.len(),
    )?;

    for (entry, &point_id) in point_ids.iter().enumerate() {
        let src = attribute.mapped_index(point_id).0 as usize * byte_stride;
        for component in 0..num_components as usize {
            let value = crate::sequential_integer_attribute_encoder::read_value_as_i32(
                attribute.buffer(),
                src + component * component_size,
                data_type,
            );
            let offset = (entry * num_components as usize + component) * 4;
            portable.buffer_mut().write(offset, &value.to_le_bytes());
        }
    }
    Ok(portable)
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
            use_single_connectivity: false,
            attribute_predictions: Vec::new(),
            attribute_quantization: Vec::new(),
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
        self.use_single_connectivity = false;
        self.attribute_predictions.clear();
        self.attribute_quantization.clear();
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

    /// Encodes the assigned mesh into an output buffer.
    ///
    /// A mesh must have been provided with [`set_mesh`](MeshEncoder::set_mesh)
    /// first. On success the bitstream is appended to `out_buffer` and nothing
    /// else is computed; use
    /// [`encode_with_info`](MeshEncoder::encode_with_info) to also get a
    /// description of the encode.
    ///
    /// # Errors
    ///
    /// Returns an error if no mesh was set, if the requested encoding method or
    /// options are unsupported, or if attribute encoding fails.
    pub fn encode(&mut self, options: &EncoderOptions, out_buffer: &mut EncoderBuffer) -> Status {
        self.options = options.clone();
        self.reset_derived_state();

        if self.mesh.is_none() {
            return Err(DracoError::general("Mesh not set".to_string()));
        }
        crate::point_cloud_encoder::validate_encodable_attributes(self.mesh.as_ref().unwrap())?;
        let (major, minor) = self.options.get_version();
        let target = if select_mesh_encoding_method(&self.options) == 1 {
            crate::version::EncodeTarget::MeshEdgebreaker
        } else {
            crate::version::EncodeTarget::MeshSequential
        };
        crate::version::validate_encodable_version(major, minor, target)?;
        Self::validate_face_indices(self.mesh.as_ref().unwrap())?;
        self.validate_predictive_traversal()?;
        self.validate_prediction_schemes(self.mesh.as_ref().unwrap())?;
        self.validate_attribute_versions(self.mesh.as_ref().unwrap())?;

        // 1. Encode Header
        self.encode_header(out_buffer)?;
        self.encode_metadata(out_buffer)?;

        // 2. Encode geometry data (connectivity + attributes)
        self.encode_geometry_data(out_buffer)?;

        Ok(())
    }

    /// Encodes the assigned mesh and describes what the encode did.
    ///
    /// The description is derived from the encode rather than produced by it,
    /// and deriving it costs a sweep of every position for its bounds plus a
    /// copy of the encoded point order per attribute. So it is the caller who
    /// decides whether that work happens: [`encode`](MeshEncoder::encode) never
    /// does it, and this does it exactly once, here, where it was asked for.
    ///
    /// # Errors
    ///
    /// The same errors as [`encode`](MeshEncoder::encode), plus a failure to
    /// derive the description. The bitstream in `out_buffer` is complete and
    /// valid in that last case; only the description is missing.
    pub fn encode_with_info(
        &mut self,
        options: &EncoderOptions,
        out_buffer: &mut EncoderBuffer,
    ) -> Result<EncodedMeshInfo, DracoError> {
        self.encode(options, out_buffer)?;
        self.build_encoded_mesh_info()
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
                return Err(DracoError::general(format!(
                    "Prediction scheme {scheme} predicts texture coordinates and cannot be used \
                     for attribute {att_id}, which is a {attribute_type:?}"
                )));
            }
        }
        Ok(())
    }

    /// Whether attribute `att_id` already wrote its quantization parameters
    /// ahead of its values, so the trailing pass must not write them again.
    ///
    /// Asks the same function the attribute encoder asks, so the parameters are
    /// written exactly once whichever side of the 2.0 boundary the target is.
    fn quantization_parameters_are_inline(&self, att_id: i32) -> bool {
        #[cfg(feature = "legacy_bitstream_encode")]
        {
            let Some(mesh) = self.mesh.as_ref() else {
                return false;
            };
            crate::sequential_integer_attribute_encoder::uses_inline_quantization_parameters(
                mesh.attribute(att_id),
                &self.options,
                att_id,
            )
        }
        #[cfg(not(feature = "legacy_bitstream_encode"))]
        {
            let _ = att_id;
            false
        }
    }

    /// The per-attribute encoder this encode will build, which is what decides
    /// whether an attribute is subject to the version gates below.
    ///
    /// An attribute only meets a prediction scheme or a quantization transform
    /// if it reaches the integer path; a float attribute with no quantization
    /// goes to the generic encoder, where a requested prediction scheme is
    /// simply never consulted. Asking the same function the encoder asks keeps
    /// the two from disagreeing about which attributes a refusal covers.
    fn attribute_encoder_type(&self, mesh: &Mesh, att_id: i32) -> SequentialAttributeEncoderType {
        let quantization_bits = self
            .options
            .get_attribute_int(att_id, "quantization_bits", -1);
        select_sequential_encoder(mesh.attribute(att_id), quantization_bits)
    }

    /// Rejects attribute coding a pre-2.2 target has no layout for *in this
    /// build*.
    ///
    /// Every pre-2.2 attribute layout is written behind `legacy_bitstream_encode`
    /// -- the quantization parameters that go inline below 2.0, and the rANS
    /// size prefixes and mode bytes the prediction schemes carry below 2.2. With
    /// the feature on there is nothing to refuse. With it off those writes are
    /// compiled out, so an encode that reaches them silently produces a stream
    /// this crate's own decoder cannot read, and the refusal takes their place.
    ///
    /// EdgeBreaker is not covered here because `encode_header` already refuses
    /// every pre-2.2 EdgeBreaker target when the feature is off. What is left is
    /// the sequential mesh at 1.3, which has no such gate.
    #[cfg(not(feature = "legacy_bitstream_encode"))]
    fn validate_attribute_versions(&self, mesh: &Mesh) -> Status {
        let (mut major, mut minor) = self.options.get_version();
        if major == 0 && minor == 0 {
            (major, minor) = DEFAULT_MESH_VERSION;
        }
        if !crate::version::version_less_than(major, minor, (2, 2)) {
            return Ok(());
        }

        for att_id in 0..mesh.num_attributes() {
            // A generic attribute is copied out raw and meets neither a
            // transform nor a prediction scheme, so no legacy layout applies.
            if self.attribute_encoder_type(mesh, att_id) == SequentialAttributeEncoderType::Generic
            {
                continue;
            }
            return Err(DracoError::unsupported_version(format!(
                "Attribute {att_id} needs the pre-2.2 layout for bitstream version \
                 {major}.{minor}, which requires the legacy_bitstream_encode feature"
            )));
        }
        Ok(())
    }

    /// With the legacy writer compiled in, every claimed version has a layout,
    /// so there is nothing to refuse.
    #[cfg(feature = "legacy_bitstream_encode")]
    fn validate_attribute_versions(&self, _mesh: &Mesh) -> Status {
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
            return Err(DracoError::unsupported_feature(format!(
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
                    return Err(DracoError::general(format!(
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
            return Err(DracoError::unsupported_version(
                "Metadata requires Draco bitstream version 1.3 or newer".to_string(),
            ));
        }

        let method = select_mesh_encoding_method(&self.options);

        #[cfg(not(feature = "legacy_bitstream_encode"))]
        if method == 1 {
            let bitstream_version = crate::version::bitstream_version(major, minor);
            if bitstream_version < 0x0202 {
                return Err(DracoError::unsupported_version(
                    "EdgeBreaker mesh encoding before bitstream 2.2 requires the \
                     legacy_bitstream_encode feature"
                        .to_string(),
                ));
            }
            if self.options.get_global_int("force_predictive_traversal", 0) != 0 {
                return Err(DracoError::unsupported_feature(
                    "force_predictive_traversal requires the legacy_bitstream_encode feature"
                        .to_string(),
                ));
            }
        }
        #[cfg(not(feature = "legacy_bitstream_encode"))]
        match self.options.get_prediction_scheme() {
            2 | 3 => {
                return Err(DracoError::unsupported_feature(
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
        buffer.encode_u8(method as u8);

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
            let faces = if use_single_connectivity {
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
                faces
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
                return Err(DracoError::general(
                    "All triangles are degenerate.".to_string(),
                ));
            }

            self.corner_table = Some(corner_table);
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
            // Sequential encoding: no corner table needed.
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

    /// Creates the faces array using the position attribute to deduplicate
    /// vertices, mimicking C++ CreateCornerTableFromPositionAttribute: each
    /// face carries the attribute value indices its points map to.
    fn create_corner_table_from_position_attribute(
        &self,
        mesh: &Mesh,
    ) -> Vec<[crate::geometry_indices::VertexIndex; 3]> {
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
            return faces;
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
        faces
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
        // 2.2, not 2.0. `uses_varint_encoding` ignores its `minor` argument and
        // flips at the major, while the decoder (and upstream
        // `MeshSequentialDecoder`) reads these as varints only from 2.2. A
        // sequential mesh written at 2.0 or 2.1 was therefore unreadable. Those
        // versions are no longer claimed for sequential meshes, so this is
        // upstream parity rather than a live fix - but a predicate that ignores
        // half its input is a trap, and this is the second bug it caused.
        let counts_are_varint = crate::version::version_at_least(major, minor, (2, 2));
        if !counts_are_varint {
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
            } else if counts_are_varint && mesh.num_points() < (1 << 21) {
                // Varint indices when the points fit in 21 bits, as upstream
                // does - but only from 2.2, which is where the decoder starts
                // reading them that way. This branch had no version gate at
                // all, so every sequential mesh below 2.2 with 65536 or more
                // points was written unreadable; 1.3 is a claimed version, so
                // this one is a live fix, not just parity.
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
        // Collected here rather than written straight to `self`, which is
        // borrowed as the `GeometryEncoder` the attribute encoders predict
        // against for as long as this loop runs.
        let mut predictions = Vec::new();

        // First pass: encode all attribute VALUES
        for i in 0..mesh.num_attributes() {
            let att = mesh.attribute(i);
            let decoder_type = decoder_types[i as usize];
            let quantization_bits = self.options.get_attribute_int(i, "quantization_bits", -1);

            match decoder_type {
                3 => {
                    // Normal attribute with octahedral encoding
                    let mut encoder = SequentialNormalAttributeEncoder::new();
                    encoder
                        .init(
                            self.point_cloud().expect("point_cloud set"),
                            i,
                            &self.options,
                        )
                        .map_err(|e| {
                            DracoError::general(format!("Failed to init normal encoder: {e}"))
                        })?;
                    encoder.encode_values(
                        self.point_cloud().expect("point_cloud set"),
                        &self.point_ids,
                        out_buffer,
                        &self.options,
                        self,
                    )?;
                    if let Some((method, transform)) = encoder.selected_prediction() {
                        predictions.push((i, method, transform));
                    }
                    normal_encoders.push(Some(encoder));
                    quantization_transforms.push(None);
                    portable_attributes.push(None);
                }
                2 => {
                    // Quantized attribute (mapping already applied at start of encode_attributes)
                    let mut q_transform = AttributeQuantizationTransform::new();
                    q_transform
                        .compute_parameters(att, quantization_bits)
                        .map_err(|e| {
                            DracoError::general(format!(
                                "Failed to compute quantization parameters: {e}"
                            ))
                        })?;
                    let mut portable = PointAttribute::default();
                    q_transform
                        .transform_attribute(
                            att,
                            EntryToPointIdMap::from_point_indices(&self.point_ids),
                            &mut portable,
                        )
                        .map_err(|e| {
                            DracoError::general(format!("Failed to quantize attribute: {e}"))
                        })?;

                    let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                    att_encoder.init(i);
                    att_encoder.encode_values(
                        mesh as &PointCloud,
                        &self.point_ids,
                        out_buffer,
                        &self.options,
                        self,
                        Some(&portable),
                        true,
                    )?;
                    if let Some((method, transform)) = att_encoder.selected_prediction() {
                        predictions.push((i, method, transform));
                    }

                    self.attribute_quantization.push((i, q_transform.clone()));
                    quantization_transforms.push(Some(q_transform));
                    portable_attributes.push(Some(portable));
                    normal_encoders.push(None);
                }
                1 => {
                    // Integer attribute
                    let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                    att_encoder.init(i);
                    att_encoder.encode_values(
                        mesh as &PointCloud,
                        &self.point_ids,
                        out_buffer,
                        &self.options,
                        self,
                        None,
                        true,
                    )?;
                    if let Some((method, transform)) = att_encoder.selected_prediction() {
                        predictions.push((i, method, transform));
                    }
                    quantization_transforms.push(None);
                    portable_attributes.push(None);
                    normal_encoders.push(None);
                }
                0 => {
                    // Generic/float attribute
                    let mut att_encoder = SequentialAttributeEncoder::new();
                    att_encoder.init(i);
                    att_encoder.encode_values(mesh as &PointCloud, &self.point_ids, out_buffer)?;
                    quantization_transforms.push(None);
                    portable_attributes.push(None);
                    normal_encoders.push(None);
                }
                _ => {
                    return Err(DracoError::general(format!(
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
                        encoder
                            .encode_data_needed_by_portable_transform(out_buffer)
                            .map_err(|err| {
                                DracoError::general(format!(
                                    "Failed to encode normal transform data: {err}"
                                ))
                            })?;
                    }
                }
                2 => {
                    // Quantized attribute - encode quantization parameters,
                    // unless the target version already carried them inline
                    // ahead of the values.
                    if self.quantization_parameters_are_inline(i) {
                        continue;
                    }
                    if let Some(ref q_transform) = quantization_transforms[i as usize] {
                        q_transform.encode_parameters(out_buffer).map_err(|e| {
                            DracoError::general(format!(
                                "Failed to encode quantization parameters: {e}"
                            ))
                        })?;
                    }
                }
                1 | 0 => {
                    // No transform data for integer/generic attributes
                }
                _ => {}
            }
        }

        self.attribute_predictions.extend(predictions);
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
            return Err(DracoError::general(format!(
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
            .ok_or_else(|| DracoError::general("corner_table must be set".to_string()))?;
        let attr_conn = self
            .edgebreaker_attribute_connectivity
            .get(data_id)
            .ok_or_else(|| DracoError::general("Invalid attribute connectivity id".to_string()))?;

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
            return Err(DracoError::general(
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
            return Err(DracoError::general(
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
            // Collected rather than written straight to `self`, which is
            // borrowed as the mesh for as long as this loop runs.
            let mut quantized: Vec<(i32, AttributeQuantizationTransform)> = Vec::new();
            for (local_i, &att_id) in attr_ids.iter().enumerate() {
                let att = mesh.attribute(att_id);
                let is_parent_attribute = att.attribute_type() == GeometryAttributeType::Position
                    && position_is_a_prediction_parent(mesh, &self.options);
                if decoder_types[local_i] != 2 {
                    // An already-integral parent still needs a portable form,
                    // for the reason `integral_portable_attribute` gives: the
                    // predictor must read what the decoder will reconstruct,
                    // not the original the mesh was handed. Only a parent, as
                    // upstream only rebuilds the map under `is_parent_encoder`.
                    if is_parent_attribute && decoder_types[local_i] == 1 {
                        let mut portable = integral_portable_attribute(att, point_ids)?;
                        // The same rebuild the quantized arm below does, and for
                        // the same reason: the values are in encoding order and
                        // a predictor reads its parent as
                        // `mapped_index(point_id)`. Without it the map stays the
                        // identity, the encoder reads the entry sitting at the
                        // point's own index, and the decoder -- whose parent
                        // carries the rebuilt map -- reads a different one.
                        rebuild_parent_point_map(att, &mut portable, point_ids, mesh.num_points())?;
                        portables.push((att_id, portable));
                    }
                    quantization_transforms.push(None);
                    continue;
                }
                let quantization_bits =
                    self.options
                        .get_attribute_int(att_id, "quantization_bits", -1);
                let mut q_transform = AttributeQuantizationTransform::new();
                q_transform
                    .compute_parameters(att, quantization_bits)
                    .map_err(|e| {
                        DracoError::general(format!(
                            "Failed to compute quantization parameters: {e}"
                        ))
                    })?;
                let mut portable = PointAttribute::default();
                q_transform
                    .transform_attribute(
                        att,
                        EntryToPointIdMap::from_point_indices(point_ids),
                        &mut portable,
                    )
                    .map_err(|e| {
                        DracoError::general(format!("Failed to quantize attribute: {e}"))
                    })?;

                // Only a parent needs the rebuilt map, which is the guard
                // upstream spells `is_parent_encoder()`. What declares a parent
                // is `position_is_a_prediction_parent` above -- not the speed
                // alone, which is what this used to say.
                if is_parent_attribute {
                    rebuild_parent_point_map(att, &mut portable, point_ids, mesh.num_points())?;
                }

                portables.push((att_id, portable));
                quantized.push((att_id, q_transform.clone()));
                quantization_transforms.push(Some(q_transform));
            }
            // Accumulated across groups, not replaced: attributes are encoded one
            // group at a time and the position lives in its own, so replacing
            // here would take the position's portable values away from every
            // later group's predictors.
            self.attribute_quantization.extend(quantized);
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
        // See the sibling collection in `encode_attributes`: `self` is the
        // `GeometryEncoder` the attribute encoders borrow for the whole loop.
        let mut predictions = Vec::new();

        for (local_i, &att_id) in attr_ids.iter().enumerate() {
            let att = mesh.attribute(att_id);
            let decoder_type = decoder_types[local_i];
            let _ = att;

            match decoder_type {
                3 => {
                    let mut encoder = SequentialNormalAttributeEncoder::new();
                    encoder
                        .init(
                            self.point_cloud().expect("point_cloud set"),
                            att_id,
                            &self.options,
                        )
                        .map_err(|e| {
                            DracoError::general(format!("Failed to init normal encoder: {e}"))
                        })?;
                    encoder.encode_values(
                        self.point_cloud().expect("point_cloud set"),
                        point_ids,
                        out_buffer,
                        &self.options,
                        self,
                    )?;
                    if let Some((method, transform)) = encoder.selected_prediction() {
                        predictions.push((att_id, method, transform));
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
                            DracoError::general(format!("Missing portable attribute for {att_id}"))
                        })?;

                    let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                    att_encoder.init(att_id);
                    att_encoder.encode_values(
                        mesh as &PointCloud,
                        point_ids,
                        out_buffer,
                        &self.options,
                        self,
                        Some(portable),
                        true,
                    )?;
                    if let Some((method, transform)) = att_encoder.selected_prediction() {
                        predictions.push((att_id, method, transform));
                    }
                    normal_encoders.push(None);
                }
                1 => {
                    let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                    att_encoder.init(att_id);
                    att_encoder.encode_values(
                        mesh as &PointCloud,
                        point_ids,
                        out_buffer,
                        &self.options,
                        self,
                        None,
                        true,
                    )?;
                    if let Some((method, transform)) = att_encoder.selected_prediction() {
                        predictions.push((att_id, method, transform));
                    }
                    normal_encoders.push(None);
                }
                0 => {
                    let mut att_encoder = SequentialAttributeEncoder::new();
                    att_encoder.init(att_id);
                    att_encoder.encode_values(mesh as &PointCloud, point_ids, out_buffer)?;
                    normal_encoders.push(None);
                }
                _ => {
                    return Err(DracoError::general(format!(
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
                        encoder
                            .encode_data_needed_by_portable_transform(out_buffer)
                            .map_err(|err| {
                                DracoError::general(format!(
                                    "Failed to encode normal transform data: {err}"
                                ))
                            })?;
                    }
                }
                2 => {
                    if self.quantization_parameters_are_inline(attr_ids[local_i]) {
                        continue;
                    }
                    if let Some(ref q_transform) = quantization_transforms[local_i] {
                        q_transform.encode_parameters(out_buffer).map_err(|e| {
                            DracoError::general(format!(
                                "Failed to encode quantization parameters: {e}"
                            ))
                        })?;
                    }
                }
                1 | 0 => {}
                _ => {}
            }
        }

        self.attribute_predictions.extend(predictions);
        Ok(())
    }

    fn compute_number_of_encoded_faces(&mut self) {
        if let Some(ref mesh) = self.mesh {
            self.num_encoded_faces = mesh.num_faces();
        }
    }

    fn build_encoded_mesh_info(&mut self) -> Result<EncodedMeshInfo, DracoError> {
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
            let mesh = self
                .mesh
                .as_ref()
                .expect("mesh must be set before encoding");
            let encoder_type = self.attribute_encoder_type(mesh, att_id);
            let quantization_bits = match encoder_type {
                SequentialAttributeEncoderType::Quantization
                | SequentialAttributeEncoderType::Normals => Some(self.options.get_attribute_int(
                    att_id,
                    "quantization_bits",
                    -1,
                )),
                // A `quantization_bits` option on an integer or generic
                // attribute never reaches a transform, so reporting it would
                // describe the request rather than the encode.
                SequentialAttributeEncoderType::Integer
                | SequentialAttributeEncoderType::Generic => None,
            };
            let prediction = self
                .attribute_predictions
                .iter()
                .find(|(id, _, _)| *id == att_id)
                .map(|&(_, method, transform)| (method, transform));
            let att = mesh.attribute(att_id);
            attributes.push(EncodedAttributeInfo {
                source_attribute_id: att_id,
                attribute_type: att.attribute_type(),
                data_type: att.data_type(),
                num_components: att.num_components(),
                normalized: att.normalized(),
                unique_id: att.unique_id(),
                num_encoded_values,
                encoder_type,
                quantization_bits,
                prediction,
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

        let (mut major, mut minor) = self.options.get_version();
        if major == 0 && minor == 0 {
            (major, minor) = DEFAULT_MESH_VERSION;
        }
        let traversal = (self.method == 1).then(|| {
            select_edgebreaker_traversal(
                self.options.get_speed() as usize,
                num_faces,
                self.options.get_global_int("force_predictive_traversal", 0) == 1,
            )
        });
        Ok(EncodedMeshInfo {
            encoding_method: self.method,
            bitstream_version: (major, minor),
            traversal,
            speed: self.options.get_speed(),
            single_connectivity: self.use_single_connectivity,
            num_encoded_faces: num_faces,
            num_encoded_points: encoded_num_points,
            attributes,
        })
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
            // The encode has already computed these for this attribute, and
            // computing them again means sweeping every value for its minimum
            // a second time. Recompute only if the attribute was quantized by
            // some path that did not record it, so this reports the same
            // bounds either way.
            let recorded = self
                .attribute_quantization
                .iter()
                .find(|(id, _)| *id == att_id)
                .map(|(_, transform)| transform.clone());
            let q_transform = match recorded {
                Some(transform) => transform,
                None => {
                    let mut transform = AttributeQuantizationTransform::new();
                    transform
                        .compute_parameters(att, quantization_bits)
                        .map_err(|e| {
                            DracoError::general(format!(
                                "Failed to compute position quantization parameters: {e}"
                            ))
                        })?;
                    transform
                }
            };

            // These are the bounds of the attribute as the decoder will see it,
            // so each extreme goes through the same quantize/dequantize round
            // trip the encoded values do. The round trip is monotonic per
            // component, so the extremes of the round-tripped values are the
            // round-tripped extremes -- folding the original and transforming
            // six scalars gives the same answer as building the portable and
            // dequantized attributes to fold the result, without two full
            // passes over every point and the two attributes they allocate.
            // `quantization_round_trip_monotonic_test` pins that property.
            let (min, max) = Self::position_bounds_from_attribute(att, point_ids)?;
            let (Some(min), Some(max)) = (min, max) else {
                return Ok((None, None));
            };
            let round_trip = |bound: Vec<f64>| -> Result<Vec<f64>, DracoError> {
                bound
                    .into_iter()
                    .enumerate()
                    .map(|(component, value)| {
                        q_transform
                            .round_trip_component(component, value as f32)
                            .map(f64::from)
                            .map_err(|e| {
                                DracoError::general(format!(
                                    "Failed to quantize position bounds for encoded mesh info: {e}"
                                ))
                            })
                    })
                    .collect()
            };
            return Ok((Some(round_trip(min)?), Some(round_trip(max)?)));
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
            DracoError::general("Position attribute has invalid byte stride".to_string())
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
                return Err(DracoError::general(
                    "Position attribute point map contains an invalid entry".to_string(),
                ));
            }

            // A point's three components are twelve contiguous bytes, so one
            // slice of a fixed size answers what three offset computations and
            // three bounds checks answered per point before.
            const POSITION_BYTES: usize = 3 * 4;
            let value_offset = (value_index.0 as usize)
                .checked_mul(stride)
                .ok_or_else(|| {
                    DracoError::general("Position attribute offset overflow".to_string())
                })?;
            let end = value_offset.checked_add(POSITION_BYTES).ok_or_else(|| {
                DracoError::general("Position attribute offset overflow".to_string())
            })?;
            let Some(point_bytes) = bytes.get(value_offset..end) else {
                return Err(DracoError::general(
                    "Position attribute buffer is shorter than metadata".to_string(),
                ));
            };
            for component in 0..3 {
                let at = component * 4;
                let value = f32::from_le_bytes([
                    point_bytes[at],
                    point_bytes[at + 1],
                    point_bytes[at + 2],
                    point_bytes[at + 3],
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
