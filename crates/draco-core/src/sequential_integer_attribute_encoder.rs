use crate::attribute_quantization_transform::AttributeQuantizationTransform;
use crate::attribute_transform::AttributeTransform;
use crate::data_buffer::DataBuffer;
use crate::draco_types::DataType;
use crate::encoder_buffer::EncoderBuffer;
use crate::encoder_options::EncoderOptions;
use crate::geometry_attribute::GeometryAttributeType;
use crate::geometry_indices::PointIndex;
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::point_cloud::PointCloud;
use crate::point_cloud_encoder::GeometryEncoder;
use crate::prediction_scheme::PredictionScheme;
use crate::prediction_scheme::{
    PredictionSchemeEncoder, PredictionSchemeMethod, PredictionSchemeTransformType,
};
use crate::prediction_scheme_constrained_multi_parallelogram::MeshPredictionSchemeConstrainedMultiParallelogramEncoder;
use crate::prediction_scheme_delta::PredictionSchemeDeltaEncoder;
use crate::prediction_scheme_geometric_normal::{
    MeshPredictionSchemeGeometricNormalEncoder, PredictionSchemeGeometricNormalEncodingTransform,
};
use crate::prediction_scheme_parallelogram::MeshPredictionSchemeParallelogramEncoder;
use crate::prediction_scheme_selection::select_prediction_method;
use crate::prediction_scheme_tex_coords_portable::{
    MeshPredictionSchemeTexCoordsPortableEncoder,
    PredictionSchemeTexCoordsPortableEncodingTransform,
};
use crate::prediction_scheme_wrap::PredictionSchemeWrapEncodingTransform;
use crate::sequential_attribute_encoder::SequentialAttributeEncoder;
use crate::symbol_encoding::{encode_symbols, SymbolEncodingOptions};

pub struct SequentialIntegerAttributeEncoder {
    pub base: SequentialAttributeEncoder,
    prediction_scheme: Option<Box<dyn PredictionSchemeEncoder<'static, i32, i32>>>,
    /// Stores the quantization transform if one was applied, for later encoding
    quantization_transform: Option<AttributeQuantizationTransform>,
}

impl Default for SequentialIntegerAttributeEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialIntegerAttributeEncoder {
    pub fn new() -> Self {
        Self {
            base: SequentialAttributeEncoder::new(),
            prediction_scheme: None,
            quantization_transform: None,
        }
    }

    pub fn set_prediction_scheme(
        &mut self,
        scheme: Box<dyn PredictionSchemeEncoder<'static, i32, i32>>,
    ) {
        self.prediction_scheme = Some(scheme);
    }

    pub fn init(&mut self, attribute_id: i32) -> bool {
        self.base.init(attribute_id)
    }

    /// Encodes the quantization transform parameters if a quantization transform was applied.
    /// This should be called AFTER encode_values(), matching the C++ encoding order:
    /// 1. EncodePortableAttributes (encode_values) - prediction method + compressed data
    /// 2. EncodeDataNeededByPortableTransforms (this method) - quantization parameters
    pub fn encode_data_needed_by_portable_transform(&self, out_buffer: &mut EncoderBuffer) -> bool {
        if let Some(ref q_transform) = self.quantization_transform {
            q_transform.encode_parameters(out_buffer)
        } else {
            true // No transform to encode
        }
    }

