use crate::geometry_attribute::PointAttribute;
use crate::geometry_indices::{CornerIndex, INVALID_CORNER_INDEX};
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::prediction_scheme::{
    PredictionScheme, PredictionSchemeMethod, PredictionSchemeTransformType,
};
use crate::prediction_scheme_parallelogram::ParallelogramDataType;
use std::marker::PhantomData;

#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "decoder")]
use crate::prediction_scheme::{PredictionSchemeDecoder, PredictionSchemeDecodingTransform};
#[cfg(feature = "decoder")]
use crate::rans_bit_decoder::RAnsBitDecoder;

#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
#[cfg(feature = "encoder")]
use crate::prediction_scheme::{PredictionSchemeEncoder, PredictionSchemeEncodingTransform};
#[cfg(feature = "encoder")]
use crate::rans_bit_encoder::RAnsBitEncoder;
#[cfg(feature = "encoder")]
use crate::shannon_entropy::ShannonEntropyTracker;

pub const MAX_NUM_PARALLELOGRAMS: usize = 4;

#[cfg(feature = "encoder")]
pub struct MeshPredictionSchemeConstrainedMultiParallelogramEncoder<
    'a,
    DataType,
    CorrType,
    Transform,
> {
    mesh_data: MeshPredictionSchemeData<'a>,
    transform: Transform,
    is_crease_edge: [Vec<bool>; MAX_NUM_PARALLELOGRAMS],
    entropy_tracker: ShannonEntropyTracker,
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "encoder")]
impl<'a, DataType, CorrType, Transform>
    MeshPredictionSchemeConstrainedMultiParallelogramEncoder<'a, DataType, CorrType, Transform>
where
    Transform: PredictionSchemeEncodingTransform<DataType, CorrType>,
{
    pub fn new(transform: Transform, mesh_data: MeshPredictionSchemeData<'a>) -> Self {
        Self {
            mesh_data,
            transform,
            is_crease_edge: Default::default(),
            entropy_tracker: ShannonEntropyTracker::new(),
            _marker: PhantomData,
        }
    }

    fn convert_signed_int_to_symbol(val: i64) -> u32 {
        if val >= 0 {
            (val as u32) << 1
        } else {
            ((-val as u32) << 1) - 1
        }
    }
}

#[cfg(feature = "encoder")]
impl<'a, DataType, CorrType, Transform> PredictionScheme<'a>
    for MeshPredictionSchemeConstrainedMultiParallelogramEncoder<'a, DataType, CorrType, Transform>
where
    Transform: PredictionSchemeEncodingTransform<DataType, CorrType>,
{
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram
    }

    fn is_initialized(&self) -> bool {
        self.mesh_data.corner_table().is_some()
    }

    fn get_num_parent_attributes(&self) -> i32 {
        0
    }

    fn get_parent_attribute_type(
        &self,
        _i: i32,
    ) -> crate::geometry_attribute::GeometryAttributeType {
        crate::geometry_attribute::GeometryAttributeType::Generic
    }

    fn set_parent_attribute(&mut self, _att: &'a PointAttribute) -> bool {
        false
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }
}

#[cfg(feature = "encoder")]
struct Error {
    num_bits: i64,
    residual_error: i64,
}

#[cfg(feature = "encoder")]
impl Error {
    fn new() -> Self {
        Self {
            num_bits: 0,
            residual_error: 0,
        }
    }
}

#[cfg(feature = "encoder")]
impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        self.num_bits == other.num_bits && self.residual_error == other.residual_error
    }
}

#[cfg(feature = "encoder")]
impl PartialOrd for Error {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match self.num_bits.partial_cmp(&other.num_bits) {
            Some(std::cmp::Ordering::Equal) => {
                self.residual_error.partial_cmp(&other.residual_error)
            }
            other => other,
        }
    }
}

#[cfg(feature = "encoder")]
impl<'a, DataType, CorrType, Transform> PredictionSchemeEncoder<'a, DataType, CorrType>
    for MeshPredictionSchemeConstrainedMultiParallelogramEncoder<'a, DataType, CorrType, Transform>
