//! Turning ETC1S blocks into BC1, the block format a desktop GPU takes.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `convert_etc1s_to_dxt1` and `prepare_bc1_single_color_table` in
//! `transcoder/basisu_transcoder.cpp`, and the table generators guarded there
//! by `BASISD_WRITE_NEW_DXT1_TABLES`.
//!
//! The two formats do not describe a block the same way. ETC1S gives every
//! texel one of four colours spaced along a fixed intensity curve around a
//! base colour; BC1 gives it one of four colours spaced evenly between two
//! endpoints. Neither can express the other exactly, so this is a search for
//! the closest BC1 block — and because running that search per block would be
//! far too slow, the answers are precomputed for every combination of base
//! colour channel, intensity table, selector range and selector mapping.
//!
//! That is what the two baked tables hold: 15360 solved endpoint pairs each,
//! for BC1's five-bit and six-bit channels. They are baked rather than
//! computed on load because the search behind them is some 79 million
//! candidate evaluations, which is a visible stall in a browser.
//! [`bake_bc1_tables`] regenerates them, and a test proves the committed bytes
//! still agree with the algorithm they came from.

use crate::etc1s::{block_color5, block_colors5};

/// Selector ranges the encoder is known to produce, in table order.
const SELECTOR_RANGES: [(u8, u8); 6] = [(0, 3), (1, 3), (0, 2), (1, 2), (2, 3), (0, 1)];

/// How ETC1S selector values may be mapped onto BC1 selector values.
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
/// Solutions per table: eight intensity tables × 32 base values × range × mapping.
const SOLUTIONS: usize = 8 * 32 * RANGES * MAPPINGS;
/// Bytes per stored solution: low, high, and a 16-bit error.
const SOLUTION_BYTES: usize = 4;

/// Solved endpoints for BC1's five-bit red and blue channels.
static TABLE_5: &[u8; SOLUTIONS * SOLUTION_BYTES] = include_bytes!("tables/etc1s_to_bc1_5.bin");
/// Solved endpoints for BC1's six-bit green channel.
static TABLE_6: &[u8; SOLUTIONS * SOLUTION_BYTES] = include_bytes!("tables/etc1s_to_bc1_6.bin");

/// The low endpoint, high endpoint and squared error of one solution.
fn solution(table: &[u8], index: usize) -> (u32, u32, u32) {
    let at = index * SOLUTION_BYTES;
    (
        table[at] as u32,
        table[at + 1] as u32,
        u16::from_le_bytes([table[at + 2], table[at + 3]]) as u32,
    )
}

/// Expand a five-bit BC1 channel to eight bits.
fn expand5(value: u32) -> u32 {
    (value << 3) | (value >> 2)
}

/// Expand a six-bit BC1 channel to eight bits.
fn expand6(value: u32) -> u32 {
    (value << 2) | (value >> 4)
}

/// Pack three unscaled BC1 channels into the 5:6:5 word.
fn pack565(red: u32, green: u32, blue: u32) -> u32 {
    (red << 11) | (green << 5) | blue
}

/// Generate one of the two tables, exactly as the reference's generator does.
///
/// `bits` is the BC1 channel width: five for red and blue, six for green. The
/// search is a plain exhaustive one over every endpoint pair, scoring squared
/// error against the ETC1S colours the selectors in range would pick.
pub fn bake_bc1_tables(bits: u32) -> Vec<u8> {
    let expand = if bits == 5 { expand5 } else { expand6 };
    let levels = 1u32 << bits;
    let mut out = Vec::with_capacity(SOLUTIONS * SOLUTION_BYTES);

    for inten in 0..8u8 {
        for base in 0..32u8 {
            // An ETC1S base colour's three channels are independent, so the
            // generator solves one grey axis and every channel reads it.
            let block = block_colors5([base; 3], inten);

            for (low_selector, high_selector) in SELECTOR_RANGES {
                for mapping in SELECTOR_MAPPINGS {
                    let mut best = (0u32, 0u32, u64::MAX);
                    for high in 0..levels {
                        for low in 0..levels {
                            let mut colors = [0u32; 4];
                            colors[0] = expand(low);
                            colors[3] = expand(high);
                            colors[1] = (colors[0] * 2 + colors[3]) / 3;
                            colors[2] = (colors[3] * 2 + colors[0]) / 3;

                            let mut error = 0u64;
                            for selector in low_selector..=high_selector {
                                let target = block[selector as usize][1] as i64;
                                let candidate = colors[mapping[selector as usize] as usize] as i64;
                                error += ((target - candidate) * (target - candidate)) as u64;
                            }
                            if error < best.2 {
                                best = (low, high, error);
                            }
                        }
                    }
                    out.push(best.0 as u8);
                    out.push(best.1 as u8);
                    out.extend_from_slice(&(best.2 as u16).to_le_bytes());
                }
            }
        }
    }
    out
}

