use crate::corner_table::CornerTable;
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
use crate::geometry_attribute::PointAttribute;
use crate::geometry_indices::{CornerIndex, PointIndex, INVALID_CORNER_INDEX};
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::point_cloud::PointCloud;
use crate::point_cloud_decoder::PointCloudDecoder;
use crate::prediction_scheme::{
    PredictionScheme, PredictionSchemeDecoder, PredictionSchemeMethod,
    PredictionSchemeTransformType,
};
use crate::prediction_scheme_constrained_multi_parallelogram::MeshPredictionSchemeConstrainedMultiParallelogramDecoder;
use crate::prediction_scheme_delta::PredictionSchemeDeltaDecoder;
use crate::prediction_scheme_geometric_normal::MeshPredictionSchemeGeometricNormalDecoder;
#[cfg(feature = "legacy_bitstream_decode")]
use crate::prediction_scheme_multi_parallelogram::MeshPredictionSchemeMultiParallelogramDecoder;
use crate::prediction_scheme_normal_octahedron_canonicalized_decoding_transform::PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform;
use crate::prediction_scheme_parallelogram::MeshPredictionSchemeParallelogramDecoder;
#[cfg(feature = "legacy_bitstream_decode")]
use crate::prediction_scheme_tex_coords_deprecated::MeshPredictionSchemeTexCoordsDeprecatedDecoder;
use crate::prediction_scheme_tex_coords_portable::MeshPredictionSchemeTexCoordsPortableDecoder;
use crate::prediction_scheme_wrap::PredictionSchemeWrapDecodingTransform;
use crate::symbol_encoding::{decode_symbols, SymbolEncodingOptions};

pub struct SequentialIntegerAttributeDecoder {
    attribute: i32,
    prediction_scheme: Option<Box<dyn PredictionSchemeDecoder<'static, i32, i32>>>,
}

fn build_vertex_to_data_map_from_data_to_corner_map(
    corner_table: &CornerTable,
    data_to_corner_map: &[u32],
    vertex_to_data_map: &mut Vec<i32>,
) -> bool {
    vertex_to_data_map.resize(corner_table.num_vertices(), -1);
    for (data_id, &corner_u32) in data_to_corner_map.iter().enumerate() {
        let corner_id = CornerIndex(corner_u32);
        if corner_id == INVALID_CORNER_INDEX {
            continue;
        }
        if corner_id.0 as usize >= corner_table.num_corners() {
            return false;
        }
        let v = corner_table.vertex(corner_id).0 as usize;
        let Some(slot) = vertex_to_data_map.get_mut(v) else {
            return false;
        };
        *slot = data_id as i32;
    }
    true
}

impl Default for SequentialIntegerAttributeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialIntegerAttributeDecoder {
    pub fn new() -> Self {
        Self {
            attribute: -1,
            prediction_scheme: None,
        }
    }

    pub fn init(&mut self, _decoder: &PointCloudDecoder, attribute_id: i32) -> bool {
        self.attribute = attribute_id;
        true
    }

    pub fn attribute_id(&self) -> i32 {
        self.attribute
    }

    pub fn set_prediction_scheme(
        &mut self,
        scheme: Box<dyn PredictionSchemeDecoder<'static, i32, i32>>,
    ) {
        self.prediction_scheme = Some(scheme);
    }