where
    DataType: ParallelogramDataType + Into<i64> + Copy + Default + From<i32>,
    CorrType: Copy + Default + From<DataType> + std::ops::Sub<Output = CorrType> + From<i32>,
    Transform: PredictionSchemeEncodingTransform<DataType, CorrType>,
    i64: From<DataType>,
{
    fn compute_correction_values(
        &mut self,
        in_data: &[DataType],
        out_corr: &mut [CorrType],
        size: usize,
        num_components: usize,
        _entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> bool {
        self.transform.init(in_data, size, num_components);

        if num_components == 0 || !size.is_multiple_of(num_components) {
            return false;
        }
        let num_entries = size / num_components;

        let corner_table = match self.mesh_data.corner_table() {
            Some(ct) => ct,
            None => return false,
        };
        let vertex_to_data_map = match self.mesh_data.vertex_to_data_map() {
            Some(map) => map,
            None => return false,
        };

        for i in 0..MAX_NUM_PARALLELOGRAMS {
            self.is_crease_edge[i].clear();
        }

        let mut pred_vals = vec![vec![DataType::default(); num_components]; MAX_NUM_PARALLELOGRAMS];
        let mut multi_pred_vals = vec![DataType::default(); num_components];
        let mut entropy_symbols = vec![0u32; num_components];
        let mut predicted_val = vec![DataType::default(); num_components];
        let mut corr_val = vec![CorrType::default(); num_components];
        let mut tmp_entropy_symbols = vec![0u32; num_components];
        let mut tmp_pred_vals = vec![DataType::default(); num_components];

        // Track total parallelograms and used parallelograms for overhead calculation
        let mut total_parallelograms: [i64; MAX_NUM_PARALLELOGRAMS] = [0; MAX_NUM_PARALLELOGRAMS];
        let mut total_used_parallelograms: [i64; MAX_NUM_PARALLELOGRAMS] =
            [0; MAX_NUM_PARALLELOGRAMS];
        #[cfg(feature = "debug_logs")]
        let debug_cmp = crate::debug_env_enabled("DRACO_DEBUG_CMP");
        #[cfg(not(feature = "debug_logs"))]
        let debug_cmp = false;

        // C++ encoder processes vertices from the end because this prediction uses
        // data from previous entries that could be overwritten when an entry is processed.
        // We iterate BACKWARD from (num_entries - 1) down to 1, matching C++.
        for data_id in (1..num_entries).rev() {
            let data_offset = data_id * num_components;

            let corner_id = if let Some(map) = self.mesh_data.data_to_corner_map() {
                if data_id < map.len() {
                    CornerIndex(map[data_id])
                } else {
                    INVALID_CORNER_INDEX
                }
            } else if data_id < corner_table.num_vertices() {
                corner_table.left_most_corner(crate::geometry_indices::VertexIndex(data_id as u32))
            } else {
                INVALID_CORNER_INDEX
            };

            if corner_id == INVALID_CORNER_INDEX {
                predicted_val.fill(DataType::default());
                if data_id > 0 {
                    let prev_offset = (data_id - 1) * num_components;
                    for c in 0..num_components {
                        predicted_val[c] = in_data[prev_offset + c];
                    }
                }

                corr_val.fill(CorrType::default());
                self.transform.compute_correction(
                    &in_data[data_offset..data_offset + num_components],
                    &predicted_val,
                    &mut corr_val,
                );
                for c in 0..num_components {
                    out_corr[data_offset + c] = corr_val[c];
                    // Update entropy tracker with delta residuals?
                    // The C++ implementation seems to only update entropy tracker for the chosen configuration.
                    // If no parallelogram, it falls back to delta.
                    // We should probably update tracker here too to keep it consistent?
                    // But wait, the tracker is used to estimate bits for *parallelogram* residuals.
                    // If we use delta, the residuals might have different distribution.
                    // However, the entropy tracker is global for the attribute.
                    // Let's assume we should update it.
                    // But wait, `ComputeError` uses `entropy_tracker_.Peek`.
                    // And after selection, we call `entropy_tracker_.Push`.
                    // So yes, we should push.

                    // But wait, `ComputeError` calculates `num_bits` based on `entropy_tracker`.
                    // If we don't use `ComputeError` here (because no choice), we still need to push the symbols
                    // so that future `ComputeError` calls have correct context.

                    // But `out_corr` are the residuals.
                    // We need to convert them to symbols.
                    // `CorrType` might not be easily convertible to `i64`.
                    // But `DataType` is.
                    // `compute_correction` computes `out_corr`.
                    // We can compute `dif` manually as `in_data - predicted`.
                    // `DataType` subtraction?
                    // `DataType` has `Into<i64>`.
                    let val = in_data[data_offset + c].into();
                    let pred = predicted_val[c].into();
                    let dif = val - pred;
                    entropy_symbols[c] = Self::convert_signed_int_to_symbol(dif);
                }
                self.entropy_tracker.push(&entropy_symbols);
                continue;
            }

            let mut corners = [INVALID_CORNER_INDEX; MAX_NUM_PARALLELOGRAMS];
            let mut num_parallelograms = 0;

            let start_c = corner_id;
            let mut c = start_c;
            let mut first_pass = true;
            while c != INVALID_CORNER_INDEX {
                let opp = corner_table.opposite(c);
                if opp != INVALID_CORNER_INDEX {
                    let opp_v = corner_table.vertex(opp);
                    // Match C++ ComputeParallelogramPrediction(): next/prev must be
                    // taken from the opposite corner (oci), not from |c|.
                    let next_v = corner_table.vertex(corner_table.next(opp));
                    let prev_v = corner_table.vertex(corner_table.previous(opp));

                    let opp_data_id = *vertex_to_data_map.get(opp_v.0 as usize).unwrap_or(&-1);
                    let next_data_id = *vertex_to_data_map.get(next_v.0 as usize).unwrap_or(&-1);
                    let prev_data_id = *vertex_to_data_map.get(prev_v.0 as usize).unwrap_or(&-1);

                    if opp_data_id != -1
                        && next_data_id != -1
                        && prev_data_id != -1
                        && (opp_data_id as usize) < data_id
                        && (next_data_id as usize) < data_id
                        && (prev_data_id as usize) < data_id
                        && num_parallelograms < MAX_NUM_PARALLELOGRAMS
                    {
                        corners[num_parallelograms] = c;
                        num_parallelograms += 1;
                        if num_parallelograms == MAX_NUM_PARALLELOGRAMS {
                            break;
                        }
                    }
                }

                // Proceed to the next corner attached to the vertex.
                // First swing left and if we reach a boundary, swing right from
                // the start corner.
                c = if first_pass {
                    corner_table.swing_left(c)
                } else {
                    corner_table.swing_right(c)
                };
                if c == start_c {
                    break;
                }
                if c == INVALID_CORNER_INDEX && first_pass {
                    first_pass = false;
                    c = corner_table.swing_right(start_c);
                }
            }

            if num_parallelograms == 0 {
                predicted_val.fill(DataType::default());
                if data_id > 0 {
                    let prev_offset = (data_id - 1) * num_components;
                    for c in 0..num_components {
                        predicted_val[c] = in_data[prev_offset + c];
                    }
                }

                corr_val.fill(CorrType::default());
                self.transform.compute_correction(
                    &in_data[data_offset..data_offset + num_components],
                    &predicted_val,
                    &mut corr_val,
                );
                for c in 0..num_components {
                    out_corr[data_offset + c] = corr_val[c];
                    // For entropy tracking, C++ uses predicted - actual
                    let val = in_data[data_offset + c].into();
                    let pred = predicted_val[c].into();
                    let dif = pred - val; // predicted - actual, like C++
                    entropy_symbols[c] = Self::convert_signed_int_to_symbol(dif);
                }
                self.entropy_tracker.push(&entropy_symbols);
                continue;
            }

            for i in 0..num_parallelograms {
                let ci = corners[i];
                let oci = corner_table.opposite(ci);
                let vert_opp = vertex_to_data_map[corner_table.vertex(oci).0 as usize];
                // BUG FIX: Must use oci (opposite corner), not ci, to get next/prev vertices
                // This matches C++ ComputeParallelogramPrediction() behavior
                let vert_next =
                    vertex_to_data_map[corner_table.vertex(corner_table.next(oci)).0 as usize];
                let vert_prev =
                    vertex_to_data_map[corner_table.vertex(corner_table.previous(oci)).0 as usize];

                let v_opp_off = (vert_opp as usize) * num_components;
                let v_next_off = (vert_next as usize) * num_components;
                let v_prev_off = (vert_prev as usize) * num_components;

                for k in 0..num_components {
                    pred_vals[i][k] = DataType::compute_parallelogram_prediction(
                        in_data[v_next_off + k],
                        in_data[v_prev_off + k],
                        in_data[v_opp_off + k],
                    );
                }
            }

            let mut best_error = Error {
                num_bits: i64::MAX,
                residual_error: i64::MAX,
            };
            let mut best_config = 0u8;
            let mut best_num_used = 0;

            // C++ increments total_parallelograms BEFORE evaluating any configurations.
            // This is critical for matching the overhead calculation exactly.
            let context = num_parallelograms - 1;
            total_parallelograms[context] += num_parallelograms as i64;

            let num_configs = 1 << num_parallelograms;
            // Config 0 is valid (all creases = delta prediction)
            for config in 0..num_configs {
                let mut num_used = 0;
                for k in 0..num_components {
                    multi_pred_vals[k] = DataType::default();
                }

                for i in 0..num_parallelograms {
                    if (config & (1 << i)) != 0 {
                        num_used += 1;
                    }
                }

                if num_used == 0 {
                    // Delta prediction (config 0: all parallelograms marked as creases)
                    predicted_val.fill(DataType::default());
                    if data_id > 0 {
                        let prev_offset = (data_id - 1) * num_components;
                        for c in 0..num_components {
                            predicted_val[c] = in_data[prev_offset + c];
                        }
                    }

                    let mut error = Error::new();
                    for c in 0..num_components {
                        // For entropy tracking, C++ uses predicted - actual
                        let val = in_data[data_offset + c].into();
                        let pred = predicted_val[c].into();
                        let dif = pred - val; // predicted - actual, like C++
                        error.residual_error += dif.abs();
                        entropy_symbols[c] = Self::convert_signed_int_to_symbol(dif);
                    }

                    let entropy_data = self.entropy_tracker.peek(&entropy_symbols);
                    error.num_bits =
                        ShannonEntropyTracker::get_number_of_data_bits_static(&entropy_data)
                            + ShannonEntropyTracker::get_number_of_r_ans_table_bits_static(
                                &entropy_data,
                            );

                    // Add overhead bits - C++ uses total cumulative overhead
                    // For config 0: no parallelograms used, so total_used stays the same
                    let overhead_bits = Self::compute_overhead_bits(
                        total_used_parallelograms[context],
                        total_parallelograms[context],
                    );
                    error.num_bits += overhead_bits;

                    if error < best_error {
                        best_error = error;
                        best_config = config as u8;
                        best_num_used = 0;
                    }
                    continue;
                }

                // Multi-parallelogram prediction
                // Encoder must use same accumulation as decoder: AddAsUnsigned (wrapping add)
                for k in 0..num_components {
                    let mut sum: i32 = 0;
                    for i in 0..num_parallelograms {
                        if (config & (1 << i)) != 0 {
                            let pred_val: i64 = pred_vals[i][k].into();
                            // AddAsUnsigned: convert to unsigned, add, convert back
                            sum = (sum as u32).wrapping_add(pred_val as u32) as i32;
                        }
                    }
                    // C++ uses truncating integer division (not rounding)
                    let val = sum / num_used as i32;
                    multi_pred_vals[k] = DataType::from(val);
                }

                let mut error = Error::new();
                for c in 0..num_components {
                    // For entropy tracking, C++ uses predicted - actual
                    let val = in_data[data_offset + c].into();
                    let pred = multi_pred_vals[c].into();
                    let dif = pred - val; // predicted - actual, like C++
                    error.residual_error += dif.abs();
                    entropy_symbols[c] = Self::convert_signed_int_to_symbol(dif);
                }

                let entropy_data = self.entropy_tracker.peek(&entropy_symbols);
                error.num_bits =
                    ShannonEntropyTracker::get_number_of_data_bits_static(&entropy_data)
                        + ShannonEntropyTracker::get_number_of_r_ans_table_bits_static(
                            &entropy_data,
                        );

                // Add overhead bits - C++ computes overhead assuming this config is chosen
                // If num_used parallelograms are used, total_used increases by num_used
                let overhead_bits = Self::compute_overhead_bits(
                    total_used_parallelograms[context] + num_used as i64,
                    total_parallelograms[context],
                );

                error.num_bits += overhead_bits;

                if error < best_error {
                    best_error = error;
                    best_config = config as u8;
                    best_num_used = num_used;
                }
            }

            // Diagnostic logging: compare cumulative vs marginal cost choices if requested
            if debug_cmp {
                use std::sync::atomic::AtomicUsize;
                static _CMP_DIV_COUNT: AtomicUsize = AtomicUsize::new(0);
                const _MAX_CMP_DIV_PRINT: usize = 200;

                // Compute marginal-choice simulation (old bug style): per-vertex overhead as local cost
                let mut marginal_best_error = Error::new();
                marginal_best_error.num_bits = i64::MAX;
                let mut _marginal_best_config: u8 = 0;
                for config in 0..(1 << num_parallelograms) {
                    let mut num_used = 0;
                    for i in 0..num_parallelograms {
                        if (config & (1 << i)) != 0 {
                            num_used += 1;
                        }
                    }
                    // compute residual/rans bits same as above
                    tmp_entropy_symbols.fill(0);
                    tmp_pred_vals.fill(DataType::default());
                    if num_used == 0 {
                        if data_id > 0 {
                            let prev_offset = (data_id - 1) * num_components;
                            for c in 0..num_components {
                                tmp_pred_vals[c] = in_data[prev_offset + c];
                            }
                        }
                    } else {
                        for k in 0..num_components {
                            let mut sum: i32 = 0;
                            for i in 0..num_parallelograms {
                                if (config & (1 << i)) != 0 {
                                    let pred_val: i64 = pred_vals[i][k].into();
                                    sum = (sum as u32).wrapping_add(pred_val as u32) as i32;
                                }
                            }
                            let val = sum / num_used as i32;
                            tmp_pred_vals[k] = DataType::from(val);
                        }
                    }
                    let mut tmp_err = Error::new();
                    for c in 0..num_components {
                        let val = in_data[data_offset + c].into();
                        let pred = tmp_pred_vals[c].into();
                        let dif = val - pred;
                        tmp_err.residual_error += dif.abs();
                        tmp_entropy_symbols[c] = Self::convert_signed_int_to_symbol(dif);
                    }
                    let entropy_data = self.entropy_tracker.peek(&tmp_entropy_symbols);
                    tmp_err.num_bits =
                        ShannonEntropyTracker::get_number_of_data_bits_static(&entropy_data)
                            + ShannonEntropyTracker::get_number_of_r_ans_table_bits_static(
                                &entropy_data,
                            );
                    // marginal overhead bits (local) as previously implemented: compute binary entropy with historical p
                    let p = if total_parallelograms[context] == 0 {
                        0.0
                    } else {
                        total_used_parallelograms[context] as f64
                            / total_parallelograms[context] as f64
                    };
                    let p = p.clamp(0.001, 0.999);
                    let num_bits_local = num_parallelograms as i64;
                    let num_ones = num_used as i64;
                    let num_zeros = num_bits_local - num_ones;
                    let local_cost =
                        -(num_ones as f64) * p.log2() - (num_zeros as f64) * (1.0 - p).log2();
                    let local_cost_bits = local_cost.ceil() as i64;
                    tmp_err.num_bits += local_cost_bits;
                    if tmp_err < marginal_best_error {
                        marginal_best_error = tmp_err;
                        _marginal_best_config = config as u8;
                    }
                }
            }

            // Apply best config - update total_used_parallelograms (total_parallelograms already updated above)
            // C++ updates total_used_parallelograms AFTER choosing the best config
            total_used_parallelograms[context] += best_num_used as i64;

            for i in 0..num_parallelograms {
                let is_used = (best_config & (1 << i)) != 0;
                // is_crease_edge stores true if NOT used (crease).
                self.is_crease_edge[context].push(!is_used);
            }

            // Recompute prediction for best config and update output/tracker
            if best_num_used == 0 {
                predicted_val.fill(DataType::default());
                if data_id > 0 {
                    let prev_offset = (data_id - 1) * num_components;
                    for c in 0..num_components {
                        predicted_val[c] = in_data[prev_offset + c];
                    }
                }

                corr_val.fill(CorrType::default());
                self.transform.compute_correction(
                    &in_data[data_offset..data_offset + num_components],
                    &predicted_val,
                    &mut corr_val,
                );
                for c in 0..num_components {
                    out_corr[data_offset + c] = corr_val[c];
                    // For entropy tracking, C++ uses predicted - actual
                    // (opposite of what the transform uses for correction)
                    let val = in_data[data_offset + c].into();
                    let pred = predicted_val[c].into();
                    let dif = pred - val; // predicted - actual, like C++
                    entropy_symbols[c] = Self::convert_signed_int_to_symbol(dif);
                }
            } else {
                // Encoder must use same accumulation as decoder: AddAsUnsigned (wrapping add)
                for k in 0..num_components {
                    let mut sum: i32 = 0;
                    for i in 0..num_parallelograms {
                        if (best_config & (1 << i)) != 0 {
                            let pred_val: i64 = pred_vals[i][k].into();
                            // AddAsUnsigned: convert to unsigned, add, convert back
                            sum = (sum as u32).wrapping_add(pred_val as u32) as i32;
                        }
                    }
                    // C++ uses truncating integer division (not rounding)
                    let val = sum / best_num_used as i32;
                    multi_pred_vals[k] = DataType::from(val);
                }

                corr_val.fill(CorrType::default());
                self.transform.compute_correction(
                    &in_data[data_offset..data_offset + num_components],
                    &multi_pred_vals,
                    &mut corr_val,
                );
                for c in 0..num_components {
                    out_corr[data_offset + c] = corr_val[c];
                    // For entropy tracking, C++ uses predicted - actual
                    // (opposite of what the transform uses for correction)
                    let val = in_data[data_offset + c].into();
                    let pred = multi_pred_vals[c].into();
                    let dif = pred - val; // predicted - actual, like C++
                    entropy_symbols[c] = Self::convert_signed_int_to_symbol(dif);
                }
            }
            self.entropy_tracker.push(&entropy_symbols);
        }

        // First element is always fixed because it cannot be predicted.
        // Use zero prediction like C++ does.
        predicted_val.fill(DataType::default());
        corr_val.fill(CorrType::default());
        self.transform.compute_correction(
            &in_data[0..num_components],
            &predicted_val,
            &mut corr_val,
        );
        for c in 0..num_components {
            out_corr[c] = corr_val[c];
        }

        true
    }

    fn encode_prediction_data(&mut self, buffer: &mut Vec<u8>) -> bool {
        let mut enc = EncoderBuffer::new();

        // C++ bitstream order: crease edges FIRST, then transform data.
        // Encode crease edges.
        for i in 0..MAX_NUM_PARALLELOGRAMS {
            let num_flags = self.is_crease_edge[i].len() as u32;
            enc.encode_varint(num_flags as u64);

            if num_flags > 0 {
                let mut ans_encoder = RAnsBitEncoder::new();
                ans_encoder.start_encoding();

                // C++ encoder processes vertices BACKWARD (high to low) and writes flags
                // in REVERSE order (from last group to first). Since ANS is LIFO, this
                // results in flags decoding in the order they were collected (backward),
                // which is the same order the decoder expects.
                //
                // Rust encoder now also iterates backward, so is_crease_edge is in same order as C++.
                // We must write in reverse order like C++ to match the bitstream exactly.
                //
                // |i| is the context = num_parallelograms - 1, so num_used = i + 1
                let num_used_parallelograms = i + 1;
                let flags = &self.is_crease_edge[i];

                // Write flags in reverse order: start from last group, step backward
                let mut j = flags.len() as i32 - num_used_parallelograms as i32;
                while j >= 0 {
                    for k in 0..num_used_parallelograms {
                        ans_encoder.encode_bit(flags[(j as usize) + k]);
                    }
                    j -= num_used_parallelograms as i32;
                }

                ans_encoder.end_encoding(&mut enc);
            }
        }

        // Encode underlying transform data second (e.g. Wrap min/max bounds).
        let mut transform_data = Vec::new();
        if !self.transform.encode_transform_data(&mut transform_data) {
            return false;
        }
        enc.encode_data(&transform_data);

        buffer.extend_from_slice(enc.data());
        true
    }
}

#[cfg(feature = "encoder")]
impl<'a, DataType, CorrType, Transform>
    MeshPredictionSchemeConstrainedMultiParallelogramEncoder<'a, DataType, CorrType, Transform>
{
    /// Computes the total cumulative overhead bits for the entire overhead stream.
    /// This matches C++ ComputeOverheadBits() which returns:
    ///   ceil(total_parallelograms * binary_shannon_entropy(total_used / total))
    ///
    /// The key insight is that C++ computes the TOTAL bits needed to encode ALL
    /// overhead flags seen so far, not just the marginal cost of the current vertex.
    fn compute_overhead_bits(total_used_parallelograms: i64, total_parallelograms: i64) -> i64 {
        // C++ uses ComputeBinaryShannonEntropy and then multiplies by total_parallelograms
        let entropy = crate::shannon_entropy::compute_binary_shannon_entropy(
            total_parallelograms as u32,
            total_used_parallelograms as u32,
        );
        // Round up to the nearest full bit.
        ((total_parallelograms as f64) * entropy).ceil() as i64
    }
}

#[cfg(feature = "decoder")]
pub struct MeshPredictionSchemeConstrainedMultiParallelogramDecoder<
    'a,
    DataType,
    CorrType,
    Transform,
> {
    mesh_data: MeshPredictionSchemeData<'a>,
    transform: Transform,
    is_crease_edge: [Vec<bool>; MAX_NUM_PARALLELOGRAMS],
    _marker: PhantomData<(DataType, CorrType)>,
}

#[cfg(feature = "decoder")]
impl<'a, DataType, CorrType, Transform>
    MeshPredictionSchemeConstrainedMultiParallelogramDecoder<'a, DataType, CorrType, Transform>
where
    Transform: PredictionSchemeDecodingTransform<DataType, CorrType>,
{
    pub fn new(transform: Transform, mesh_data: MeshPredictionSchemeData<'a>) -> Self {
        Self {
            mesh_data,
            transform,
            is_crease_edge: Default::default(),
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "decoder")]
impl<'a, DataType, CorrType, Transform> PredictionScheme<'a>
    for MeshPredictionSchemeConstrainedMultiParallelogramDecoder<'a, DataType, CorrType, Transform>
where
    Transform: PredictionSchemeDecodingTransform<DataType, CorrType>,
{
    fn get_prediction_method(&self) -> PredictionSchemeMethod {
        PredictionSchemeMethod::MeshPredictionConstrainedMultiParallelogram
    }

    fn is_initialized(&self) -> bool {
        self.mesh_data.corner_table().is_some()
    }

    fn get_num_parent_attributes(&self) -> i32 {
        0
    }

    fn get_parent_attribute_type(
        &self,
        _i: i32,
    ) -> crate::geometry_attribute::GeometryAttributeType {
        crate::geometry_attribute::GeometryAttributeType::Generic
    }

    fn set_parent_attribute(&mut self, _att: &'a PointAttribute) -> bool {
        false
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }
}

#[cfg(feature = "decoder")]
impl<'a, DataType, CorrType, Transform> PredictionSchemeDecoder<'a, DataType, CorrType>
    for MeshPredictionSchemeConstrainedMultiParallelogramDecoder<'a, DataType, CorrType, Transform>
where
    DataType: ParallelogramDataType + Into<i64> + Copy + Default + From<i32>,
    CorrType: Copy + Default + From<DataType> + std::ops::Sub<Output = CorrType> + From<i32>,
    Transform: PredictionSchemeDecodingTransform<DataType, CorrType>,
    i64: From<DataType>,
{
    fn decode_prediction_data(&mut self, buffer: &mut DecoderBuffer) -> bool {
        // Draco bitstream order (see C++ MeshPredictionSchemeConstrainedMultiParallelogramDecoder):
        // 1) (optional) mode for < v2.2
        // 2) crease-edge flag streams
        // 3) underlying transform data (e.g. Wrap bounds)

        // Decode crease edges.
        let corner_table = match self.mesh_data.corner_table() {
            Some(ct) => ct,
            None => {
                return false;
            }
        };

        for i in 0..MAX_NUM_PARALLELOGRAMS {
            let num_flags = match buffer.decode_varint() {
                Ok(v) => v as u32,
                Err(_) => return false,
            };

            if num_flags > corner_table.num_corners() as u32 {
                return false;
            }

            if num_flags > 0 {
                self.is_crease_edge[i].resize(num_flags as usize, false);
                let mut ans_decoder = RAnsBitDecoder::new();
                if !ans_decoder.start_decoding(buffer) {
                    return false;
                }
                for j in 0..num_flags {
                    self.is_crease_edge[i][j as usize] = ans_decoder.decode_next_bit();
                }
                ans_decoder.end_decoding();
            }
        }

        // Decode underlying transform data last (e.g. Wrap min/max bounds).
        if !self.transform.decode_transform_data(buffer) {
            return false;
        }
        true
    }

    fn compute_original_values(
        &mut self,
        in_corr: &[CorrType],
        out_data: &mut [DataType],
        size: usize,
        num_components: usize,
        _entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> bool {
        self.transform.init(num_components);

        if size == 0 {
            return true;
        }
        if num_components == 0 || !size.is_multiple_of(num_components) {
            return false;
        }
        if size < num_components {
            return false;
        }
        let num_entries = size / num_components;

        let Some(corner_table) = self.mesh_data.corner_table() else {
            return false;
        };
        let Some(vertex_to_data_map) = self.mesh_data.vertex_to_data_map() else {
            return false;
        };
        if in_corr.len() < size || out_data.len() < size {
            return false;
        }

        let mut multi_pred_vals = vec![DataType::default(); num_components];
        let zero_vals = vec![DataType::default(); num_components];
        let mut predicted_val = vec![DataType::default(); num_components];

        // Current position in is_crease_edge
        let mut is_crease_edge_pos = [0usize; MAX_NUM_PARALLELOGRAMS];

        // First value
        if size > 0 {
            self.transform.compute_original_value(
                &zero_vals,
                &in_corr[0..num_components],
                &mut out_data[0..num_components],
            );
        }

        for data_id in 1..num_entries {
            let data_offset = data_id * num_components;

            let corner_id = if let Some(map) = self.mesh_data.data_to_corner_map() {
                if data_id < map.len() {
                    CornerIndex(map[data_id])
                } else {
                    INVALID_CORNER_INDEX
                }
            } else if data_id < corner_table.num_vertices() {
                corner_table.left_most_corner(crate::geometry_indices::VertexIndex(data_id as u32))
            } else {
                INVALID_CORNER_INDEX
            };

            if corner_id == INVALID_CORNER_INDEX {
                let prev_offset = (data_id - 1) * num_components;
                predicted_val.fill(DataType::default());
                for c in 0..num_components {
                    predicted_val[c] = out_data[prev_offset + c];
                }
                self.transform.compute_original_value(
                    &predicted_val,
                    &in_corr[data_offset..data_offset + num_components],
                    &mut out_data[data_offset..data_offset + num_components],
                );
                continue;
            }

            let mut corners = [INVALID_CORNER_INDEX; MAX_NUM_PARALLELOGRAMS];
            let mut num_parallelograms = 0;

            let start_c = corner_id;
            let mut c = start_c;
            let mut first_pass = true;
            while c != INVALID_CORNER_INDEX {
                let opp = corner_table.opposite(c);
                if opp != INVALID_CORNER_INDEX {
                    let opp_v = corner_table.vertex(opp);
                    // Match C++ ComputeParallelogramPrediction(): next/prev must be
                    // taken from the opposite corner (oci), not from |c|.
                    let next_v = corner_table.vertex(corner_table.next(opp));
                    let prev_v = corner_table.vertex(corner_table.previous(opp));

                    let opp_data_id = *vertex_to_data_map.get(opp_v.0 as usize).unwrap_or(&-1);
                    let next_data_id = *vertex_to_data_map.get(next_v.0 as usize).unwrap_or(&-1);
                    let prev_data_id = *vertex_to_data_map.get(prev_v.0 as usize).unwrap_or(&-1);

                    if opp_data_id != -1
                        && next_data_id != -1
                        && prev_data_id != -1
                        && (opp_data_id as usize) < data_id
                        && (next_data_id as usize) < data_id
                        && (prev_data_id as usize) < data_id
                        && num_parallelograms < MAX_NUM_PARALLELOGRAMS
                    {
                        corners[num_parallelograms] = c;
                        num_parallelograms += 1;
                        if num_parallelograms == MAX_NUM_PARALLELOGRAMS {
                            break;
                        }
                    }
                }

                // Proceed to the next corner attached to the vertex.
                c = if first_pass {
                    corner_table.swing_left(c)
                } else {
                    corner_table.swing_right(c)
                };
                if c == start_c {
                    break;
                }
                if c == INVALID_CORNER_INDEX && first_pass {
                    first_pass = false;
                    c = corner_table.swing_right(start_c);
                }
            }

            let mut num_used_parallelograms = 0;
            if num_parallelograms > 0 {
                for k in 0..num_components {
                    multi_pred_vals[k] = DataType::default();
                }

                for i in 0..num_parallelograms {
                    let context = num_parallelograms - 1;
                    let pos = is_crease_edge_pos[context];
                    is_crease_edge_pos[context] += 1;

                    // Check bounds - if we've exhausted the flags, this is an error
                    if pos >= self.is_crease_edge[context].len() {
                        // This should never happen if encoder/decoder are in sync
                        eprintln!("ERROR: is_crease_edge bounds exceeded: pos={} >= len={}, context={}, data_id={}", 
                            pos, self.is_crease_edge[context].len(), context, data_id);
                        return false;
                    }
                    let is_crease = self.is_crease_edge[context][pos];

                    if !is_crease {
                        // Compute prediction for this parallelogram
                        let ci = corners[i];
                        let oci = corner_table.opposite(ci);
                        let Some(&vert_opp) =
                            vertex_to_data_map.get(corner_table.vertex(oci).0 as usize)
                        else {
                            return false;
                        };
                        let Some(&vert_next) = vertex_to_data_map
                            .get(corner_table.vertex(corner_table.next(oci)).0 as usize)
                        else {
                            return false;
                        };
                        let Some(&vert_prev) = vertex_to_data_map
                            .get(corner_table.vertex(corner_table.previous(oci)).0 as usize)
                        else {
                            return false;
                        };
                        if vert_opp < 0 || vert_next < 0 || vert_prev < 0 {
                            return false;
                        }

                        let v_opp_off = (vert_opp as usize) * num_components;
                        let v_next_off = (vert_next as usize) * num_components;
                        let v_prev_off = (vert_prev as usize) * num_components;
                        if v_opp_off + num_components > out_data.len()
                            || v_next_off + num_components > out_data.len()
                            || v_prev_off + num_components > out_data.len()
                        {
                            return false;
                        }

                        for k in 0..num_components {
                            let p = DataType::compute_parallelogram_prediction(
                                out_data[v_next_off + k],
                                out_data[v_prev_off + k],
                                out_data[v_opp_off + k],
                            );
                            // Use add_as_unsigned for C++ compatible accumulation
                            multi_pred_vals[k] = DataType::add_as_unsigned(multi_pred_vals[k], p);
                        }
                        num_used_parallelograms += 1;
                    }
                }
            }

            if num_used_parallelograms == 0 {
                let prev_offset = (data_id - 1) * num_components;
                predicted_val.fill(DataType::default());
                for c in 0..num_components {
                    predicted_val[c] = out_data[prev_offset + c];
                }
                self.transform.compute_original_value(
                    &predicted_val,
                    &in_corr[data_offset..data_offset + num_components],
                    &mut out_data[data_offset..data_offset + num_components],
                );
            } else {
                // C++ decoder uses truncating integer division (not rounding)
                for c in 0..num_components {
                    let val: i64 = multi_pred_vals[c].into();
                    let averaged = (val / num_used_parallelograms as i64) as i32;
                    multi_pred_vals[c] = DataType::from(averaged);
                }
                self.transform.compute_original_value(
                    &multi_pred_vals,
                    &in_corr[data_offset..data_offset + num_components],
                    &mut out_data[data_offset..data_offset + num_components],
                );
            }
        }
        true
    }
}