/// For each eight-bit value, the BC1 endpoints that reproduce it best.
///
/// `(low, high)` per target value. Computed on first use rather than baked: a
/// 256-entry table over a 32×32 or 64×64 search is a few milliseconds, not a
/// stall.
fn single_color_table(bits: u32, low_levels: u32, selector1: bool) -> [(u32, u32); 256] {
    let expand = if bits == 5 { expand5 } else { expand6 };
    let levels = 1u32 << bits;
    let mut table = [(0u32, 0u32); 256];
    for (target, entry) in table.iter_mut().enumerate() {
        let target = target as i32;
        let mut lowest = 256i32;
        for low in 0..low_levels {
            for high in 0..levels {
                let low_expanded = expand(low) as i32;
                let high_expanded = expand(high) as i32;
                let error = if selector1 {
                    // Selector 1 sits a third of the way from high to low, and
                    // the tie-break prefers endpoints that lie close together.
                    ((high_expanded * 2 + low_expanded) / 3 - target).abs()
                        + (high_expanded - low_expanded).abs() * 3 / 100
                } else {
                    // Selector 0 is the high endpoint itself.
                    (high_expanded - target).abs()
                };
                if error < lowest {
                    lowest = error;
                    *entry = (low, high);
                }
            }
        }
    }
    table
}

/// One BC1 block: two 5:6:5 endpoints and four rows of four two-bit selectors.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bc1Block {
    low: u32,
    high: u32,
    selectors: [u8; 4],
}

impl Bc1Block {
    /// The eight bytes a GPU expects.
    pub fn to_bytes(self) -> [u8; 8] {
        let mut bytes = [0u8; 8];
        bytes[0..2].copy_from_slice(&(self.low as u16).to_le_bytes());
        bytes[2..4].copy_from_slice(&(self.high as u16).to_le_bytes());
        bytes[4..8].copy_from_slice(&self.selectors);
        bytes
    }
}

/// The small tables the conversion needs, built once rather than per block.
pub struct Bc1Converter {
    match5_equals_1: [(u32, u32); 256],
    match6_equals_1: [(u32, u32); 256],
    match5_equals_0: [(u32, u32); 256],
    match6_equals_0: [(u32, u32); 256],
    /// Per mapping, the byte that rewrites a row of four ETC1S selectors into
    /// BC1 selectors, and the same for a block whose endpoints came out swapped.
    rows: [[u8; 256]; MAPPINGS],
    rows_inverted: [[u8; 256]; MAPPINGS],
}

impl Default for Bc1Converter {
    fn default() -> Self {
        Self::new()
    }
}

