//! Raw byte buffer for attribute data.
//!
//! [`DataBuffer`] is the contiguous little-endian byte store behind every
//! [`PointAttribute`](crate::PointAttribute), with typed read/write helpers;
//! [`DataBufferDescriptor`] records its id and size. Port of Draco's
//! `data_buffer.h`.

use std::io::{self, Write};

#[derive(Debug, Default, Clone)]
pub struct DataBufferDescriptor {
    pub buffer_id: i64,
    pub buffer_update_count: i64,
}

#[derive(Debug, Default, Clone)]
pub struct DataBuffer {
    data: Vec<u8>,
    descriptor: DataBufferDescriptor,
}

impl DataBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, data: &[u8], offset: Option<usize>) {
        let offset = offset.unwrap_or(0);
        let end = offset + data.len();

        if end > self.data.len() {
            self.data.resize(end, 0);
        }

        self.data[offset..end].copy_from_slice(data);
        self.descriptor.buffer_update_count += 1;
    }

    pub fn resize(&mut self, new_size: usize) {
        self.data.resize(new_size, 0);
    }

    pub fn try_resize(&mut self, new_size: usize) -> Result<(), std::collections::TryReserveError> {
        if new_size > self.data.len() {
            self.data.try_reserve_exact(new_size - self.data.len())?;
        }
        self.data.resize(new_size, 0);
        Ok(())
    }

    pub fn write_data_to_stream<W: Write>(&self, stream: &mut W) -> io::Result<()> {
        stream.write_all(&self.data)
    }

    pub fn read(&self, byte_pos: usize, out_data: &mut [u8]) {
        let len = out_data.len();
        out_data.copy_from_slice(&self.data[byte_pos..byte_pos + len]);
    }

    pub fn try_read(&self, byte_pos: usize, out_data: &mut [u8]) -> bool {
        let Some(end) = byte_pos.checked_add(out_data.len()) else {
            return false;
        };
        let Some(src) = self.data.get(byte_pos..end) else {
            return false;
        };
        out_data.copy_from_slice(src);
        true
    }

    pub fn write(&mut self, byte_pos: usize, in_data: &[u8]) {
        let len = in_data.len();
        self.data[byte_pos..byte_pos + len].copy_from_slice(in_data);
    }

    pub fn try_write(&mut self, byte_pos: usize, in_data: &[u8]) -> bool {
        let Some(end) = byte_pos.checked_add(in_data.len()) else {
            return false;
        };
        let Some(dst) = self.data.get_mut(byte_pos..end) else {
            return false;
        };
        dst.copy_from_slice(in_data);
        true
    }

    /// Write a slice of `f32` values at `byte_pos` as little-endian bytes,
    /// resizing the buffer if the range runs past its current length.
    ///
    /// The slice is written in one pass with no intermediate allocation,
    /// which is what the wasm writer bridges use for position, normal and
    /// texcoord attributes.
    pub fn write_f32s_le(&mut self, byte_pos: usize, values: &[f32]) {
        let len = values.len() * 4;
        let end = byte_pos + len;
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        let dst = &mut self.data[byte_pos..end];
        for (index, value) in values.iter().enumerate() {
            dst[index * 4..(index + 1) * 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    pub fn copy(
        &mut self,
        dst_offset: usize,
        src_buf: &DataBuffer,
        src_offset: usize,
        size: usize,
    ) {
        let src_slice = &src_buf.data[src_offset..src_offset + size];
        if dst_offset + size > self.data.len() {
            self.data.resize(dst_offset + size, 0);
        }
        self.data[dst_offset..dst_offset + size].copy_from_slice(src_slice);
    }

    pub fn set_update_count(&mut self, count: i64) {
        self.descriptor.buffer_update_count = count;
    }

    pub fn update_count(&self) -> i64 {
        self.descriptor.buffer_update_count
    }

    pub fn data_size(&self) -> usize {
        self.data.len()
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn buffer_id(&self) -> i64 {
        self.descriptor.buffer_id
    }

    pub fn set_buffer_id(&mut self, buffer_id: i64) {
        self.descriptor.buffer_id = buffer_id;
    }
}

#[cfg(test)]
mod tests {
    use super::DataBuffer;

    #[test]
    fn try_read_write_reject_out_of_bounds_ranges() {
        let mut buffer = DataBuffer::new();
        buffer.resize(4);

        assert!(buffer.try_write(1, &[1, 2, 3]));
        assert!(!buffer.try_write(2, &[1, 2, 3]));

        let mut bytes = [0u8; 2];
        assert!(buffer.try_read(1, &mut bytes));
        assert_eq!(bytes, [1, 2]);
        assert!(!buffer.try_read(3, &mut bytes));
    }

    #[test]
    fn try_resize_rejects_impossible_size() {
        let mut buffer = DataBuffer::new();

        assert!(buffer.try_resize(usize::MAX).is_err());
        assert_eq!(buffer.data_size(), 0);
    }

    #[test]
    fn write_f32s_le_writes_little_endian() {
        let mut buffer = DataBuffer::new();
        buffer.resize(12);
        buffer.write_f32s_le(0, &[1.0, -2.5, 0.0]);
        assert_eq!(
            buffer.data(),
            &[0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x20, 0xc0, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn write_f32s_le_resizes_past_the_end() {
        let mut buffer = DataBuffer::new();
        buffer.write_f32s_le(4, &[1.0]);
        assert_eq!(buffer.data_size(), 8);
        assert_eq!(&buffer.data()[..4], &[0, 0, 0, 0]);
    }
}
