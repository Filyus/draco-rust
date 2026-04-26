use crate::attribute_quantization_transform::AttributeQuantizationTransform;
use crate::attribute_transform::AttributeTransform;
use crate::compression_config::EncodedGeometryType;
use crate::compression_config::MeshEncodingMethod;
use crate::corner_table::CornerTable;
use crate::draco_types::DataType;
use crate::encoder_buffer::EncoderBuffer;
use crate::encoder_options::EncoderOptions;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::{FaceIndex, PointIndex};
use crate::mesh::Mesh;
use crate::mesh_edgebreaker_encoder::{EdgebreakerAttributeConnectivity, MeshEdgebreakerEncoder};
use crate::point_cloud::PointCloud;
use crate::point_cloud_encoder::GeometryEncoder;
use crate::sequential_attribute_encoder::SequentialAttributeEncoder;
use crate::sequential_integer_attribute_encoder::SequentialIntegerAttributeEncoder;
use crate::sequential_normal_attribute_encoder::SequentialNormalAttributeEncoder;
use crate::status::{DracoError, Status};
use crate::version::{
    has_header_flags, uses_varint_encoding, uses_varint_unique_id, DEFAULT_MESH_VERSION,
};

/// MeshEncoder provides basic functionality for encoding mesh data.
/// This is an abstract base that can be specialized for different mesh encoding methods.
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
    method: i32,
    /// Maps point indices to vertex indices in the corner table.
    /// Used when position-based deduplication is enabled.
    point_to_vertex_map: Option<Vec<u32>>,
    /// Whether we're using single connectivity (all attributes share same corner table).
    use_single_connectivity: bool,
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
}

