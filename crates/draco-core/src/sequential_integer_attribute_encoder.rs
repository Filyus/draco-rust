//! Integer sequential attribute encoder.
//!
//! [`SequentialIntegerAttributeEncoder`] quantizes (when needed) and encodes
//! integer attribute values, applying the chosen prediction scheme and writing
//! prediction residuals. Encode-side counterpart of
//! `SequentialIntegerAttributeDecoder`. Port of Draco's
//! `sequential_integer_attribute_encoder.h`.

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
    EntryToPointIdMap, PredictionSchemeEncoder, PredictionSchemeMethod,
    PredictionSchemeTransformType,
};
use crate::prediction_scheme_constrained_multi_parallelogram::MeshPredictionSchemeConstrainedMultiParallelogramEncoder;
use crate::prediction_scheme_delta::PredictionSchemeDeltaEncoder;
use crate::prediction_scheme_geometric_normal::MeshPredictionSchemeGeometricNormalEncoder;
#[cfg(feature = "legacy_bitstream_encode")]
use crate::prediction_scheme_multi_parallelogram::MeshPredictionSchemeMultiParallelogramEncoder;
use crate::prediction_scheme_normal_octahedron_canonicalized_encoding_transform::PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform;
use crate::prediction_scheme_parallelogram::MeshPredictionSchemeParallelogramEncoder;
use crate::prediction_scheme_selection::select_prediction_method;
#[cfg(feature = "legacy_bitstream_encode")]
use crate::prediction_scheme_tex_coords_deprecated::MeshPredictionSchemeTexCoordsDeprecatedEncoder;
use crate::prediction_scheme_tex_coords_portable::{
    MeshPredictionSchemeTexCoordsPortableEncoder,
    PredictionSchemeTexCoordsPortableEncodingTransform,
};
use crate::prediction_scheme_wrap::PredictionSchemeWrapEncodingTransform;
use crate::sequential_attribute_encoder::SequentialAttributeEncoder;
use crate::status::{DracoError, Status};
use crate::symbol_encoding::{encode_symbols, SymbolEncodingOptions};

/// Which transform family this encoder builds its prediction schemes with.
///
/// Upstream expresses the same choice as a virtual: the base
/// `SequentialIntegerAttributeEncoder::CreateIntPredictionScheme` names
/// `PredictionSchemeWrapEncodingTransform`, and `SequentialNormalAttributeEncoder`
/// overrides it to name the canonicalized octahedron transform instead. That is
/// the only axis that varies between the two, so it is the only thing this
/// carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IntPredictionTransformFamily {
    /// Every attribute but an octahedral normal.
    #[default]
    Wrap,
    /// Octahedral normals, carrying the transform's `max_quantized_value` and
    /// whether to emit the canonicalized transform (id 3) or the pre-0.10.0
    /// non-canonicalized one (id 2).
    NormalOctahedron {
        max_quantized_value: i32,
        canonicalized: bool,
    },
}

impl IntPredictionTransformFamily {
    fn build_octahedron(
        max_quantized_value: i32,
        canonicalized: bool,
    ) -> PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform {
        let mut transform = PredictionSchemeNormalOctahedronCanonicalizedEncodingTransform::new(
            max_quantized_value,
        );
        transform.set_canonicalized(canonicalized);
        transform
    }
}

/// Whether this attribute's quantization parameters belong inline, before the
/// integer values, rather than after them.
///
/// Shared by the encoder and by `MeshEncoder`'s trailing-parameter pass, so the
/// two cannot both write them or both skip them. Keyed on
/// [`select_sequential_encoder`] rather than on the data type directly: a
/// quantized normal takes the octahedron transform, which has its own inline
/// block at a different version boundary just above this one.
///
/// [`select_sequential_encoder`]: crate::sequential_attribute_encoder::select_sequential_encoder
#[cfg(all(feature = "encoder", feature = "legacy_bitstream_encode"))]
pub(crate) fn uses_inline_quantization_parameters(
    attribute: &crate::geometry_attribute::PointAttribute,
    options: &EncoderOptions,
    att_id: i32,
) -> bool {
    use crate::sequential_attribute_encoder::{
        select_sequential_encoder, SequentialAttributeEncoderType,
    };

    let quantization_bits = options.get_attribute_int(att_id, "quantization_bits", -1);
    if select_sequential_encoder(attribute, quantization_bits)
        != SequentialAttributeEncoderType::Quantization
    {
        return false;
    }
    let (major, minor) = options.get_version();
    let bitstream_version = crate::version::bitstream_version(major, minor);
    // Upstream decides on the version alone: `DecodeQuantizedDataInfo` is called
    // from `DecodeIntegerValues` for every stream below 2.0 and from
    // `DecodeDataNeededByPortableTransform` at 2.0 and up, and the attribute
    // decoder it lives in is shared by meshes and point clouds. Splitting the
    // rule by geometry type put a point cloud's parameters where no C++ decoder
    // looks for them, and this crate's own decoder had the matching split, so
    // the round trip agreed with itself.
    bitstream_version != 0 && bitstream_version < 0x0200
}

