//! Binary (two-symbol) rANS decoder.
//!
//! [`RAnsBitDecoder`] decodes a stream of bits whose probability of being zero
//! was modeled by the encoder. Used for boolean side-channels such as crease
//! flags and EdgeBreaker symbols. Port of Draco's `rans_bit_decoder.h`.

use crate::ans::AnsDecoder;
use crate::decoder_buffer::DecoderBuffer;

#[derive(Default)]
pub struct RAnsBitDecoder<'a> {
    ans_decoder: Option<AnsDecoder<'a>>,
    prob_zero: u8,
}

impl<'a> RAnsBitDecoder<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_decoding(&mut self, source_buffer: &mut DecoderBuffer<'a>) -> bool {
        self.clear();

        // Read zero_prob
        if let Ok(prob) = source_buffer.decode::<u8>() {
            #[cfg(feature = "debug_logs")]
            {
                debug_log!("DEBUG: RAnsBitDecoder prob_zero: {}", prob);
            }
            self.prob_zero = prob;
        } else {
            return false;
        }

        // Read size_in_bytes.
        // C++: v < 2.2 uses fixed u32, v >= 2.2 uses varint.
        let bitstream_version = source_buffer.bitstream_version();
        let size: u32 = if bitstream_version < 0x0202 {
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            {
                return false;
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            match source_buffer.decode::<u32>() {
                Ok(v) => v,
                Err(_) => return false,
            }
        } else {
            match source_buffer.decode_varint() {
                Ok(v) => v as u32,
                Err(_) => return false,
            }
        };
        #[cfg(feature = "debug_logs")]
        {
            debug_log!("DEBUG: RAnsBitDecoder size: {}", size);
        }

        if let Ok(slice) = source_buffer.decode_slice(size as usize) {
            #[cfg(feature = "debug_logs")]
            {
                debug_log!("DEBUG: RAnsBitDecoder slice: {:?}", slice);
            }
            let mut decoder = AnsDecoder::new(slice);
            // Binary rABS uses the 1..=3 byte final-state encoding (no 0xC0 tag).
            if decoder.read_init(crate::ans::ANS_L_BASE, false) {
                self.ans_decoder = Some(decoder);
                return true;
            }
        }

        false
    }

    /// Reads the next bit, answering `false` both for an encoded zero and for a
    /// decoder that was never started.
    ///
    /// Unlike [`DirectBitDecoder::decode_next_bit`] this cannot report reading
    /// past the encoded bits, and the difference is not an oversight. The
    /// obvious test -- the rABS state having fallen below `l_base` after
    /// renormalization, with no bytes left to refill it -- is not a fault
    /// signal: measured over this crate's own round-trip suite it holds on
    /// 21049 of 79359 reads (26%), every one of them in a stream this crate
    /// encoded and decoded back byte-exactly. The rABS tail legitimately draws
    /// from state alone. Upstream has no check here either.
    ///
    /// So over-reading an rANS stream has to be prevented by bounding the read
    /// count against something structural, which is what the callers do: the
    /// crease-edge flags against the corner count, the texture-coordinate
    /// orientations against the entry count.
    ///
    /// [`DirectBitDecoder::decode_next_bit`]: crate::direct_bit_decoder::DirectBitDecoder::decode_next_bit
    pub fn decode_next_bit(&mut self) -> bool {
        if let Some(decoder) = &mut self.ans_decoder {
            decoder.rabs_desc_read(self.prob_zero)
        } else {
            false
        }
    }

    pub fn decode_least_significant_bits32(&mut self, nbits: i32, value: &mut u32) -> bool {
        if nbits <= 0 || nbits > 32 || self.ans_decoder.is_none() {
            return false;
        }

        // Match Draco C++: accumulate bits MSB-first.
        *value = 0;
        for _ in 0..nbits {
            let bit = self.decode_next_bit();
            *value = (*value << 1) + (bit as u32);
        }
        true
    }

    pub fn end_decoding(&mut self) {
        self.ans_decoder = None;
    }

    fn clear(&mut self) {
        self.ans_decoder = None;
        self.prob_zero = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_least_significant_bits32_rejects_invalid_bit_counts() {
        let mut decoder = RAnsBitDecoder::new();
        let mut value = 123;

        assert!(!decoder.decode_least_significant_bits32(0, &mut value));
        assert!(!decoder.decode_least_significant_bits32(33, &mut value));
        assert_eq!(value, 123);
    }

    #[test]
    fn decode_least_significant_bits32_rejects_unstarted_decoder() {
        let mut decoder = RAnsBitDecoder::new();
        let mut value = 123;

        assert!(!decoder.decode_least_significant_bits32(1, &mut value));
        assert_eq!(value, 123);
    }
}