    // Symmetric to decode_values: requires 7 parameters for mesh encoding including
    // traversal order, corner table for prediction schemes, and buffer management.
    // Parameter count matches C++ API design for complex mesh attribute encoding.
    #[allow(clippy::too_many_arguments)]
    pub fn encode_values(
        &mut self,
        point_cloud: &PointCloud,
        point_ids: &[PointIndex],
        out_buffer: &mut EncoderBuffer,
        options: &EncoderOptions,
        encoder: &dyn GeometryEncoder,
        pre_computed_portable_attribute: Option<&crate::geometry_attribute::PointAttribute>,
        transform_already_encoded: bool,
    ) -> bool {
        let att_id = self.base.attribute_id();
        if att_id < 0 || att_id >= point_cloud.num_attributes() {
            return false;
        }

        let attribute = point_cloud.attribute(att_id);

        let mut local_portable_attribute = crate::geometry_attribute::PointAttribute::default();
        let mut is_portable_attribute = false;

        // Attribute transform handling:
        // - For mesh encoding (transform_already_encoded == true): attribute transform is
        //   handled externally (e.g., by MeshEncoder which writes transform type and params).
        // - For point cloud encoding (transform_already_encoded == false): we need to apply
        //   the transform here but NOT write transform type/params - those are written later
        //   via encode_data_needed_by_portable_transform().
        let current_attribute = if transform_already_encoded {
            // Mesh path: transform already encoded, just use provided portable attribute
            if let Some(pa) = pre_computed_portable_attribute {
                is_portable_attribute = true;
                pa
            } else {
                attribute
            }
        } else if let Some(pa) = pre_computed_portable_attribute {
            // Portable attribute already prepared externally (e.g., normal encoding)
            is_portable_attribute = true;
            pa
        } else {
            // Point cloud path: check if we need to apply quantization
            let quantization_bits = options.get_attribute_int(att_id, "quantization_bits", -1);
            if quantization_bits > 0
                && (attribute.data_type() == DataType::Float32
                    || attribute.data_type() == DataType::Float64)
            {
                // Apply quantization transform (but don't write params yet - that happens
                // in encode_data_needed_by_portable_transform)
                let mut q_transform = AttributeQuantizationTransform::new();
                if !q_transform.compute_parameters(attribute, quantization_bits) {
                    return false;
                }
                if !q_transform.transform_attribute(
                    attribute,
                    point_ids,
                    &mut local_portable_attribute,
                ) {
                    return false;
                }
                // Store transform for later encoding
                self.quantization_transform = Some(q_transform);
                is_portable_attribute = true;
                &local_portable_attribute
            } else {
                attribute
            }
        };

        // 1. Gather values
        let num_components = current_attribute.num_components() as usize;
        let num_points = point_ids.len();
        let num_values = num_points * num_components;
        #[cfg(feature = "debug_logs")]
        {
            println!(
                "DEBUG: encode_values: num_points={} num_components={} num_values={}",
                num_points, num_components, num_values
            );
            println!("DEBUG: is_portable_attribute={}", is_portable_attribute);
        }

        let mut values = Vec::with_capacity(num_values);
        let byte_stride = current_attribute.byte_stride() as usize;
        let data_type = current_attribute.data_type();
        let component_size = data_type.byte_length();
        for i in 0..num_points {
            let entry_index = if is_portable_attribute {
                crate::geometry_indices::AttributeValueIndex(i as u32)
            } else {
                let pid = point_ids[i];
                attribute.mapped_index(pid)
            };
            let entry_offset = entry_index.0 as usize * byte_stride;

            for c in 0..num_components {
                let component_offset = entry_offset + c * component_size;
                let val =
                    read_value_as_i32(current_attribute.buffer(), component_offset, data_type);
                values.push(val);
            }
        }

        // Debug: print encoded values
        #[cfg(feature = "debug_logs")]
        {
            if num_components == 3 {
                println!("DEBUG encoder values (first 25 x/y/z):");
                for i in 0..std::cmp::min(25, num_points) {
                    let x = values[i * 3];
                    let y = values[i * 3 + 1];
                    let z = values[i * 3 + 2];
                    println!(
                        "  data_id={} -> point_ids[{}]={:?}: quantized({}, {}, {})",
                        i, i, point_ids[i], x, y, z
                    );
                }
            }
        }

        // 2. Prediction Selection
        let preferred_scheme = options.get_prediction_scheme();
        let mut selected_method;

        if preferred_scheme != -1 {
            selected_method = match preferred_scheme {
                0 => PredictionSchemeMethod::Difference,
                1 => PredictionSchemeMethod::MeshPredictionParallelogram,
                2 => PredictionSchemeMethod::MeshPredictionMultiParallelogram,
                3 => PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated,
                4 => PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram,
                5 => PredictionSchemeMethod::MeshPredictionTexCoordsPortable,
                6 => PredictionSchemeMethod::MeshPredictionGeometricNormal,
                _ => PredictionSchemeMethod::None,
            };
        } else {
            selected_method = select_prediction_method(att_id, options, encoder);
        }

        // 3. Apply Prediction
        let mut corrections = vec![0i32; num_values];
        let mut selected_transform_type = PredictionSchemeTransformType::Wrap;
        let mut predictor_delta = None;
        let mut predictor_parallelogram = None;
        let mut predictor_constrained_multi_parallelogram = None;
        let mut predictor_tex_coords_portable = None;
        let mut predictor_geometric_normal = None;

        // Maps need to live long enough
        let mut vertex_to_data_map = Vec::new();
        let mut data_to_corner_map = Vec::new();

        if let Some(ref mut scheme) = self.prediction_scheme {
            selected_method = scheme.get_prediction_method();
            selected_transform_type = scheme.get_transform_type();
            if !scheme.compute_correction_values(
                &values,
                &mut corrections,
                num_values,
                num_components,
                None,
            ) {
                return false;
            }
        } else {
            match selected_method {
                PredictionSchemeMethod::Difference => {
                    let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                    let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                    selected_transform_type = predictor.get_transform_type();
                    if !predictor.compute_correction_values(
                        &values,
                        &mut corrections,
                        num_values,
                        num_components,
                        None,
                    ) {
                        return false;
                    }
                    predictor_delta = Some(predictor);
                }
                PredictionSchemeMethod::MeshPredictionParallelogram => {
                    if let Some(_mesh) = encoder.mesh() {
                        if let Some(corner_table) = encoder.corner_table() {
                            // Generate maps
                            // For Edgebreaker, vertex_to_data_map is indexed by corner table VertexIndex.
                            // For Sequential, it's indexed by mesh PointIndex (which equals VertexIndex).
                            let is_edgebreaker = encoder.get_encoding_method() == Some(1);

                            // vertex_to_data_map must be indexed by corner table VertexIndex
                            let map_size = corner_table.num_vertices();
                            vertex_to_data_map.resize(map_size, -1);
                            data_to_corner_map.resize(num_points, 0);

                            if is_edgebreaker {
                                // For Edgebreaker, get both maps from the encoder.
                                // These maps were computed during connectivity encoding and
                                // are consistent with each other.
                                if let Some(map) = encoder.get_data_to_corner_map() {
                                    if map.len() == num_points {
                                        data_to_corner_map.copy_from_slice(map);
                                    }
                                }
                                if let Some(map) = encoder.get_vertex_to_data_map() {
                                    // Use the pre-computed vertex_to_data_map from the encoder
                                    replace_vec_from_slice(&mut vertex_to_data_map, map);
                                }
                            } else {
                                // Sequential encoding: PointIndex == VertexIndex (1:1 mapping)
                                for (i, &point_id) in point_ids.iter().enumerate() {
                                    if (point_id.0 as usize) < vertex_to_data_map.len()
                                        && vertex_to_data_map[point_id.0 as usize] == -1
                                    {
                                        vertex_to_data_map[point_id.0 as usize] = i as i32;
                                    }
                                    let ci = corner_table.left_most_corner(
                                        crate::geometry_indices::VertexIndex(point_id.0),
                                    );
                                    data_to_corner_map[i] = ci.0;
                                }
                            }

                            let mut mesh_data = MeshPredictionSchemeData::new();
                            mesh_data.set(corner_table, &data_to_corner_map, &vertex_to_data_map);

                            #[cfg(feature = "debug_logs")]
                            {
                                let head = vertex_to_data_map.iter().take(16).collect::<Vec<_>>();
                                let tail =
                                    vertex_to_data_map.iter().rev().take(16).collect::<Vec<_>>();
                                eprintln!(
                                    "Parallelogram encoder: vertex_to_data_map size={}, head={:?}, tail(reversed)={:?}",
                                    vertex_to_data_map.len(),
                                    head,
                                    tail
                                );
                                eprintln!(
                                    "Parallelogram encoder: data_to_corner_map head={:?}",
                                    data_to_corner_map.iter().take(16).collect::<Vec<_>>()
                                );
                            }

                            let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                            let mut predictor = MeshPredictionSchemeParallelogramEncoder::new(
                                current_attribute,
                                transform,
                                mesh_data,
                            );
                            selected_transform_type = predictor.get_transform_type();

                            if !predictor.compute_correction_values(
                                &values,
                                &mut corrections,
                                num_values,
                                num_components,
                                None,
                            ) {
                                return false;
                            }
                            predictor_parallelogram = Some(predictor);
                        } else {
                            // Compatibility fallback: match C++ factory behavior and use
                            // Difference when a mesh-only prediction scheme cannot be created.
                            selected_method = PredictionSchemeMethod::Difference;
                            let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                            let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                            selected_transform_type = predictor.get_transform_type();
                            if !predictor.compute_correction_values(
                                &values,
                                &mut corrections,
                                num_values,
                                num_components,
                                None,
                            ) {
                                return false;
                            }
                            predictor_delta = Some(predictor);
                        }
                    } else {
                        // Compatibility fallback: mesh-only prediction schemes degrade to
                        // Difference for non-mesh geometry, matching C++.
                        selected_method = PredictionSchemeMethod::Difference;
                        let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                        let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                        selected_transform_type = predictor.get_transform_type();
                        if !predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        ) {
                            return false;
                        }
                        predictor_delta = Some(predictor);
                    }
                }
                PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => {
                    if let Some(_mesh) = encoder.mesh() {
                        if let Some(corner_table) = encoder.corner_table() {
                            // Generate maps - vertex_to_data_map indexed by corner table VertexIndex
                            let is_edgebreaker = encoder.get_encoding_method() == Some(1);

                            let map_size = corner_table.num_vertices();
                            vertex_to_data_map.resize(map_size, -1);
                            data_to_corner_map.resize(num_points, 0);

                            if is_edgebreaker {
                                // For Edgebreaker, get both maps from the encoder.
                                if let Some(map) = encoder.get_data_to_corner_map() {
                                    if map.len() == num_points {
                                        data_to_corner_map.copy_from_slice(map);
                                    }
                                }
                                if let Some(map) = encoder.get_vertex_to_data_map() {
                                    replace_vec_from_slice(&mut vertex_to_data_map, map);
                                }
                            } else {
                                for (i, &point_id) in point_ids.iter().enumerate() {
                                    if (point_id.0 as usize) < vertex_to_data_map.len()
                                        && vertex_to_data_map[point_id.0 as usize] == -1
                                    {
                                        vertex_to_data_map[point_id.0 as usize] = i as i32;
                                    }
                                    let ci = corner_table.left_most_corner(
                                        crate::geometry_indices::VertexIndex(point_id.0),
                                    );
                                    data_to_corner_map[i] = ci.0;
                                }
                            }

                            let mut mesh_data = MeshPredictionSchemeData::new();
                            mesh_data.set(corner_table, &data_to_corner_map, &vertex_to_data_map);

                            let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                            let mut predictor =
                                MeshPredictionSchemeConstrainedMultiParallelogramEncoder::new(
                                    transform, mesh_data,
                                );
                            selected_transform_type = predictor.get_transform_type();

                            if !predictor.compute_correction_values(
                                &values,
                                &mut corrections,
                                num_values,
                                num_components,
                                None,
                            ) {
                                return false;
                            }
                            predictor_constrained_multi_parallelogram = Some(predictor);
                        } else {
                            // Compatibility fallback: match C++ factory behavior and use
                            // Difference when a mesh-only prediction scheme cannot be created.
                            selected_method = PredictionSchemeMethod::Difference;
                            let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                            let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                            if !predictor.compute_correction_values(
                                &values,
                                &mut corrections,
                                num_values,
                                num_components,
                                None,
                            ) {
                                return false;
                            }
                            predictor_delta = Some(predictor);
                        }
                    } else {
                        // Compatibility fallback: mesh-only prediction schemes degrade to
                        // Difference for non-mesh geometry, matching C++.
                        selected_method = PredictionSchemeMethod::Difference;
                        let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                        let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                        if !predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        ) {
                            return false;
                        }
                        predictor_delta = Some(predictor);
                    }
                }
                PredictionSchemeMethod::MeshPredictionTexCoordsPortable => {
                    if let Some(_mesh) = encoder.mesh() {
                        if let Some(corner_table) = encoder.corner_table() {
                            let is_edgebreaker = encoder.get_encoding_method() == Some(1);

                            // vertex_to_data_map indexed by corner table VertexIndex
                            let map_size = corner_table.num_vertices();
                            vertex_to_data_map.resize(map_size, -1);
                            data_to_corner_map.resize(num_points, 0);

                            if is_edgebreaker {
                                // For Edgebreaker, get both maps from the encoder.
                                if let Some(map) = encoder.get_data_to_corner_map() {
                                    if map.len() == num_points {
                                        data_to_corner_map.copy_from_slice(map);
                                    }
                                }
                                if let Some(map) = encoder.get_vertex_to_data_map() {
                                    replace_vec_from_slice(&mut vertex_to_data_map, map);
                                }
                            } else {
                                for (i, &point_id) in point_ids.iter().enumerate() {
                                    if (point_id.0 as usize) < vertex_to_data_map.len()
                                        && vertex_to_data_map[point_id.0 as usize] == -1
                                    {
                                        vertex_to_data_map[point_id.0 as usize] = i as i32;
                                    }
                                    let ci = corner_table.left_most_corner(
                                        crate::geometry_indices::VertexIndex(point_id.0),
                                    );
                                    data_to_corner_map[i] = ci.0;
                                }
                            }

                            let mut mesh_data = MeshPredictionSchemeData::new();
                            mesh_data.set(corner_table, &data_to_corner_map, &vertex_to_data_map);

                            let transform =
                                PredictionSchemeTexCoordsPortableEncodingTransform::new();
                            let mut predictor =
                                MeshPredictionSchemeTexCoordsPortableEncoder::new(transform);
                            selected_transform_type = predictor.get_transform_type();

                            let pos_att = encoder
                                .point_cloud()
                                .unwrap()
                                .named_attribute(GeometryAttributeType::Position);
                            if let Some(pos_att) = pos_att {
                                if !predictor.set_parent_attribute(pos_att) {
                                    return false;
                                }
                            } else {
                                return false;
                            }

                            predictor.init(&mesh_data);

                            let entry_to_point_id_map: Vec<u32> =
                                point_ids.iter().map(|p| p.0).collect();

                            if !predictor.compute_correction_values(
                                &values,
                                &mut corrections,
                                num_values,
                                num_components,
                                Some(crate::prediction_scheme::EntryToPointIdMap::from_u32_slice(
                                    &entry_to_point_id_map,
                                )),
                            ) {
                                return false;
                            }
                            predictor_tex_coords_portable = Some(predictor);
                        } else {
                            // Compatibility fallback: match C++ factory behavior and use
                            // Difference when a mesh-only prediction scheme cannot be created.
                            selected_method = PredictionSchemeMethod::Difference;
                            let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                            let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                            selected_transform_type = predictor.get_transform_type();
                            if !predictor.compute_correction_values(
                                &values,
                                &mut corrections,
                                num_values,
                                num_components,
                                None,
                            ) {
                                return false;
                            }
                            predictor_delta = Some(predictor);
                        }
                    } else {
                        // Compatibility fallback: mesh-only prediction schemes degrade to
                        // Difference for non-mesh geometry, matching C++.
                        selected_method = PredictionSchemeMethod::Difference;
                        let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                        let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                        selected_transform_type = predictor.get_transform_type();
                        if !predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        ) {
                            return false;
                        }
                        predictor_delta = Some(predictor);
                    }
                }
                PredictionSchemeMethod::MeshPredictionGeometricNormal => {
                    if let Some(_mesh) = encoder.mesh() {
                        if let Some(corner_table) = encoder.corner_table() {
                            let is_edgebreaker = encoder.get_encoding_method() == Some(1);

                            // vertex_to_data_map indexed by corner table VertexIndex
                            let map_size = corner_table.num_vertices();
                            vertex_to_data_map.resize(map_size, -1);
                            data_to_corner_map.resize(num_points, 0);

                            if is_edgebreaker {
                                // For Edgebreaker, get both maps from the encoder.
                                if let Some(map) = encoder.get_data_to_corner_map() {
                                    if map.len() == num_points {
                                        data_to_corner_map.copy_from_slice(map);
                                    }
                                }
                                if let Some(map) = encoder.get_vertex_to_data_map() {
                                    replace_vec_from_slice(&mut vertex_to_data_map, map);
                                }
                            } else {
                                for (i, &point_id) in point_ids.iter().enumerate() {
                                    if (point_id.0 as usize) < vertex_to_data_map.len()
                                        && vertex_to_data_map[point_id.0 as usize] == -1
                                    {
                                        vertex_to_data_map[point_id.0 as usize] = i as i32;
                                    }
                                    let ci = corner_table.left_most_corner(
                                        crate::geometry_indices::VertexIndex(point_id.0),
                                    );
                                    data_to_corner_map[i] = ci.0;
                                }
                            }

                            let mut mesh_data = MeshPredictionSchemeData::new();
                            mesh_data.set(corner_table, &data_to_corner_map, &vertex_to_data_map);

                            let transform = PredictionSchemeGeometricNormalEncodingTransform::new();
                            let mut predictor =
                                MeshPredictionSchemeGeometricNormalEncoder::new(transform);
                            selected_transform_type = predictor.get_transform_type();

                            predictor.init(&mesh_data);

                            let entry_to_point_id_map: Vec<u32> =
                                point_ids.iter().map(|p| p.0).collect();

                            if !predictor.compute_correction_values(
                                &values,
                                &mut corrections,
                                num_values,
                                num_components,
                                Some(crate::prediction_scheme::EntryToPointIdMap::from_u32_slice(
                                    &entry_to_point_id_map,
                                )),
                            ) {
                                return false;
                            }
                            predictor_geometric_normal = Some(predictor);
                        } else {
                            // Compatibility fallback: match C++ factory behavior and use
                            // Difference when a mesh-only prediction scheme cannot be created.
                            selected_method = PredictionSchemeMethod::Difference;
                            let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                            let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                            selected_transform_type = predictor.get_transform_type();
                            if !predictor.compute_correction_values(
                                &values,
                                &mut corrections,
                                num_values,
                                num_components,
                                None,
                            ) {
                                return false;
                            }
                            predictor_delta = Some(predictor);
                        }
                    } else {
                        // Compatibility fallback: mesh-only prediction schemes degrade to
                        // Difference for non-mesh geometry, matching C++.
                        selected_method = PredictionSchemeMethod::Difference;
                        let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                        let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                        selected_transform_type = predictor.get_transform_type();
                        if !predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        ) {
                            return false;
                        }
                        predictor_delta = Some(predictor);
                    }
                }
                PredictionSchemeMethod::None => {
                    corrections.copy_from_slice(&values);
                }
                _ => return false,
            }
        }

        // Precompute prediction-data bytes so we can append them after symbols.
        let mut pred_data_opt: Option<Vec<u8>> = None;
        if let Some(ref mut scheme) = self.prediction_scheme {
            let mut pred_data = Vec::new();
            if !scheme.encode_prediction_data(&mut pred_data) {
                return false;
            }
            pred_data_opt = Some(pred_data);
        } else if let Some(mut predictor) = predictor_delta {
            let mut pred_data = Vec::new();
            if !predictor.encode_prediction_data(&mut pred_data) {
                return false;
            }
            pred_data_opt = Some(pred_data);
        } else if let Some(mut predictor) = predictor_parallelogram {
            let mut pred_data = Vec::new();
            if !predictor.encode_prediction_data(&mut pred_data) {
                return false;
            }
            pred_data_opt = Some(pred_data);
        } else if let Some(mut predictor) = predictor_constrained_multi_parallelogram {
            let mut pred_data = Vec::new();
            if !predictor.encode_prediction_data(&mut pred_data) {
                return false;
            }
            pred_data_opt = Some(pred_data);
        } else if let Some(mut predictor) = predictor_tex_coords_portable {
            let mut pred_data = Vec::new();
            if !predictor.encode_prediction_data(&mut pred_data) {
                return false;
            }
            pred_data_opt = Some(pred_data);
        } else if let Some(mut predictor) = predictor_geometric_normal {
            let mut pred_data = Vec::new();
            if !predictor.encode_prediction_data(&mut pred_data) {
                return false;
            }
            pred_data_opt = Some(pred_data);
        }

        // 4. Encode Prediction Method and Transform Type
        #[cfg(feature = "debug_logs")]
        if crate::debug_env_enabled("DRACO_DEBUG_CMP_CPP") {
            eprintln!(
                "RUST: Encoding prediction method {} (0x{:x}), transform type {:?}",
                selected_method as i8, selected_method as u8, selected_transform_type
            );
        }
        out_buffer.encode_u8(selected_method as u8);

        if selected_method != PredictionSchemeMethod::None {
            // Encode transform type
            out_buffer.encode_u8(selected_transform_type as u8);
        }

        // 5. Convert corrections to symbols (ZigZag) if needed
        // For normal octahedron encoding, corrections are already positive, so skip ZigZag
        let are_corrections_positive = if let Some(ref scheme) = self.prediction_scheme {
            scheme.are_corrections_positive()
        } else {
            false
        };

        let symbols: Vec<u32> = if are_corrections_positive {
            // Corrections are already unsigned - just cast
            corrections.iter().map(|&c| c as u32).collect()
        } else {
            // Apply ZigZag encoding
            corrections
                .iter()
                .map(|&c| ((c << 1) ^ (c >> 31)) as u32)
                .collect()
        };

        // 6. Encode symbols
        // Write compression level/type (1 = compressed with symbols)
        out_buffer.encode_u8(1);

        let mut symbol_options = SymbolEncodingOptions::default();
        symbol_options.compression_level = 10 - options.get_encoding_speed();

        let _start_len = out_buffer.size();
        let ok = encode_symbols(&symbols, num_components, &symbol_options, out_buffer);
        if !ok {
            return false;
        }

        // 7. Encode Prediction Data (after symbols)
        if selected_method != PredictionSchemeMethod::None {
            if let Some(pd) = pred_data_opt {
                out_buffer.encode_data(&pd);
            }
        }

        true
    }
}

