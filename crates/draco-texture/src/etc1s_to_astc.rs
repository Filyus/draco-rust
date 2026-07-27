//! Turning ETC1S blocks into ASTC 4x4, for a phone with no ETC path.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `convert_etc1s_to_astc_4x4`, `transcoder_init_astc`,
//! `astc_pack_block_cem_12_weight_range0` and
//! `astc_pack_block_cem_12_weight_range2` in
//! `transcoder/basisu_transcoder.cpp`.
//!
//! Where UASTC only had to be restated as ASTC, ETC1S has to be solved into
//! it: four colours along a line become two endpoints and a weight each, and
//! alpha rides a second plane. Three shapes come out of that, and which one a
//! block takes is decided by how many distinct selectors it actually uses —
//! one is a void extent, two fit block truncation coding exactly, and anything
//! more goes through a table of solved endpoints, the same arrangement the BC1
//! path uses.
//!
//! One thing here is carried without being exercised. Each branch ends by
//! restating a descending endpoint pair the other way round, because ASTC
//! reads low-sums-higher-than-high as blue contract. Removing that pass leaves
//! every fixture byte-identical, and the reason looks structural rather than
//! lucky: an ETC1S block's four colours are its base plus an ascending
//! modifier table, so the colour at the lower selector is never the brighter
//! one. It is kept because the reference has it and a fixture is not a proof,
//! but nothing here checks it.
//!
//! The reference has two further branches behind
//! `BASISD_SUPPORT_ASTC_HIGHER_OPAQUE_QUALITY`, which trade a second 64 KiB
//! table for slightly better opaque blocks. They are compiled out of every
//! emscripten build, which is what the browser transcoder this is gated
//! against is, and what this crate is for. Building them would produce blocks
//! no oracle here can check.

use crate::astc_pack::{encode_trits, BitWriter};
use crate::etc1s::{block_colors5, selector_extremes};

/// The solved endpoints, one entry per (base colour, intensity, range, mapping).
///
/// Four bytes each: low endpoint, high endpoint, and the error the pair leaves,
/// which is what picks between mappings. Baked from the reference's own
/// `basisu_transcoder_tables_astc.inc` rather than restated as source.
const SOLUTIONS: &[u8] = include_bytes!("tables/etc1s_to_astc.bin");

/// Selector ranges the encoder produces, in table order.
const SELECTOR_RANGES: [(u8, u8); 6] = [(0, 3), (1, 3), (0, 2), (1, 2), (2, 3), (0, 1)];
/// How each mapping rewrites the four ETC1S selectors as ASTC weights.
const SELECTOR_MAPPINGS: [[u8; 4]; 10] = [
    [0, 0, 1, 1],
    [0, 0, 1, 2],
    [0, 0, 1, 3],
    [0, 0, 2, 3],
    [0, 1, 1, 1],
    [0, 1, 2, 2],
    [0, 1, 2, 3],
    [0, 2, 3, 3],
    [1, 2, 2, 2],
    [1, 2, 3, 3],
];
const MAPPINGS: usize = SELECTOR_MAPPINGS.len();
const RANGES: usize = SELECTOR_RANGES.len();

/// What each of the 48 quantized endpoint values decodes to.
///
/// The [0,47] range is four bits and a trit, so this is the ASTC endpoint
/// unquantization rule rather than a table anyone chose.
fn ise_to_unquant() -> [u32; 48] {
    let mut table = [0u32; 48];
    for trit in 0..3u32 {
        for bit in 0..16u32 {
            let a = if bit & 1 != 0 { 511 } else { 0 };
            let b = (bit >> 1) | ((bit >> 1) << 6);
            let mut unq = trit * 22 + b;
            unq ^= a;
            unq = (a & 0x80) | (unq >> 2);
            table[(bit | (trit << 4)) as usize] = unq;
        }
    }
    table
}

/// One solved endpoint pair and what it costs.
#[derive(Clone, Copy)]
struct Solution {
    low: u8,
    high: u8,
    error: u16,
}

/// The tables that depend only on the format, built once per level.
pub struct AstcConverter {
    unquant: [u32; 48],
    /// Which table row a (low, high) selector pair uses.
    range_index: [[usize; 4]; 4],
    /// The mapping with the smallest error, per base colour and intensity.
    best_grayscale: [[[u8; RANGES]; 8]; 32],
    /// The best [0,47] pair for one 8-bit value, at weight zero and at one.
    single_color_0: [u8; 256],
    single_color_1: [(u8, u8); 256],
}

impl Default for AstcConverter {
    fn default() -> Self {
        Self::new()
    }
}

