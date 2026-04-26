use crate::encoder_buffer::EncoderBuffer;

#[derive(Default)]
pub struct DirectBitEncoder {
    bits: Vec<u32>,
    local_bits: u32,
    num_local_bits: u32,
}

impl DirectBitEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_encoding(&mut self) {
        self.clear();
    }

    pub fn clear(&mut self) {
        self.bits.clear();
        self.local_bits = 0;
        self.num_local_bits = 0;
    }

    pub fn encode_bit(&mut self, bit: bool) {
        if bit {
            self.local_bits |= 1u32 << (31 - self.num_local_bits);
        }
        self.num_local_bits += 1;
        if self.num_local_bits == 32 {
            self.bits.push(self.local_bits);
            self.num_local_bits = 0;
            self.local_bits = 0;
        }
    }

    pub fn encode_least_significant_bits32(&mut self, nbits: u32, mut value: u32) {
        assert!(nbits > 0 && nbits <= 32);

        let remaining = 32 - self.num_local_bits;

        // Make sure there are no leading bits that should not be encoded.
        value <<= 32 - nbits;

        if nbits <= remaining {
            value >>= self.num_local_bits;
            self.local_bits |= value;
            self.num_local_bits += nbits;
            if self.num_local_bits == 32 {
                self.bits.push(self.local_bits);
                self.local_bits = 0;
                self.num_local_bits = 0;
            }
        } else {
            value >>= 32 - nbits;
            self.num_local_bits = nbits - remaining;
            let value_l = value >> self.num_local_bits;
            self.local_bits |= value_l;
            self.bits.push(self.local_bits);
            self.local_bits = value << (32 - self.num_local_bits);
        }
    }

    pub fn end_encoding(&mut self, target_buffer: &mut EncoderBuffer) {
        self.bits.push(self.local_bits);
        let size_in_bytes = (self.bits.len() as u32) * 4;
        target_buffer.encode_u32(size_in_bytes);
        for &w in &self.bits {
            target_buffer.encode_u32(w);
        }
        self.clear();
    }
}
