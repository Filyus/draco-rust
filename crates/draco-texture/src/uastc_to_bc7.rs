//! Restating a UASTC block as a BC7 block.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `transcode_uastc_to_bc7`, `determine_unique_pbits`, `determine_shared_pbits`
//! and `bc7_convert_partition_index_3_to_2` in
//! `transcoder/basisu_transcoder.cpp`.
//!
//! This is the one transcode here that is close to free, and by design: UASTC
//! was drawn up so that each of its nineteen modes lands on a BC7 mode with
//! the same number of subsets and the same partition, so a block is restated
//! rather than searched for. What work remains is arithmetic — the endpoints
//! move from ASTC's quantization to BC7's, and where BC7 stores an extra low
//! bit per endpoint, the pair of candidate roundings is scored and the better
//! taken.
//!
//! That scoring is done in `f32`, as the reference does it. Rust and C++ agree
//! on IEEE-754 for the operations involved, so the same block scores the same
//! way; the gate compares whole images against the reference build precisely
//! because that is the sort of claim worth checking rather than assuming.

use crate::bc7::Bc7Block;
use crate::bc7_tables::{
    ASTC_BC7_COMMON_PARTITIONS2, ASTC_BC7_COMMON_PARTITIONS3, ASTC_TO_BC7_PARTITION_PERM,
    BC7_3_ASTC2_COMMON_PARTITIONS,
};
use crate::uastc::{unquantize, Unpacked, MODE_COMPONENTS, MODE_ENDPOINT_RANGES};

/// The weight index BC7 mode 5 uses for a block of one colour.
const MODE_5_OPTIMAL_INDEX: u8 = 1;
/// The same for mode 6.
const MODE_6_OPTIMAL_INDEX: u8 = 5;

/// The best endpoint pair for one target value, and how far off it is.
#[derive(Debug, Clone, Copy, Default)]
struct EndpointError {
    error: u32,
    low: u8,
    high: u8,
}

/// The tables the solid-colour case reads.
///
/// Computed rather than baked: 256 values over a 128×128 search is a few
/// milliseconds, unlike the ETC1S-to-BC1 tables, which are three orders of
/// magnitude larger.
pub struct Bc7Converter {
    /// Mode 6 endpoints per value, per p-bit.
    mode6: [[EndpointError; 2]; 256],
    /// Mode 5 endpoints per value.
    mode5: [EndpointError; 256],
}

impl Default for Bc7Converter {
    fn default() -> Self {
        Self::new()
    }
}

impl Bc7Converter {
    /// Build the per-file tables.
    pub fn new() -> Self {
        // BC7 mode 6 stores seven bits and a shared low bit, so a candidate
        // endpoint is `(value << 1) | pbit`; mode 5 stores seven bits and
        // repeats the top one.
        let mut mode6 = [[EndpointError::default(); 2]; 256];
        for (target, entry) in mode6.iter_mut().enumerate() {
            for (pbit, slot) in entry.iter_mut().enumerate() {
                *slot = best_endpoints(target as i32, |value| (value << 1) | pbit as i32, 5);
            }
        }
        let mut mode5 = [EndpointError::default(); 256];
        for (target, slot) in mode5.iter_mut().enumerate() {
            *slot = best_endpoints(target as i32, |value| (value << 1) | (value >> 6), 1);
        }
        Bc7Converter { mode6, mode5 }
    }

