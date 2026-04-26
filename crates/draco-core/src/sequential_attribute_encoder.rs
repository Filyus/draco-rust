use crate::encoder_buffer::EncoderBuffer;
use crate::geometry_attribute::PointAttribute;
use crate::geometry_indices::PointIndex;
use crate::point_cloud::PointCloud;

pub struct SequentialAttributeEncoder {
    attribute_id: i32,
}

impl Default for SequentialAttributeEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialAttributeEncoder {
    pub fn new() -> Self {
        Self { attribute_id: -1 }
    }

    pub fn attribute_id(&self) -> i32 {
        self.attribute_id
    }

    pub fn init(&mut self, attribute_id: i32) -> bool {
        self.attribute_id = attribute_id;
        true
    }

    pub fn initialize_standalone(&mut self, _attribute: &PointAttribute) -> bool {
        true
    }

    pub fn transform_attribute_to_portable_format(&mut self, _point_ids: &[PointIndex]) -> bool {
        true
    }

    pub fn encode_values(
        &mut self,
        point_cloud: &PointCloud,
        point_ids: &[PointIndex],
        out_buffer: &mut EncoderBuffer,
    ) -> bool {
        let att = point_cloud.attribute(self.attribute_id);
        let entry_size = att.byte_stride() as usize;
        let buffer_data = att.buffer().data();

        for &p_id in point_ids {
            let mapped_index = att.mapped_index(p_id).0 as usize;
            let offset = mapped_index * entry_size;
            if offset + entry_size > buffer_data.len() {
                return false;
            }
            let bytes = &buffer_data[offset..offset + entry_size];
            out_buffer.encode_data(bytes);
        }
        true
    }
}
