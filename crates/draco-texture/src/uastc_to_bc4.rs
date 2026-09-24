//! Re-packing a UASTC block's channels into BC4.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `encode_bc4` in `transcoder/basisu_transcoder.cpp`.
//!
//! Every other UASTC target here restates what a block already says — BC7,
//! ASTC and ETC2 all read the same endpoints and weights out of a different
//! bit layout. BC4 cannot work that way: it carries a single channel, so the
//! sixteen texel values have to be re-derived and packed again, which is
//! what this is. The selector search is exact — the threshold test it uses
//! picks the same selector an exhaustive check of all eight would — so the
//! reference writes it without a comment and this port does too.
//!
//! Which matters where: a roughness mask in BC1 spends its three channels on
//! one, and a tangent-space normal map through BC1 falls apart, five bits a
//! channel against BC4's eight.

use crate::bc4::Bc4Block;

/// The selector each step of the search picks, indexed by how many
/// thresholds the texel value clears.
///
/// BC4's selector zero reads as the high endpoint and seven as the low, so
/// the order runs from one end to the other.
const STEPS: [u8; 8] = [1, 7, 6, 5, 4, 3, 2, 0];

/// The block every texel of which is `value`: what [`pack`] makes of sixteen
/// equal values, and the reference's `write_bc4_solid_block`, so a caller
/// that already knows the block is one colour can skip the search.
pub(crate) fn solid(value: u8) -> Bc4Block {
    Bc4Block {
        low: value,
        high: value,
        selectors: [0; 6],
    }
}

/// Pack sixteen one-channel values into a BC4 block.
///
/// The search is the reference's: endpoints at the extremes, and a texel's
/// selector read off how far up the interpolated ramp it sits. That test is
/// exact for BC4's uniform ramp, so there is nothing to improve by checking
/// all eight candidates.
pub(crate) fn pack(values: &[u8; 16]) -> Bc4Block {
    // Two single reductions rather than one over a pair: each of these over
    // sixteen bytes vectorizes to a handful of instructions, where the fold of
    // a tuple was compiled a byte at a time.
    let lowest = values.iter().fold(u8::MAX, |low, &value| low.min(value));
    let highest = values.iter().fold(u8::MIN, |high, &value| high.max(value));

    if lowest == highest {
        return solid(lowest);
    }

    // BC4 floors its interpolation divisions, compensated here by the bias.
    let delta = highest - lowest;
    let thresholds: [i32; 7] = [13, 11, 9, 7, 5, 3, 1].map(|step| delta as i32 * step);
    let bias = 4 - lowest as i32 * 14;

    // BC4 writes its top endpoint into the first byte, and the selector
    // ramp counts down from there; `STEPS` reads the same way.
    // Sixteen three-bit selectors are 48 bits, gathered in one word and
    // written once rather than read and rewritten texel by texel.
    let mut selectors = 0u64;
    for (texel, &value) in values.iter().enumerate() {
        let scaled = value as i32 * 14 + bias;
        let rank: usize = thresholds
            .iter()
            .map(|&threshold| usize::from(scaled >= threshold))
            .sum();
        selectors |= u64::from(STEPS[rank]) << (texel * 3);
    }
    let bytes = selectors.to_le_bytes();
    Bc4Block {
        low: highest,
        high: lowest,
        selectors: [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]],
    }
}