fn read_value_as_i32(buffer: &DataBuffer, offset: usize, data_type: DataType) -> i32 {
    match data_type {
        DataType::Int8 => {
            let mut bytes = [0u8; 1];
            buffer.read(offset, &mut bytes);
            bytes[0] as i8 as i32
        }
        DataType::Uint8 => {
            let mut bytes = [0u8; 1];
            buffer.read(offset, &mut bytes);
            bytes[0] as i32
        }
        DataType::Int16 => {
            let mut bytes = [0u8; 2];
            buffer.read(offset, &mut bytes);
            i16::from_le_bytes(bytes) as i32
        }
        DataType::Uint16 => {
            let mut bytes = [0u8; 2];
            buffer.read(offset, &mut bytes);
            u16::from_le_bytes(bytes) as i32
        }
        DataType::Int32 => {
            let mut bytes = [0u8; 4];
            buffer.read(offset, &mut bytes);
            i32::from_le_bytes(bytes)
        }
        DataType::Uint32 => {
            let mut bytes = [0u8; 4];
            buffer.read(offset, &mut bytes);
            u32::from_le_bytes(bytes) as i32
        }
        _ => 0,
    }
}

#[inline]
fn replace_vec_from_slice<T: Copy>(dst: &mut Vec<T>, src: &[T]) {
    if dst.len() == src.len() {
        dst.copy_from_slice(src);
    } else {
        dst.clear();
        dst.extend_from_slice(src);
    }
}
