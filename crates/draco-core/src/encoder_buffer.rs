use crate::version::DEFAULT_MESH_VERSION;

/// Output buffer for writing Draco bitstream data.
///
/// `EncoderBuffer` accumulates encoded bytes and supports both byte-aligned
/// writes and packed bit-level encoding for entropy-coded sections. Encoders
/// write into it; after encoding, [`data`](EncoderBuffer::data) returns the
/// finished bitstream as a byte slice ready to hand to a `DecoderBuffer` or
/// write to a file.
pub struct EncoderBuffer {
    buffer: Vec<u8>,
    bit_encoder_active: bool,
    bit_start_pos: usize,
    current_bit_offset: usize,
    version_major: u8,
    version_minor: u8,
    encode_bit_sequence_size: bool,
}

impl Default for EncoderBuffer {
    fn default() -> Self {
        Self {
            buffer: Vec::new(),
            bit_encoder_active: false,
            bit_start_pos: 0,
            current_bit_offset: 0,
            // Default to the latest mesh version so standalone encode/decode
            // (e.g. encode_symbols/decode_symbols) agrees with DecoderBuffer::new().
            version_major: DEFAULT_MESH_VERSION.0,
            version_minor: DEFAULT_MESH_VERSION.1,
            encode_bit_sequence_size: false,
        }
    }
}

impl EncoderBuffer {
    /// Creates an empty encoder buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the target Draco bitstream version.
    pub fn set_version(&mut self, major: u8, minor: u8) {
        self.version_major = major;
        self.version_minor = minor;
    }

    /// Returns the target major bitstream version.
    pub fn version_major(&self) -> u8 {
        self.version_major
    }

    /// Returns the target minor bitstream version.
    pub fn version_minor(&self) -> u8 {
        self.version_minor
    }