impl AstcConverter {
    /// Derive everything that does not come out of the baked table.
    pub fn new() -> Self {
        let unquant = ise_to_unquant();

        let mut range_index = [[0usize; 4]; 4];
        for (index, (low, high)) in SELECTOR_RANGES.iter().enumerate() {
            range_index[*low as usize][*high as usize] = index;
        }

        let mut best_grayscale = [[[0u8; RANGES]; 8]; 32];
        for (base, per_base) in best_grayscale.iter_mut().enumerate() {
            for (inten, per_inten) in per_base.iter_mut().enumerate() {
                for (range, best) in per_inten.iter_mut().enumerate() {
                    let mut lowest = u32::MAX;
                    for mapping in 0..MAPPINGS {
                        let solution = solution(base as u8, inten as u8, range, mapping);
                        if (solution.error as u32) < lowest {
                            lowest = solution.error as u32;
                            *best = mapping as u8;
                        }
                    }
                }
            }
        }

        // The best single [0,47] value for an 8-bit one, and the best pair for
        // it read at weight 1 of 3 - which is what a solid block writes.
        let mut single_color_0 = [0u8; 256];
        let mut single_color_1 = [(0u8, 0u8); 256];
        for value in 0..256i32 {
            let mut lowest = i32::MAX;
            for (low, quantized) in unquant.iter().enumerate() {
                let error = (*quantized as i32 - value).abs();
                if error < lowest {
                    lowest = error;
                    single_color_0[value as usize] = low as u8;
                }
            }

            let mut lowest = i32::MAX;
            for (low, low_value) in unquant.iter().enumerate() {
                for (high, high_value) in unquant.iter().enumerate() {
                    let l = (low_value | (low_value << 8)) as i32;
                    let h = (high_value | (high_value << 8)) as i32;
                    // What weight 1 of 3 reads back as, which is the weight a
                    // solid block writes at.
                    let read = ((l * (64 - 21) + h * 21 + 32) / 64) >> 8;
                    let error = (read - value).abs();
                    if error < lowest {
                        lowest = error;
                        single_color_1[value as usize] = (low as u8, high as u8);
                    }
                }
            }
        }

        AstcConverter {
            unquant,
            range_index,
            best_grayscale,
            single_color_0,
            single_color_1,
        }
    }
}

/// One entry of the baked table.
fn solution(base: u8, inten: u8, range: usize, mapping: usize) -> Solution {
    let at =
        ((inten as usize * 32 + base as usize) * RANGES * MAPPINGS + range * MAPPINGS + mapping)
            * 4;
    Solution {
        low: SOLUTIONS[at],
        high: SOLUTIONS[at + 1],
        error: u16::from_le_bytes([SOLUTIONS[at + 2], SOLUTIONS[at + 3]]),
    }
}

/// One ETC1S block as the codebooks resolved it.
#[derive(Clone, Copy, Default)]
pub struct Block {
    /// Base colour, five bits per channel.
    pub color5: [u8; 3],
    /// Which of the eight intensity tables the block uses.
    pub inten5: u8,
    /// Four rows of four two-bit selectors.
    pub selectors: [u8; 4],
}

/// How many distinct selector values a block uses.
fn unique_count(selectors: [u8; 4]) -> u32 {
    let mut seen = 0u8;
    for row in selectors {
        for texel in 0..4 {
            seen |= 1 << ((row >> (texel * 2)) & 3);
        }
    }
    seen.count_ones()
}

/// The endpoints and weights an ASTC block is packed from.
///
/// Eight endpoint values, and thirty-two weights because every block this
/// writes is dual plane: colour on the first, alpha on the second.
#[derive(Default)]
struct Params {
    endpoints: [u8; 10],
    weights: [u8; 32],
}

impl AstcConverter {
    /// Convert one ETC1S block, with its alpha block if the file has one.
    pub fn convert(&self, color: Block, alpha: Option<Block>) -> [u8; 16] {
        let (low, high) = selector_extremes(color.selectors);
        let unique = unique_count(color.selectors);

        // A file with no alpha slice behaves as one whose alpha is constant
        // 255, which is the case both of the lossless branches want.
        let alpha_unique = alpha.map_or(1, |block| unique_count(block.selectors));
        let constant_alpha = match alpha {
            Some(block) if alpha_unique == 1 => {
                let (alpha_low, _) = selector_extremes(block.selectors);
                block_colors5(block.color5, block.inten5)[alpha_low as usize][1]
            }
            _ => 255,
        };

        if unique == 1 && alpha_unique == 1 {
            let color = block_colors5(color.color5, color.inten5)[low as usize];
            return void_extent(color, constant_alpha);
        }

        if unique <= 2 && alpha_unique <= 2 {
            return self.truncation_block(color, low, high, alpha);
        }

        self.solved_block(color, low, high, alpha, alpha_unique)
    }

