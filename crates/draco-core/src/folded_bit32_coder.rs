#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
#[cfg(feature = "decoder")]
use crate::rans_bit_decoder::RAnsBitDecoder;
#[cfg(feature = "encoder")]
use crate::rans_bit_encoder::RAnsBitEncoder;

#[cfg(feature = "encoder")]
pub struct FoldedBit32Encoder {
    folded_number_encoders: Vec<RAnsBitEncoder>,
    bit_encoder: RAnsBitEncoder,
}

#[cfg(feature = "encoder")]
impl Default for FoldedBit32Encoder {
    fn default() -> Self {
        Self {
            folded_number_encoders: (0..32).map(|_| RAnsBitEncoder::new()).collect(),
            bit_encoder: RAnsBitEncoder::new(),
        }
    }
}

#[cfg(feature = "encoder")]
impl FoldedBit32Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_encoding(&mut self) {
        for enc in &mut self.folded_number_encoders {
            enc.start_encoding();
        }
        self.bit_encoder.start_encoding();
    }

    pub fn encode_bit(&mut self, bit: bool) {
        self.bit_encoder.encode_bit(bit);
    }

    pub fn encode_least_significant_bits32(&mut self, nbits: u32, value: u32) {
        assert!(nbits > 0 && nbits <= 32);
        let mut selector = 1u32 << (nbits - 1);
        for i in 0..nbits {
            let bit = (value & selector) != 0;
            self.folded_number_encoders[i as usize].encode_bit(bit);
            selector >>= 1;
        }
    }

    pub fn end_encoding(&mut self, target_buffer: &mut EncoderBuffer) {
        for enc in &mut self.folded_number_encoders {
            enc.end_encoding(target_buffer);
        }
        self.bit_encoder.end_encoding(target_buffer);
    }
}

#[cfg(feature = "decoder")]
pub struct FoldedBit32Decoder<'a> {
    folded_number_decoders: Vec<RAnsBitDecoder<'a>>,
    bit_decoder: RAnsBitDecoder<'a>,
}

#[cfg(feature = "decoder")]
impl<'a> Default for FoldedBit32Decoder<'a> {
    fn default() -> Self {
        Self {
            folded_number_decoders: (0..32).map(|_| RAnsBitDecoder::new()).collect(),
            bit_decoder: RAnsBitDecoder::new(),
        }
    }
}

#[cfg(feature = "decoder")]
impl<'a> FoldedBit32Decoder<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_decoding(&mut self, source_buffer: &mut DecoderBuffer<'a>) -> bool {
        for dec in &mut self.folded_number_decoders {
            if !dec.start_decoding(source_buffer) {
                return false;
            }
        }
        self.bit_decoder.start_decoding(source_buffer)
    }

    pub fn decode_next_bit(&mut self) -> bool {
        self.bit_decoder.decode_next_bit()
    }

    pub fn decode_least_significant_bits32(&mut self, nbits: u32, value: &mut u32) -> bool {
        if nbits == 0 || nbits > 32 {
            return false;
        }
        let mut result = 0u32;
        for i in 0..nbits {
            let Some(decoder) = self.folded_number_decoders.get_mut(i as usize) else {
                return false;
            };
            let bit = decoder.decode_next_bit();
            result = (result << 1) + (bit as u32);
        }
        *value = result;
        true
    }

    pub fn end_decoding(&mut self) {
        self.bit_decoder.end_decoding();
        for dec in &mut self.folded_number_decoders {
            dec.end_decoding();
        }
    }
}

#[cfg(all(test, feature = "decoder"))]
mod tests {
    use super::*;

    #[test]
    fn decode_least_significant_bits32_rejects_invalid_bit_counts() {
        let mut decoder = FoldedBit32Decoder::new();
        let mut value = 123;

        assert!(!decoder.decode_least_significant_bits32(0, &mut value));
        assert!(!decoder.decode_least_significant_bits32(33, &mut value));
        assert_eq!(value, 123);
    }
}
