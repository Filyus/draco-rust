//! Restating a UASTC block as the ASTC block it already nearly is.
//!
//! Ported from `BinomialLLC/basis_universal`, revision `9bebe16`, Apache-2.0:
//! `transcode_uastc_to_astc`, `pack_astc_block`, `pack_astc_solid_block`,
//! `astc_pack_bise` and the trit and quint encoders in
//! `transcoder/basisu_transcoder.cpp`.
//!
//! UASTC is a restricted profile of ASTC — same block size, same endpoint
//! quantization, same weight grid — so this is not a conversion but a rewrite
//! into ASTC's own bit layout. Nothing is approximated and no table of solved
//! endpoints is needed: the block mode word is looked up per UASTC mode, the
//! endpoints go back through ASTC's integer sequence encoding, and the weights
//! are written out bit-reversed from the end of the block.
//!
//! Which matters where: on Android and iOS ASTC is at essentially 100% while
//! s3tc is at 28% and 40%, so this is what lets a phone take a UASTC texture
//! compressed instead of expanding it eightfold into pixels.

use crate::astc_tables::{
    ASTC_PARTITION_SEED2, ASTC_PARTITION_SEED3, ASTC_PARTITION_SEED7, QUINT_ENCODE, TRIT_ENCODE,
    UASTC_MODE_ASTC_BLOCK_MODE, UASTC_MODE_CEM,
};
use crate::uastc::{
    bise_range, planes_of, Unpacked, MODE_ENDPOINT_RANGES, MODE_SUBSETS, MODE_WEIGHT_BITS,
};

/// Bit widths of the fields at the head of an ASTC block.
const BLOCK_MODE_BITS: u32 = 11;
const PART_BITS: u32 = 2;
const CEM_BITS: u32 = 4;
const PARTITION_INDEX_BITS: u32 = 10;
const CCS_BITS: u32 = 2;

/// Writes bits into a block, least significant first.
struct BitWriter {
    bytes: [u8; 16],
}

impl BitWriter {
    fn set(&mut self, offset: &mut u32, value: u32, count: u32) {
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
    fn set_small(&mut self, offset: &mut u32, value: u32, count: u32) {
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

fn extract(bits: u32, low: u32, high: u32) -> u32 {
    (bits >> low) & ((1 << (high - low + 1)) - 1)
}

/// Restate one unpacked UASTC block as an ASTC block.
pub(crate) fn convert(unpacked: &Unpacked) -> [u8; 16] {
    if unpacked.mode == 8 {
        return solid_block(unpacked.solid_color);
    }

    let mode = unpacked.mode;
    let mut writer = BitWriter { bytes: [0; 16] };
    let block_mode = UASTC_MODE_ASTC_BLOCK_MODE[mode] as u32;
    writer.bytes[0] = block_mode as u8;
    writer.bytes[1] = (block_mode >> 8) as u8;

    let subsets = MODE_SUBSETS[mode] as u32;
    let cem = UASTC_MODE_CEM[mode] as u32;
    let planes = planes_of(mode);
    let weight_bits = MODE_WEIGHT_BITS[mode] as u32;
    let total_weights = if planes == 2 { 32 } else { 16 };

    let mut offset = BLOCK_MODE_BITS;
    writer.set_small(&mut offset, subsets - 1, PART_BITS);
    if subsets == 1 {
        writer.set_small(&mut offset, cem, CEM_BITS);
    } else {
        writer.set(
            &mut offset,
            partition_seed(mode, unpacked.common_pattern),
            PARTITION_INDEX_BITS,
        );
        // Every subset shares one endpoint mode, which ASTC says by writing
        // the mode followed by two zero bits.
        writer.set_small(&mut offset, (cem << 2) & 63, CEM_BITS + 2);
    }

    if planes == 2 {
        // The component selector sits just below the weights, which grow up
        // from the end of the block.
        let mut ccs_offset = 128 - total_weights * weight_bits - CCS_BITS;
        writer.set_small(
            &mut ccs_offset,
            unpacked.component_selector as u32,
            CCS_BITS,
        );
    }

    let endpoint_pairs = (1 + (cem >> 2)) * subsets;
    pack_bise(
        &mut writer,
        &unpacked.endpoints,
        offset,
        (endpoint_pairs * 2) as usize,
        MODE_ENDPOINT_RANGES[mode] as usize,
    );

    // ASTC stores its weights from the end of the block backwards, each one
    // bit-reversed. Reversing a value of n bits is a table small enough to
    // build here rather than carry.
    for index in 0..total_weights as usize {
        let weight = unpacked.weights[index] as u32;
        let reversed = reverse_bits(weight, weight_bits);
        let at = 128 - weight_bits - index as u32 * weight_bits;
        let shifted = reversed << (at & 7);
        let byte = (at >> 3) as usize;
        writer.bytes[byte] |= shifted as u8;
        if byte + 1 < 16 {
            writer.bytes[byte + 1] |= (shifted >> 8) as u8;
        }
    }

    writer.bytes
}

/// The ASTC partition seed a UASTC pattern names.
fn partition_seed(mode: usize, pattern: u32) -> u32 {
    let at = pattern as usize;
    match mode {
        3 => ASTC_PARTITION_SEED3[at] as u32,
        7 => ASTC_PARTITION_SEED7[at] as u32,
        _ => ASTC_PARTITION_SEED2[at] as u32,
    }
}

/// Reverse the low `count` bits of `value`.
fn reverse_bits(value: u32, count: u32) -> u32 {
    let mut reversed = 0;
    for bit in 0..count {
        reversed |= ((value >> bit) & 1) << (count - 1 - bit);
    }
    reversed
}

/// A block of one colour, which ASTC calls a void extent.
fn solid_block(color: [u8; 4]) -> [u8; 16] {
    let mut writer = BitWriter { bytes: [0; 16] };
    // The void-extent block mode, with every coordinate range left undefined.
    writer.bytes[0] = 0xfc;
    writer.bytes[1] = 0xfd;
    writer.bytes[2] = 0xff;
    writer.bytes[3] = 0xff;
    for byte in writer.bytes.iter_mut().take(8).skip(4) {
        *byte = 0xff;
    }
    let mut offset = 64;
    for channel in color {
        // Sixteen bits per channel, the eight-bit value repeated.
        let value = channel as u32;
        writer.set(&mut offset, value | (value << 8), 16);
    }
    writer.bytes
}

/// Write values back through ASTC's integer sequence encoding.
fn pack_bise(writer: &mut BitWriter, values: &[u8], mut offset: u32, count: usize, range: usize) {
    let (bits, trits, quints) = bise_range(range);
    if trits {
        for group in values.chunks(5).take(count.div_ceil(5)) {
            let mut padded = [0u8; 5];
            padded[..group.len().min(5)].copy_from_slice(&group[..group.len().min(5)]);
            encode_trits(writer, &padded, &mut offset, bits);
        }
    } else if quints {
        for group in values.chunks(3).take(count.div_ceil(3)) {
            let mut padded = [0u8; 3];
            padded[..group.len().min(3)].copy_from_slice(&group[..group.len().min(3)]);
            encode_quints(writer, &padded, &mut offset, bits);
        }
    } else {
        for value in values.iter().take(count) {
            writer.set_small(&mut offset, *value as u32, bits);
        }
    }
}

/// Five values whose leftover digits are base three, packed as eight bits.
fn encode_trits(writer: &mut BitWriter, values: &[u8; 5], offset: &mut u32, bits: u32) {
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
fn encode_quints(writer: &mut BitWriter, values: &[u8; 3], offset: &mut u32, bits: u32) {
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
