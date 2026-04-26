use crate::status::DracoError;
use crate::version::DEFAULT_MESH_VERSION;
use std::mem;

/// Input buffer for reading compressed Draco data.
///
/// `DecoderBuffer` provides sequential byte and bit-level access to compressed data.
/// It supports both byte-aligned reads (integers, floats, strings) and bit-level
/// reads for entropy-coded data.
///
/// # Example
///
/// ```ignore
/// use draco_core::DecoderBuffer;
///
/// let data = &[0x44, 0x52, 0x41, 0x43, 0x4F]; // "DRACO" header
/// let mut buffer = DecoderBuffer::new(data);
///
/// assert_eq!(buffer.decode_u8().unwrap(), 0x44);
/// assert_eq!(buffer.remaining_size(), 4);
/// ```
pub struct DecoderBuffer<'a> {
    data: &'a [u8],
    pos: usize,
    bit_decoder_active: bool,
    bit_start_pos: usize,
    current_bit_offset: usize,
    bit_stream_end_pos: usize,
    bit_sequence_size_known: bool,
    version_major: u8,
    version_minor: u8,
}

impl<'a> DecoderBuffer<'a> {
    /// Creates a new `DecoderBuffer` from a byte slice.
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit_decoder_active: false,
            bit_start_pos: 0,
            current_bit_offset: 0,
            bit_stream_end_pos: 0,
            bit_sequence_size_known: false,
            // Default to latest mesh version to match encoder output format
            version_major: DEFAULT_MESH_VERSION.0,
            version_minor: DEFAULT_MESH_VERSION.1,
        }
    }

    /// Sets the Draco bitstream version for version-dependent decoding.
    pub fn set_version(&mut self, major: u8, minor: u8) {
        self.version_major = major;
        self.version_minor = minor;
    }

    /// Returns the major version number.
    pub fn version_major(&self) -> u8 {
        self.version_major
    }

    /// Returns the minor version number.
    pub fn version_minor(&self) -> u8 {
        self.version_minor
    }

    /// Returns the current read position in bytes.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Sets the read position.
    ///
    /// # Errors
    ///
    /// Returns `DracoError::BufferError` if:
    /// - Bit decoding is currently active
    /// - Position is beyond the buffer length
    pub fn set_position(&mut self, pos: usize) -> Result<(), DracoError> {
        if self.bit_decoder_active {
            return Err(DracoError::BufferError(
                "Cannot set position while bit decoding is active".into(),
            ));
        }
        if pos > self.data.len() {
            return Err(DracoError::BufferError(format!(
                "Position {} exceeds buffer length {}",
                pos,
                self.data.len()
            )));
        }
        self.pos = pos;
        Ok(())
    }

    /// Returns the number of bytes remaining in the buffer.
    pub fn remaining_size(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// Peeks at the next `len` bytes without advancing the position.
    pub fn peek_bytes(&self, len: usize) -> Vec<u8> {
        let end = std::cmp::min(self.pos + len, self.data.len());
        self.data[self.pos..end].to_vec()
    }

    /// Starts bit-level decoding mode.
    ///
    /// When `decode_size` is true, reads the bit sequence size from the buffer.
    /// Returns the size in bytes.
    ///
    /// # Errors
    ///
    /// Returns `DracoError::BufferError` if bit decoding is already active.
    pub fn start_bit_decoding(&mut self, decode_size: bool) -> Result<u64, DracoError> {
        if self.bit_decoder_active {
            return Err(DracoError::BufferError(
                "Bit decoding already active".into(),
            ));
        }
        let bitstream_version = ((self.version_major as u16) << 8) | (self.version_minor as u16);
        // Draco stores the bit-sequence size in BYTES (not bits) when |decode_size| is true.
        let mut size_bytes: u64 = 0;
        if decode_size {
            if bitstream_version < 0x0202 {
                if !cfg!(feature = "legacy_bitstream_decode") {
                    return Err(DracoError::BitstreamVersionUnsupported);
                }
                size_bytes = self.decode_u64()?;
            } else {
                size_bytes = self.decode_varint()?;
            }
        }

        self.bit_start_pos = self.pos;
        self.bit_decoder_active = true;
        self.current_bit_offset = 0;
        self.bit_sequence_size_known = decode_size;

        if decode_size {
            let size_bytes = usize::try_from(size_bytes)
                .map_err(|_| DracoError::BufferError("Bit stream size too large".into()))?;
            self.bit_stream_end_pos =
                self.bit_start_pos.checked_add(size_bytes).ok_or_else(|| {
                    DracoError::BufferError("Bit stream end position overflow".into())
                })?;
        } else {
            // If size is not encoded, assume the rest of the buffer.
            self.bit_stream_end_pos = self.data.len();
        }

        Ok(size_bytes)
    }

    /// Ends bit-level decoding mode and advances the byte position.
    pub fn end_bit_decoding(&mut self) {
        self.bit_decoder_active = false;
        // Draco behavior:
        // - When decoding with size known, the caller typically skips by the stored byte size.
        // - When decoding without size, advance by the number of decoded bits (rounded up).
        if self.bit_sequence_size_known {
            self.pos = self.bit_stream_end_pos;
        } else {
            let bytes_consumed = self.current_bit_offset.div_ceil(8);
            self.pos = self.bit_start_pos + bytes_consumed;
        }
    }

    /// Decodes `nbits` least significant bits as a u32.
    ///
    /// # Errors
    ///
    /// Returns `DracoError::BufferError` if bit decoding is not active or end of stream.
    #[inline(always)]
    pub fn decode_least_significant_bits32(&mut self, nbits: u32) -> Result<u32, DracoError> {
        if !self.bit_decoder_active {
            return Err(DracoError::BufferError("Bit decoding not active".into()));
        }
        self.decode_least_significant_bits32_fast(nbits)
    }

    /// Optimized version for hot paths - reads multiple bytes at once.
    #[inline(always)]
    pub fn decode_least_significant_bits32_fast(&mut self, nbits: u32) -> Result<u32, DracoError> {
        if nbits == 0 {
            return Ok(0);
        }

        let total_bit_offset = self.current_bit_offset;
        let byte_offset = self.bit_start_pos + total_bit_offset / 8;
        let bit_shift = (total_bit_offset % 8) as u32;

        if byte_offset >= self.bit_stream_end_pos || byte_offset >= self.data.len() {
            return Err(DracoError::BufferError(
                "Unexpected end of bit stream".into(),
            ));
        }
        let available_end = self.bit_stream_end_pos.min(self.data.len());
        let remaining = available_end - byte_offset;

        // Fast path: read 8 bytes at once when enough data remains (avoids per-byte loop).
        let raw = if remaining >= 8 {
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&self.data[byte_offset..byte_offset + 8]);
            u64::from_le_bytes(bytes)
        } else {
            let needed_bytes = ((bit_shift + nbits + 7) / 8) as usize;
            if remaining < needed_bytes {
                return Err(DracoError::BufferError(
                    "Unexpected end of bit stream".into(),
                ));
            }
            let mut v = 0u64;
            for i in 0..needed_bytes {
                v |= (self.data[byte_offset + i] as u64) << (i * 8);
            }
            v
        };
        let value = ((raw >> bit_shift) as u32) & ((1u32 << nbits) - 1);

        self.current_bit_offset += nbits as usize;
        Ok(value)
    }

    #[inline]
    #[allow(dead_code)]
    fn get_bit(&mut self) -> Result<u32, DracoError> {
        let total_bit_offset = self.current_bit_offset;
        let byte_offset = self.bit_start_pos + total_bit_offset / 8;
        let bit_shift = total_bit_offset % 8;

        if byte_offset < self.bit_stream_end_pos && byte_offset < self.data.len() {
            let bit = (self.data[byte_offset] >> bit_shift) & 1;
            self.current_bit_offset += 1;
            Ok(bit as u32)
        } else {
            Err(DracoError::BufferError(
                "Unexpected end of bit stream".into(),
            ))
        }
    }

    /// Decodes a value of type T using raw memory copy.
    ///
    /// # Errors
    ///
    /// Returns `DracoError::BufferError` if:
    /// - Bit decoding is active
    /// - Not enough bytes remaining
    pub fn decode<T: Copy + bytemuck::Pod>(&mut self) -> Result<T, DracoError> {
        if self.bit_decoder_active {
            return Err(DracoError::BufferError(
                "Cannot decode bytes while bit decoding is active".into(),
            ));
        }
        let size = mem::size_of::<T>();
        if self.pos + size > self.data.len() {
            return Err(DracoError::BufferError(format!(
                "Unexpected end of buffer: need {} bytes, have {}",
                size,
                self.remaining_size()
            )));
        }

        // Safety: bytemuck::Pod guarantees T can be safely read from any bit pattern
        let val = bytemuck::pod_read_unaligned::<T>(&self.data[self.pos..self.pos + size]);
        self.pos += size;
        Ok(val)
    }

    /// Decodes a single byte.
    pub fn decode_u8(&mut self) -> Result<u8, DracoError> {
        self.decode::<u8>()
    }

    /// Decodes a little-endian u16.
    pub fn decode_u16(&mut self) -> Result<u16, DracoError> {
        let mut bytes = [0u8; 2];
        self.decode_bytes(&mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    /// Decodes a little-endian u32.
    pub fn decode_u32(&mut self) -> Result<u32, DracoError> {
        let mut bytes = [0u8; 4];
        self.decode_bytes(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// Decodes a little-endian u64.
    pub fn decode_u64(&mut self) -> Result<u64, DracoError> {
        let mut bytes = [0u8; 8];
        self.decode_bytes(&mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }

    /// Decodes a little-endian f32.
    pub fn decode_f32(&mut self) -> Result<f32, DracoError> {
        let mut bytes = [0u8; 4];
        self.decode_bytes(&mut bytes)?;
        Ok(f32::from_le_bytes(bytes))
    }

    /// Decodes a little-endian f64.
    pub fn decode_f64(&mut self) -> Result<f64, DracoError> {
        let mut bytes = [0u8; 8];
        self.decode_bytes(&mut bytes)?;
        Ok(f64::from_le_bytes(bytes))
    }

    /// Decodes a null-terminated string.
    pub fn decode_string(&mut self) -> Result<String, DracoError> {
        let mut bytes = Vec::new();
        loop {
            let b = self.decode_u8()?;
            if b == 0 {
                break;
            }
            bytes.push(b);
        }
        String::from_utf8(bytes)
            .map_err(|e| DracoError::BufferError(format!("Invalid UTF-8 string: {}", e)))
    }

    /// Decodes bytes into the provided buffer.
    ///
    /// # Errors
    ///
    /// Returns `DracoError::BufferError` if not enough bytes remaining.
    pub fn decode_bytes(&mut self, out: &mut [u8]) -> Result<(), DracoError> {
        let size = out.len();
        if self.pos + size > self.data.len() {
            return Err(DracoError::BufferError(format!(
                "Unexpected end of buffer: need {} bytes, have {}",
                size,
                self.remaining_size()
            )));
        }
        out.copy_from_slice(&self.data[self.pos..self.pos + size]);
        self.pos += size;
        Ok(())
    }

    /// Decodes a variable-length unsigned integer (varint).
    pub fn decode_varint(&mut self) -> Result<u64, DracoError> {
        let mut val = 0u64;
        let mut shift = 0;
        loop {
            let b = self.decode_u8()?;
            val |= ((b & 0x7F) as u64) << shift;
            if (b & 0x80) == 0 {
                break;
            }
            shift += 7;
            if shift >= 64 {
                return Err(DracoError::BufferError("Varint exceeds 64 bits".into()));
            }
        }
        Ok(val)
    }

    /// Decodes a Draco-compatible signed varint.
    ///
    /// Uses unsigned varint encoding with ConvertSymbolToSignedInt transformation.
    pub fn decode_varint_signed_i32(&mut self) -> Result<i32, DracoError> {
        let symbol = self.decode_varint()? as u32;
        let is_positive = (symbol & 1) == 0;
        let v = symbol >> 1;
        if is_positive {
            Ok(v as i32)
        } else {
            Ok(-(v as i32) - 1)
        }
    }

    /// Returns a slice of the remaining data without advancing.
    pub fn remaining_data(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    /// Advances the position by `n` bytes without reading.
    pub fn advance(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n).min(self.data.len());
    }

    /// Advances the position by `n` bytes without reading.
    ///
    /// # Errors
    ///
    /// Returns `DracoError::BufferError` if the requested advance would move
    /// beyond the end of the input buffer.
    pub fn try_advance(&mut self, n: usize) -> Result<(), DracoError> {
        let new_pos = self
            .pos
            .checked_add(n)
            .ok_or_else(|| DracoError::BufferError("Buffer advance overflow".into()))?;
        if new_pos > self.data.len() {
            return Err(DracoError::BufferError(format!(
                "Cannot advance buffer by {} bytes: need position {}, buffer length {}",
                n,
                new_pos,
                self.data.len()
            )));
        }
        self.pos = new_pos;
        Ok(())
    }

    /// Decodes and returns a slice of the specified size.
    ///
    /// # Errors
    ///
    /// Returns `DracoError::BufferError` if not enough bytes remaining.
    pub fn decode_slice(&mut self, size: usize) -> Result<&'a [u8], DracoError> {
        if self.pos + size > self.data.len() {
            return Err(DracoError::BufferError(format!(
                "Unexpected end of buffer: need {} bytes, have {}",
                size,
                self.remaining_size()
            )));
        }
        let slice = &self.data[self.pos..self.pos + size];
        self.pos += size;
        Ok(slice)
    }
}

#[cfg(test)]
mod tests {
    use super::DecoderBuffer;

    #[test]
    fn bit_decode_respects_declared_byte_size() {
        let data = [1, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        let mut buffer = DecoderBuffer::new(&data);

        assert_eq!(buffer.start_bit_decoding(true).unwrap(), 1);
        assert!(buffer.decode_least_significant_bits32(16).is_err());
    }

    #[test]
    fn try_advance_rejects_out_of_bounds_skip() {
        let data = [0u8; 4];
        let mut buffer = DecoderBuffer::new(&data);

        assert!(buffer.try_advance(5).is_err());
        assert_eq!(buffer.position(), 0);
        assert!(buffer.try_advance(4).is_ok());
        assert_eq!(buffer.position(), 4);
    }
}