    // Complex mesh decoding requires all 8 parameters: mesh data, traversal maps,
    // corner table for prediction, and optional portable attribute output.
    // Refactoring into a struct would obscure the data flow and break C++ API parity.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_values(
        &mut self,
        point_cloud: &mut PointCloud,
        point_ids: &[PointIndex],
        in_buffer: &mut DecoderBuffer,
        corner_table: Option<&CornerTable>,
        data_to_corner_map_override: Option<&[u32]>,
        vertex_to_data_map_override: Option<&[i32]>,
        portable_attribute: Option<&mut PointAttribute>,
        portable_parent_attribute: Option<&PointAttribute>,
        pre_integer_decode: Option<&mut dyn FnMut(&mut DecoderBuffer<'_>) -> bool>,
    ) -> bool {
        let att_id = self.attribute;
        if att_id < 0 {
            return false;
        }

        let num_points = point_ids.len();
        if num_points == 0 {
            return true;
        }

        let attribute = if let Some(ref pa) = portable_attribute {
            &**pa
        } else {
            let Ok(attribute) = point_cloud.try_attribute(att_id) else {
                return false;
            };
            attribute
        };

        let num_components = attribute.num_components() as usize;
        let num_values = num_points * num_components;

        // 3. Decode Prediction Method and (optional) prepare predictor
        let method_byte = match in_buffer.decode_u8() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("Failed to decode prediction method");
                return false;
            }
        };

        // Draco stores prediction method as int8 (0xFE == -2 == None).
        // Accept 0xFF as None as well for older Rust-produced streams that used
        // the wrong sentinel before this decoder matched the C++ enum exactly.
        let selected_method = if method_byte == 0xFF {
            PredictionSchemeMethod::None
        } else if method_byte == 0xFE {
            PredictionSchemeMethod::None
        } else {
            match PredictionSchemeMethod::try_from(method_byte) {
                Ok(m) => m,
                Err(_) => {
                    return false;
                }
            }
        };

        let mut selected_transform: Option<PredictionSchemeTransformType> = None;
        if selected_method != PredictionSchemeMethod::None {
            // Draco stores prediction transform type as int8 (0xFF == -1 == None).
            let transform_byte = match in_buffer.decode_u8() {
                Ok(v) => v,
                Err(_) => return false,
            };
            if transform_byte != 0xFF {
                match PredictionSchemeTransformType::try_from(transform_byte) {
                    Ok(t) => selected_transform = Some(t),
                    Err(_) => {
                        return false;
                    }
                }
            }
        }

        if let Some(ref scheme) = self.prediction_scheme {
            // println!("DEBUG: Decoder scheme method: {:?}", scheme.get_prediction_method());
            if scheme.get_prediction_method() != selected_method {
                eprintln!(
                    "Prediction method mismatch. Stream: {:?}, Scheme: {:?}",
                    selected_method,
                    scheme.get_prediction_method()
                );
                return false;
            }
        }

        let mut predictor_opt: Option<
            PredictionSchemeDeltaDecoder<i32, i32, PredictionSchemeWrapDecodingTransform<i32>>,
        > = None;
        let mut predictor_normal_octa_diff_opt: Option<
            PredictionSchemeDeltaDecoder<
                i32,
                i32,
                PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform,
            >,
        > = None;
        let mut predictor_parallelogram_opt: Option<
            MeshPredictionSchemeParallelogramDecoder<
                i32,
                i32,
                PredictionSchemeWrapDecodingTransform<i32>,
            >,
        > = None;
        #[cfg(feature = "legacy_bitstream_decode")]
        let mut predictor_multi_parallelogram_opt: Option<
            MeshPredictionSchemeMultiParallelogramDecoder<
                '_,
                i32,
                i32,
                PredictionSchemeWrapDecodingTransform<i32>,
            >,
        > = None;
        let mut predictor_constrained_multi_parallelogram_opt: Option<
            MeshPredictionSchemeConstrainedMultiParallelogramDecoder<
                '_,
                i32,
                i32,
                PredictionSchemeWrapDecodingTransform<i32>,
            >,
        > = None;
        #[cfg(feature = "legacy_bitstream_decode")]
        let mut predictor_tex_coords_deprecated_opt: Option<
            MeshPredictionSchemeTexCoordsDeprecatedDecoder<
                '_,
                PredictionSchemeWrapDecodingTransform<i32>,
            >,
        > = None;
        let mut predictor_tex_coords_opt: Option<MeshPredictionSchemeTexCoordsPortableDecoder> =
            None;
        let mut predictor_geometric_normal_opt: Option<MeshPredictionSchemeGeometricNormalDecoder> =
            None;

        // Maps need to live long enough
        let mut vertex_to_data_map: Vec<i32> = Vec::new();
        let mut data_to_corner_map: Vec<u32> = Vec::new();
        match selected_method {
            _ if self.prediction_scheme.is_some() => {
                // Do nothing, scheme already set
            }
            PredictionSchemeMethod::Difference => match selected_transform {
                Some(PredictionSchemeTransformType::NormalOctahedronCanonicalized) => {
                    let transform =
                        PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform::new();
                    let predictor = PredictionSchemeDeltaDecoder::new(transform);
                    predictor_normal_octa_diff_opt = Some(predictor);
                }
                _ => {
                    let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
                    let predictor = PredictionSchemeDeltaDecoder::new(transform);
                    predictor_opt = Some(predictor);
                }
            },
            PredictionSchemeMethod::MeshPredictionParallelogram => {
                if let Some(corner_table) = corner_table {
                    // Generate maps
                    data_to_corner_map.resize(num_points, 0);

                    // vertex_to_data_map_override takes priority when available
                    // (it's built by the decoder's own DFS traversal)
                    if let Some(map) = vertex_to_data_map_override {
                        // Use the pre-built vertex_to_data_map from mesh decoder
                        if map.len() != corner_table.num_vertices() {
                            eprintln!("Invalid vertex_to_data_map_override length");
                            return false;
                        }
                        vertex_to_data_map.resize(map.len(), 0);
                        vertex_to_data_map.copy_from_slice(map);

                        // Also set data_to_corner_map if override is available
                        if let Some(dcm) = data_to_corner_map_override {
                            if dcm.len() != num_points {
                                eprintln!("Invalid data_to_corner_map_override length");
                                return false;
                            }
                            data_to_corner_map.copy_from_slice(dcm);
                        }
                    } else if let Some(map) = data_to_corner_map_override {
                        if map.len() != num_points {
                            eprintln!("Invalid data_to_corner_map_override length");
                            return false;
                        }
                        data_to_corner_map.copy_from_slice(map);

                        // When using an override, the corner table may contain seam-split
                        // vertices with ids outside the original point range. Build the
                        // vertex->data map from the data->corner map.
                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    } else {
                        // Build vertex_to_data_map from data_to_corner_map using corner table vertex IDs
                        // This is the same logic as the 'if' branch above
                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    }

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, &data_to_corner_map, &vertex_to_data_map);

                    let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
                    let predictor = MeshPredictionSchemeParallelogramDecoder::new(
                        attribute, transform, mesh_data,
                    );
                    predictor_parallelogram_opt = Some(predictor);
                } else {
                    eprintln!("Parallelogram prediction requires corner table");
                    return false;
                }
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                if let Some(corner_table) = corner_table {
                    data_to_corner_map.resize(num_points, 0);

                    if let Some(map) = vertex_to_data_map_override {
                        if map.len() != corner_table.num_vertices() {
                            eprintln!("Invalid vertex_to_data_map_override length");
                            return false;
                        }
                        vertex_to_data_map.resize(map.len(), 0);
                        vertex_to_data_map.copy_from_slice(map);

                        if let Some(dcm) = data_to_corner_map_override {
                            if dcm.len() != num_points {
                                eprintln!("Invalid data_to_corner_map_override length");
                                return false;
                            }
                            data_to_corner_map.copy_from_slice(dcm);
                        }
                    } else if let Some(map) = data_to_corner_map_override {
                        if map.len() != num_points {
                            eprintln!("Invalid data_to_corner_map_override length");
                            return false;
                        }
                        data_to_corner_map.copy_from_slice(map);

                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    } else {
                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    }

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, &data_to_corner_map, &vertex_to_data_map);

                    let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
                    let predictor =
                        MeshPredictionSchemeMultiParallelogramDecoder::new(transform, mesh_data);
                    predictor_multi_parallelogram_opt = Some(predictor);
                } else {
                    eprintln!("MultiParallelogram prediction requires corner table");
                    return false;
                }
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                eprintln!("MultiParallelogram prediction is disabled");
                return false;
            }
            PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => {
                if let Some(corner_table) = corner_table {
                    // Generate maps
                    data_to_corner_map.resize(num_points, 0);

                    // vertex_to_data_map_override takes priority when available
                    // (it's built by the decoder's own DFS traversal)
                    if let Some(map) = vertex_to_data_map_override {
                        // Use the pre-built vertex_to_data_map from mesh decoder
                        if map.len() != corner_table.num_vertices() {
                            eprintln!("Invalid vertex_to_data_map_override length");
                            return false;
                        }
                        vertex_to_data_map.resize(map.len(), 0);
                        vertex_to_data_map.copy_from_slice(map);

                        // Also set data_to_corner_map if override is available
                        if let Some(dcm) = data_to_corner_map_override {
                            if dcm.len() != num_points {
                                eprintln!("Invalid data_to_corner_map_override length");
                                return false;
                            }
                            data_to_corner_map.copy_from_slice(dcm);
                        }
                    } else if let Some(map) = data_to_corner_map_override {
                        if map.len() != num_points {
                            eprintln!("Invalid data_to_corner_map_override length");
                            return false;
                        }
                        data_to_corner_map.copy_from_slice(map);

                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    } else {
                        // Build vertex_to_data_map from data_to_corner_map using corner table vertex IDs
                        // This is the same logic as the 'if' branch above
                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    }

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, &data_to_corner_map, &vertex_to_data_map);

                    let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
                    let predictor = MeshPredictionSchemeConstrainedMultiParallelogramDecoder::new(
                        transform, mesh_data,
                    );
                    predictor_constrained_multi_parallelogram_opt = Some(predictor);
                } else {
                    eprintln!("ConstrainedMultiParallelogram prediction requires corner table");
                    return false;
                }
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                if let Some(corner_table) = corner_table {
                    data_to_corner_map.resize(num_points, 0);

                    if let Some(map) = vertex_to_data_map_override {
                        if map.len() != corner_table.num_vertices() {
                            eprintln!("Invalid vertex_to_data_map_override length");
                            return false;
                        }
                        vertex_to_data_map.resize(map.len(), 0);
                        vertex_to_data_map.copy_from_slice(map);

                        if let Some(dcm) = data_to_corner_map_override {
                            if dcm.len() != num_points {
                                eprintln!("Invalid data_to_corner_map_override length");
                                return false;
                            }
                            data_to_corner_map.copy_from_slice(dcm);
                        }
                    } else if let Some(map) = data_to_corner_map_override {
                        if map.len() != num_points {
                            eprintln!("Invalid data_to_corner_map_override length");
                            return false;
                        }
                        data_to_corner_map.copy_from_slice(map);

                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    } else {
                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    }

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, &data_to_corner_map, &vertex_to_data_map);

                    let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
                    let mut predictor =
                        MeshPredictionSchemeTexCoordsDeprecatedDecoder::new(transform);
                    predictor.init(&mesh_data);

                    let pos_att_id = point_cloud.named_attribute_id(
                        crate::geometry_attribute::GeometryAttributeType::Position,
                    );
                    if pos_att_id >= 0 {
                        let pos_att = if let Some(attribute) = portable_parent_attribute {
                            attribute
                        } else {
                            let Ok(attribute) = point_cloud.try_attribute(pos_att_id) else {
                                return false;
                            };
                            attribute
                        };
                        if !predictor.set_parent_attribute(pos_att) {
                            eprintln!("Failed to set parent attribute for TexCoordsDeprecated");
                            return false;
                        }
                    } else {
                        eprintln!("Position attribute not found for TexCoordsDeprecated");
                        return false;
                    }

                    predictor_tex_coords_deprecated_opt = Some(predictor);
                } else {
                    eprintln!("TexCoordsDeprecated prediction requires corner table");
                    return false;
                }
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                eprintln!("TexCoordsDeprecated prediction is disabled");
                return false;
            }
            PredictionSchemeMethod::MeshPredictionTexCoordsPortable => {
                if let Some(corner_table) = corner_table {
                    data_to_corner_map.resize(num_points, 0);

                    // vertex_to_data_map_override takes priority when available
                    // (it's built by the decoder's own DFS traversal)
                    if let Some(map) = vertex_to_data_map_override {
                        // Use the pre-built vertex_to_data_map from mesh decoder
                        if map.len() != corner_table.num_vertices() {
                            eprintln!("Invalid vertex_to_data_map_override length");
                            return false;
                        }
                        vertex_to_data_map.resize(map.len(), 0);
                        vertex_to_data_map.copy_from_slice(map);

                        // Also set data_to_corner_map if override is available
                        if let Some(dcm) = data_to_corner_map_override {
                            if dcm.len() != num_points {
                                eprintln!("Invalid data_to_corner_map_override length");
                                return false;
                            }
                            data_to_corner_map.copy_from_slice(dcm);
                        }
                    } else if let Some(map) = data_to_corner_map_override {
                        if map.len() != num_points {
                            eprintln!("Invalid data_to_corner_map_override length");
                            return false;
                        }
                        data_to_corner_map.copy_from_slice(map);

                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    } else {
                        // Build vertex_to_data_map from data_to_corner_map using corner table vertex IDs
                        // This is the same logic as the 'if' branch above
                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    }

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, &data_to_corner_map, &vertex_to_data_map);

                    let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
                    let mut predictor =
                        MeshPredictionSchemeTexCoordsPortableDecoder::new(transform);
                    predictor.init(&mesh_data);

                    // Set parent attribute (Position)
                    let pos_att_id = point_cloud.named_attribute_id(
                        crate::geometry_attribute::GeometryAttributeType::Position,
                    );
                    if pos_att_id >= 0 {
                        let pos_att = if let Some(attribute) = portable_parent_attribute {
                            attribute
                        } else {
                            let Ok(attribute) = point_cloud.try_attribute(pos_att_id) else {
                                return false;
                            };
                            attribute
                        };
                        if !predictor.set_parent_attribute(pos_att) {
                            eprintln!("Failed to set parent attribute for TexCoordsPortable");
                            return false;
                        }
                    } else {
                        eprintln!("Position attribute not found for TexCoordsPortable");
                        return false;
                    }

                    predictor_tex_coords_opt = Some(predictor);
                } else {
                    eprintln!("TexCoordsPortable prediction requires corner table");
                    return false;
                }
            }
            PredictionSchemeMethod::MeshPredictionGeometricNormal => {
                if let Some(corner_table) = corner_table {
                    data_to_corner_map.resize(num_points, 0);

                    // vertex_to_data_map_override takes priority when available
                    // (it's built by the decoder's own DFS traversal)
                    if let Some(map) = vertex_to_data_map_override {
                        // Use the pre-built vertex_to_data_map from mesh decoder
                        if map.len() != corner_table.num_vertices() {
                            eprintln!("Invalid vertex_to_data_map_override length");
                            return false;
                        }
                        vertex_to_data_map.resize(map.len(), 0);
                        vertex_to_data_map.copy_from_slice(map);

                        // Also set data_to_corner_map if override is available
                        if let Some(dcm) = data_to_corner_map_override {
                            if dcm.len() != num_points {
                                eprintln!("Invalid data_to_corner_map_override length");
                                return false;
                            }
                            data_to_corner_map.copy_from_slice(dcm);
                        }
                    } else if let Some(map) = data_to_corner_map_override {
                        if map.len() != num_points {
                            eprintln!("Invalid data_to_corner_map_override length");
                            return false;
                        }
                        data_to_corner_map.copy_from_slice(map);

                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    } else {
                        // Build vertex_to_data_map from data_to_corner_map using corner table vertex IDs
                        // This is the same logic as the 'if' branch above
                        if !build_vertex_to_data_map_from_data_to_corner_map(
                            corner_table,
                            &data_to_corner_map,
                            &mut vertex_to_data_map,
                        ) {
                            eprintln!("Invalid data_to_corner_map corner id");
                            return false;
                        }
                    }

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, &data_to_corner_map, &vertex_to_data_map);

                    let transform =
                        PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform::new();
                    let mut predictor = MeshPredictionSchemeGeometricNormalDecoder::new(transform);
                    predictor.init(&mesh_data);

                    // Provide mapping from decoded-entry index to original point id.
                    predictor.set_entry_to_point_id_map(
                        crate::prediction_scheme::EntryToPointIdMap::from_point_indices(point_ids),
                    );

                    // Set parent attribute (Position)
                    let pos_att_id = point_cloud.named_attribute_id(
                        crate::geometry_attribute::GeometryAttributeType::Position,
                    );
                    if pos_att_id >= 0 {
                        let pos_att = if let Some(attribute) = portable_parent_attribute {
                            attribute
                        } else {
                            let Ok(attribute) = point_cloud.try_attribute(pos_att_id) else {
                                return false;
                            };
                            attribute
                        };
                        if !predictor.set_parent_attribute(pos_att) {
                            eprintln!("Failed to set parent attribute for GeometricNormal");
                            return false;
                        }
                    } else {
                        eprintln!("Position attribute not found for GeometricNormal");
                        return false;
                    }

                    predictor_geometric_normal_opt = Some(predictor);
                } else {
                    eprintln!("GeometricNormal prediction requires corner table");
                    return false;
                }
            }
            PredictionSchemeMethod::None => {}
            _ => {
                eprintln!("Unsupported prediction method: {:?}", selected_method);
                return false;
            }
        }

        // 1. Decode correction symbols.
        // For v < 2.0, transform-specific parameters (quantization, octahedron)
        // are stored BEFORE the integer values. The caller provides a hook.
        if let Some(hook) = pre_integer_decode {
            if !hook(in_buffer) {
                return false;
            }
        }
        // Draco supports both entropy-coded symbols (compressed=1) and raw symbols (compressed=0).
        let compressed = match in_buffer.decode_u8() {
            Ok(v) => v,
            Err(_) => return false,
        };

        // Check if the prediction scheme produces positive corrections (no ZigZag needed)
        // Octahedron transforms (for normals) produce positive corrections
        let are_corrections_positive = match selected_transform {
            Some(PredictionSchemeTransformType::NormalOctahedron)
            | Some(PredictionSchemeTransformType::NormalOctahedronCanonicalized) => true,
            _ => {
                // Fallback: check self.prediction_scheme if it's set
                if let Some(ref scheme) = self.prediction_scheme {
                    scheme.are_corrections_positive()
                } else {
                    false
                }
            }
        };

        let needs_zigzag_conversion = !are_corrections_positive;
        let corrections: Vec<i32> = if compressed > 0 {
            // Entropy-coded symbols are zigzag encoded UNLESS the prediction scheme
            // guarantees positive corrections (e.g., normal octahedron transform)
            let mut symbols = vec![0u32; num_values];
            let options = SymbolEncodingOptions::default();
            if !decode_symbols(
                num_values,
                num_components,
                &options,
                in_buffer,
                &mut symbols,
            ) {
                return false;
            }
            symbols_to_corrections(symbols, needs_zigzag_conversion)
        } else {
            // Raw uncompressed integers. Read directly as bytes.
            // ZigZag conversion is needed unless the scheme guarantees positive corrections.
            let num_bytes = match in_buffer.decode_u8() {
                Ok(v) => v as usize,
                Err(_) => return false,
            };
            if num_bytes > 4 {
                return false;
            }

            let mut raw_corrections = Vec::with_capacity(num_values);
            if num_bytes == 0 {
                // All values are zero — nothing to read from the buffer.
                raw_corrections.resize(num_values, 0);
            } else if num_bytes == 4 {
                let Some(byte_len) = num_values.checked_mul(4) else {
                    return false;
                };
                let bytes = match in_buffer.decode_slice(byte_len) {
                    Ok(bytes) => bytes,
                    Err(_) => return false,
                };
                for chunk in bytes.chunks_exact(4) {
                    let symbol = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    raw_corrections.push(symbol_to_correction(symbol, needs_zigzag_conversion));
                }
            } else {
                for _ in 0..num_values {
                    let mut tmp = [0u8; 4];
                    if in_buffer.decode_bytes(&mut tmp[..num_bytes]).is_err() {
                        return false;
                    }
                    let symbol = u32::from_le_bytes(tmp);
                    raw_corrections.push(symbol_to_correction(symbol, needs_zigzag_conversion));
                }
            }
            raw_corrections
        };

        // Initialize values array only when a prediction scheme needs to write
        // reconstructed values. With no prediction, corrections already are
        // the decoded values and can be stored directly.
        let mut values = if selected_method == PredictionSchemeMethod::None {
            Vec::new()
        } else {
            vec![0i32; num_values]
        };

        // 3. Decode prediction scheme data (if any).
        match selected_method {
            _ if self.prediction_scheme.is_some() => {
                let Some(scheme) = self.prediction_scheme.as_mut() else {
                    eprintln!("Prediction scheme was selected but not initialized");
                    return false;
                };
                if !scheme.decode_prediction_data(in_buffer) {
                    eprintln!(
                        "Failed to decode prediction data (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            PredictionSchemeMethod::Difference => {
                if let Some(predictor) = predictor_normal_octa_diff_opt.as_mut() {
                    if !predictor.decode_prediction_data(in_buffer) {
                        eprintln!(
                            "Failed to decode prediction data (att_id={}, method={:?}, transform={:?})",
                            att_id, selected_method, selected_transform
                        );
                        return false;
                    }
                } else {
                    let Some(predictor) = predictor_opt.as_mut() else {
                        eprintln!("Difference predictor was selected but not initialized");
                        return false;
                    };
                    if !predictor.decode_prediction_data(in_buffer) {
                        eprintln!(
                            "Failed to decode prediction data (att_id={}, method={:?}, transform={:?})",
                            att_id, selected_method, selected_transform
                        );
                        return false;
                    }
                }
            }
            PredictionSchemeMethod::MeshPredictionParallelogram => {
                let Some(predictor) = predictor_parallelogram_opt.as_mut() else {
                    eprintln!("Parallelogram predictor was selected but not initialized");
                    return false;
                };
                if !predictor.decode_prediction_data(in_buffer) {
                    eprintln!(
                        "Failed to decode prediction data (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                let Some(predictor) = predictor_multi_parallelogram_opt.as_mut() else {
                    eprintln!("MultiParallelogram predictor was selected but not initialized");
                    return false;
                };
                if !predictor.decode_prediction_data(in_buffer) {
                    eprintln!(
                        "Failed to decode prediction data (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                eprintln!("MultiParallelogram prediction is disabled");
                return false;
            }
            PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => {
                let Some(predictor) = predictor_constrained_multi_parallelogram_opt.as_mut() else {
                    eprintln!(
                        "ConstrainedMultiParallelogram predictor was selected but not initialized"
                    );
                    return false;
                };
                if !predictor.decode_prediction_data(in_buffer) {
                    eprintln!(
                        "Failed to decode prediction data (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                let Some(predictor) = predictor_tex_coords_deprecated_opt.as_mut() else {
                    eprintln!("TexCoordsDeprecated predictor was selected but not initialized");
                    return false;
                };
                if !predictor.decode_prediction_data(in_buffer) {
                    eprintln!(
                        "Failed to decode prediction data (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                eprintln!("TexCoordsDeprecated prediction is disabled");
                return false;
            }
            PredictionSchemeMethod::MeshPredictionTexCoordsPortable => {
                let Some(predictor) = predictor_tex_coords_opt.as_mut() else {
                    eprintln!("TexCoordsPortable predictor was selected but not initialized");
                    return false;
                };
                if !predictor.decode_prediction_data(in_buffer) {
                    eprintln!(
                        "Failed to decode prediction data (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            PredictionSchemeMethod::MeshPredictionGeometricNormal => {
                let Some(predictor) = predictor_geometric_normal_opt.as_mut() else {
                    eprintln!("GeometricNormal predictor was selected but not initialized");
                    return false;
                };
                if !predictor.decode_prediction_data(in_buffer) {
                    eprintln!(
                        "Failed to decode prediction data (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            PredictionSchemeMethod::None => {}
            _ => {
                return false;
            }
        }

        // 4. Apply Inverse Prediction.
        match selected_method {
            _ if self.prediction_scheme.is_some() => {
                let Some(scheme) = self.prediction_scheme.as_mut() else {
                    eprintln!("Prediction scheme was selected but not initialized");
                    return false;
                };
                let map_opt = match selected_method {
                    PredictionSchemeMethod::MeshPredictionParallelogram
                    | PredictionSchemeMethod::MeshPredictionMultiParallelogram
                    | PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram
                    | PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated
                    | PredictionSchemeMethod::MeshPredictionTexCoordsPortable
                    | PredictionSchemeMethod::MeshPredictionGeometricNormal => Some(
                        crate::prediction_scheme::EntryToPointIdMap::from_point_indices(point_ids),
                    ),
                    _ => None,
                };
                if !scheme.compute_original_values(
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    map_opt,
                ) {
                    eprintln!(
                        "Failed to compute original values (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            PredictionSchemeMethod::Difference => {
                if let Some(predictor) = predictor_normal_octa_diff_opt.as_mut() {
                    if !predictor.compute_original_values(
                        &corrections,
                        &mut values,
                        num_values,
                        num_components,
                        None,
                    ) {
                        eprintln!(
                            "Failed to compute original values (att_id={}, method={:?}, transform={:?})",
                            att_id, selected_method, selected_transform
                        );
                        return false;
                    }
                } else {
                    let Some(predictor) = predictor_opt.as_mut() else {
                        eprintln!("Difference predictor was selected but not initialized");
                        return false;
                    };
                    if !predictor.compute_original_values(
                        &corrections,
                        &mut values,
                        num_values,
                        num_components,
                        None,
                    ) {
                        eprintln!(
                            "Failed to compute original values (att_id={}, method={:?}, transform={:?})",
                            att_id, selected_method, selected_transform
                        );
                        return false;
                    }
                }
            }
            PredictionSchemeMethod::MeshPredictionParallelogram => {
                let Some(predictor) = predictor_parallelogram_opt.as_mut() else {
                    eprintln!("Parallelogram predictor was selected but not initialized");
                    return false;
                };
                if !predictor.compute_original_values(
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    None,
                ) {
                    eprintln!(
                        "Failed to compute original values (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                let Some(predictor) = predictor_multi_parallelogram_opt.as_mut() else {
                    eprintln!("MultiParallelogram predictor was selected but not initialized");
                    return false;
                };
                if !predictor.compute_original_values(
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    None,
                ) {
                    eprintln!(
                        "Failed to compute original values (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                eprintln!("MultiParallelogram prediction is disabled");
                return false;
            }
            PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => {
                let Some(predictor) = predictor_constrained_multi_parallelogram_opt.as_mut() else {
                    eprintln!(
                        "ConstrainedMultiParallelogram predictor was selected but not initialized"
                    );
                    return false;
                };
                if !predictor.compute_original_values(
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    None,
                ) {
                    eprintln!(
                        "Failed to compute original values (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                let Some(predictor) = predictor_tex_coords_deprecated_opt.as_mut() else {
                    eprintln!("TexCoordsDeprecated predictor was selected but not initialized");
                    return false;
                };
                if !predictor.compute_original_values(
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    Some(
                        crate::prediction_scheme::EntryToPointIdMap::from_point_indices(point_ids),
                    ),
                ) {
                    eprintln!(
                        "Failed to compute original values (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                eprintln!("TexCoordsDeprecated prediction is disabled");
                return false;
            }
            PredictionSchemeMethod::MeshPredictionTexCoordsPortable => {
                let Some(predictor) = predictor_tex_coords_opt.as_mut() else {
                    eprintln!("TexCoordsPortable predictor was selected but not initialized");
                    return false;
                };
                if !predictor.compute_original_values(
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    Some(
                        crate::prediction_scheme::EntryToPointIdMap::from_point_indices(point_ids),
                    ),
                ) {
                    eprintln!(
                        "Failed to compute original values (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            PredictionSchemeMethod::MeshPredictionGeometricNormal => {
                let Some(predictor) = predictor_geometric_normal_opt.as_mut() else {
                    eprintln!("GeometricNormal predictor was selected but not initialized");
                    return false;
                };
                if !predictor.compute_original_values(
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    Some(
                        crate::prediction_scheme::EntryToPointIdMap::from_point_indices(point_ids),
                    ),
                ) {
                    eprintln!(
                        "Failed to compute original values (att_id={}, method={:?}, transform={:?})",
                        att_id, selected_method, selected_transform
                    );
                    return false;
                }
            }
            PredictionSchemeMethod::None => {
                values = corrections;
            }
            _ => {
                eprintln!("Unsupported prediction method: {:?}", selected_method);
                return false;
            }
        }

        #[cfg(feature = "debug_logs")]
        {
            if num_points > 0 {
                println!(
                    "Sequential Decoded: Point 0 ID = {:?}, Value[0] = {}",
                    point_ids[0], values[0]
                );
                // Debug: print all decoded values (quantized) and where they go
                println!("DEBUG decoded values (first 25 x/y/z):");
                if num_components >= 3 {
                    for i in 0..std::cmp::min(25, num_points) {
                        let x = values[i * num_components];
                        let y = values[i * num_components + 1];
                        let z = values[i * num_components + 2];
                        println!(
                            "  data_id={} -> point_ids[{}]={:?}: quantized({}, {}, {})",
                            i, i, point_ids[i], x, y, z
                        );
                    }
                }
            }
        }

        // 5. Store values (+ optional inverse transform)
        if let Some(portable_att) = portable_attribute {
            if !store_i32_values_to_attribute(portable_att, &values, num_points, num_components) {
                return false;
            }
        } else {
            let Ok(dst_attribute) = point_cloud.try_attribute_mut(att_id) else {
                return false;
            };
            if !store_i32_values_to_attribute(dst_attribute, &values, num_points, num_components) {
                return false;
            }
        }

        true
    }
}

#[inline]
fn symbol_to_correction(symbol: u32, needs_zigzag_conversion: bool) -> i32 {
    if needs_zigzag_conversion {
        ((symbol >> 1) as i32) ^ (-((symbol & 1) as i32))
    } else {
        symbol as i32
    }
}

#[inline]
fn symbols_to_corrections(symbols: Vec<u32>, needs_zigzag_conversion: bool) -> Vec<i32> {
    symbols
        .into_iter()
        .map(|symbol| symbol_to_correction(symbol, needs_zigzag_conversion))
        .collect()
}

/// Store decoded i32 values into an attribute buffer.
/// Uses bulk memcpy when the attribute layout matches i32/u32 tightly packed.
#[inline]
fn store_i32_values_to_attribute(
    attr: &mut PointAttribute,
    values: &[i32],
    num_points: usize,
    num_components: usize,
) -> bool {
    let Ok(byte_stride) = usize::try_from(attr.byte_stride()) else {
        return false;
    };
    let data_type = attr.data_type();
    let component_size = data_type.byte_length();
    let Some(packed_row) = num_components.checked_mul(component_size) else {
        return false;
    };
    let Some(num_values_required) = num_points.checked_mul(num_components) else {
        return false;
    };
    if values.len() < num_values_required {
        return false;
    }

    // Ensure buffer is large enough for num_points entries.
    let Some(required) = num_points.checked_mul(byte_stride) else {
        return false;
    };
    if attr.buffer().data_size() < required && attr.buffer_mut().try_resize(required).is_err() {
        return false;
    }

    // Fast path: i32/u32 tightly packed — bulk memcpy the entire values array.
    if (data_type == DataType::Int32 || data_type == DataType::Uint32) && byte_stride == packed_row
    {
        let src: &[u8] = bytemuck::cast_slice(&values[..num_values_required]);
        let dst = attr.buffer_mut().data_mut();
        let Some(dst) = dst.get_mut(..src.len()) else {
            return false;
        };
        dst.copy_from_slice(src);
        return true;
    }

    // Slow path: per-component write with type conversion.
    let dst_buffer = attr.buffer_mut();
    for i in 0..num_points {
        let Some(entry_offset) = i.checked_mul(byte_stride) else {
            return false;
        };
        for c in 0..num_components {
            let Some(component_byte_offset) = c.checked_mul(component_size) else {
                return false;
            };
            let Some(component_offset) = entry_offset.checked_add(component_byte_offset) else {
                return false;
            };
            if !write_value_from_i32(
                dst_buffer,
                component_offset,
                data_type,
                values[i * num_components + c],
            ) {
                return false;
            }
        }
    }
    true
}

#[inline(always)]
fn write_value_from_i32(
    buffer: &mut crate::data_buffer::DataBuffer,
    offset: usize,
    data_type: DataType,
    val: i32,
) -> bool {
    match data_type {
        DataType::Int8 => buffer.try_write(offset, &(val as i8).to_le_bytes()),
        DataType::Uint8 => buffer.try_write(offset, &(val as u8).to_le_bytes()),
        DataType::Int16 => buffer.try_write(offset, &(val as i16).to_le_bytes()),
        DataType::Uint16 => buffer.try_write(offset, &(val as u16).to_le_bytes()),
        DataType::Int32 => buffer.try_write(offset, &val.to_le_bytes()),
        DataType::Uint32 => buffer.try_write(offset, &(val as u32).to_le_bytes()),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
    use crate::geometry_indices::VertexIndex;
    use crate::point_cloud::PointCloud;

    #[test]
    fn store_i32_values_rejects_short_decoded_values() {
        let mut attr = PointAttribute::new();
        attr.init(GeometryAttributeType::Generic, 3, DataType::Int16, false, 2);

        assert!(!store_i32_values_to_attribute(&mut attr, &[1, 2, 3], 2, 3));
    }

    #[test]
    fn store_i32_values_rejects_impossible_required_size() {
        let mut attr = PointAttribute::new();
        attr.init(GeometryAttributeType::Generic, 1, DataType::Int32, false, 1);

        assert!(!store_i32_values_to_attribute(
            &mut attr,
            &[1],
            usize::MAX,
            1,
        ));
    }

    #[test]
    fn vertex_to_data_map_builder_accepts_valid_corners() {
        let mut corner_table = CornerTable::new(1);
        assert!(corner_table.init(&[[VertexIndex(0), VertexIndex(1), VertexIndex(2),]]));
        let mut vertex_to_data_map = Vec::new();

        assert!(build_vertex_to_data_map_from_data_to_corner_map(
            &corner_table,
            &[0, 1, 2],
            &mut vertex_to_data_map,
        ));
        assert_eq!(vertex_to_data_map, vec![0, 1, 2]);
    }

    #[test]
    fn vertex_to_data_map_builder_rejects_out_of_range_corner() {
        let mut corner_table = CornerTable::new(1);
        assert!(corner_table.init(&[[VertexIndex(0), VertexIndex(1), VertexIndex(2),]]));
        let mut vertex_to_data_map = Vec::new();

        assert!(!build_vertex_to_data_map_from_data_to_corner_map(
            &corner_table,
            &[3],
            &mut vertex_to_data_map,
        ));
    }

    #[test]
    fn decode_values_rejects_invalid_attribute_id() {
        let mut decoder = SequentialIntegerAttributeDecoder::new();
        decoder.init(&PointCloudDecoder::new(), 0);
        let mut point_cloud = PointCloud::new();
        let mut buffer = DecoderBuffer::new(&[]);
        let point_ids = [PointIndex(0)];

        assert!(!decoder.decode_values(
            &mut point_cloud,
            &point_ids,
            &mut buffer,
            None,
            None,
            None,
            None,
            None,
            None,
        ));
    }

    #[test]
    fn decode_values_with_portable_attribute_allows_missing_destination_id() {
        let mut decoder = SequentialIntegerAttributeDecoder::new();
        decoder.init(&PointCloudDecoder::new(), 0);
        let mut point_cloud = PointCloud::new();
        let mut portable = PointAttribute::new();
        portable.init(GeometryAttributeType::Generic, 1, DataType::Int32, false, 1);
        let bytes = [0xfe, 0, 0, 0, 0];
        let mut buffer = DecoderBuffer::new(&bytes);
        let point_ids = [PointIndex(0)];

        assert!(decoder.decode_values(
            &mut point_cloud,
            &point_ids,
            &mut buffer,
            None,
            None,
            None,
            Some(&mut portable),
            None,
            None,
        ));
    }
}
