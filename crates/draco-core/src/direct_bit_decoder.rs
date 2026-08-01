//! Uncompressed (direct) bit decoder.
//!
//! [`DirectBitDecoder`] reads raw, non-entropy-coded bits packed into 32-bit
//! words. Used where Draco stores values verbatim rather than rANS-coded. Port
//! of Draco's `direct_bit_decoder.h`.

use crate::decoder_buffer::DecoderBuffer;

#[derive(Default)]
pub struct DirectBitDecoder {
    bits: Vec<u32>,
    pos: usize,
    num_used_bits: u32,
}

impl DirectBitDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.bits.clear();
        self.pos = 0;
        self.num_used_bits = 0;
    }

    pub fn start_decoding<'a>(&mut self, source_buffer: &mut DecoderBuffer<'a>) -> bool {
        self.clear();
        let size_in_bytes = match source_buffer.decode_u32() {
            Ok(v) => v,
            Err(_) => return false,
        };

        if size_in_bytes == 0 || (size_in_bytes & 0x3) != 0 {
            return false;
        }
        let num_words = (size_in_bytes / 4) as usize;
        if source_buffer.remaining_size() < size_in_bytes as usize {
            return false;
        }
        self.bits.reserve(num_words);
        for _ in 0..num_words {
            let w = match source_buffer.decode_u32() {
                Ok(v) => v,
                Err(_) => return false,
            };
            self.bits.push(w);
        }
        self.pos = 0;
        self.num_used_bits = 0;
        true
    }

    /// Reads the next bit, or `None` once the buffer is spent.
    ///
    /// Returned an unqualified `bool` through 1.x, so a caller reading past the
    /// end got a zero bit and no way to tell it apart from an encoded zero --
    /// and the loops that drive these reads take their counts from the same
    /// stream. Here exhaustion is exact: the bits live in a `Vec<u32>` filled
    /// by `start_decoding`, and `pos` past its end is the whole condition.
    ///
    /// Its rANS sibling cannot answer the same question; see
    /// [`RAnsBitDecoder::decode_next_bit`].
    ///
    /// [`RAnsBitDecoder::decode_next_bit`]: crate::rans_bit_decoder::RAnsBitDecoder::decode_next_bit
    pub fn decode_next_bit(&mut self) -> Option<bool> {
        if self.pos >= self.bits.len() {
            return None;
        }
        let selector = 1u32 << (31 - self.num_used_bits);
        let bit = (self.bits[self.pos] & selector) != 0;
        self.num_used_bits += 1;
        if self.num_used_bits == 32 {
            self.pos += 1;
            self.num_used_bits = 0;
        }
        Some(bit)
    }

    pub fn decode_least_significant_bits32(&mut self, nbits: u32, value: &mut u32) -> bool {
        if nbits == 0 || nbits > 32 {
            return false;
        }
        let remaining = 32 - self.num_used_bits;
        if nbits <= remaining {
            let Some(&word) = self.bits.get(self.pos) else {
                return false;
            };
            *value = (word << self.num_used_bits) >> (32 - nbits);
            self.num_used_bits += nbits;
            if self.num_used_bits == 32 {
                self.pos += 1;
                self.num_used_bits = 0;
            }
        } else {
            let Some(next_pos) = self.pos.checked_add(1) else {
                return false;
            };
            let (Some(&word_l), Some(&word_r)) = (self.bits.get(self.pos), self.bits.get(next_pos))
            else {
                return false;
            };
            let value_l = word_l << self.num_used_bits;
            self.num_used_bits = nbits - remaining;
            self.pos = next_pos;
            let value_r = word_r >> (32 - self.num_used_bits);
            *value = (value_l >> (32 - self.num_used_bits - remaining)) | value_r;
        }
        true
    }

    pub fn end_decoding(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_least_significant_bits32_rejects_invalid_bit_counts() {
        let mut decoder = DirectBitDecoder::new();
        decoder.bits.push(0xffff_ffff);
        let mut value = 123;

        assert!(!decoder.decode_least_significant_bits32(0, &mut value));
        assert!(!decoder.decode_least_significant_bits32(33, &mut value));
        assert_eq!(value, 123);
    }

    #[test]
    fn decode_least_significant_bits32_rejects_missing_second_word() {
        let mut decoder = DirectBitDecoder::new();
        decoder.bits.push(0xffff_ffff);
        decoder.num_used_bits = 31;
        let mut value = 0;

        assert!(!decoder.decode_least_significant_bits32(2, &mut value));
    }

    /// A read past the last bit is `None`, not a zero bit.
    ///
    /// This returned `false` through 1.x, which a caller could not tell from an
    /// encoded zero -- and the loops driving these reads take their counts from
    /// the same stream, so a count that overshoots read zeros forever.
    #[test]
    fn a_read_past_the_last_bit_is_not_a_zero_bit() {
        let mut decoder = DirectBitDecoder::new();
        decoder.bits.push(0x8000_0000); // one set bit, then 31 zeros

        assert_eq!(decoder.decode_next_bit(), Some(true));
        for _ in 1..32 {
            assert_eq!(decoder.decode_next_bit(), Some(false));
        }
        assert_eq!(
            decoder.decode_next_bit(),
            None,
            "the 33rd bit is past the only word"
        );
    }
}
