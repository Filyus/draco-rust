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
}
