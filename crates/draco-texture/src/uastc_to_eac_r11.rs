//! Re-packing a UASTC block's channels into EAC R11.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `pack_eac` in `transcoder/basisu_transcoder.cpp`.
//!
//! Like [`crate::uastc_to_bc4`] this re-derives the texel values rather than
//! restating the block — R11 carries a single channel, and no UASTC mode
//! says anything about that. The packer searches four of EAC's sixteen
//! modifier tables per block and keeps whichever came closest; the reference
//! also has a sixteen-table variant it reaches through a high-quality flag,
//! which nothing in the KTX2 path sets, so this is the four-table one.

use crate::eac_r11::EacR11Block;
use crate::uastc_to_etc::{EAC_MAX_SELECTOR, EAC_MIN_SELECTOR, EAC_MODIFIERS};

/// The four tables the search considers, in reference order.
const TABLES: [usize; 4] = [2, 8, 11, 13];

/// The table the narrow path uses when the block's span fits without loss.
const SINGLE_TABLE: usize = 13;

/// The selector values that walk table 13 from two below the base to three
/// above it.
const SINGLE_TABLE_STEPS: [u8; 6] = [2, 1, 0, 4, 5, 6];

/// Where each texel's three-bit selector sits in the block's 48-bit field.
///
/// EAC packs its selectors starting from the high bit of the sixth byte,
/// walking the block down its columns, which for a row-major raster is this
/// order.
const BIT_OFFSETS: [u32; 16] = [45, 33, 21, 9, 42, 30, 18, 6, 39, 27, 15, 3, 36, 24, 12, 0];

/// Pack sixteen one-channel values into an EAC R11 block.
///
/// The search is the reference's: the base and a multiplier are fit per
/// table from the texel extremes, every texel takes its closest of the eight
/// modifiers, and the table whose squared error came out smallest wins.
pub(crate) fn pack(values: &[u8; 16]) -> EacR11Block {
    let lowest = *values.iter().min().unwrap_or(&0);
    let highest = *values.iter().max().unwrap_or(&0);

    if lowest == highest {
        return EacR11Block::solid(lowest);
    }

    let range = (highest - lowest) as u32;
    if range <= 5 {
        // Table 13 is lossless over a span of five, so there is nothing to
        // search: base the block two below the top and read every texel off.
        let base = (highest as i32 - 2).clamp(0, 255) - 3;
        let mut bits = 0u64;
        for (texel, &value) in values.iter().enumerate() {
            let step = (value as i32 - base).clamp(0, 5);
            bits |= (SINGLE_TABLE_STEPS[step as usize] as u64) << BIT_OFFSETS[texel];
        }
        let mut block = EacR11Block {
            base: (base + 3) as u8,
            table: SINGLE_TABLE as u8,
            multiplier: 1,
            selectors: [0; 6],
        };
        block.set_selector_bits(bits);
        return block;
    }

    let range = range as f32;
    let spans: [f32; 4] = std::array::from_fn(|t| {
        let modifiers = EAC_MODIFIERS[TABLES[t]];
        (modifiers[EAC_MAX_SELECTOR] - modifiers[EAC_MIN_SELECTOR]) as f32
    });
    let bases: [i32; 4] = std::array::from_fn(|t| {
        let modifiers = EAC_MODIFIERS[TABLES[t]];
        let fraction = (0 - modifiers[EAC_MIN_SELECTOR] as i32) as f32 / spans[t];
        ((lowest as f32 + range * fraction).round() as i32).clamp(0, 255)
    });
    let multipliers: [i32; 4] =
        std::array::from_fn(|t| ((range / spans[t]).round() as i32).clamp(1, 15));

    let mut errors = [0u32; 4];
    let mut selectors = [[0u8; 16]; 4];
    for (t, &table) in TABLES.iter().enumerate() {
        let modifiers = EAC_MODIFIERS[table];
        let (base, multiplier) = (bases[t], multipliers[t]);
        for (texel, &value) in values.iter().enumerate() {
            let candidate = (0..8usize)
                .map(|step| {
                    // The reference clamps the restored value only for
                    // extreme texels; everywhere else the comparison runs on
                    // the unclamped arithmetic, which is what makes near-
                    // black and near-white choose a different step than the
                    // clamped value would.
                    let restored = multiplier * modifiers[step] as i32 + base;
                    let restored = if !(7..=248).contains(&value) {
                        restored.clamp(0, 255)
                    } else {
                        restored
                    };
                    ((restored - value as i32).unsigned_abs() << 3) | step as u32
                })
                .min()
                .unwrap_or(0);
            selectors[t][texel] = (candidate & 7) as u8;
            errors[t] += (candidate >> 3).saturating_mul(candidate >> 3);
        }
    }

    let best = (0..4usize).min_by_key(|&t| errors[t]).unwrap_or(0);
    let mut bits = 0u64;
    for texel in 0..16usize {
        bits |= (selectors[best][texel] as u64) << BIT_OFFSETS[texel];
    }
    let mut block = EacR11Block {
        base: bases[best] as u8,
        table: TABLES[best] as u8,
        multiplier: multipliers[best] as u8,
        selectors: [0; 6],
    };
    block.set_selector_bits(bits);
    block
}
