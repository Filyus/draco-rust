use crate::attribute_transform::AttributeTransformType;
use crate::data_buffer::DataBuffer;

#[derive(Debug, Clone)]
pub struct AttributeTransformData {
    transform_type: AttributeTransformType,
    buffer: DataBuffer,
}

impl Default for AttributeTransformData {
    fn default() -> Self {
        Self {
            transform_type: AttributeTransformType::InvalidTransform,
            buffer: DataBuffer::new(),
        }
    }
}

impl AttributeTransformData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn transform_type(&self) -> AttributeTransformType {
        self.transform_type
    }

    pub fn set_transform_type(&mut self, transform_type: AttributeTransformType) {
        self.transform_type = transform_type;
    }

    pub fn get_parameter_value<T: Copy + bytemuck::Pod>(&self, byte_offset: usize) -> Option<T> {
        let size = std::mem::size_of::<T>();
        if byte_offset.checked_add(size)? > self.buffer.data_size() {
            return None;
        }

        let mut val: T = bytemuck::Zeroable::zeroed();
        let slice = bytemuck::bytes_of_mut(&mut val);
        if !self.buffer.try_read(byte_offset, slice) {
            return None;
        }
        Some(val)
    }

    pub fn set_parameter_value<T: Copy + bytemuck::Pod>(&mut self, byte_offset: usize, in_data: T) {
        let size = std::mem::size_of::<T>();
        let Some(end) = byte_offset.checked_add(size) else {
            return;
        };
        if end > self.buffer.data_size() && self.buffer.try_resize(end).is_err() {
            return;
        }
        let slice = bytemuck::bytes_of(&in_data);
        let _ = self.buffer.try_write(byte_offset, slice);
    }

    pub fn append_parameter_value<T: Copy + bytemuck::Pod>(&mut self, in_data: T) {
        self.set_parameter_value(self.buffer.data_size(), in_data);
    }
}

#[cfg(test)]
mod tests {
    use super::AttributeTransformData;

    #[test]
    fn get_parameter_value_rejects_overflowing_offset() {
        let data = AttributeTransformData::new();

        assert_eq!(data.get_parameter_value::<u32>(usize::MAX), None);
    }

    #[test]
    fn set_parameter_value_ignores_overflowing_offset() {
        let mut data = AttributeTransformData::new();
        data.set_parameter_value::<u32>(usize::MAX, 7);

        assert_eq!(data.get_parameter_value::<u32>(0), None);
    }
}
