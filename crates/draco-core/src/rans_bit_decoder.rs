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
                println!("DEBUG: RAnsBitDecoder prob_zero: {}", prob);
            }
            self.prob_zero = prob;
        } else {
            return false;
        }

        // Read size_in_bytes.
        // C++: v < 2.2 uses fixed u32, v >= 2.2 uses varint.
        let bitstream_version =
            ((source_buffer.version_major() as u16) << 8) | (source_buffer.version_minor() as u16);
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
            println!("DEBUG: RAnsBitDecoder size: {}", size);
        }

        if let Ok(slice) = source_buffer.decode_slice(size as usize) {
            #[cfg(feature = "debug_logs")]
            {
                println!("DEBUG: RAnsBitDecoder slice: {:?}", slice);
            }
            let mut decoder = AnsDecoder::new(slice);
            if decoder.read_init(crate::ans::ANS_L_BASE) {
                self.ans_decoder = Some(decoder);
                return true;
            }
        }

        false
    }

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