impl Bc1Converter {
    /// Build the per-file tables.
    pub fn new() -> Self {
        // BC1 numbers its four colours low, high, then the two interpolants,
        // so a selector in ordinary dark-to-light order has to be permuted.
        const LINEAR_TO_BC1: [u8; 4] = [0, 2, 3, 1];
        const BC1_INVERTED: [u8; 4] = [1, 0, 3, 2];

        let mut rows = [[0u8; 256]; MAPPINGS];
        let mut rows_inverted = [[0u8; 256]; MAPPINGS];
        for (mapping, (row, inverted)) in rows.iter_mut().zip(rows_inverted.iter_mut()).enumerate()
        {
            let direct: [u8; 4] =
                std::array::from_fn(|s| LINEAR_TO_BC1[SELECTOR_MAPPINGS[mapping][s] as usize]);
            let flipped: [u8; 4] = std::array::from_fn(|s| BC1_INVERTED[direct[s] as usize]);
            for byte in 0..256usize {
                let mut translated = 0u8;
                let mut translated_inverted = 0u8;
                for texel in 0..4 {
                    let selector = (byte >> (texel * 2)) & 3;
                    translated |= direct[selector] << (texel * 2);
                    translated_inverted |= flipped[selector] << (texel * 2);
                }
                row[byte] = translated;
                inverted[byte] = translated_inverted;
            }
        }

        Bc1Converter {
            match5_equals_1: single_color_table(5, 32, true),
            match6_equals_1: single_color_table(6, 64, true),
            // The selector-0 tables fix the low endpoint at zero: only the
            // high one has to land on the colour.
            match5_equals_0: single_color_table(5, 1, false),
            match6_equals_0: single_color_table(6, 1, false),
            rows,
            rows_inverted,
        }
    }

    /// Convert one ETC1S block into the closest BC1 block.
    ///
    /// `three_color` allows BC1's punch-through mode, which it signals by
    /// `low <= high` and which makes the fourth colour transparent black. A
    /// standalone BC1 texture may use it; the colour half of a BC3 block may
    /// not, because some GPUs do not implement it there. When it is not
    /// allowed, endpoints that came out equal are pushed apart instead.
    pub fn convert(
        &self,
        color5: [u8; 3],
        inten5: u8,
        selectors: [u8; 4],
        three_color: bool,
    ) -> Bc1Block {
        let (lowest, highest) = crate::etc1s::selector_extremes(selectors);

        if lowest == highest {
            return self.uniform_block(color5, inten5, lowest, three_color);
        }
        if inten5 >= 7 && lowest == 0 && highest == 3 && unique_selectors(selectors) == 2 {
            return self.two_color_block(color5, inten5, selectors);
        }
        self.searched_block(color5, inten5, selectors, lowest, highest, three_color)
    }

    /// Every texel is one colour, so the block is a single-colour BC1 block.
    fn uniform_block(
        &self,
        color5: [u8; 3],
        inten5: u8,
        selector: u8,
        three_color: bool,
    ) -> Bc1Block {
        let color = block_color5(color5, inten5, selector);
        let entry = |table: &[(u32, u32); 256], channel: usize| table[color[channel] as usize];
        let (red, green, blue) = (
            entry(&self.match5_equals_1, 0),
            entry(&self.match6_equals_1, 1),
            entry(&self.match5_equals_1, 2),
        );
        let mut high = pack565(red.1, green.1, blue.1);
        let mut low = pack565(red.0, green.0, blue.0);
        let mut mask = 0xAAu8;

        if !three_color && low == high {
            // Equal endpoints would mean punch-through, so they are pushed
            // apart and the selectors follow them.
            mask = 0;
            if low > 0 {
                low -= 1;
            } else {
                high = 1;
                low = 0;
                mask = 0x55;
            }
        }
        if high < low {
            std::mem::swap(&mut high, &mut low);
            mask ^= 0x55;
        }
        Bc1Block {
            low: high,
            high: low,
            selectors: [mask; 4],
        }
    }