    /// Both halves use at most two values, so eight-bit endpoints and one-bit
    /// weights reproduce the block exactly. ASTC calls this CEM 12; it is
    /// block truncation coding, and it is lossless here.
    fn truncation_block(&self, color: Block, low: u8, high: u8, alpha: Option<Block>) -> [u8; 16] {
        let mut params = Params::default();
        let colors = block_colors5(color.color5, color.inten5);
        let (dark, light) = (colors[low as usize], colors[high as usize]);
        for (channel, (lo, hi)) in dark.iter().zip(light.iter()).enumerate() {
            params.endpoints[channel * 2] = *lo;
            params.endpoints[channel * 2 + 1] = *hi;
        }
        let invert = invert_if_descending(&mut params.endpoints, |value| value as u32);

        match alpha {
            Some(block) => {
                let (alpha_low, alpha_high) = selector_extremes(block.selectors);
                let values = block_colors5(block.color5, block.inten5);
                params.endpoints[6] = values[alpha_low as usize][1];
                params.endpoints[7] = values[alpha_high as usize][1];
                for texel in 0..16usize {
                    let selector = selector_at(block.selectors, texel);
                    params.weights[texel * 2 + 1] = u8::from(selector == alpha_high);
                }
            }
            None => {
                params.endpoints[6] = 255;
                params.endpoints[7] = 255;
            }
        }

        for texel in 0..16usize {
            let selector = selector_at(color.selectors, texel);
            let weight = u8::from(selector == high);
            params.weights[texel * 2] = if invert { 1 - weight } else { weight };
        }

        pack_weight_range0(&params)
    }

    /// Anything else: [0,47] endpoints and two-bit weights, which is the one
    /// shape that can carry every block, at slightly less than BC1's quality.
    fn solved_block(
        &self,
        color: Block,
        low: u8,
        high: u8,
        alpha: Option<Block>,
        alpha_unique: u32,
    ) -> [u8; 16] {
        let mut params = Params::default();

        match alpha {
            Some(block) => {
                let (alpha_low, alpha_high) = selector_extremes(block.selectors);
                if alpha_low == alpha_high {
                    let value = block_colors5(block.color5, block.inten5)[alpha_low as usize][1];
                    let (lo, hi) = self.single_color_1[value as usize];
                    params.endpoints[6] = lo;
                    params.endpoints[7] = hi;
                    for texel in 0..16usize {
                        params.weights[texel * 2 + 1] = 1;
                    }
                } else if block.inten5 >= 7
                    && alpha_unique == 2
                    && alpha_low == 0
                    && alpha_high == 3
                {
                    // Only the two outer colours, on the widest intensity
                    // table: the solved table has no entry for that, so the
                    // two values are encoded on their own.
                    let values = block_colors5(block.color5, block.inten5);
                    params.endpoints[6] = self.single_color_0[values[0][1] as usize];
                    params.endpoints[7] = self.single_color_0[values[3][1] as usize];
                    for texel in 0..16usize {
                        let selector = selector_at(block.selectors, texel);
                        params.weights[texel * 2 + 1] = if selector == alpha_high { 3 } else { 0 };
                    }
                } else {
                    let range = self.range_index[alpha_low as usize][alpha_high as usize];
                    let mapping = self.best_grayscale[block.color5[1] as usize]
                        [block.inten5 as usize][range] as usize;
                    let solved = solution(block.color5[1], block.inten5, range, mapping);
                    params.endpoints[6] = solved.low;
                    params.endpoints[7] = solved.high;
                    let translate = SELECTOR_MAPPINGS[mapping];
                    for texel in 0..16usize {
                        let selector = selector_at(block.selectors, texel);
                        params.weights[texel * 2 + 1] = translate[selector as usize];
                    }
                }
            }
            None => {
                // 1 unquantizes to 255, so this is opaque.
                params.endpoints[6] = 1;
                params.endpoints[7] = 1;
            }
        }

        let unquant = |value: u8| self.unquant[value as usize];
        if low == high {
            let solid = block_colors5(color.color5, color.inten5)[low as usize];
            for (channel, value) in solid.iter().enumerate() {
                let (lo, hi) = self.single_color_1[*value as usize];
                params.endpoints[channel * 2] = lo;
                params.endpoints[channel * 2 + 1] = hi;
            }
            let invert = invert_if_descending(&mut params.endpoints, unquant);
            for texel in 0..16usize {
                params.weights[texel * 2] = if invert { 2 } else { 1 };
            }
        } else if color.inten5 >= 7 && unique_count(color.selectors) == 2 && low == 0 && high == 3 {
            let colors = block_colors5(color.color5, color.inten5);
            let (darkest, lightest) = (colors[0], colors[3]);
            for (channel, (lo, hi)) in darkest.iter().zip(lightest.iter()).enumerate() {
                params.endpoints[channel * 2] = self.single_color_0[*lo as usize];
                params.endpoints[channel * 2 + 1] = self.single_color_0[*hi as usize];
            }
            let invert = invert_if_descending(&mut params.endpoints, unquant);
            for texel in 0..16usize {
                let selector = selector_at(color.selectors, texel);
                let weight = if selector == low { 0 } else { 3 };
                params.weights[texel * 2] = if invert { 3 - weight } else { weight };
            }
        } else {
            // One mapping serves all three channels, chosen by their total
            // error rather than each channel's own.
            let range = self.range_index[low as usize][high as usize];
            let mut best = 0usize;
            let mut lowest = u32::MAX;
            for mapping in 0..MAPPINGS {
                let total: u32 = (0..3)
                    .map(|channel| {
                        solution(color.color5[channel], color.inten5, range, mapping).error as u32
                    })
                    .sum();
                if total < lowest {
                    lowest = total;
                    best = mapping;
                }
            }
            for channel in 0..3usize {
                let solved = solution(color.color5[channel], color.inten5, range, best);
                params.endpoints[channel * 2] = solved.low;
                params.endpoints[channel * 2 + 1] = solved.high;
            }
            let invert = invert_if_descending(&mut params.endpoints, unquant);
            let translate = SELECTOR_MAPPINGS[best];
            for texel in 0..16usize {
                let selector = selector_at(color.selectors, texel);
                let weight = translate[selector as usize];
                params.weights[texel * 2] = if invert { 3 - weight } else { weight };
            }
        }

        pack_weight_range2(&params)
    }
}

