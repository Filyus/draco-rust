pub const ANS_P8_PRECISION: u32 = 256;
pub const ANS_L_BASE: u32 = 4096;
pub const ANS_IO_BASE: u32 = 256;

pub struct AnsCoder {
    pub buf: Vec<u8>,
    pub state: u32,
    pub l_base: u32,
}

impl Default for AnsCoder {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            state: ANS_L_BASE,
            l_base: ANS_L_BASE,
        }
    }
}

impl AnsCoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write_init(&mut self, l_base: u32) {
        self.buf.clear();
        self.l_base = l_base;
        self.state = l_base;
    }

    pub fn write_end(&mut self) -> Result<usize, crate::status::DracoError> {
        let state = self.state - self.l_base;
        if state < (1 << 6) {
            self.buf.push(state as u8);
        } else if state < (1 << 14) {
            self.buf.push((state & 0xFF) as u8);
            self.buf.push(((0x01 << 6) + ((state >> 8) & 0x3F)) as u8);
        } else if state < (1 << 22) {
            self.buf.push((state & 0xFF) as u8);
            self.buf.push(((state >> 8) & 0xFF) as u8);
            self.buf.push(((0x02 << 6) + ((state >> 16) & 0x3F)) as u8);
        } else if state < (1 << 30) {
            self.buf.push((state & 0xFF) as u8);
            self.buf.push(((state >> 8) & 0xFF) as u8);
            self.buf.push(((state >> 16) & 0xFF) as u8);
            self.buf.push(((0x03 << 6) + ((state >> 24) & 0x3F)) as u8);
        } else {
            return Err(crate::status::DracoError::DracoError(format!(
                "State is too large to be serialized: {}",
                state
            )));
        }
        Ok(self.buf.len())
    }

    #[inline]
    pub fn rabs_desc_write(&mut self, val: bool, p0: u8) {
        let p = ANS_P8_PRECISION - p0 as u32;
        let l_s = if val { p } else { p0 as u32 };

        if self.state >= ANS_L_BASE / ANS_P8_PRECISION * ANS_IO_BASE * l_s {
            // ANS_IO_BASE is 256.
            self.buf.push((self.state & 0xFF) as u8);
            self.state >>= 8;
        }

        let quot = self.state / l_s;
        let rem = self.state - quot * l_s;
        self.state = quot * ANS_P8_PRECISION + rem + if val { 0 } else { p };
    }

    #[inline]
    pub fn rabs_desc_write_bits(&mut self, val: u32, bit_length: u32) {
        let limit = (self.l_base >> bit_length) * ANS_IO_BASE;
        if self.state >= limit {
            // ANS_IO_BASE is 256.
            self.buf.push((self.state & 0xFF) as u8);
            self.state >>= 8;
        }

        let mask = (1 << bit_length) - 1;
        let quot = self.state >> bit_length;
        let rem = self.state & mask;

        self.state = (quot << (bit_length + 8)) + rem + val;
    }

    pub fn data(&self) -> &[u8] {
        &self.buf
    }
}

pub struct AnsDecoder<'a> {
    pub buf: &'a [u8],
    pub buf_offset: usize,
    pub state: u32,
    pub l_base: u32,
}

impl<'a> AnsDecoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            buf_offset: 0,
            state: 0,
            l_base: ANS_L_BASE,
        }
    }

    #[inline(always)]
    pub fn read_normalize(&mut self) {
        while self.state < self.l_base && self.buf_offset > 0 {
            self.buf_offset -= 1;
            self.state = (self.state << 8) | self.buf[self.buf_offset] as u32;
        }
    }

    pub fn read_init(&mut self, l_base: u32) -> bool {
        self.l_base = l_base;
        self.buf_offset = self.buf.len();
        if self.buf_offset == 0 {
            return false;
        }

        let val = self.buf[self.buf_offset - 1];
        self.buf_offset -= 1;

        if (val & 0xC0) == 0x00 {
            self.state = (val & 0x3F) as u32 + self.l_base;
        } else if (val & 0xC0) == 0x40 {
            if self.buf_offset == 0 {
                return false;
            }
            let val0 = self.buf[self.buf_offset - 1];
            self.buf_offset -= 1;
            let state = ((val as u32 & 0x3F) << 8) | val0 as u32;
            self.state = state + self.l_base;
        } else if (val & 0xC0) == 0x80 {
            if self.buf_offset < 2 {
                return false;
            }
            let val0 = self.buf[self.buf_offset - 1];
            let val1 = self.buf[self.buf_offset - 2];
            self.buf_offset -= 2;
            let state = ((val as u32 & 0x3F) << 16) | ((val0 as u32) << 8) | val1 as u32;
            self.state = state + self.l_base;
        } else
        /* 0xC0 */
        {
            if self.buf_offset < 3 {
                return false;
            }
            let val0 = self.buf[self.buf_offset - 1];
            let val1 = self.buf[self.buf_offset - 2];
            let val2 = self.buf[self.buf_offset - 3];
            self.buf_offset -= 3;
            let state = ((val as u32 & 0x3F) << 24)
                | ((val0 as u32) << 16)
                | ((val1 as u32) << 8)
                | val2 as u32;
            self.state = state + self.l_base;
        }
        true
    }

    pub fn rabs_desc_read(&mut self, p0: u8) -> bool {
        let p = ANS_P8_PRECISION - p0 as u32;
        self.read_normalize();

        let x = self.state;
        let quot = x / ANS_P8_PRECISION;
        let rem = x % ANS_P8_PRECISION;
        let xn = quot * p;
        let val = rem < p;

        if val {
            self.state = xn + rem;
        } else {
            self.state = x - xn - p;
        }
        val
    }
}