    /// The block uses only its darkest and lightest colours, far apart.
    ///
    /// At the widest intensity tables those two are near black and near white,
    /// which BC1's endpoints can hit exactly — so this skips the search and
    /// places them directly.
    fn two_color_block(&self, color5: [u8; 3], inten5: u8, selectors: [u8; 4]) -> Bc1Block {
        let colors = block_colors5(color5, inten5);
        let high_color = |channel: usize, at: usize| -> u32 {
            let value = colors[at][channel] as usize;
            match channel {
                1 => self.match6_equals_0[value].1,
                _ => self.match5_equals_0[value].1,
            }
        };
        let mut high = pack565(high_color(0, 0), high_color(1, 0), high_color(2, 0));
        let mut low = pack565(high_color(0, 3), high_color(1, 3), high_color(2, 3));
        let (mut dark, mut light) = (0u8, 1u8);

        if low == high {
            if low > 0 {
                low -= 1;
                dark = 0;
                light = 0;
            } else {
                high = 1;
                low = 0;
                dark = 1;
                light = 1;
            }
        }
        if high < low {
            std::mem::swap(&mut high, &mut low);
            dark = 1;
            light = 0;
        }

        let rows = std::array::from_fn(|row| {
            let mut byte = 0u8;
            for texel in 0..4 {
                let selector = (selectors[row] >> (texel * 2)) & 3;
                let value = if selector == 3 { light } else { dark };
                byte |= value << (texel * 2);
            }
            byte
        });
        Bc1Block {
            low: high,
            high: low,
            selectors: rows,
        }
    }

    /// The general case: pick the mapping whose solved endpoints fit best.
    fn searched_block(
        &self,
        color5: [u8; 3],
        inten5: u8,
        selectors: [u8; 4],
        lowest: u8,
        highest: u8,
        three_color: bool,
    ) -> Bc1Block {
        let range = SELECTOR_RANGES
            .iter()
            .position(|(low, high)| *low == lowest && *high == highest)
            // The encoder only emits the six ranges above; anything else is
            // widened to the full range, which is legal if not the tightest.
            .unwrap_or(0);
        let row = |channel: usize| {
            (inten5 as usize * 32 + color5[channel] as usize) * (RANGES * MAPPINGS)
                + range * MAPPINGS
        };
        let (red, green, blue) = (row(0), row(1), row(2));

        let mut best_mapping = 0usize;
        let mut best_error = u32::MAX;
        for mapping in 0..MAPPINGS {
            let error = solution(TABLE_5, red + mapping).2
                + solution(TABLE_6, green + mapping).2
                + solution(TABLE_5, blue + mapping).2;
            if error < best_error {
                best_error = error;
                best_mapping = mapping;
            }
        }

        let red = solution(TABLE_5, red + best_mapping);
        let green = solution(TABLE_6, green + best_mapping);
        let blue = solution(TABLE_5, blue + best_mapping);

        let mut low = pack565(red.0, green.0, blue.0);
        let mut high = pack565(red.1, green.1, blue.1);
        let mut rows = &self.rows[best_mapping];
        if low < high {
            std::mem::swap(&mut low, &mut high);
            rows = &self.rows_inverted[best_mapping];
        }

        if low == high {
            // Equal endpoints again mean punch-through. Left alone the block
            // is one flat colour either way, so it only has to be broken up
            // when punch-through is not allowed.
            let mut mask = 0u8;
            if !three_color {
                if high > 0 {
                    high -= 1;
                } else {
                    high = 0;
                    low = 1;
                    mask = 0x55;
                }
            }
            return Bc1Block {
                low,
                high,
                selectors: [mask; 4],
            };
        }

        Bc1Block {
            low,
            high,
            selectors: std::array::from_fn(|row| rows[selectors[row] as usize]),
        }
    }
}

/// The lowest and highest selector value a block uses.
/// How many distinct selector values a block uses.
pub(crate) fn unique_selectors(selectors: [u8; 4]) -> u32 {
    let mut seen = 0u8;
    for row in selectors {
        for texel in 0..4 {
            seen |= 1 << ((row >> (texel * 2)) & 3);
        }
    }
    seen.count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The baked tables still say what the generator says.
    ///
    /// Without this the blobs are two files of numbers nobody can check. With
    /// it they are a cache of a documented search, and a rebuild that drifts
    /// from the algorithm fails here rather than in someone's texture.
    #[test]
    fn tables_match_the_generator() {
        assert_eq!(bake_bc1_tables(5), TABLE_5.as_slice(), "the five-bit table");
        assert_eq!(bake_bc1_tables(6), TABLE_6.as_slice(), "the six-bit table");
    }
}