    /// Restate one unpacked UASTC block as a BC7 block.
    pub(crate) fn convert(&self, unpacked: &Unpacked) -> Bc7Block {
        let mode = unpacked.mode;
        let mut block = Bc7Block::default();
        let range = MODE_ENDPOINT_RANGES[mode] as usize;
        let components = MODE_COMPONENTS[mode] as usize;
        let endpoint = |at: usize| unquantize(unpacked.endpoints[at] as u32, range) as f32 / 255.0;

        match mode {
            // One subset, colour and alpha on one set of weights: BC7 mode 6,
            // which is the same shape.
            0 | 5 | 10 | 12 | 14 | 15 | 18 => {
                block.mode = 6;
                let (mut low, mut high) = ([0.0f32; 4], [0.0f32; 4]);
                if components == 2 {
                    // Luminance and alpha: the one value fills red, green and
                    // blue, and BC7 has no two-component mode to say so.
                    low[0] = endpoint(0);
                    high[0] = endpoint(1);
                    low[1] = low[0];
                    high[1] = high[0];
                    low[2] = low[0];
                    high[2] = high[0];
                    low[3] = endpoint(2);
                    high[3] = endpoint(3);
                } else {
                    for component in 0..3 {
                        low[component] = endpoint(component * 2);
                        high[component] = endpoint(component * 2 + 1);
                    }
                    if components == 4 {
                        low[3] = endpoint(6);
                        high[3] = endpoint(7);
                    } else {
                        low[3] = 1.0;
                        high[3] = 1.0;
                    }
                }

                let (min, max, pbits) =
                    unique_pbits(if components == 2 { 4 } else { components }, 7, &low, &high);
                block.low[0] = min;
                block.high[0] = max;
                if components == 3 {
                    // Mode 6 always stores alpha, so a mode without one says
                    // "opaque" at the precision mode 6 keeps.
                    block.low[0][3] = 127;
                    block.high[0][3] = 127;
                }
                block.pbits[0] = pbits;
                block.selectors = remap_weights(mode, &unpacked.weights);
            }

            // One subset, four-step weights, endpoints already at full range.
            1 => {
                block.mode = 3;
                let raw = |at: usize| unpacked.endpoints[at] as f32 / 255.0;
                let low = [raw(0), raw(2), raw(4), 1.0];
                let high = [raw(1), raw(3), raw(5), 1.0];
                let (min, max, pbits) = unique_pbits(3, 7, &low, &high);
                for subset in 0..2 {
                    block.low[subset][..3].copy_from_slice(&min[..3]);
                    block.high[subset][..3].copy_from_slice(&max[..3]);
                    block.pbits[subset] = pbits;
                }
                block.selectors = copy_weights(&unpacked.weights);
            }

            // Two subsets, shared p-bit per subset: BC7 mode 1.
            2 => {
                block.mode = 1;
                let (partition, invert) =
                    ASTC_BC7_COMMON_PARTITIONS2[unpacked.common_pattern as usize];
                block.partition = partition as usize;
                for subset in 0..2 {
                    let mut low = [0.0f32; 4];
                    let mut high = [0.0f32; 4];
                    low[3] = 1.0;
                    high[3] = 1.0;
                    for component in 0..3 {
                        // Range 8 is four bits, widened by repetition rather
                        // than through the general unquantizer.
                        let l = unpacked.endpoints[component * 2 + subset * 6] as u32;
                        let h = unpacked.endpoints[component * 2 + subset * 6 + 1] as u32;
                        low[component] = ((l << 4) | l) as f32 / 255.0;
                        high[component] = ((h << 4) | h) as f32 / 255.0;
                    }
                    let (min, max, pbits) = shared_pbits(3, 6, &low, &high);
                    let target = if invert { 1 - subset } else { subset };
                    block.low[target][..3].copy_from_slice(&min[..3]);
                    block.high[target][..3].copy_from_slice(&max[..3]);
                    block.pbits[target][0] = pbits[0];
                }
                block.selectors = copy_weights(&unpacked.weights);
            }

            // Three subsets, five-bit endpoints, no p-bits: BC7 mode 2.
            3 => {
                block.mode = 2;
                let (partition, permutation) =
                    ASTC_BC7_COMMON_PARTITIONS3[unpacked.common_pattern as usize];
                block.partition = partition as usize;
                for (subset, &mapped) in ASTC_TO_BC7_PARTITION_PERM[permutation as usize]
                    .iter()
                    .enumerate()
                {
                    let target = mapped as usize;
                    for component in 0..3 {
                        let low = unquantize(
                            unpacked.endpoints[component * 2 + subset * 6] as u32,
                            range,
                        );
                        let high = unquantize(
                            unpacked.endpoints[component * 2 + 1 + subset * 6] as u32,
                            range,
                        );
                        block.low[target][component] = ((low * 31 + 127) / 255) as u8;
                        block.high[target][component] = ((high * 31 + 127) / 255) as u8;
                    }
                }
                block.selectors = copy_weights(&unpacked.weights);
            }

            // Two subsets with unique p-bits: BC7 mode 3.
            4 => {
                block.mode = 3;
                let (partition, invert) =
                    ASTC_BC7_COMMON_PARTITIONS2[unpacked.common_pattern as usize];
                block.partition = partition as usize;
                for subset in 0..2 {
                    let mut low = [0.0f32; 4];
                    let mut high = [0.0f32; 4];
                    low[3] = 1.0;
                    high[3] = 1.0;
                    for component in 0..3 {
                        low[component] = endpoint(component * 2 + subset * 6);
                        high[component] = endpoint(component * 2 + subset * 6 + 1);
                    }
                    let (min, max, pbits) = unique_pbits(3, 7, &low, &high);
                    let target = if invert { 1 - subset } else { subset };
                    block.low[target][..3].copy_from_slice(&min[..3]);
                    block.high[target][..3].copy_from_slice(&max[..3]);
                    block.low[target][3] = 127;
                    block.high[target][3] = 127;
                    block.pbits[target] = pbits;
                }
                block.selectors = copy_weights(&unpacked.weights);
            }

            // Dual plane: BC7 mode 5, which reaches the same effect by
            // swapping the separately interpolated channel with alpha.
            6 | 11 | 13 | 17 => {
                block.mode = 5;
                let selector = unpacked.component_selector;
                block.rotation = ((selector + 1) & 3) as u32;

                if components == 2 {
                    let low =
                        ((unquantize(unpacked.endpoints[0] as u32, range) * 127 + 127) / 255) as u8;
                    let high =
                        ((unquantize(unpacked.endpoints[1] as u32, range) * 127 + 127) / 255) as u8;
                    block.low[0] = [
                        low,
                        low,
                        low,
                        unquantize(unpacked.endpoints[2] as u32, range) as u8,
                    ];
                    block.high[0] = [
                        high,
                        high,
                        high,
                        unquantize(unpacked.endpoints[3] as u32, range) as u8,
                    ];
                } else {
                    for astc in 0..4usize {
                        let bc7 = if astc == selector {
                            3
                        } else if astc == 3 {
                            selector
                        } else {
                            astc
                        };
                        let (mut low, mut high) = (255u32, 255u32);
                        if astc < components {
                            low = unquantize(unpacked.endpoints[astc * 2] as u32, range);
                            high = unquantize(unpacked.endpoints[astc * 2 + 1] as u32, range);
                        }
                        if bc7 < 3 {
                            // Mode 5 keeps colour at seven bits and alpha at eight.
                            low = (low * 127 + 127) / 255;
                            high = (high * 127 + 127) / 255;
                        }
                        block.low[0][bc7] = low as u8;
                        block.high[0][bc7] = high as u8;
                    }
                }

                for texel in 0..16 {
                    let (first, second) =
                        (unpacked.weights[texel * 2], unpacked.weights[texel * 2 + 1]);
                    // Mode 13's single weight bit becomes mode 5's two.
                    let widen = |weight: u8| {
                        if mode == 13 {
                            u8::from(weight != 0) * 3
                        } else {
                            weight
                        }
                    };
                    block.selectors[texel] = widen(first);
                    block.alpha_selectors[texel] = widen(second);
                }
            }

            // Two subsets carried on a BC7 three-subset partition.
            7 => {
                block.mode = 2;
                let (partition, k) =
                    BC7_3_ASTC2_COMMON_PARTITIONS[unpacked.common_pattern as usize];
                block.partition = partition as usize;
                for bc7_subset in 0..3usize {
                    let astc_subset = partition_index_3_to_2(bc7_subset as u32, k as u32) as usize;
                    for component in 0..3 {
                        let low = unquantize(
                            unpacked.endpoints[component * 2 + astc_subset * 6] as u32,
                            range,
                        );
                        let high = unquantize(
                            unpacked.endpoints[component * 2 + 1 + astc_subset * 6] as u32,
                            range,
                        );
                        block.low[bc7_subset][component] = ((low * 31 + 127) / 255) as u8;
                        block.high[bc7_subset][component] = ((high * 31 + 127) / 255) as u8;
                    }
                }
                block.selectors = copy_weights(&unpacked.weights);
            }

            // One colour everywhere.
            8 => {
                let color = unpacked.solid_color;
                let sum = |pbit: usize| -> u32 {
                    color
                        .iter()
                        .map(|c| self.mode6[*c as usize][pbit].error)
                        .sum()
                };
                let (error0, error1) = (sum(0), sum(1));
                if error0 > 0 && error1 > 0 {
                    // Mode 5 stores alpha exactly, so a colour neither p-bit
                    // reaches is better served there.
                    block.mode = 5;
                    for (component, &value) in color.iter().enumerate().take(3) {
                        block.low[0][component] = self.mode5[value as usize].low;
                        block.high[0][component] = self.mode5[value as usize].high;
                    }
                    block.low[0][3] = color[3];
                    block.high[0][3] = color[3];
                    block.selectors = [MODE_5_OPTIMAL_INDEX; 16];
                } else {
                    block.mode = 6;
                    let pbit = usize::from(error1 < error0);
                    for (component, &value) in color.iter().enumerate() {
                        block.low[0][component] = self.mode6[value as usize][pbit].low;
                        block.high[0][component] = self.mode6[value as usize][pbit].high;
                    }
                    block.pbits[0] = [pbit as u32, pbit as u32];
                    block.selectors = [MODE_6_OPTIMAL_INDEX; 16];
                }
            }

            // Two subsets with alpha: BC7 mode 7.
            9 | 16 => {
                block.mode = 7;
                let (partition, invert) =
                    ASTC_BC7_COMMON_PARTITIONS2[unpacked.common_pattern as usize];
                block.partition = partition as usize;
                for subset in 0..2 {
                    let (low, high) = if components == 2 {
                        let luma = (endpoint(subset * 4), endpoint(1 + subset * 4));
                        (
                            [luma.0, luma.0, luma.0, endpoint(2 + subset * 4)],
                            [luma.1, luma.1, luma.1, endpoint(3 + subset * 4)],
                        )
                    } else {
                        (
                            [
                                endpoint(subset * 8),
                                endpoint(2 + subset * 8),
                                endpoint(4 + subset * 8),
                                endpoint(6 + subset * 8),
                            ],
                            [
                                endpoint(1 + subset * 8),
                                endpoint(3 + subset * 8),
                                endpoint(5 + subset * 8),
                                endpoint(7 + subset * 8),
                            ],
                        )
                    };
                    let (min, max, pbits) = unique_pbits(4, 5, &low, &high);
                    let target = if invert { 1 - subset } else { subset };
                    block.low[target] = min;
                    block.high[target] = max;
                    block.pbits[target] = pbits;
                }
                block.selectors = copy_weights(&unpacked.weights);
            }

            _ => unreachable!("UASTC has nineteen modes and every one is handled"),
        }

        block
    }
}

