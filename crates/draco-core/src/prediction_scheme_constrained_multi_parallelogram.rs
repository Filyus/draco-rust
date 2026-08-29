//! Constrained multi-parallelogram mesh predictor.
//!
//! Like multi-parallelogram, but the encoder selects, per vertex, the best
//! subset of incident triangles and codes the choice as crease/selection flags
//! so the decoder reproduces it exactly. Draco's highest-quality mesh position
//! predictor. Port of Draco's
//! `prediction_scheme_constrained_multi_parallelogram_*`.

use crate::geometry_indices::{CornerIndex, INVALID_CORNER_INDEX};
use crate::mesh_prediction_scheme_data::MeshPredictionSchemeData;
use crate::portable_attribute::PredictionParent;
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
use crate::status::{DracoError, Status};

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
    /// Target bitstream version (0 = default/2.2). Pre-2.2 crease-edge rANS
    /// streams need a fixed-u32 size prefix instead of a varint.
    bitstream_version: u16,
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
            bitstream_version: 0,
            _marker: PhantomData,
        }
    }

    /// Sets the target bitstream version so crease-edge rANS streams use the
    /// correct (pre-2.2 u32 vs 2.2+ varint) size-prefix encoding.
    pub fn set_bitstream_version(&mut self, version: u16) {
        self.bitstream_version = version;
    }

    /// Zig-zags a residual for the entropy estimate, as C++
    /// `ConvertSignedIntToSymbol` does.
    ///
    /// Stated the way upstream states it - map -1 to 0, -2 to 1, then shift and
    /// set the low bit - rather than as `(-val << 1) - 1`. The two agree on
    /// every value either can represent, but negating first overflows on
    /// `i64::MIN`, which a residual reaches when the attribute spans the full
    /// range: the encoder panicked in a debug build on a mesh C++ encodes
    /// without complaint.
    fn convert_signed_int_to_symbol(val: i64) -> u32 {
        if val >= 0 {
            (val as u32) << 1
        } else {
            let magnitude = -(val.wrapping_add(1));
            ((magnitude as u32) << 1) | 1
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

    fn set_parent_attribute(&mut self, _parent: PredictionParent<'a>) -> Status {
        Err(DracoError::invalid_parameter(
            "The constrained multi-parallelogram prediction scheme takes no parent attribute"
                .to_string(),
        ))
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
/// Advances `arr` to its next lexicographic permutation in place (`false` <
/// `true`), matching `std::next_permutation`. Returns `false` and leaves
/// `arr` sorted ascending when the sequence was already the last
/// permutation.
///
/// The encoder's config search needs this exact ordering, not just the same
/// *set* of configurations: ties in encoded cost between two configurations
/// are broken by whichever the search visits first, and the config, not just
/// its cost, is itself part of the encoded stream (which parallelogram edges
/// are marked as creases). Enumerating configurations in a different order --
/// bitmask-ascending, say -- picks a different, equally valid, wrong-for-parity
/// winner whenever a tie occurs.
#[cfg(feature = "encoder")]
fn next_permutation(arr: &mut [bool]) -> bool {
    if arr.len() < 2 {
        return false;
    }
    let mut i = arr.len() - 1;
    loop {
        if i == 0 {
            arr.reverse();
            return false;
        }
        i -= 1;
        // arr[i] < arr[i + 1], spelled out since clippy reads `<` on bool as
        // a mistake -- here it is exactly the false-before-true order the
        // permutation needs.
        if !arr[i] && arr[i + 1] {
            break;
        }
    }
    let mut j = arr.len() - 1;
    while arr[j] <= arr[i] {
        j -= 1;
    }
    arr.swap(i, j);
    arr[i + 1..].reverse();
    true
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
    ) -> Status {
        self.transform.init(in_data, size, num_components);

        if num_components == 0 || !size.is_multiple_of(num_components) {
            return Err(DracoError::invalid_parameter(format!(
                "{size} values do not divide into {num_components} components"
            )));
        }
        if size == 0 {
            // No values, so no entry 0 for the tail of this function to encode.
            return Ok(());
        }
        let num_entries = size / num_components;

        let missing = |what: &str| {
            DracoError::general(format!(
                "Constrained multi-parallelogram prediction has no {what}"
            ))
        };
        let Some(corner_table) = self.mesh_data.corner_table() else {
            return Err(missing("corner table"));
        };
        let Some(vertex_to_data_map) = self.mesh_data.vertex_to_data_map() else {
            return Err(missing("vertex-to-data map"));
        };

        for i in 0..MAX_NUM_PARALLELOGRAMS {
            self.is_crease_edge[i].clear();
        }

        let mut pred_vals = vec![vec![DataType::default(); num_components]; MAX_NUM_PARALLELOGRAMS];
        let mut multi_pred_vals = vec![DataType::default(); num_components];
        let mut entropy_symbols = vec![0u32; num_components];
        let mut predicted_val = vec![DataType::default(); num_components];
        let mut corr_val = vec![CorrType::default(); num_components];

        // Track total parallelograms and used parallelograms for overhead calculation
        let mut total_parallelograms: [i64; MAX_NUM_PARALLELOGRAMS] = [0; MAX_NUM_PARALLELOGRAMS];
        let mut total_used_parallelograms: [i64; MAX_NUM_PARALLELOGRAMS] =
            [0; MAX_NUM_PARALLELOGRAMS];
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
                    // An entry that predicted nothing still feeds the tracker.
                    // It is one running symbol history for the whole attribute,
                    // and every later entry picks its configuration by the cost
                    // this history estimates; skipping the entries that fell
                    // back to delta would score those choices against a history
                    // that is not the one being written.
                    //
                    // The difference is recomputed from the value and the
                    // prediction rather than read out of `out_corr`: the
                    // tracker counts symbols built from `i64`, and `DataType`
                    // is the half of the pair that converts.
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
            let mut swing_steps = 0usize;
            let max_swing_steps = corner_table.num_corners().saturating_add(1);
            while c != INVALID_CORNER_INDEX {
                swing_steps += 1;
                if swing_steps > max_swing_steps {
                    return Err(DracoError::general(
                        "Corner fan does not close after every corner was visited".to_string(),
                    ));
                }
                let opp = corner_table.opposite(c);
                if opp != INVALID_CORNER_INDEX {
                    let opp_v = corner_table.vertex(opp);
                    // Match C++ ComputeParallelogramPrediction(): next/prev must be
                    // taken from the opposite corner (oci), not from |c|.
                    let next_v = corner_table.vertex_after(opp);
                    let prev_v = corner_table.vertex_before(opp);

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
                let vert_next = vertex_to_data_map[corner_table.vertex_after(oci).0 as usize];
                let vert_prev = vertex_to_data_map[corner_table.vertex_before(oci).0 as usize];

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

            // Set from the delta baseline immediately below -- there is always
            // at least that one candidate, so no placeholder value is needed.
            let mut best_error;
            let mut best_config;
            let mut best_num_used;

            // C++ increments total_parallelograms BEFORE evaluating any configurations.
            // This is critical for matching the overhead calculation exactly.
            let context = num_parallelograms - 1;
            total_parallelograms[context] += num_parallelograms as i64;

            // Delta prediction (no parallelogram used), evaluated once up front --
            // matches C++'s baseline computed before the config search, not
            // config 0 of a bitmask loop.
            {
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
                error.num_bits = self.entropy_tracker.number_of_data_bits(&entropy_data)
                    + ShannonEntropyTracker::get_number_of_r_ans_table_bits_static(&entropy_data);

                // For the baseline: no parallelograms used, so total_used stays
                // the same.
                let overhead_bits = Self::compute_overhead_bits(
                    total_used_parallelograms[context],
                    total_parallelograms[context],
                );
                error.num_bits += overhead_bits;

                best_error = error;
                best_config = 0;
                best_num_used = 0;
            }

            // Multi-parallelogram configurations, searched by increasing count of
            // parallelograms used and, within each count, in the same
            // next_permutation order C++ visits them in -- see next_permutation's
            // doc comment for why the order itself, not just the set of configs
            // it covers, has to match.
            // Bounded by MAX_NUM_PARALLELOGRAMS, and this runs once per
            // vertex: a `Vec` here is an allocation per vertex for at most
            // four bools.
            let mut excluded_storage = [true; MAX_NUM_PARALLELOGRAMS];
            let excluded = &mut excluded_storage[..num_parallelograms];
            for num_used in 1..=num_parallelograms {
                for slot in excluded.iter_mut().take(num_used) {
                    *slot = false;
                }
                for slot in excluded.iter_mut().skip(num_used) {
                    *slot = true;
                }

                // Every config with this many parallelograms used pays the same
                // overhead, so it is computed once per count rather than once
                // per permutation -- it costs two logarithms.
                let overhead_bits = Self::compute_overhead_bits(
                    total_used_parallelograms[context] + num_used as i64,
                    total_parallelograms[context],
                );

                loop {
                    let mut config: u32 = 0;
                    for (i, &is_excluded) in excluded.iter().enumerate() {
                        if !is_excluded {
                            config |= 1 << i;
                        }
                    }

                    // Encoder must use same accumulation as decoder: AddAsUnsigned
                    // (wrapping add).
                    for k in 0..num_components {
                        let mut sum: i32 = 0;
                        for i in 0..num_parallelograms {
                            if (config & (1 << i)) != 0 {
                                let pred_val: i64 = pred_vals[i][k].into();
                                sum = (sum as u32).wrapping_add(pred_val as u32) as i32;
                            }
                        }
                        // C++ uses truncating integer division (not rounding).
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
                    error.num_bits = self.entropy_tracker.number_of_data_bits(&entropy_data)
                        + ShannonEntropyTracker::get_number_of_r_ans_table_bits_static(
                            &entropy_data,
                        );

                    // Overhead bits assuming this config is chosen: total_used
                    // increased by num_used, which is what the hoisted estimate
                    // above already assumes.
                    error.num_bits += overhead_bits;

                    if error < best_error {
                        best_error = error;
                        best_config = config as u8;
                        best_num_used = num_used as i32;
                    }

                    if !next_permutation(excluded) {
                        break;
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
                    let val = sum / best_num_used;
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

        Ok(())
    }

    fn encode_prediction_data(&mut self, buffer: &mut Vec<u8>) -> Status {
        let mut enc = EncoderBuffer::new();
        // Propagate the target version so crease-edge rANS streams pick the right
        // size-prefix encoding (pre-2.2 u32 vs 2.2+ varint).
        enc.set_version(
            (self.bitstream_version >> 8) as u8,
            (self.bitstream_version & 0xff) as u8,
        );

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
        self.transform.encode_transform_data(&mut transform_data)?;
        enc.encode_data(&transform_data);

        buffer.extend_from_slice(enc.data());
        Ok(())
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
pub struct MeshPredictionSchemeConstrainedMultiParallelogramDecoder<'a, DataType, Transform> {
    mesh_data: MeshPredictionSchemeData<'a>,
    transform: Transform,
    is_crease_edge: [Vec<bool>; MAX_NUM_PARALLELOGRAMS],
    _marker: PhantomData<DataType>,
}

#[cfg(feature = "decoder")]
impl<'a, DataType, Transform>
    MeshPredictionSchemeConstrainedMultiParallelogramDecoder<'a, DataType, Transform>
where
    Transform: PredictionSchemeDecodingTransform<DataType>,
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
impl<'a, DataType, Transform> PredictionScheme<'a>
    for MeshPredictionSchemeConstrainedMultiParallelogramDecoder<'a, DataType, Transform>
where
    Transform: PredictionSchemeDecodingTransform<DataType>,
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

    fn set_parent_attribute(&mut self, _parent: PredictionParent<'a>) -> Status {
        Err(DracoError::invalid_parameter(
            "The constrained multi-parallelogram prediction scheme takes no parent attribute"
                .to_string(),
        ))
    }

    fn get_transform_type(&self) -> PredictionSchemeTransformType {
        self.transform.get_type()
    }
}

#[cfg(feature = "decoder")]
impl<'a, DataType, Transform> PredictionSchemeDecoder<'a, DataType>
    for MeshPredictionSchemeConstrainedMultiParallelogramDecoder<'a, DataType, Transform>
where
    DataType: ParallelogramDataType + Into<i64> + Copy + Default + From<i32>,
    Transform: PredictionSchemeDecodingTransform<DataType>,
    i64: From<DataType>,
{
    fn decode_prediction_data(&mut self, buffer: &mut DecoderBuffer) -> Status {
        // Draco bitstream order (see C++ MeshPredictionSchemeConstrainedMultiParallelogramDecoder):
        // 1) (optional) mode for < v2.2
        // 2) crease-edge flag streams
        // 3) underlying transform data (e.g. Wrap bounds)

        // Pre-2.2 streams prefix a prediction-mode byte (only the optimal
        // multi-parallelogram mode is supported). 2.2+ dropped it; without this
        // read the mode byte is consumed as the first context's flag count,
        // leaving every crease-edge stream empty.
        #[cfg(feature = "legacy_bitstream_decode")]
        {
            let bitstream_version = buffer.bitstream_version();
            if bitstream_version < 0x0202 {
                match buffer.decode_u8() {
                    Ok(0) => {} // OPTIMAL_MULTI_PARALLELOGRAM
                    Ok(mode) => {
                        return Err(DracoError::unsupported_feature(format!(
                            "Constrained multi-parallelogram prediction mode {mode}"
                        )))
                    }
                    Err(_) => {
                        return Err(DracoError::buffer(
                            "Stream ends before the pre-2.2 prediction mode byte".to_string(),
                        ))
                    }
                }
            }
        }

        // Decode crease edges.
        let Some(corner_table) = self.mesh_data.corner_table() else {
            return Err(DracoError::general(
                "Constrained multi-parallelogram prediction has no corner table".to_string(),
            ));
        };

        for i in 0..MAX_NUM_PARALLELOGRAMS {
            let num_flags = buffer.decode_varint().map_err(|_| {
                DracoError::buffer(format!(
                    "Stream ends before the crease-edge flag count for context {i}"
                ))
            })? as u32;

            if num_flags > corner_table.num_corners() as u32 {
                return Err(DracoError::general(format!(
                    "Context {i} declares {num_flags} crease-edge flags, more than the {} corners",
                    corner_table.num_corners()
                )));
            }

            if num_flags > 0 {
                self.is_crease_edge[i].resize(num_flags as usize, false);
                let mut ans_decoder = RAnsBitDecoder::new();
                if !ans_decoder.start_decoding(buffer) {
                    return Err(DracoError::buffer(format!(
                        "Crease-edge rANS stream for context {i} is truncated"
                    )));
                }
                for j in 0..num_flags {
                    self.is_crease_edge[i][j as usize] = ans_decoder.decode_next_bit();
                }
                ans_decoder.end_decoding();
            }
        }

        // Decode underlying transform data last (e.g. Wrap min/max bounds).
        self.transform.decode_transform_data(buffer)
    }

    fn compute_original_values(
        &mut self,
        data: &mut [DataType],
        size: usize,
        num_components: usize,
        _entry_to_point_id_map: Option<crate::prediction_scheme::EntryToPointIdMap<'_>>,
    ) -> Status {
        self.transform.init(num_components)?;

        if size == 0 {
            return Ok(());
        }
        if num_components == 0 || !size.is_multiple_of(num_components) || size < num_components {
            return Err(DracoError::invalid_parameter(format!(
                "{size} values do not divide into {num_components} components"
            )));
        }
        let num_entries = size / num_components;

        let missing = |what: &str| {
            DracoError::general(format!(
                "Constrained multi-parallelogram prediction has no {what}"
            ))
        };
        let Some(corner_table) = self.mesh_data.corner_table() else {
            return Err(missing("corner table"));
        };
        let Some(vertex_to_data_map) = self.mesh_data.vertex_to_data_map() else {
            return Err(missing("vertex-to-data map"));
        };
        if data.len() < size {
            return Err(DracoError::general(format!(
                "Constrained multi-parallelogram prediction needs {size} values, has {}",
                data.len()
            )));
        }

        // The qualifying test in the fan walk below reads a data id as a `u32`,
        // which is what collapses "not decoded yet" (`-1`) and "not before this
        // entry" into one unsigned comparison. That reading needs `data_id` to
        // fit in a `u32` as well; a map of `i32` data ids cannot describe more
        // entries than that in the first place, so this rejects only a buffer
        // no stream could have produced.
        if num_entries > u32::MAX as usize {
            return Err(DracoError::general(format!(
                "Constrained multi-parallelogram prediction cannot index {num_entries} entries"
            )));
        }

        let mut multi_pred_vals = vec![DataType::default(); num_components];
        let zero_vals = vec![DataType::default(); num_components];
        let mut predicted_val = vec![DataType::default(); num_components];

        // Current position in is_crease_edge
        let mut is_crease_edge_pos = [0usize; MAX_NUM_PARALLELOGRAMS];

        // First value
        if size > 0 {
            self.transform
                .compute_original_value(&zero_vals, &mut data[0..num_components]);
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
                // The fill wrote a default into every component the copy below
                // then overwrote, and the copy walked the two buffers a
                // component at a time with a bounds check on each. Both are one
                // equal-length slice copy: `predicted_val` was sized to
                // `num_components` and `prev_offset` is the previous entry, so
                // the two slices are the same length by construction.
                predicted_val.copy_from_slice(&data[prev_offset..prev_offset + num_components]);
                self.transform.compute_original_value(
                    &predicted_val,
                    &mut data[data_offset..data_offset + num_components],
                );
                continue;
            }

            // (opp, next, prev) data ids for the qualifying corner's opposite
            // corner, resolved once by the test below and carried forward so the
            // prediction pass further down can read them back instead of asking
            // the corner table and `vertex_to_data_map` again. The corner itself
            // was recorded here too and never read back: the pass below works
            // entirely from these three ids.
            let mut corner_data_ids = [(0u32, 0u32, 0u32); MAX_NUM_PARALLELOGRAMS];
            let mut num_parallelograms = 0;

            let start_c = corner_id;
            let mut c = start_c;
            let mut first_pass = true;
            let mut swing_steps = 0usize;
            let max_swing_steps = corner_table.num_corners().saturating_add(1);
            while c != INVALID_CORNER_INDEX {
                swing_steps += 1;
                if swing_steps > max_swing_steps {
                    return Err(DracoError::general(
                        "Corner fan does not close after every corner was visited".to_string(),
                    ));
                }
                let opp = corner_table.opposite(c);
                if opp != INVALID_CORNER_INDEX {
                    let opp_v = corner_table.vertex(opp);
                    // Match C++ ComputeParallelogramPrediction(): next/prev must be
                    // taken from the opposite corner (oci), not from |c|.
                    let next_v = corner_table.vertex_after(opp);
                    let prev_v = corner_table.vertex_before(opp);

                    let opp_data_id = *vertex_to_data_map.get(opp_v.0 as usize).unwrap_or(&-1);
                    let next_data_id = *vertex_to_data_map.get(next_v.0 as usize).unwrap_or(&-1);
                    let prev_data_id = *vertex_to_data_map.get(prev_v.0 as usize).unwrap_or(&-1);

                    // Six data-dependent branches asked one question: are all
                    // three neighbours decoded, and decoded before this entry.
                    // `-1` is the only value the map holds for "not decoded",
                    // and read as a `u32` it is `4,294,967,295` -- past every
                    // data id, since `data_id` is below `num_entries` and that
                    // was bounded to a `u32` above. So `x >= 0 && x < data_id`
                    // is exactly `(x as u32) < data_id as u32`, and the largest
                    // of the three answers for all three. Two maxima and one
                    // comparison, all of it `cmov`, where the conjunction had
                    // six branches on decoded data with no pattern to learn.
                    let newest = (opp_data_id as u32)
                        .max(next_data_id as u32)
                        .max(prev_data_id as u32);
                    if newest < data_id as u32 && num_parallelograms < MAX_NUM_PARALLELOGRAMS {
                        corner_data_ids[num_parallelograms] =
                            (opp_data_id as u32, next_data_id as u32, prev_data_id as u32);
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
                // No zeroing pass: the first parallelogram that is used *sets*
                // the accumulator and the rest add to it. A sum of wrapping
                // adds does not care whether it started at zero, and the zero
                // it started at cost a `memset` **call** per entry -- `9,213`
                // of them in a speed-0 grid decode, for twelve bytes each,
                // because LLVM recognises a zero-fill over a runtime-length
                // `Copy` slice and calls out to it whether it is written as
                // `fill`, as an indexed loop, or as `iter_mut`. If every
                // parallelogram turns out to be a crease the accumulator is
                // never written, and it is never read either: that is the
                // `num_used_parallelograms == 0` arm below.

                // The context is the parallelogram count, so it is fixed for
                // the whole of this entry: the flag row and the position in it
                // are read once here instead of re-indexed on every
                // parallelogram, which cost a bounds check on the outer array
                // and a reload of the counter each time round.
                let context = num_parallelograms - 1;
                let creases = &self.is_crease_edge[context];
                let first_pos = is_crease_edge_pos[context];
                is_crease_edge_pos[context] += num_parallelograms;
                if first_pos + num_parallelograms > creases.len() {
                    // This should never happen if encoder/decoder are in sync
                    debug_log!("ERROR: is_crease_edge bounds exceeded: pos={} >= len={}, context={}, data_id={}",
                        first_pos, creases.len(), context, data_id);
                    return Err(DracoError::general(format!(
                        "Entry {data_id} reads crease-edge flag {} of {} in context {context}",
                        first_pos + num_parallelograms - 1,
                        creases.len()
                    )));
                }

                for (i, &is_crease) in creases[first_pos..first_pos + num_parallelograms]
                    .iter()
                    .enumerate()
                {
                    if !is_crease {
                        // Compute prediction for this parallelogram. The
                        // qualifying pass above already asked the corner
                        // table and vertex_to_data_map for `oci` and its
                        // three neighbour vertices' data ids -- neither
                        // changes between the two passes, so the answer here
                        // is provably the one stored then, not a fresh one.
                        // They are held as `u32` because that pass proved all
                        // three non-negative, which is why no test for that
                        // stands here.
                        let (vert_opp, vert_next, vert_prev) = corner_data_ids[i];

                        let v_opp_off = (vert_opp as usize) * num_components;
                        let v_next_off = (vert_next as usize) * num_components;
                        let v_prev_off = (vert_prev as usize) * num_components;
                        // One comparison for the three: the widest offset is
                        // the only one that can reach past the end.
                        if v_opp_off.max(v_next_off).max(v_prev_off) + num_components > data.len() {
                            return Err(DracoError::general(
                                "Parallelogram corner reads past the decoded values".to_string(),
                            ));
                        }

                        // Slice each neighbour region to exactly num_components so the
                        // inner loop is bounds-check-free: the guard above proves the
                        // subslices are in range, and zipping equal-length slices indexes
                        // via iterators rather than `[k]`. `data` is only read here,
                        // so the three shared immutable reborrows coexist.
                        let v_opp = &data[v_opp_off..v_opp_off + num_components];
                        let v_next = &data[v_next_off..v_next_off + num_components];
                        let v_prev = &data[v_prev_off..v_prev_off + num_components];
                        let accumulate = num_used_parallelograms > 0;
                        for (((pv, &n), &pr), &op) in multi_pred_vals
                            .iter_mut()
                            .zip(v_next)
                            .zip(v_prev)
                            .zip(v_opp)
                        {
                            let p = DataType::compute_parallelogram_prediction(n, pr, op);
                            // Use add_as_unsigned for C++ compatible accumulation
                            *pv = if accumulate {
                                DataType::add_as_unsigned(*pv, p)
                            } else {
                                p
                            };
                        }
                        num_used_parallelograms += 1;
                    }
                }
            }

            if num_used_parallelograms == 0 {
                let prev_offset = (data_id - 1) * num_components;
                // As above: one equal-length copy in place of a fill the copy
                // overwrote and a component-at-a-time indexed loop.
                predicted_val.copy_from_slice(&data[prev_offset..prev_offset + num_components]);
                self.transform.compute_original_value(
                    &predicted_val,
                    &mut data[data_offset..data_offset + num_components],
                );
            } else {
                // C++ decoder uses truncating integer division (not rounding)
                let divisor = num_used_parallelograms as i64;
                for pred in multi_pred_vals.iter_mut() {
                    let val: i64 = (*pred).into();
                    *pred = DataType::from((val / divisor) as i32);
                }
                self.transform.compute_original_value(
                    &multi_pred_vals,
                    &mut data[data_offset..data_offset + num_components],
                );
            }
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "decoder"))]
mod tests {
    use super::*;
    use crate::corner_table::CornerTable;
    use crate::geometry_indices::VertexIndex;
    use crate::prediction_scheme::PredictionSchemeDecoder;
    use crate::prediction_scheme_wrap::PredictionSchemeWrapDecodingTransform;

    #[test]
    fn crease_edge_flag_count_above_the_corner_count_is_refused_before_reading() {
        let mut corner_table = CornerTable::new(1);
        corner_table.init(&[[VertexIndex(0), VertexIndex(1), VertexIndex(2)]]);
        let mut mesh_data = MeshPredictionSchemeData::new();
        mesh_data.set(&corner_table, &[1, 2, 0], &[2, 0, 1]);

        // One face = three corners. A first context declaring four flags names
        // more reads than the mesh has corners, and the refusal has to come
        // before the rANS stream starts: `decode_next_bit` answers `false` for
        // an encoded zero and for a spent stream alike, so the declared count
        // checked against the corner count is the only bound on the reads.
        let bytes = [4u8]; // varint: context 0 declares 4 crease-edge flags
        let mut buffer = DecoderBuffer::new(&bytes);
        buffer.set_version(2, 2);

        let mut decoder = MeshPredictionSchemeConstrainedMultiParallelogramDecoder::<
            i32,
            PredictionSchemeWrapDecodingTransform<i32>,
        >::new(PredictionSchemeWrapDecodingTransform::new(), mesh_data);

        let err = decoder.decode_prediction_data(&mut buffer).unwrap_err();
        assert!(err.message().contains("more than the 3 corners"), "{err}");
    }
}
