use crate::ans::AnsCoder;
use crate::bit_utils::{count_one_bits32, reverse_bits32};
use crate::encoder_buffer::EncoderBuffer;

#[derive(Default)]
pub struct RAnsBitEncoder {
    bit_counts: [u64; 2],
    bits: Vec<u32>,
    local_bits: u32,
    num_local_bits: u32,
}

impl RAnsBitEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_encoding(&mut self) {
        self.clear();
    }

    pub fn clear(&mut self) {
        self.bit_counts = [0; 2];
        self.bits.clear();
        self.local_bits = 0;
        self.num_local_bits = 0;
    }

    pub fn encode_bit(&mut self, bit: bool) {
        if bit {
            self.bit_counts[1] += 1;
            self.local_bits |= 1 << self.num_local_bits;
        } else {
            self.bit_counts[0] += 1;
        }
        self.num_local_bits += 1;

        if self.num_local_bits == 32 {
            self.bits.push(self.local_bits);
            self.num_local_bits = 0;
            self.local_bits = 0;
        }
    }

    pub fn encode_least_significant_bits32(&mut self, nbits: u32, value: u32) {
        assert!(nbits <= 32);
        assert!(nbits > 0);

        let reversed = reverse_bits32(value) >> (32 - nbits);
        let ones = count_one_bits32(reversed);
        self.bit_counts[0] += (nbits - ones) as u64;
        self.bit_counts[1] += ones as u64;

        let remaining = 32 - self.num_local_bits;

        if nbits <= remaining {
            self.local_bits |= reversed << self.num_local_bits;
            self.num_local_bits += nbits;
            if self.num_local_bits == 32 {
                self.bits.push(self.local_bits);
                self.local_bits = 0;
                self.num_local_bits = 0;
            }
        } else {
            self.local_bits |= reversed << self.num_local_bits;
            self.bits.push(self.local_bits);
            self.local_bits = reversed >> remaining;
            self.num_local_bits = nbits - remaining;
        }
    }

    pub fn end_encoding(&mut self, target_buffer: &mut EncoderBuffer) {
        #[cfg(feature = "debug_logs")]
        {
            println!(
                "DEBUG: RAnsBitEncoder bit_counts: [{}, {}]",
                self.bit_counts[0], self.bit_counts[1]
            );
        }
        let total = self.bit_counts[1] + self.bit_counts[0];
        let total = if total == 0 { 1 } else { total };

        let zero_prob_raw = ((self.bit_counts[0] as f64 / total as f64) * 256.0 + 0.5) as u32;
        let mut zero_prob = if zero_prob_raw < 255 {
            zero_prob_raw as u8
        } else {
            255
        };
        if zero_prob == 0 {
            zero_prob += 1;
        }

        let mut ans_coder = AnsCoder::new();
        ans_coder.write_init(crate::ans::ANS_L_BASE);

        // Encode remaining local bits
        for i in (0..self.num_local_bits).rev() {
            let bit = (self.local_bits >> i) & 1;
            ans_coder.rabs_desc_write(bit != 0, zero_prob);
        }

        // Encode stored bits
        for &val in self.bits.iter().rev() {
            for i in (0..32).rev() {
                let bit = (val >> i) & 1;
                ans_coder.rabs_desc_write(bit != 0, zero_prob);
            }
        }

        let size = ans_coder
            .write_end()
            .expect("ANS state should always be valid for bit encoding");

        target_buffer.encode_u8(zero_prob);
        #[cfg(feature = "debug_logs")]
        {
            println!("DEBUG: RAnsBitEncoder zero_prob: {}", zero_prob);
        }

        target_buffer.encode_varint(size as u64);
        #[cfg(feature = "debug_logs")]
        {
            println!("DEBUG: RAnsBitEncoder size: {}", size);
        }

        let data = ans_coder.data();
        #[cfg(feature = "debug_logs")]
        {
            println!("DEBUG: RAnsBitEncoder data: {:?}", data);
        }
        target_buffer.encode_data(data);
    }
}
