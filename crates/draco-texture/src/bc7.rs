//! Packing a described BC7 block into its 128 bits.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `encode_bc7_block` in `transcoder/basisu_transcoder.cpp` and the mode
//! descriptor tables in `transcoder/basisu_transcoder_uastc.h`.
//!
//! BC7 is eight formats sharing one block. Each mode decides how many colour
//! regions a block has, how precisely its endpoints are stored, whether alpha
//! is interpolated along with the colour or on weights of its own, and how
//! many bits a weight gets. Describing a block is therefore the whole problem
//! and writing it out is bit packing in a fixed order, which is all this does;
//! what decides the description is `uastc_to_bc7`.
//!
//! One rule shapes the packing. Each subset's first texel — its anchor —
//! stores one bit fewer than the rest, with the missing high bit fixed at
//! zero. A subset whose anchor weight would have had that bit set is therefore
//! written the other way round: its endpoints swap and its weights inverted,
//! which describes the same colours from the other end.

use crate::bc7_tables::{
    BC7_ANCHOR_SECOND, BC7_ANCHOR_THIRD_1, BC7_ANCHOR_THIRD_2, BC7_PARTITION2, BC7_PARTITION3,
};

/// Which subset each texel belongs to, for a one-subset mode.
const BC7_PARTITION1: [u8; 16] = [0; 16];

const BC7_NUM_SUBSETS: [usize; 8] = [3, 2, 3, 2, 1, 1, 1, 2];
const BC7_PARTITION_BITS: [u32; 8] = [4, 6, 6, 6, 0, 0, 0, 6];
const BC7_COLOR_INDEX_BITCOUNT: [u32; 8] = [3, 3, 2, 2, 2, 2, 4, 2];
const BC7_ALPHA_INDEX_BITCOUNT: [u32; 8] = [0, 0, 0, 0, 3, 2, 4, 2];
const BC7_MODE_HAS_P_BITS: [bool; 8] = [true, true, false, true, false, false, true, true];
const BC7_MODE_HAS_SHARED_P_BITS: [bool; 8] =
    [false, true, false, false, false, false, false, false];
const BC7_COLOR_PRECISION: [u32; 8] = [4, 6, 5, 7, 5, 7, 7, 5];
const BC7_ALPHA_PRECISION: [u32; 8] = [0, 0, 0, 0, 6, 8, 7, 5];

/// Whether a mode interpolates alpha on weights of its own.
fn has_separate_alpha(mode: usize) -> bool {
    mode == 4 || mode == 5
}

fn color_index_size(mode: usize, index_selector: u32) -> u32 {
    BC7_COLOR_INDEX_BITCOUNT[mode] + index_selector
}

fn alpha_index_size(mode: usize, index_selector: u32) -> u32 {
    BC7_ALPHA_INDEX_BITCOUNT[mode] - index_selector
}

/// A BC7 block in the terms its mode uses, before packing.
#[derive(Debug, Clone, Copy)]
pub struct Bc7Block {
    /// Which of the eight BC7 modes.
    pub mode: usize,
    /// Which partition pattern, for a mode with more than one subset.
    pub partition: usize,
    /// One colour weight per texel.
    pub selectors: [u8; 16],
    /// One alpha weight per texel, for a mode that interpolates it separately.
    pub alpha_selectors: [u8; 16],
    /// Low endpoint per subset, as R, G, B, A.
    pub low: [[u8; 4]; 3],
    /// High endpoint per subset.
    pub high: [[u8; 4]; 3],
    /// The extra low bit each endpoint may carry, per subset.
    pub pbits: [[u32; 2]; 3],
    /// Mode 4 only: whether the colour and alpha weights swap roles.
    pub index_selector: u32,
    /// Modes 4 and 5 only: which channel alpha was swapped with.
    pub rotation: u32,
}

impl Default for Bc7Block {
    fn default() -> Self {
        Bc7Block {
            mode: 6,
            partition: 0,
            selectors: [0; 16],
            alpha_selectors: [0; 16],
            low: [[0; 4]; 3],
            high: [[0; 4]; 3],
            pbits: [[0; 2]; 3],
            index_selector: 0,
            rotation: 0,
        }
    }
}

/// Writes bits into a block, least significant first.
struct BitWriter {
    bytes: [u8; 16],
    offset: usize,
}

impl BitWriter {
    fn write(&mut self, mut value: u32, mut count: u32) {
        while count > 0 {
            let taken = (8 - (self.offset & 7) as u32).min(count);
            self.bytes[self.offset >> 3] |= (value << (self.offset & 7)) as u8;
            value >>= taken;
            count -= taken;
            self.offset += taken as usize;
        }
    }
}