/// The selector of texel `index`, counted across rows.
fn selector_at(selectors: [u8; 4], index: usize) -> u8 {
    let (x, y) = (index & 3, index >> 2);
    (selectors[y] >> (x * 2)) & 3
}

/// Put the darker endpoint first, and say whether that swapped them.
///
/// ASTC reads a subset whose low endpoint sums higher than its high one as
/// blue contract, which means something else entirely - so the pair is stated
/// the other way round and the weights inverted to match.
fn invert_if_descending(endpoints: &mut [u8; 10], value: impl Fn(u8) -> u32) -> bool {
    let sum = |offset: usize| -> u32 {
        (0..3)
            .map(|channel| value(endpoints[channel * 2 + offset]))
            .sum()
    };
    if sum(1) < sum(0) {
        for channel in 0..3usize {
            endpoints.swap(channel * 2, channel * 2 + 1);
        }
        return true;
    }
    false
}

/// A block of one colour, which ASTC calls a void extent.
fn void_extent(color: [u8; 3], alpha: u8) -> [u8; 16] {
    let mut writer = BitWriter { bytes: [0; 16] };
    writer.bytes[0] = 0xfc;
    writer.bytes[1] = 0xfd;
    writer.bytes[2] = 0xff;
    writer.bytes[3] = 0xff;
    for byte in writer.bytes.iter_mut().take(8).skip(4) {
        *byte = 0xff;
    }
    let mut offset = 64;
    for channel in [color[0], color[1], color[2], alpha] {
        let value = channel as u32;
        writer.set(&mut offset, value | (value << 8), 16);
    }
    writer.bytes
}

/// CEM 12, eight-bit endpoints, one-bit weights.
fn pack_weight_range0(params: &Params) -> [u8; 16] {
    let mut writer = BitWriter { bytes: [0; 16] };
    writer.bytes[0] = 0x41;
    writer.bytes[1] = 0x84;
    writer.bytes[2] = 0x01;
    writer.bytes[11] = 0xc0;

    let mut offset = 17;
    for value in params.endpoints.iter().take(8) {
        writer.set(&mut offset, *value as u32, 8);
    }
    for (index, weight) in params.weights.iter().enumerate() {
        let at = 127 - index;
        writer.bytes[at >> 3] |= weight << (at & 7);
    }
    writer.bytes
}

/// CEM 12, [0,47] endpoints, two-bit weights.
fn pack_weight_range2(params: &Params) -> [u8; 16] {
    let mut writer = BitWriter { bytes: [0; 16] };
    writer.bytes[0] = 0x42;
    writer.bytes[1] = 0x84;
    writer.bytes[2] = 0x01;
    writer.bytes[7] = 0xc0;

    // Eight endpoints in two groups of five, which is what the trit encoding
    // packs at a time - the ninth and tenth values are zero and unread.
    let mut offset = 17;
    let mut group = [0u8; 5];
    group.copy_from_slice(&params.endpoints[0..5]);
    encode_trits(&mut writer, &group, &mut offset, 4);
    group.copy_from_slice(&params.endpoints[5..10]);
    encode_trits(&mut writer, &group, &mut offset, 4);

    const REVERSE: [u8; 4] = [0, 2, 1, 3];
    for (index, weight) in params.weights.iter().enumerate() {
        let at = 126 - index * 2;
        writer.bytes[at >> 3] |= REVERSE[*weight as usize] << (at & 7);
    }
    writer.bytes
}
