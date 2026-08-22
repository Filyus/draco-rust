//! Integer sequential attribute decoder.
//!
//! [`SequentialIntegerAttributeDecoder`] decodes integer (and quantized
//! portable) attribute values, applying the inverse prediction scheme selected
//! by the bitstream to reconstruct each value from its predecessors. It is the
//! decode workhorse for positions, texture coordinates, and other quantized
//! attributes. Port of Draco's `sequential_integer_attribute_decoder.h`.

use crate::corner_table::CornerTable;
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
use crate::geometry_attribute::PointAttribute;
use crate::geometry_indices::{CornerIndex, INVALID_CORNER_INDEX};
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::point_cloud::PointCloud;
use crate::point_cloud_decoder::PointCloudDecoder;
use crate::prediction_scheme::{
    EntryToPointIdMap, PredictionScheme, PredictionSchemeDecoder, PredictionSchemeMethod,
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
use crate::status::{DracoError, Status};
use crate::symbol_encoding::{decode_symbols, SymbolEncodingOptions};

pub struct SequentialIntegerAttributeDecoder {
    attribute: i32,
    prediction_scheme: Option<Box<dyn PredictionSchemeDecoder<'static, i32, i32>>>,
}

fn build_vertex_to_data_map_from_data_to_corner_map(
    corner_table: &CornerTable,
    data_to_corner_map: &[u32],
    vertex_to_data_map: &mut Vec<i32>,
) -> Status {
    vertex_to_data_map.resize(corner_table.num_vertices(), -1);
    for (data_id, &corner_u32) in data_to_corner_map.iter().enumerate() {
        let corner_id = CornerIndex(corner_u32);
        if corner_id == INVALID_CORNER_INDEX {
            continue;
        }
        if corner_id.0 as usize >= corner_table.num_corners() {
            return Err(DracoError::general(format!(
                "Entry {data_id} maps to corner {corner_u32}, past the {} in the table",
                corner_table.num_corners()
            )));
        }
        let v = corner_table.vertex(corner_id).0 as usize;
        let Some(slot) = vertex_to_data_map.get_mut(v) else {
            return Err(DracoError::general(format!(
                "Corner {corner_u32} maps to vertex {v}, past the {} in the table",
                corner_table.num_vertices()
            )));
        };
        *slot = data_id as i32;
    }
    Ok(())
}

/// The corner and vertex maps a mesh predictor reads, borrowed from the mesh
/// decoder's own traversal when it has already built them.
///
/// Both maps are read-only from here on, and on the EdgeBreaker path the
/// decoder hands down arrays it built itself. Copying those into owned
/// buffers costs two allocations the size of the point and vertex counts,
/// plus two `memcpy`s, per attribute -- for data nothing writes to. The owned
/// vectors are filled only for the cases with no override to borrow.
fn prediction_maps<'a>(
    corner_table: &CornerTable,
    num_points: usize,
    data_to_corner_map_override: Option<&'a [u32]>,
    vertex_to_data_map_override: Option<&'a [i32]>,
    data_to_corner_map: &'a mut Vec<u32>,
    vertex_to_data_map: &'a mut Vec<i32>,
) -> Result<(&'a [u32], &'a [i32]), DracoError> {
    let data_to_corner: &[u32] = match data_to_corner_map_override {
        Some(map) if map.len() == num_points => map,
        Some(_) => {
            return Err(DracoError::general(
                "Invalid data_to_corner_map_override length".to_string(),
            ))
        }
        // No override: the map stays empty of meaning, as it was when this
        // was a `resize(num_points, 0)` -- the vertex map below is what the
        // predictor actually reads in that case.
        None => {
            data_to_corner_map.clear();
            data_to_corner_map.resize(num_points, 0);
            data_to_corner_map
        }
    };

    let vertex_to_data: &[i32] = match vertex_to_data_map_override {
        Some(map) if map.len() == corner_table.num_vertices() => map,
        Some(_) => {
            return Err(DracoError::general(
                "Invalid vertex_to_data_map_override length".to_string(),
            ))
        }
        // The corner table may carry seam-split vertices with ids outside the
        // original point range, so this is derived rather than assumed.
        None => {
            build_vertex_to_data_map_from_data_to_corner_map(
                corner_table,
                data_to_corner,
                vertex_to_data_map,
            )?;
            vertex_to_data_map
        }
    };

    Ok((data_to_corner, vertex_to_data))
}