pub struct SequentialIntegerAttributeEncoder {
    pub base: SequentialAttributeEncoder,
    /// Stores the quantization transform if one was applied, for later encoding
    quantization_transform: Option<AttributeQuantizationTransform>,
    transform_family: IntPredictionTransformFamily,
    /// What `encode_values` settled on, recorded where it writes the two bytes
    /// into the stream. The choice is the encoder's to make -- a caller asking
    /// for a scheme gets it only when the attribute and the mesh support one,
    /// and several arms fall back to `Difference` -- so this is the only place
    /// that knows the answer.
    selected_prediction: Option<(PredictionSchemeMethod, PredictionSchemeTransformType)>,
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
            quantization_transform: None,
            transform_family: IntPredictionTransformFamily::Wrap,
            selected_prediction: None,
        }
    }

    /// The prediction scheme and transform the last `encode_values` chose, or
    /// `None` if it has not run.
    pub fn selected_prediction(
        &self,
    ) -> Option<(PredictionSchemeMethod, PredictionSchemeTransformType)> {
        self.selected_prediction
    }

    /// Selects the transform family the prediction schemes are built with.
    ///
    /// Counterpart of overriding `CreateIntPredictionScheme`. Defaults to wrap,
    /// so only the normal encoder needs to call this.
    pub fn set_transform_family(&mut self, family: IntPredictionTransformFamily) {
        self.transform_family = family;
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
            q_transform.encode_parameters(out_buffer).is_ok()
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
    ) -> Status {
        let att_id = self.base.attribute_id();
        if att_id < 0 || att_id >= point_cloud.num_attributes() {
            return Err(DracoError::invalid_parameter(format!(
                "Attribute {att_id} is outside the {} the geometry has",
                point_cloud.num_attributes()
            )));
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
                q_transform.compute_parameters(attribute, quantization_bits)?;
                q_transform.transform_attribute(
                    attribute,
                    EntryToPointIdMap::from_point_indices(point_ids),
                    &mut local_portable_attribute,
                )?;
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
            debug_log!(
                "DEBUG: encode_values: num_points={} num_components={} num_values={}",
                num_points,
                num_components,
                num_values
            );
            debug_log!("DEBUG: is_portable_attribute={}", is_portable_attribute);
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
                debug_log!("DEBUG encoder values (first 25 x/y/z):");
                for i in 0..std::cmp::min(25, num_points) {
                    let x = values[i * 3];
                    let y = values[i * 3 + 1];
                    let z = values[i * 3 + 2];
                    debug_log!(
                        "  data_id={} -> point_ids[{}]={:?}: quantized({}, {}, {})",
                        i,
                        i,
                        point_ids[i],
                        x,
                        y,
                        z
                    );
                }
            }
        }

        // 2. Prediction Selection
        // Per attribute, then global, then the automatic choice -- upstream's
        // GetPredictionMethodFromOptions reads it off the attribute.
        let preferred_scheme = options.get_attribute_prediction_scheme(att_id);
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
        let mut predictor_delta_octahedron = None;
        let mut predictor_parallelogram = None;
        #[cfg(feature = "legacy_bitstream_encode")]
        let mut predictor_multi_parallelogram = None;
        #[cfg(feature = "legacy_bitstream_encode")]
        let mut predictor_tex_coords_deprecated = None;
        let mut predictor_constrained_multi_parallelogram = None;
        let mut predictor_tex_coords_portable = None;
        let mut predictor_geometric_normal = None;

        // Maps need to live long enough
        let mut vertex_to_data_map = Vec::new();
        let mut data_to_corner_map = Vec::new();

        match selected_method {
            // Delta over whichever transform family this encoder was given.
            // The decoder makes the same split on the way back in, keyed on
            // the transform byte -- see the Difference arm of
            // `sequential_integer_attribute_decoder.rs`.
            PredictionSchemeMethod::Difference => match self.transform_family {
                IntPredictionTransformFamily::NormalOctahedron {
                    max_quantized_value,
                    canonicalized,
                } => {
                    let transform = IntPredictionTransformFamily::build_octahedron(
                        max_quantized_value,
                        canonicalized,
                    );
                    let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                    selected_transform_type = predictor.get_transform_type();
                    predictor.compute_correction_values(
                        &values,
                        &mut corrections,
                        num_values,
                        num_components,
                        None,
                    )?;
                    predictor_delta_octahedron = Some(predictor);
                }
                IntPredictionTransformFamily::Wrap => {
                    let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                    let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                    selected_transform_type = predictor.get_transform_type();
                    predictor.compute_correction_values(
                        &values,
                        &mut corrections,
                        num_values,
                        num_components,
                        None,
                    )?;
                    predictor_delta = Some(predictor);
                }
            },
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
                            let tail = vertex_to_data_map.iter().rev().take(16).collect::<Vec<_>>();
                            debug_log!(
                                "Parallelogram encoder: vertex_to_data_map size={}, head={:?}, tail(reversed)={:?}",
                                vertex_to_data_map.len(),
                                head,
                                tail
                            );
                            debug_log!(
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

                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        )?;
                        predictor_parallelogram = Some(predictor);
                    } else {
                        // Compatibility fallback: match C++ factory behavior and use
                        // Difference when a mesh-only prediction scheme cannot be created.
                        selected_method = PredictionSchemeMethod::Difference;
                        let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                        let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                        selected_transform_type = predictor.get_transform_type();
                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        )?;
                        predictor_delta = Some(predictor);
                    }
                } else {
                    // Compatibility fallback: mesh-only prediction schemes degrade to
                    // Difference for non-mesh geometry, matching C++.
                    selected_method = PredictionSchemeMethod::Difference;
                    let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                    let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                    selected_transform_type = predictor.get_transform_type();
                    predictor.compute_correction_values(
                        &values,
                        &mut corrections,
                        num_values,
                        num_components,
                        None,
                    )?;
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
                        let (vmaj, vmin) = options.get_version();
                        predictor
                            .set_bitstream_version(crate::version::bitstream_version(vmaj, vmin));
                        selected_transform_type = predictor.get_transform_type();

                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        )?;
                        predictor_constrained_multi_parallelogram = Some(predictor);
                    } else {
                        // Compatibility fallback: match C++ factory behavior and use
                        // Difference when a mesh-only prediction scheme cannot be created.
                        selected_method = PredictionSchemeMethod::Difference;
                        let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                        let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        )?;
                        predictor_delta = Some(predictor);
                    }
                } else {
                    // Compatibility fallback: mesh-only prediction schemes degrade to
                    // Difference for non-mesh geometry, matching C++.
                    selected_method = PredictionSchemeMethod::Difference;
                    let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                    let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                    predictor.compute_correction_values(
                        &values,
                        &mut corrections,
                        num_values,
                        num_components,
                        None,
                    )?;
                    predictor_delta = Some(predictor);
                }
            }
            #[cfg(feature = "legacy_bitstream_encode")]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                if let Some(_mesh) = encoder.mesh() {
                    if let Some(corner_table) = encoder.corner_table() {
                        let is_edgebreaker = encoder.get_encoding_method() == Some(1);

                        let map_size = corner_table.num_vertices();
                        vertex_to_data_map.resize(map_size, -1);
                        data_to_corner_map.resize(num_points, 0);

                        if is_edgebreaker {
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
                        let mut predictor = MeshPredictionSchemeMultiParallelogramEncoder::new(
                            transform, mesh_data,
                        );
                        selected_transform_type = predictor.get_transform_type();

                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        )?;
                        predictor_multi_parallelogram = Some(predictor);
                    } else {
                        selected_method = PredictionSchemeMethod::Difference;
                        let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                        let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                        selected_transform_type = predictor.get_transform_type();
                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        )?;
                        predictor_delta = Some(predictor);
                    }
                } else {
                    selected_method = PredictionSchemeMethod::Difference;
                    let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                    let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                    selected_transform_type = predictor.get_transform_type();
                    predictor.compute_correction_values(
                        &values,
                        &mut corrections,
                        num_values,
                        num_components,
                        None,
                    )?;
                    predictor_delta = Some(predictor);
                }
            }
            #[cfg(not(feature = "legacy_bitstream_encode"))]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                return Err(DracoError::unsupported_feature(
                    "MultiParallelogram prediction needs the legacy_bitstream_encode feature"
                        .to_string(),
                ))
            }
            #[cfg(feature = "legacy_bitstream_encode")]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                if let Some(_mesh) = encoder.mesh() {
                    if let Some(corner_table) = encoder.corner_table() {
                        let is_edgebreaker = encoder.get_encoding_method() == Some(1);

                        let map_size = corner_table.num_vertices();
                        vertex_to_data_map.resize(map_size, -1);
                        data_to_corner_map.resize(num_points, 0);

                        if is_edgebreaker {
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
                            MeshPredictionSchemeTexCoordsDeprecatedEncoder::new(transform);
                        let (version_major, version_minor) = options.get_version();
                        predictor.set_bitstream_version(version_major, version_minor);
                        selected_transform_type = predictor.get_transform_type();

                        let pos_att = encoder
                            .point_cloud()
                            .unwrap()
                            .named_attribute(GeometryAttributeType::Position);
                        let Some(pos_att) = pos_att else {
                            return Err(DracoError::invalid_parameter(
                                "Texture-coordinate prediction needs a position attribute"
                                    .to_string(),
                            ));
                        };
                        predictor.set_parent_attribute(pos_att)?;
                        predictor.init(&mesh_data);

                        let entry_to_point_id_map: Vec<u32> =
                            point_ids.iter().map(|p| p.0).collect();

                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            Some(crate::prediction_scheme::EntryToPointIdMap::from_u32_slice(
                                &entry_to_point_id_map,
                            )),
                        )?;
                        predictor_tex_coords_deprecated = Some(predictor);
                    } else {
                        selected_method = PredictionSchemeMethod::Difference;
                        let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                        let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                        selected_transform_type = predictor.get_transform_type();
                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        )?;
                        predictor_delta = Some(predictor);
                    }
                } else {
                    selected_method = PredictionSchemeMethod::Difference;
                    let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                    let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                    selected_transform_type = predictor.get_transform_type();
                    predictor.compute_correction_values(
                        &values,
                        &mut corrections,
                        num_values,
                        num_components,
                        None,
                    )?;
                    predictor_delta = Some(predictor);
                }
            }
            #[cfg(not(feature = "legacy_bitstream_encode"))]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                return Err(DracoError::unsupported_feature(
                    "TexCoordsDeprecated prediction needs the legacy_bitstream_encode feature"
                        .to_string(),
                ))
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

                        let transform = PredictionSchemeTexCoordsPortableEncodingTransform::new();
                        let mut predictor =
                            MeshPredictionSchemeTexCoordsPortableEncoder::new(transform);
                        let (version_major, version_minor) = options.get_version();
                        predictor.set_bitstream_version(version_major, version_minor);
                        selected_transform_type = predictor.get_transform_type();

                        // The portable position, not the original floats: the
                        // predictor works in quantized space and the decoder
                        // has nothing else to predict from. C++ reaches it
                        // through PointCloudEncoder::GetPortableAttribute in
                        // SequentialAttributeEncoder::InitPredictionScheme,
                        // and fails outright when it is missing.
                        let pos_att_id = encoder
                            .point_cloud()
                            .unwrap()
                            .named_attribute_id(GeometryAttributeType::Position);
                        if pos_att_id < 0 {
                            return Err(DracoError::invalid_parameter(
                                "Texture-coordinate prediction needs a position attribute"
                                    .to_string(),
                            ));
                        }
                        let Some(pos_att) = encoder.get_portable_attribute(pos_att_id) else {
                            return Err(DracoError::general(
                                "No portable position attribute for TexCoordsPortable".to_string(),
                            ));
                        };
                        predictor.set_parent_attribute(pos_att)?;

                        predictor.init(&mesh_data);

                        let entry_to_point_id_map: Vec<u32> =
                            point_ids.iter().map(|p| p.0).collect();

                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            Some(crate::prediction_scheme::EntryToPointIdMap::from_u32_slice(
                                &entry_to_point_id_map,
                            )),
                        )?;
                        predictor_tex_coords_portable = Some(predictor);
                    } else {
                        // Compatibility fallback: match C++ factory behavior and use
                        // Difference when a mesh-only prediction scheme cannot be created.
                        selected_method = PredictionSchemeMethod::Difference;
                        let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                        let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                        selected_transform_type = predictor.get_transform_type();
                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        )?;
                        predictor_delta = Some(predictor);
                    }
                } else {
                    // Compatibility fallback: mesh-only prediction schemes degrade to
                    // Difference for non-mesh geometry, matching C++.
                    selected_method = PredictionSchemeMethod::Difference;
                    let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                    let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                    selected_transform_type = predictor.get_transform_type();
                    predictor.compute_correction_values(
                        &values,
                        &mut corrections,
                        num_values,
                        num_components,
                        None,
                    )?;
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

                        // This scheme predicts in octahedral coordinates, so
                        // it only means anything over the octahedron
                        // transform -- upstream templates it on that and
                        // reads the quantization bits back off it.
                        if let IntPredictionTransformFamily::NormalOctahedron {
                            max_quantized_value,
                            canonicalized,
                        } = self.transform_family
                        {
                            let transform = IntPredictionTransformFamily::build_octahedron(
                                max_quantized_value,
                                canonicalized,
                            );
                            let mut predictor =
                                MeshPredictionSchemeGeometricNormalEncoder::new(transform);
                            let (version_major, version_minor) = options.get_version();
                            predictor.set_bitstream_version(version_major, version_minor);
                            selected_transform_type = predictor.get_transform_type();

                            predictor.init(&mesh_data);

                            // The parent the predictor reads positions from,
                            // in quantized space -- upstream binds it in
                            // SetPredictionSchemeParentAttributes. Without it
                            // `is_initialized` is false and the call below
                            // refuses.
                            let pos_att_id =
                                point_cloud.named_attribute_id(GeometryAttributeType::Position);
                            if pos_att_id < 0 {
                                return Err(DracoError::invalid_parameter(
                                    "Geometric normal prediction needs a position attribute"
                                        .to_string(),
                                ));
                            }
                            let Some(pos_att) = encoder.get_portable_attribute(pos_att_id) else {
                                return Err(DracoError::general(
                                    "No portable position attribute for GeometricNormal"
                                        .to_string(),
                                ));
                            };
                            predictor.set_parent_attribute(pos_att)?;

                            let entry_to_point_id_map: Vec<u32> =
                                point_ids.iter().map(|p| p.0).collect();

                            predictor.compute_correction_values(
                                &values,
                                &mut corrections,
                                num_values,
                                num_components,
                                Some(crate::prediction_scheme::EntryToPointIdMap::from_u32_slice(
                                    &entry_to_point_id_map,
                                )),
                            )?;
                            predictor_geometric_normal = Some(predictor);
                        } else {
                            // Reached by a normal attribute whose values are
                            // already integral, so nothing quantized it and
                            // there are no octahedral coordinates to predict.
                            //
                            // Upstream reaches the same combination and
                            // handles it badly: its encoder factory has no
                            // per-transform specialization where the
                            // decoder's does, so it builds this scheme over
                            // the wrap transform and asks that for
                            // quantization bits -- a stub returning -1 behind
                            // DRACO_DCHECK(false), carrying upstream's own
                            // TODO. Debug asserts; release predicts from an
                            // uninitialized toolbox.
                            //
                            // Take instead the fallback the same factory uses
                            // when a mesh scheme cannot be built, and encode a
                            // delta.
                            selected_method = PredictionSchemeMethod::Difference;
                            let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                            let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                            selected_transform_type = predictor.get_transform_type();
                            predictor.compute_correction_values(
                                &values,
                                &mut corrections,
                                num_values,
                                num_components,
                                None,
                            )?;
                            predictor_delta = Some(predictor);
                        }
                    } else {
                        // Compatibility fallback: match C++ factory behavior and use
                        // Difference when a mesh-only prediction scheme cannot be created.
                        selected_method = PredictionSchemeMethod::Difference;
                        let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                        let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                        selected_transform_type = predictor.get_transform_type();
                        predictor.compute_correction_values(
                            &values,
                            &mut corrections,
                            num_values,
                            num_components,
                            None,
                        )?;
                        predictor_delta = Some(predictor);
                    }
                } else {
                    // Compatibility fallback: mesh-only prediction schemes degrade to
                    // Difference for non-mesh geometry, matching C++.
                    selected_method = PredictionSchemeMethod::Difference;
                    let transform = PredictionSchemeWrapEncodingTransform::<i32>::new();
                    let mut predictor = PredictionSchemeDeltaEncoder::new(transform);
                    selected_transform_type = predictor.get_transform_type();
                    predictor.compute_correction_values(
                        &values,
                        &mut corrections,
                        num_values,
                        num_components,
                        None,
                    )?;
                    predictor_delta = Some(predictor);
                }
            }
            PredictionSchemeMethod::None => {
                corrections.copy_from_slice(&values);
            }
            _ => {
                return Err(DracoError::unsupported_feature(format!(
                    "Prediction method {selected_method:?}"
                )))
            }
        }

        // Precompute prediction-data bytes so we can append them after symbols.
        let mut pred_data_opt: Option<Vec<u8>> = None;
        try_encode_prediction_data(predictor_delta, &mut pred_data_opt)?;
        try_encode_prediction_data(predictor_delta_octahedron, &mut pred_data_opt)?;
        try_encode_prediction_data(predictor_parallelogram, &mut pred_data_opt)?;
        #[cfg(feature = "legacy_bitstream_encode")]
        try_encode_prediction_data(predictor_multi_parallelogram, &mut pred_data_opt)?;
        #[cfg(feature = "legacy_bitstream_encode")]
        try_encode_prediction_data(predictor_tex_coords_deprecated, &mut pred_data_opt)?;
        try_encode_prediction_data(
            predictor_constrained_multi_parallelogram,
            &mut pred_data_opt,
        )?;
        try_encode_prediction_data(predictor_tex_coords_portable, &mut pred_data_opt)?;
        try_encode_prediction_data(predictor_geometric_normal, &mut pred_data_opt)?;

        // Pre-2.2 prefixes the constrained-multi-parallelogram prediction data with
        // an optimal-multi-parallelogram mode byte that the decoder reads before
        // the crease-edge streams; 2.2+ dropped it. Mirror of the decode-side
        // mode-byte read. get_version() is (0, 0) for the default (2.2).
        #[cfg(feature = "legacy_bitstream_encode")]
        if selected_method == PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram {
            let (major, minor) = options.get_version();
            let bitstream_version = crate::version::bitstream_version(major, minor);
            if bitstream_version != 0 && bitstream_version < 0x0202 {
                if let Some(pd) = pred_data_opt.as_mut() {
                    pd.insert(0, 0); // OPTIMAL_MULTI_PARALLELOGRAM
                }
            }
        }

        // 4. Encode Prediction Method and Transform Type
        #[cfg(feature = "debug_logs")]
        if crate::debug_env_enabled("DRACO_DEBUG_CMP_CPP") {
            debug_log!(
                "RUST: Encoding prediction method {} (0x{:x}), transform type {:?}",
                selected_method as i8,
                selected_method as u8,
                selected_transform_type
            );
        }
        self.selected_prediction = Some((selected_method, selected_transform_type));
        out_buffer.encode_u8(selected_method as u8);

        if selected_method != PredictionSchemeMethod::None {
            // Encode transform type
            out_buffer.encode_u8(selected_transform_type as u8);
        }

        #[cfg(feature = "legacy_bitstream_encode")]
        if matches!(
            selected_transform_type,
            PredictionSchemeTransformType::NormalOctahedron
                | PredictionSchemeTransformType::NormalOctahedronCanonicalized
        ) {
            let (major, minor) = options.get_version();
            let bitstream_version = crate::version::bitstream_version(major, minor);
            // Version alone, for the same reason as `writes_inline_quantization`.
            let uses_inline_normal_transform_data =
                bitstream_version != 0 && bitstream_version < 0x0200;
            if uses_inline_normal_transform_data {
                let quantization_bits = options.get_attribute_int(att_id, "quantization_bits", -1);
                if !(2..=30).contains(&quantization_bits) {
                    return Err(DracoError::invalid_parameter(format!(
                        "Octahedral quantization bits {quantization_bits} outside the supported range 2..=30"
                    )));
                }
                out_buffer.encode_u8(quantization_bits as u8);
            }
        }

        // The same split, one version boundary later, for the plain quantization
        // transform: a pre-2.0 mesh carries its parameters here, between the
        // prediction header and the integer values, while 2.0+ writes them after
        // the values in `encode_data_needed_by_portable_transform`. The encoder
        // wrote them trailing at every version, so a pre-2.0 quantized attribute
        // produced a stream whose decode failed on the parameters it expected to
        // find in front.
        #[cfg(feature = "legacy_bitstream_encode")]
        if uses_inline_quantization_parameters(attribute, options, att_id) {
            let quantization_bits = options.get_attribute_int(att_id, "quantization_bits", -1);
            let mut transform = AttributeQuantizationTransform::new();
            transform.compute_parameters(attribute, quantization_bits)?;
            transform.encode_parameters(out_buffer)?;
        }

        // 5. Convert corrections to symbols (ZigZag) if needed
        // For normal octahedron encoding, corrections are already positive, so skip ZigZag
        //
        // Decided from the transform that is about to be written, which is what
        // the decoder reads it back from. Asking the prediction scheme instead
        // would not work here: every one of them is a local, built inside the
        // match above because they borrow the corner table, so there is no
        // object left to ask by this point.
        let are_corrections_positive = matches!(
            selected_transform_type,
            PredictionSchemeTransformType::NormalOctahedron
                | PredictionSchemeTransformType::NormalOctahedronCanonicalized
        );

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

        // The larger of the two speeds, as SetSymbolEncodingCompressionLevel is
        // handed `10 - GetSpeed()`. Reading the encoding speed alone agrees
        // only while the two are set to the same value. Saturating because the
        // speed is a caller-set option with no declared range and
        // `10 - i32::MIN` overflows.
        //
        // Out of range the level is discarded rather than clamped, which is
        // upstream's own behaviour and not a safety choice: its setter refuses
        // anything outside 0..=10 and leaves the option unset, so `EncodeSymbols`
        // reads `kDefaultSymbolCodingCompressionLevel` -- 7, what
        // `SymbolEncodingOptions::default()` already carries. The two differ in
        // what they write. Clamping a speed of 11 gives level 0, which subtracts
        // 2 from the symbol bit length; discarding it gives 7, which adjusts
        // nothing. Verified against C++ 1.5.7: speeds at or below -2 and at or
        // above 11 produced different bytes before this, and match after.
        let mut symbol_options = SymbolEncodingOptions::default();
        let compression_level = 10i32.saturating_sub(options.get_speed());
        if (0..=10).contains(&compression_level) {
            symbol_options.compression_level = compression_level;
        }

        let _start_len = out_buffer.size();
        if !encode_symbols(&symbols, num_components, &symbol_options, out_buffer) {
            return Err(DracoError::general(
                "Failed to entropy-code the prediction residuals".to_string(),
            ));
        }

        // 7. Encode Prediction Data (after symbols)
        if selected_method != PredictionSchemeMethod::None {
            if let Some(pd) = pred_data_opt {
                out_buffer.encode_data(&pd);
            }
        }

        Ok(())
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

/// If `out` is still empty and `predictor` was built, encodes its
/// prediction-data bytes into `out`. Returns false only on an actual encode
/// failure. Collapses the seven identical "try this predictor next" blocks in
/// the prediction-data emission phase.
fn try_encode_prediction_data<'a, P: PredictionSchemeEncoder<'a, i32, i32>>(
    predictor: Option<P>,
    out: &mut Option<Vec<u8>>,
) -> Status {
    if out.is_none() {
        if let Some(mut predictor) = predictor {
            let mut pred_data = Vec::new();
            predictor.encode_prediction_data(&mut pred_data)?;
            *out = Some(pred_data);
        }
    }
    Ok(())
}