impl Bc7Block {
    /// The sixteen bytes a GPU expects.
    pub fn to_bytes(mut self) -> [u8; 16] {
        let mode = self.mode;
        let subsets = BC7_NUM_SUBSETS[mode];
        let partitions = 1usize << BC7_PARTITION_BITS[mode];

        let partition: &[u8; 16] = match subsets {
            1 => &BC7_PARTITION1,
            2 => &BC7_PARTITION2[self.partition],
            _ => &BC7_PARTITION3[self.partition],
        };

        let mut anchors = [usize::MAX; 3];
        for (subset, slot) in anchors.iter_mut().enumerate().take(subsets) {
            *slot = match (subsets, subset) {
                (_, 0) => 0,
                (3, 1) => BC7_ANCHOR_THIRD_1[self.partition] as usize,
                (3, _) => BC7_ANCHOR_THIRD_2[self.partition] as usize,
                _ => BC7_ANCHOR_SECOND[self.partition] as usize,
            };
        }

        for (subset, &anchor) in anchors.iter().enumerate().take(subsets) {
            let color_indices = 1u8 << color_index_size(mode, self.index_selector);
            if self.selectors[anchor] & (color_indices >> 1) != 0 {
                for (texel, &owner) in partition.iter().enumerate() {
                    if owner as usize == subset {
                        self.selectors[texel] = (color_indices - 1) - self.selectors[texel];
                    }
                }
                if has_separate_alpha(mode) {
                    // Alpha keeps weights of its own here, so only the colour
                    // channels turn round with the colour weights.
                    for channel in 0..3 {
                        std::mem::swap(
                            &mut self.low[subset][channel],
                            &mut self.high[subset][channel],
                        );
                    }
                } else {
                    std::mem::swap(&mut self.low[subset], &mut self.high[subset]);
                }
                if !BC7_MODE_HAS_SHARED_P_BITS[mode] {
                    self.pbits[subset].swap(0, 1);
                }
            }

            if has_separate_alpha(mode) {
                let alpha_indices = 1u8 << alpha_index_size(mode, self.index_selector);
                if self.alpha_selectors[anchor] & (alpha_indices >> 1) != 0 {
                    for (texel, &owner) in partition.iter().enumerate() {
                        if owner as usize == subset {
                            self.alpha_selectors[texel] =
                                (alpha_indices - 1) - self.alpha_selectors[texel];
                        }
                    }
                    std::mem::swap(&mut self.low[subset][3], &mut self.high[subset][3]);
                }
            }
        }

        let mut writer = BitWriter {
            bytes: [0; 16],
            offset: 0,
        };
        // The mode is written as a one preceded by that many zero bits.
        writer.write(1 << mode, mode as u32 + 1);
        if mode == 4 || mode == 5 {
            writer.write(self.rotation, 2);
        }
        if mode == 4 {
            writer.write(self.index_selector, 1);
        }
        if partitions > 1 {
            writer.write(self.partition as u32, if partitions == 64 { 6 } else { 4 });
        }

        let components = if mode >= 4 { 4 } else { 3 };
        for component in 0..components {
            let bits = if component == 3 {
                BC7_ALPHA_PRECISION[mode]
            } else {
                BC7_COLOR_PRECISION[mode]
            };
            for subset in 0..subsets {
                writer.write(self.low[subset][component] as u32, bits);
                writer.write(self.high[subset][component] as u32, bits);
            }
        }

        if BC7_MODE_HAS_P_BITS[mode] {
            for subset in 0..subsets {
                writer.write(self.pbits[subset][0], 1);
                if !BC7_MODE_HAS_SHARED_P_BITS[mode] {
                    writer.write(self.pbits[subset][1], 1);
                }
            }
        }

        // Weights, with each anchor one bit short. Mode 4 can swap which set
        // is written first, which is what its index selector means.
        let swapped = self.index_selector != 0;
        for texel in 0..16 {
            let mut bits = if swapped {
                alpha_index_size(mode, self.index_selector)
            } else {
                color_index_size(mode, self.index_selector)
            };
            if anchors.contains(&texel) {
                bits -= 1;
            }
            let value = if swapped {
                self.alpha_selectors[texel]
            } else {
                self.selectors[texel]
            };
            writer.write(value as u32, bits);
        }
        if has_separate_alpha(mode) {
            for texel in 0..16 {
                let mut bits = if swapped {
                    color_index_size(mode, self.index_selector)
                } else {
                    alpha_index_size(mode, self.index_selector)
                };
                if anchors.contains(&texel) {
                    bits -= 1;
                }
                let value = if swapped {
                    self.selectors[texel]
                } else {
                    self.alpha_selectors[texel]
                };
                writer.write(value as u32, bits);
            }
        }

        debug_assert_eq!(writer.offset, 128, "a BC7 block is exactly 128 bits");
        writer.bytes
    }
}