    /// Clears all encoded bytes and resets active bit encoding state.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.bit_encoder_active = false;
        self.current_bit_offset = 0;
    }

    /// Resizes the underlying byte buffer.
    pub fn resize(&mut self, nbytes: usize) {
        self.buffer.resize(nbytes, 0);
    }

    /// Starts writing a packed bit sequence.
    pub fn start_bit_encoding(&mut self, required_bits: usize, encode_size: bool) -> bool {
        if self.bit_encoder_active {
            return false;
        }
        self.encode_bit_sequence_size = encode_size;
        if encode_size {
            // Reserve 8 bytes for the size (will be replaced by varint or fixed 8 bytes later)
            for _ in 0..8 {
                self.buffer.push(0);
            }
        }
        let required_bytes = required_bits.div_ceil(8);
        self.bit_start_pos = self.buffer.len();
        self.buffer.resize(self.bit_start_pos + required_bytes, 0);
        self.bit_encoder_active = true;
        self.current_bit_offset = 0;
        true
    }

    /// Finishes the active packed bit sequence.
    pub fn end_bit_encoding(&mut self) {
        if !self.bit_encoder_active {
            return;
        }
        self.bit_encoder_active = false;

        if self.encode_bit_sequence_size {
            let encoded_bits = self.current_bit_offset;
            let encoded_bytes = encoded_bits.div_ceil(8);
            let bitstream_version =
                ((self.version_major as u16) << 8) | (self.version_minor as u16);

            let mut var_size_buffer = Vec::new();
            if bitstream_version >= 0x0202 {
                // Encode size as varint
                let mut v = encoded_bytes as u64;
                loop {
                    let mut byte = (v & 0x7F) as u8;
                    v >>= 7;
                    if v != 0 {
                        byte |= 0x80;
                        var_size_buffer.push(byte);
                    } else {
                        var_size_buffer.push(byte);
                        break;
                    }
                }
            } else {
                // Encode size as fixed 8 bytes
                var_size_buffer.extend_from_slice(&(encoded_bytes as u64).to_le_bytes());
            }

            let size_len = var_size_buffer.len();
            let reserved_pos = self.bit_start_pos - 8;

            // Move encoded data to its final position
            let src_pos = self.bit_start_pos;
            let dst_pos = reserved_pos + size_len;

            if dst_pos != src_pos {
                self.buffer
                    .copy_within(src_pos..src_pos + encoded_bytes, dst_pos);
            }

            // Write the size
            self.buffer[reserved_pos..reserved_pos + size_len].copy_from_slice(&var_size_buffer);

            // Resize buffer to final size
            self.buffer.resize(dst_pos + encoded_bytes, 0);
        } else {
            // Just resize to actual encoded bytes
            let encoded_bytes = self.current_bit_offset.div_ceil(8);
            self.buffer.resize(self.bit_start_pos + encoded_bytes, 0);
        }
    }

    /// Writes the least significant `nbits` bits of a 32-bit value.
    pub fn encode_least_significant_bits32(&mut self, nbits: u32, value: u32) -> bool {
        if !self.bit_encoder_active {
            return false;
        }
        if nbits == 0 {
            return true;
        }

        // Pack bits efficiently into the underlying byte buffer.
        // Bits are written LSB-first, matching Draco's EncoderBuffer.
        let mut remaining_bits = nbits;
        let mut v = value;
        while remaining_bits > 0 {
            let total_bit_offset = self.current_bit_offset;
            let byte_offset = self.bit_start_pos + (total_bit_offset / 8);
            let bit_shift = (total_bit_offset % 8) as u32;
            let available = 8u32 - bit_shift;
            let take = remaining_bits.min(available);
            let mask = (1u32 << take) - 1;
            let bits = v & mask;
            self.buffer[byte_offset] |= (bits as u8) << bit_shift;

            v >>= take;
            remaining_bits -= take;
            self.current_bit_offset += take as usize;
        }
        true
    }

    /// Appends a plain-old-data value in native memory representation.
    pub fn encode<T: bytemuck::NoUninit>(&mut self, data: T) -> bool {
        if self.bit_encoder_active {
            return false;
        }
        let slice = bytemuck::bytes_of(&data);
        self.buffer.extend_from_slice(slice);
        true
    }

    /// Appends raw bytes.
    pub fn encode_data(&mut self, data: &[u8]) -> bool {
        if self.bit_encoder_active {
            return false;
        }
        self.buffer.extend_from_slice(data);
        true
    }

    /// Appends one byte.
    pub fn encode_u8(&mut self, val: u8) {
        self.buffer.push(val);
    }

    /// Appends a little-endian `u16`.
    pub fn encode_u16(&mut self, val: u16) {
        self.buffer.extend_from_slice(&val.to_le_bytes());
    }

    /// Appends a little-endian `u32`.
    pub fn encode_u32(&mut self, val: u32) {
        self.buffer.extend_from_slice(&val.to_le_bytes());
    }

    /// Appends a little-endian `u64`.
    pub fn encode_u64(&mut self, val: u64) {
        self.buffer.extend_from_slice(&val.to_le_bytes());
    }

    /// Appends an unsigned LEB128-style Draco varint.
    pub fn encode_varint<T: Into<u64>>(&mut self, val: T) {
        let mut v = val.into();
        loop {
            let mut byte = (v & 0x7F) as u8;
            v >>= 7;
            if v != 0 {
                byte |= 0x80;
                self.buffer.push(byte);
            } else {
                self.buffer.push(byte);
                break;
            }
        }
    }

    /// Draco-compatible signed varint (ConvertSignedIntToSymbol + unsigned varint).
    pub fn encode_varint_signed_i32(&mut self, val: i32) {
        let symbol: u32 = if val >= 0 {
            (val as u32) << 1
        } else {
            let mapped = (-(val + 1)) as u32;
            (mapped << 1) | 1
        };
        self.encode_varint(symbol as u64);
    }

    /// Returns the encoded byte slice.
    pub fn data(&self) -> &[u8] {
        &self.buffer
    }

    /// Returns the encoded byte length.
    pub fn size(&self) -> usize {
        self.buffer.len()
    }
}