/// Runs `decode_prediction_data` on the selected predictor, logging and failing
/// when the slot is empty or the call fails. Collapses the identical
/// extract-and-check boilerplate that the apply matches repeat per method.
/// `?Sized` lets it accept both the concrete locally-built predictors and the
/// `dyn`-typed `self.prediction_scheme`.
fn run_decode_prediction_data<'a, P: PredictionSchemeDecoder<'a, i32, i32> + ?Sized>(
    predictor: Option<&mut P>,
    buffer: &mut DecoderBuffer,
) -> Status {
    let Some(predictor) = predictor else {
        return Err(DracoError::general(
            "Predictor was selected but not initialized".to_string(),
        ));
    };
    predictor.decode_prediction_data(buffer)
}

/// Runs `compute_original_values` on the selected predictor, with the same
/// empty-slot / failure handling as [`run_decode_prediction_data`].
fn run_compute_original_values<'a, P: PredictionSchemeDecoder<'a, i32, i32> + ?Sized>(
    predictor: Option<&mut P>,
    corrections: &[i32],
    values: &mut [i32],
    num_values: usize,
    num_components: usize,
    entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
) -> Status {
    let Some(predictor) = predictor else {
        return Err(DracoError::general(
            "Predictor was selected but not initialized".to_string(),
        ));
    };
    predictor.compute_original_values(
        corrections,
        values,
        num_values,
        num_components,
        entry_to_point_id_map,
    )
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

    pub fn init(&mut self, _decoder: &PointCloudDecoder, attribute_id: i32) {
        self.attribute = attribute_id;
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
        point_ids: EntryToPointIdMap<'_>,
        in_buffer: &mut DecoderBuffer,
        corner_table: Option<&CornerTable>,
        data_to_corner_map_override: Option<&[u32]>,
        vertex_to_data_map_override: Option<&[i32]>,
        portable_attribute: Option<&mut PointAttribute>,
        portable_parent_attribute: Option<&PointAttribute>,
        pre_integer_decode: Option<&mut dyn FnMut(&mut DecoderBuffer<'_>) -> bool>,
    ) -> Status {
        let att_id = self.attribute;
        if att_id < 0 {
            return Err(DracoError::invalid_parameter(
                "Integer attribute decoder was never given an attribute".to_string(),
            ));
        }

        let num_points = point_ids.len();
        if num_points == 0 {
            return Ok(());
        }

        let attribute = if let Some(ref pa) = portable_attribute {
            &**pa
        } else {
            point_cloud.try_attribute(att_id)?
        };

        let num_components = attribute.num_components() as usize;
        // Both factors come from the bitstream, and `usize` is 32 bits on the
        // wasm32 target this ships to, where the product of a large point count
        // and 255 components wraps rather than saturating.
        let Some(num_values) = num_points.checked_mul(num_components) else {
            return Err(DracoError::general(format!(
                "{num_points} points times {num_components} components overflows"
            )));
        };

        // 3. Decode Prediction Method and (optional) prepare predictor
        let method_byte = match in_buffer.decode_u8() {
            Ok(v) => v,
            Err(_) => {
                return Err(DracoError::general(
                    "Failed to decode prediction method".to_string(),
                ));
            }
        };

        // Draco stores prediction method as int8 (0xFE == -2 == None).
        // Accept 0xFF as None as well for older Rust-produced streams that used
        // the wrong sentinel before this decoder matched the C++ enum exactly.
        let selected_method = if method_byte == 0xFF || method_byte == 0xFE {
            PredictionSchemeMethod::None
        } else {
            match PredictionSchemeMethod::try_from(method_byte) {
                Ok(m) => m,
                Err(_) => {
                    return Err(DracoError::unsupported_feature(format!(
                        "Prediction method {method_byte}"
                    )));
                }
            }
        };

        let mut selected_transform: Option<PredictionSchemeTransformType> = None;
        if selected_method != PredictionSchemeMethod::None {
            // Draco stores prediction transform type as int8 (0xFF == -1 == None).
            let transform_byte = in_buffer.decode_u8().map_err(|_| {
                DracoError::buffer("Stream ends before the prediction transform type".to_string())
            })?;
            if transform_byte != 0xFF {
                match PredictionSchemeTransformType::try_from(transform_byte) {
                    Ok(t) => selected_transform = Some(t),
                    Err(_) => {
                        return Err(DracoError::unsupported_feature(format!(
                            "Prediction transform type {transform_byte}"
                        )));
                    }
                }
            }
        }

        if let Some(ref scheme) = self.prediction_scheme {
            if scheme.get_prediction_method() != selected_method {
                return Err(DracoError::general(format!(
                    "Prediction method mismatch. Stream: {selected_method:?}, Scheme: {:?}",
                    scheme.get_prediction_method()
                )));
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
                // Pre-0.10.0 normals use the legacy non-canonicalized octahedron
                // transform (id 2). Without this case it fell through to Wrap below,
                // silently decoding to wrong normals.
                Some(PredictionSchemeTransformType::NormalOctahedron) => {
                    let mut transform =
                        PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform::new();
                    transform.set_canonicalized(false);
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
                    let (dcm, vdm) = prediction_maps(
                        corner_table,
                        num_points,
                        data_to_corner_map_override,
                        vertex_to_data_map_override,
                        &mut data_to_corner_map,
                        &mut vertex_to_data_map,
                    )?;

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, dcm, vdm);

                    let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
                    let predictor = MeshPredictionSchemeParallelogramDecoder::new(
                        attribute, transform, mesh_data,
                    );
                    predictor_parallelogram_opt = Some(predictor);
                } else {
                    return Err(DracoError::general(
                        "Parallelogram prediction requires corner table".to_string(),
                    ));
                }
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                if let Some(corner_table) = corner_table {
                    let (dcm, vdm) = prediction_maps(
                        corner_table,
                        num_points,
                        data_to_corner_map_override,
                        vertex_to_data_map_override,
                        &mut data_to_corner_map,
                        &mut vertex_to_data_map,
                    )?;

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, dcm, vdm);

                    let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
                    let predictor =
                        MeshPredictionSchemeMultiParallelogramDecoder::new(transform, mesh_data);
                    predictor_multi_parallelogram_opt = Some(predictor);
                } else {
                    return Err(DracoError::general(
                        "MultiParallelogram prediction requires corner table".to_string(),
                    ));
                }
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                return Err(DracoError::general(
                    "MultiParallelogram prediction is disabled".to_string(),
                ));
            }
            PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => {
                if let Some(corner_table) = corner_table {
                    // Generate maps
                    let (dcm, vdm) = prediction_maps(
                        corner_table,
                        num_points,
                        data_to_corner_map_override,
                        vertex_to_data_map_override,
                        &mut data_to_corner_map,
                        &mut vertex_to_data_map,
                    )?;

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, dcm, vdm);

                    let transform = PredictionSchemeWrapDecodingTransform::<i32>::new();
                    let predictor = MeshPredictionSchemeConstrainedMultiParallelogramDecoder::new(
                        transform, mesh_data,
                    );
                    predictor_constrained_multi_parallelogram_opt = Some(predictor);
                } else {
                    return Err(DracoError::general(
                        "ConstrainedMultiParallelogram prediction requires corner table"
                            .to_string(),
                    ));
                }
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                if let Some(corner_table) = corner_table {
                    let (dcm, vdm) = prediction_maps(
                        corner_table,
                        num_points,
                        data_to_corner_map_override,
                        vertex_to_data_map_override,
                        &mut data_to_corner_map,
                        &mut vertex_to_data_map,
                    )?;

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, dcm, vdm);

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
                            let attribute = point_cloud.try_attribute(pos_att_id)?;
                            attribute
                        };
                        if predictor.set_parent_attribute(pos_att).is_err() {
                            return Err(DracoError::general(
                                "Failed to set parent attribute for TexCoordsDeprecated"
                                    .to_string(),
                            ));
                        }
                    } else {
                        return Err(DracoError::general(
                            "Position attribute not found for TexCoordsDeprecated".to_string(),
                        ));
                    }

                    predictor_tex_coords_deprecated_opt = Some(predictor);
                } else {
                    return Err(DracoError::general(
                        "TexCoordsDeprecated prediction requires corner table".to_string(),
                    ));
                }
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                return Err(DracoError::general(
                    "TexCoordsDeprecated prediction is disabled".to_string(),
                ));
            }
            PredictionSchemeMethod::MeshPredictionTexCoordsPortable => {
                if let Some(corner_table) = corner_table {
                    let (dcm, vdm) = prediction_maps(
                        corner_table,
                        num_points,
                        data_to_corner_map_override,
                        vertex_to_data_map_override,
                        &mut data_to_corner_map,
                        &mut vertex_to_data_map,
                    )?;

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, dcm, vdm);

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
                            let attribute = point_cloud.try_attribute(pos_att_id)?;
                            attribute
                        };
                        if predictor.set_parent_attribute(pos_att).is_err() {
                            return Err(DracoError::general(
                                "Failed to set parent attribute for TexCoordsPortable".to_string(),
                            ));
                        }
                    } else {
                        return Err(DracoError::general(
                            "Position attribute not found for TexCoordsPortable".to_string(),
                        ));
                    }

                    predictor_tex_coords_opt = Some(predictor);
                } else {
                    return Err(DracoError::general(
                        "TexCoordsPortable prediction requires corner table".to_string(),
                    ));
                }
            }
            PredictionSchemeMethod::MeshPredictionGeometricNormal => {
                if let Some(corner_table) = corner_table {
                    let (dcm, vdm) = prediction_maps(
                        corner_table,
                        num_points,
                        data_to_corner_map_override,
                        vertex_to_data_map_override,
                        &mut data_to_corner_map,
                        &mut vertex_to_data_map,
                    )?;

                    let mut mesh_data = MeshPredictionSchemeData::new();
                    mesh_data.set(corner_table, dcm, vdm);

                    let mut transform =
                        PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform::new();
                    // Pre-0.10.0 streams use the legacy non-canonicalized octahedron
                    // transform (id 2); 0.10.0+ use the canonicalized one (id 3).
                    if matches!(
                        selected_transform,
                        Some(PredictionSchemeTransformType::NormalOctahedron)
                    ) {
                        transform.set_canonicalized(false);
                    }
                    let mut predictor = MeshPredictionSchemeGeometricNormalDecoder::new(transform);
                    predictor.init(&mesh_data);

                    // Provide mapping from decoded-entry index to original point id.
                    predictor.set_entry_to_point_id_map(point_ids);

                    // Set parent attribute (Position)
                    let pos_att_id = point_cloud.named_attribute_id(
                        crate::geometry_attribute::GeometryAttributeType::Position,
                    );
                    if pos_att_id >= 0 {
                        // Upstream InitPredictionScheme has two branches, and so
                        // does this. Bitstreams below 2.0 predict from the
                        // attribute itself, which by then already holds
                        // dequantized values; 2.0 and above predict from the
                        // portable one. The caller signals the first case by
                        // passing no portable attribute, so the fallback here is
                        // that branch and not a safety net.
                        let pos_att = match portable_parent_attribute {
                            Some(attribute) => attribute,
                            None => {
                                let attribute = point_cloud.try_attribute(pos_att_id)?;
                                attribute
                            }
                        };
                        if predictor.set_parent_attribute(pos_att).is_err() {
                            return Err(DracoError::general(
                                "Failed to set parent attribute for GeometricNormal".to_string(),
                            ));
                        }
                    } else {
                        return Err(DracoError::general(
                            "Position attribute not found for GeometricNormal".to_string(),
                        ));
                    }

                    predictor_geometric_normal_opt = Some(predictor);
                } else {
                    return Err(DracoError::general(
                        "GeometricNormal prediction requires corner table".to_string(),
                    ));
                }
            }
            PredictionSchemeMethod::None => {}
            _ => {
                return Err(DracoError::unsupported_feature(format!(
                    "Prediction method {selected_method:?}"
                )));
            }
        }

        // 1. Decode correction symbols.
        // For v < 2.0, transform-specific parameters (quantization, octahedron)
        // are stored BEFORE the integer values. The caller provides a hook.
        if let Some(hook) = pre_integer_decode {
            if !hook(in_buffer) {
                return Err(DracoError::general(
                    "Failed to decode the pre-2.0 inline transform parameters".to_string(),
                ));
            }
        }
        // Draco supports both entropy-coded symbols (compressed=1) and raw symbols (compressed=0).
        let compressed = in_buffer.decode_u8().map_err(|_| {
            DracoError::buffer("Stream ends before the compression flag".to_string())
        })?;

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
            // Empty on purpose. `num_values` comes from the header, and the
            // header is the attacker's: reserving for it here is what let a
            // 9 KB stream ask for gigabytes before decoding a single symbol.
            // `decode_symbols` grows this as symbols actually arrive.
            let mut symbols = Vec::new();
            let options = SymbolEncodingOptions::default();
            decode_symbols(
                num_values,
                num_components,
                &options,
                in_buffer,
                &mut symbols,
            )
            .map_err(|err| {
                DracoError::general(format!("Failed to decode the entropy-coded symbols: {err}"))
            })?;
            symbols_to_corrections(symbols, needs_zigzag_conversion)
        } else {
            // Raw uncompressed integers. Read directly as bytes.
            // ZigZag conversion is needed unless the scheme guarantees positive corrections.
            let num_bytes = match in_buffer.decode_u8() {
                Ok(v) => v as usize,
                Err(_) => {
                    return Err(DracoError::buffer(
                        "Stream ends before the raw correction byte width".to_string(),
                    ))
                }
            };
            if num_bytes > 4 {
                return Err(DracoError::general(format!(
                    "Raw corrections declare {num_bytes} bytes per value, at most 4 fit an i32"
                )));
            }

            let Some(mut raw_corrections) = try_reserved::<i32>(num_values) else {
                return Err(DracoError::general(format!(
                    "Failed to allocate {num_values} raw corrections"
                )));
            };
            if num_bytes == 0 {
                // All values are zero — nothing to read from the buffer.
                raw_corrections.resize(num_values, 0);
            } else if num_bytes == 4 {
                let Some(byte_len) = num_values.checked_mul(4) else {
                    return Err(DracoError::general(format!(
                        "{num_values} four-byte corrections overflow a byte count"
                    )));
                };
                let bytes = in_buffer.decode_slice(byte_len).map_err(|_| {
                    DracoError::buffer(format!(
                        "Stream holds fewer than the {byte_len} bytes of raw corrections it declares"
                    ))
                })?;
                for chunk in bytes.chunks_exact(4) {
                    let symbol = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    raw_corrections.push(symbol_to_correction(symbol, needs_zigzag_conversion));
                }
            } else {
                for _ in 0..num_values {
                    let mut tmp = [0u8; 4];
                    if in_buffer.decode_bytes(&mut tmp[..num_bytes]).is_err() {
                        return Err(DracoError::buffer(
                            "Stream ends inside the raw corrections".to_string(),
                        ));
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
            let Some(values) = try_zeroed::<i32>(num_values) else {
                return Err(DracoError::general(format!(
                    "Failed to allocate {num_values} decoded values"
                )));
            };
            values
        };

        // 3. Decode prediction scheme data (if any).
        match selected_method {
            _ if self.prediction_scheme.is_some() => {
                run_decode_prediction_data(self.prediction_scheme.as_deref_mut(), in_buffer)?;
            }
            PredictionSchemeMethod::Difference => {
                let ok = if predictor_normal_octa_diff_opt.is_some() {
                    run_decode_prediction_data(predictor_normal_octa_diff_opt.as_mut(), in_buffer)
                } else {
                    run_decode_prediction_data(predictor_opt.as_mut(), in_buffer)
                };
                ok?;
            }
            PredictionSchemeMethod::MeshPredictionParallelogram => {
                run_decode_prediction_data(predictor_parallelogram_opt.as_mut(), in_buffer)?;
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                run_decode_prediction_data(predictor_multi_parallelogram_opt.as_mut(), in_buffer)?;
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                return Err(DracoError::general(
                    "MultiParallelogram prediction is disabled".to_string(),
                ));
            }
            PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => {
                run_decode_prediction_data(
                    predictor_constrained_multi_parallelogram_opt.as_mut(),
                    in_buffer,
                )?;
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                run_decode_prediction_data(
                    predictor_tex_coords_deprecated_opt.as_mut(),
                    in_buffer,
                )?;
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                return Err(DracoError::general(
                    "TexCoordsDeprecated prediction is disabled".to_string(),
                ));
            }
            PredictionSchemeMethod::MeshPredictionTexCoordsPortable => {
                run_decode_prediction_data(predictor_tex_coords_opt.as_mut(), in_buffer)?;
            }
            PredictionSchemeMethod::MeshPredictionGeometricNormal => {
                run_decode_prediction_data(predictor_geometric_normal_opt.as_mut(), in_buffer)?;
            }
            PredictionSchemeMethod::None => {}
            _ => {
                return Err(DracoError::unsupported_feature(format!(
                    "Prediction method {selected_method:?}"
                )));
            }
        }

        // 4. Apply Inverse Prediction.
        match selected_method {
            _ if self.prediction_scheme.is_some() => {
                let map_opt = match selected_method {
                    PredictionSchemeMethod::MeshPredictionParallelogram
                    | PredictionSchemeMethod::MeshPredictionMultiParallelogram
                    | PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram
                    | PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated
                    | PredictionSchemeMethod::MeshPredictionTexCoordsPortable
                    | PredictionSchemeMethod::MeshPredictionGeometricNormal => Some(point_ids),
                    _ => None,
                };
                run_compute_original_values(
                    self.prediction_scheme.as_deref_mut(),
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    map_opt,
                )?;
            }
            PredictionSchemeMethod::Difference => {
                let ok = if predictor_normal_octa_diff_opt.is_some() {
                    run_compute_original_values(
                        predictor_normal_octa_diff_opt.as_mut(),
                        &corrections,
                        &mut values,
                        num_values,
                        num_components,
                        None,
                    )
                } else {
                    run_compute_original_values(
                        predictor_opt.as_mut(),
                        &corrections,
                        &mut values,
                        num_values,
                        num_components,
                        None,
                    )
                };
                ok?;
            }
            PredictionSchemeMethod::MeshPredictionParallelogram => {
                run_compute_original_values(
                    predictor_parallelogram_opt.as_mut(),
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    None,
                )?;
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                run_compute_original_values(
                    predictor_multi_parallelogram_opt.as_mut(),
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    None,
                )?;
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionMultiParallelogram => {
                return Err(DracoError::general(
                    "MultiParallelogram prediction is disabled".to_string(),
                ));
            }
            PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram => {
                run_compute_original_values(
                    predictor_constrained_multi_parallelogram_opt.as_mut(),
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    None,
                )?;
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                let map = Some(point_ids);
                run_compute_original_values(
                    predictor_tex_coords_deprecated_opt.as_mut(),
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    map,
                )?;
            }
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            PredictionSchemeMethod::MeshPredictionTexCoordsDeprecated => {
                return Err(DracoError::general(
                    "TexCoordsDeprecated prediction is disabled".to_string(),
                ));
            }
            PredictionSchemeMethod::MeshPredictionTexCoordsPortable => {
                let map = Some(point_ids);
                run_compute_original_values(
                    predictor_tex_coords_opt.as_mut(),
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    map,
                )?;
            }
            PredictionSchemeMethod::MeshPredictionGeometricNormal => {
                let map = Some(point_ids);
                run_compute_original_values(
                    predictor_geometric_normal_opt.as_mut(),
                    &corrections,
                    &mut values,
                    num_values,
                    num_components,
                    map,
                )?;
            }
            PredictionSchemeMethod::None => {
                values = corrections;
            }
            _ => {
                return Err(DracoError::unsupported_feature(format!(
                    "Prediction method {selected_method:?}"
                )));
            }
        }

        #[cfg(feature = "debug_logs")]
        {
            if num_points > 0 {
                debug_log!(
                    "Sequential Decoded: Point 0 ID = {:?}, Value[0] = {}",
                    crate::geometry_indices::PointIndex(point_ids.get(0).unwrap_or(u32::MAX)),
                    values[0]
                );
                // Debug: print all decoded values (quantized) and where they go
                debug_log!("DEBUG decoded values (first 25 x/y/z):");
                if num_components >= 3 {
                    for i in 0..std::cmp::min(25, num_points) {
                        let x = values[i * num_components];
                        let y = values[i * num_components + 1];
                        let z = values[i * num_components + 2];
                        debug_log!(
                            "  data_id={} -> point_ids[{}]={:?}: quantized({}, {}, {})",
                            i,
                            i,
                            crate::geometry_indices::PointIndex(
                                point_ids.get(i).unwrap_or(u32::MAX)
                            ),
                            x,
                            y,
                            z
                        );
                    }
                }
            }
        }

        // 5. Store values (+ optional inverse transform)
        if let Some(portable_att) = portable_attribute {
            if !store_i32_values_to_attribute(portable_att, &values, num_points, num_components) {
                return Err(DracoError::general(
                    "Decoded values do not fit the portable attribute".to_string(),
                ));
            }
        } else {
            let dst_attribute = point_cloud.try_attribute_mut(att_id)?;
            if !store_i32_values_to_attribute(dst_attribute, &values, num_points, num_components) {
                return Err(DracoError::general(
                    "Decoded values do not fit the destination attribute".to_string(),
                ));
            }
        }

        Ok(())
    }
}

/// Reserves room for `len` values, or reports failure instead of aborting.
///
/// `len` here is `num_points * num_components`, and both come out of the
/// bitstream, so it is as large as the file says. Every other allocation this
/// decoder makes from a declared count is already fallible; these were the last
/// infallible ones, and they are the largest — the corrections buffer is three
/// times the point-id vector that precedes it, so on a system that overcommits
/// it is the one that faults rather than the one that returns null.
fn try_reserved<T>(len: usize) -> Option<Vec<T>> {
    let mut values = Vec::new();
    values.try_reserve_exact(len).ok()?;
    Some(values)
}

/// The same, filled with `T::default()`.
fn try_zeroed<T: Clone + Default>(len: usize) -> Option<Vec<T>> {
    let mut values = try_reserved::<T>(len)?;
    values.resize(len, T::default());
    Some(values)
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
    use crate::geometry_indices::{PointIndex, VertexIndex};
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
        )
        .is_ok());
        assert_eq!(vertex_to_data_map, vec![0, 1, 2]);
    }

    #[test]
    fn vertex_to_data_map_builder_rejects_out_of_range_corner() {
        let mut corner_table = CornerTable::new(1);
        assert!(corner_table.init(&[[VertexIndex(0), VertexIndex(1), VertexIndex(2),]]));
        let mut vertex_to_data_map = Vec::new();

        let error = build_vertex_to_data_map_from_data_to_corner_map(
            &corner_table,
            &[3],
            &mut vertex_to_data_map,
        )
        .expect_err("a corner past the table must be refused");
        assert!(
            error.to_string().contains("corner 3"),
            "the error should name the corner, got: {error}"
        );
    }

    #[test]
    fn decode_values_rejects_invalid_attribute_id() {
        let mut decoder = SequentialIntegerAttributeDecoder::new();
        decoder.init(&PointCloudDecoder::new(), 0);
        let mut point_cloud = PointCloud::new();
        let mut buffer = DecoderBuffer::new(&[]);
        let point_ids = [PointIndex(0)];

        assert!(decoder
            .decode_values(
                &mut point_cloud,
                EntryToPointIdMap::from_point_indices(&point_ids),
                &mut buffer,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .is_err());
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

        assert!(decoder
            .decode_values(
                &mut point_cloud,
                EntryToPointIdMap::from_point_indices(&point_ids),
                &mut buffer,
                None,
                None,
                None,
                Some(&mut portable),
                None,
                None,
            )
            .is_ok());
    }
}