/// The weights of a single-plane mode, unchanged.
fn copy_weights(weights: &[u8; 32]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&weights[..16]);
    out
}

/// The weights of a BC7 mode 6 block, widened to its four bits.
///
/// Mode 6 always spends four bits on a weight, so a UASTC mode with fewer
/// steps has to name the nearest of mode 6's sixteen. The tables are the
/// reference's; they are not a plain scaling, because the two formats do not
/// space their steps the same way.
fn remap_weights(mode: usize, weights: &[u8; 32]) -> [u8; 16] {
    const FIVE_TO_FOUR: [u8; 32] = [
        0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 6, 7, 8, 9, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
        14, 14, 15, 15,
    ];
    const TWO_TO_FOUR: [u8; 4] = [0, 5, 10, 15];
    const THREE_TO_FOUR: [u8; 8] = [0, 2, 4, 6, 9, 11, 13, 15];

    let mut out = [0u8; 16];
    for (texel, slot) in out.iter_mut().enumerate() {
        let weight = weights[texel] as usize;
        *slot = match mode {
            18 => FIVE_TO_FOUR[weight],
            14 => TWO_TO_FOUR[weight],
            5 | 12 => THREE_TO_FOUR[weight],
            _ => weight as u8,
        };
    }
    out
}

/// Which of two ASTC subsets a BC7 three-subset partition index falls in.
fn partition_index_3_to_2(index: u32, k: u32) -> u32 {
    let mapped = match k >> 1 {
        0 => u32::from(index > 1),
        1 => u32::from(index != 0),
        _ => u32::from(index != 0 && index != 2),
    };
    if k & 1 != 0 {
        1 - mapped
    } else {
        mapped
    }
}

