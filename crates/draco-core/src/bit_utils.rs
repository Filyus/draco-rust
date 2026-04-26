pub struct BitEncoder<'a> {
    buffer: &'a mut [u8],
    bit_offset: usize,
}

impl<'a> BitEncoder<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            buffer,
            bit_offset: 0,
        }
    }

    pub fn put_bits(&mut self, data: u32, nbits: u32) {
        for i in 0..nbits {
            self.put_bit((data >> i) & 1);
        }
    }

    pub fn put_bit(&mut self, bit: u32) {
        let byte_offset = self.bit_offset / 8;
        let bit_shift = self.bit_offset % 8;
        if byte_offset < self.buffer.len() {
            if bit != 0 {
                self.buffer[byte_offset] |= 1 << bit_shift;
            } else {
                self.buffer[byte_offset] &= !(1 << bit_shift);
            }
        }
        self.bit_offset += 1;
    }

    pub fn bits(&self) -> usize {
        self.bit_offset
    }
}

pub struct BitDecoder<'a> {
    buffer: &'a [u8],
    bit_offset: usize,
}

impl<'a> BitDecoder<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Self {
            buffer,
            bit_offset: 0,
        }
    }

    pub fn get_bits(&mut self, nbits: u32) -> Option<u32> {
        let mut value = 0;
        for i in 0..nbits {
            let bit = self.get_bit()?;
            value |= bit << i;
        }
        Some(value)
    }

    pub fn get_bit(&mut self) -> Option<u32> {
        let byte_offset = self.bit_offset / 8;
        let bit_shift = self.bit_offset % 8;
        if byte_offset < self.buffer.len() {
            let bit = (self.buffer[byte_offset] >> bit_shift) & 1;
            self.bit_offset += 1;
            Some(bit as u32)
        } else {
            None
        }
    }

    pub fn bits_decoded(&self) -> usize {
        self.bit_offset
    }
}

pub fn reverse_bits32(mut n: u32) -> u32 {
    n = ((n >> 1) & 0x55555555) | ((n & 0x55555555) << 1);
    n = ((n >> 2) & 0x33333333) | ((n & 0x33333333) << 2);
    n = ((n >> 4) & 0x0F0F0F0F) | ((n & 0x0F0F0F0F) << 4);
    n = ((n >> 8) & 0x00FF00FF) | ((n & 0x00FF00FF) << 8);
    n.rotate_left(16)
}

pub fn count_one_bits32(n: u32) -> u32 {
    n.count_ones()
}
