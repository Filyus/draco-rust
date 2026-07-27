//! Writing values into an ASTC block, which both sources share.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `astc_set_bits`, `astc_encode_trits` and the quint encoder in
//! `transcoder/basisu_transcoder.cpp`.
//!
//! ASTC's own bit layout, with nothing about either Basis codec in it: a
//! block's fields are written least significant bit first from the front, its
//! weights bit-reversed from the back, and any value quantized to a range with
//! trits or quints goes through the integer sequence encoding here.

#[cfg(feature = "uastc")]
use crate::astc_tables::QUINT_ENCODE;
use crate::astc_tables::TRIT_ENCODE;

/// Writes bits into a block, least significant first.
pub(crate) struct BitWriter {
    pub bytes: [u8; 16],
}

impl BitWriter {
    pub fn set(&mut self, offset: &mut u32, value: u32, count: u32) {
        let mut value = value;
        let mut count = count;
        while count > 0 {
            let taken = count.min(8 - (*offset & 7));
            self.bytes[(*offset >> 3) as usize] |= (value << (*offset & 7)) as u8;
            *offset += taken;
            count -= taken;
            value >>= taken;
        }
    }

    /// The reference's narrow path, which writes at most nine bits and never
    /// straddles more than two bytes.
    #[cfg(feature = "uastc")]
    pub fn set_small(&mut self, offset: &mut u32, value: u32, count: u32) {
        if count == 0 {
            return;
        }
        let shift = *offset & 7;
        let shifted = value << shift;
        let index = (*offset >> 3) as usize;
        self.bytes[index] |= shifted as u8;
        if count > 8 - shift {
            self.bytes[index + 1] |= (shifted >> 8) as u8;
        }
        *offset += count;
    }
}

pub(crate) fn extract(bits: u32, low: u32, high: u32) -> u32 {
    (bits >> low) & ((1 << (high - low + 1)) - 1)
}

/// Five values whose leftover digits are base three, packed as eight bits.
pub(crate) fn encode_trits(writer: &mut BitWriter, values: &[u8; 5], offset: &mut u32, bits: u32) {
    const MULTIPLIERS: [u32; 5] = [1, 3, 9, 27, 81];
    let mask = (1u32 << bits) - 1;
    let mut trits = 0u32;
    let mut low = [0u32; 5];
    for (index, value) in values.iter().enumerate() {
        trits += (*value as u32 >> bits) * MULTIPLIERS[index];
        low[index] = *value as u32 & mask;
    }
    let packed = TRIT_ENCODE[trits as usize] as u32;

    writer.set(
        offset,
        low[0] | (extract(packed, 0, 1) << bits) | (low[1] << (2 + bits)),
        bits * 2 + 2,
    );
    writer.set(
        offset,
        extract(packed, 2, 3)
            | (low[2] << 2)
            | (extract(packed, 4, 4) << (2 + bits))
            | (low[3] << (3 + bits))
            | (extract(packed, 5, 6) << (3 + bits * 2))
            | (low[4] << (5 + bits * 2))
            | (extract(packed, 7, 7) << (5 + bits * 3)),
        bits * 3 + 6,
    );
}

/// Three values whose leftover digits are base five, packed as seven bits.
///
/// Only the UASTC path meets a quint range; ETC1S packs into [0,47], which is
/// bits and a trit.
#[cfg(feature = "uastc")]
pub(crate) fn encode_quints(writer: &mut BitWriter, values: &[u8; 3], offset: &mut u32, bits: u32) {
    const MULTIPLIERS: [u32; 3] = [1, 5, 25];
    let mask = (1u32 << bits) - 1;
    let mut quints = 0u32;
    let mut low = [0u32; 3];
    for (index, value) in values.iter().enumerate() {
        quints += (*value as u32 >> bits) * MULTIPLIERS[index];
        low[index] = *value as u32 & mask;
    }
    let packed = QUINT_ENCODE[quints as usize] as u32;

    writer.set(
        offset,
        low[0]
            | (extract(packed, 0, 2) << bits)
            | (low[1] << (3 + bits))
            | (extract(packed, 3, 4) << (3 + bits * 2))
            | (low[2] << (5 + bits * 2))
            | (extract(packed, 5, 6) << (5 + bits * 3)),
        7 + bits * 3,
    );
}