/// The best endpoint pair for one value, over a 128×128 search.
fn best_endpoints(target: i32, expand: impl Fn(i32) -> i32, weight_index: usize) -> EndpointError {
    const WEIGHTS_4: [i32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
    const WEIGHTS_2: [i32; 4] = [0, 21, 43, 64];
    // Mode 6 interpolates on four-bit weights, mode 5 on two.
    let weight = if weight_index == 5 {
        WEIGHTS_4[5]
    } else {
        WEIGHTS_2[1]
    };

    let mut best = EndpointError {
        error: u32::MAX,
        low: 0,
        high: 0,
    };
    for low in 0..128i32 {
        let low_expanded = expand(low);
        for high in 0..128i32 {
            let high_expanded = expand(high);
            let value = (low_expanded * (64 - weight) + high_expanded * weight + 32) >> 6;
            let error = ((value - target) * (value - target)) as u32;
            if error < best.error {
                best = EndpointError {
                    error,
                    low: low as u8,
                    high: high as u8,
                };
            }
        }
    }
    best
}

/// Round each endpoint to the grid a p-bit puts it on, and keep the better bit.
///
/// BC7 stores one extra low bit per endpoint, which doubles the grid but ties
/// both endpoints of a pair to the same parity. Each parity is therefore tried
/// and scored, separately for the low and the high endpoint.
fn unique_pbits(
    components: usize,
    bits: u32,
    low: &[f32; 4],
    high: &[f32; 4],
) -> ([u8; 4], [u8; 4], [u32; 2]) {
    let total_bits = bits + 1;
    let scale = ((1u32 << total_bits) - 1) as i32;

    let (mut best_low, mut best_high) = ([0u8; 4], [0u8; 4]);
    let mut pbits = [0u32; 2];
    let (mut best_low_error, mut best_high_error) = (f32::MAX, f32::MAX);

    for pbit in 0..2i32 {
        let (candidate_low, candidate_high) = quantize_pair(low, high, scale, pbit, total_bits);
        let (scaled_low, scaled_high) = (
            expand_pair(&candidate_low, total_bits),
            expand_pair(&candidate_high, total_bits),
        );

        let (mut low_error, mut high_error) = (0.0f32, 0.0f32);
        for component in 0..components {
            let a = scaled_low[component] as f32 - low[component] * 255.0;
            let b = scaled_high[component] as f32 - high[component] * 255.0;
            low_error += a * a;
            high_error += b * b;
        }
        if low_error < best_low_error {
            best_low_error = low_error;
            pbits[0] = pbit as u32;
            for component in 0..4 {
                best_low[component] = candidate_low[component] >> 1;
            }
        }
        if high_error < best_high_error {
            best_high_error = high_error;
            pbits[1] = pbit as u32;
            for component in 0..4 {
                best_high[component] = candidate_high[component] >> 1;
            }
        }
    }
    (best_low, best_high, pbits)
}

/// The same, for a mode whose two endpoints share one p-bit.
fn shared_pbits(
    components: usize,
    bits: u32,
    low: &[f32; 4],
    high: &[f32; 4],
) -> ([u8; 4], [u8; 4], [u32; 2]) {
    let total_bits = bits + 1;
    let scale = ((1u32 << total_bits) - 1) as i32;

    let (mut best_low, mut best_high) = ([0u8; 4], [0u8; 4]);
    let mut pbits = [0u32; 2];
    let mut best_error = f32::MAX;

    for pbit in 0..2i32 {
        let (candidate_low, candidate_high) = quantize_pair(low, high, scale, pbit, total_bits);
        let (scaled_low, scaled_high) = (
            expand_pair(&candidate_low, total_bits),
            expand_pair(&candidate_high, total_bits),
        );

        let mut error = 0.0f32;
        for component in 0..components {
            let a = scaled_low[component] as f32 / 255.0 - low[component];
            let b = scaled_high[component] as f32 / 255.0 - high[component];
            error += a * a + b * b;
        }
        if error < best_error {
            best_error = error;
            pbits = [pbit as u32, pbit as u32];
            for component in 0..4 {
                best_low[component] = candidate_low[component] >> 1;
                best_high[component] = candidate_high[component] >> 1;
            }
        }
    }
    (best_low, best_high, pbits)
}

/// Round a pair of endpoints onto the grid one parity allows.
fn quantize_pair(
    low: &[f32; 4],
    high: &[f32; 4],
    scale: i32,
    pbit: i32,
    total_bits: u32,
) -> ([u8; 4], [u8; 4]) {
    let _ = total_bits;
    let round = |value: f32| -> u8 {
        let stepped = ((value * scale as f32 - pbit as f32) / 2.0 + 0.5) as i32 * 2 + pbit;
        stepped.clamp(pbit, scale - 1 + pbit) as u8
    };
    (
        [round(low[0]), round(low[1]), round(low[2]), round(low[3])],
        [
            round(high[0]),
            round(high[1]),
            round(high[2]),
            round(high[3]),
        ],
    )
}

/// Widen a quantized endpoint back to eight bits by repeating its top bits.
fn expand_pair(values: &[u8; 4], total_bits: u32) -> [u8; 4] {
    let mut out = [0u8; 4];
    for (slot, value) in out.iter_mut().zip(values.iter()) {
        let widened = (*value as u32) << (8 - total_bits);
        *slot = (widened | (widened >> total_bits)) as u8;
    }
    out
}