impl MeshEncoder {
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
            method: 0,
            point_to_vertex_map: None,
            use_single_connectivity: false,
        }
    }

    pub fn set_mesh(&mut self, mesh: Mesh) {
        self.mesh = Some(mesh);
    }

    pub fn mesh(&self) -> Option<&Mesh> {
        self.mesh.as_ref()
    }

    pub fn num_encoded_faces(&self) -> usize {
        self.num_encoded_faces
    }

    pub fn corner_table(&self) -> Option<&CornerTable> {
        self.corner_table.as_ref()
    }

    pub fn encode(&mut self, options: &EncoderOptions, out_buffer: &mut EncoderBuffer) -> Status {
        self.options = options.clone();

        if self.mesh.is_none() {
            return Err(DracoError::DracoError("Mesh not set".to_string()));
        }

        // 1. Encode Header
        self.encode_header(out_buffer)?;

        // 2. Encode geometry data (connectivity + attributes)
        self.encode_geometry_data(out_buffer)?;

        Ok(())
    }

    #[allow(dead_code)]
    fn encode_metadata(&self, buffer: &mut EncoderBuffer) -> Status {
        buffer.encode_varint(0u64); // 0 metadata
        Ok(())
    }

    fn encode_header(&self, buffer: &mut EncoderBuffer) -> Status {
        buffer.encode_data(b"DRACO");

        let (mut major, mut minor) = self.options.get_version();
        if major == 0 && minor == 0 {
            // Default to latest mesh version
            (major, minor) = DEFAULT_MESH_VERSION;
        }

        buffer.encode_u8(major);
        buffer.encode_u8(minor);
        buffer.set_version(major, minor);
        buffer.encode_u8(self.get_geometry_type() as u8);

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
        buffer.encode_u8(method);

        if has_header_flags(major, minor) {
            buffer.encode_u16(0); // Flags
        }
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
        let (point_ids, data_to_corner_map, vertex_to_data_map) = encoder.encode_connectivity(
            mesh,
            corner_table,
            &self.edgebreaker_attribute_connectivity,
            out_buffer,
            self.options.get_speed() as usize,
        )?;
        #[cfg(feature = "debug_logs")]
        {
            println!("DEBUG: encode_edgebreaker_connectivity: point_ids.len()={}, data_to_corner_map.len()={}, vertex_to_data_map.len()={}",
                 point_ids.len(), data_to_corner_map.len(), vertex_to_data_map.len());
        }
        self.point_ids = point_ids;

        // Draco stores corner mapping in attribute (data) order.
        self.data_to_corner_map = Some(data_to_corner_map);
        self.vertex_to_data_map = Some(vertex_to_data_map);

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
            eprintln!(
                "Rust created faces (first 12): {:?}",
                faces
                    .iter()
                    .take(12)
                    .map(|f| [f[0].0, f[1].0, f[2].0])
                    .collect::<Vec<_>>()
            );
            eprintln!(
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

        out_buffer.encode_u8(num_encoders as u8);

        // Phase 1: attributes decoder identifiers.
        // For single-encoder mode: one encoder with att_data_id = -1 (uses position connectivity)
        if num_encoders > 0 && is_edgebreaker {
            // att_data_id (i8), encoder_type (u8), traversal_method (u8)
            // -1 means use position connectivity (single connectivity mode)
            out_buffer.encode_u8((-1i8) as u8); // att_data_id = -1
            out_buffer.encode_u8(0); // element_type = MESH_VERTEX_ATTRIBUTE

            // Traversal method: PREDICTION_DEGREE (1) for speed 0, DEPTH_FIRST (0) otherwise
            // This must match the traversal used in MeshEdgebreakerEncoder
            let encoding_speed = self.options.get_speed();
            let traversal_method: u8 = if encoding_speed == 0 { 1 } else { 0 };
            out_buffer.encode_u8(traversal_method);
        }
        // For sequential, nothing is written in phase 1 (EncodeAttributesEncoderIdentifier does nothing)

        let mut decoder_types: Vec<u8> = Vec::with_capacity(mesh.num_attributes() as usize);
        // Use the buffer's version (set in encode_header) for version checks
        let major = out_buffer.version_major();
        let minor = out_buffer.version_minor();

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
                    println!("DEBUG: Encoder encoding attribute {} metadata. Type: {:?}, Components: {}, Data: {:?}", i, att.attribute_type(), att.num_components(), att.data_type());
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
                let is_quantized = quantization_bits > 0
                    && (att.data_type() == DataType::Float32
                        || att.data_type() == DataType::Float64);
                let is_normal = att.attribute_type() == GeometryAttributeType::Normal;

                let decoder_type: u8 = if is_quantized {
                    if is_normal {
                        3
                    } else {
                        2
                    }
                } else if att.data_type() != DataType::Float32 {
                    1
                } else {
                    0
                };
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

        out_buffer.encode_u8(groups.len() as u8);

        let traversal_method: u8 = if self.options.get_speed() == 0 { 1 } else { 0 };
        for (att_data_id, _) in &groups {
            out_buffer.encode_u8(*att_data_id as u8);
            out_buffer.encode_u8(0);
            out_buffer.encode_u8(traversal_method);
        }

        let major = out_buffer.version_major();
        let minor = out_buffer.version_minor();
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
        let is_quantized = quantization_bits > 0
            && (att.data_type() == DataType::Float32 || att.data_type() == DataType::Float64);
        let is_normal = att.attribute_type() == GeometryAttributeType::Normal;

        if is_quantized {
            if is_normal {
                3
            } else {
                2
            }
        } else if att.data_type() != DataType::Float32 {
            1
        } else {
            0
        }
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
            self.active_corner_table = None;
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

        let mut point_ids = Vec::with_capacity(attr_ct.vertex_corners.len());
        let mut data_to_corner_map = Vec::with_capacity(attr_ct.vertex_corners.len());
        let mut vertex_to_data_map = vec![-1i32; attr_ct.num_vertices()];
        for (data_id, &corner) in attr_ct.vertex_corners.iter().enumerate() {
            if corner == crate::geometry_indices::INVALID_CORNER_INDEX {
                point_ids.push(PointIndex(0));
                data_to_corner_map.push(crate::geometry_indices::INVALID_CORNER_INDEX.0);
                continue;
            }
            let face = mesh.face(FaceIndex(corner.0 / 3));
            let point_id = face[(corner.0 % 3) as usize];
            point_ids.push(point_id);
            data_to_corner_map.push(corner.0);
            let vertex = attr_ct.vertex(corner);
            if vertex != crate::geometry_indices::INVALID_VERTEX_INDEX
                && (vertex.0 as usize) < vertex_to_data_map.len()
            {
                vertex_to_data_map[vertex.0 as usize] = data_id as i32;
            }
        }

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
        let mesh = self
            .mesh
            .as_ref()
            .expect("mesh must be set before encoding");
        let mut quantization_transforms: Vec<Option<AttributeQuantizationTransform>> = Vec::new();
        let mut normal_encoders: Vec<Option<SequentialNormalAttributeEncoder>> = Vec::new();

        for (local_i, &att_id) in attr_ids.iter().enumerate() {
            let att = mesh.attribute(att_id);
            let decoder_type = decoder_types[local_i];
            let quantization_bits = self
                .options
                .get_attribute_int(att_id, "quantization_bits", -1);

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
                    quantization_transforms.push(None);
                }
                2 => {
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

                    let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                    att_encoder.init(att_id);
                    if !att_encoder.encode_values(
                        mesh as &PointCloud,
                        point_ids,
                        out_buffer,
                        &self.options,
                        self,
                        Some(&portable),
                        true,
                    ) {
                        return Err(DracoError::DracoError(format!(
                            "Failed to encode attribute {}",
                            att_id
                        )));
                    }
                    quantization_transforms.push(Some(q_transform));
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
                    quantization_transforms.push(None);
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
                    quantization_transforms.push(None);
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

        for (local_i, &decoder_type) in decoder_types.iter().enumerate() {
            match decoder_type {
                3 => {
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
}

impl Default for MeshEncoder {
    fn default() -> Self {
        Self::new()
    }
}
