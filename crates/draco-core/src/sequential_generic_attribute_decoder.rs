//! Generic (untyped) sequential attribute decoder.
//!
//! [`SequentialGenericAttributeDecoder`] reads attribute values verbatim,
//! byte-for-byte, for attributes that have no specialized prediction or
//! transform. Port of Draco's `sequential_generic_attribute_decoder` path.

use crate::decoder_buffer::DecoderBuffer;
use crate::point_cloud::PointCloud;
use crate::point_cloud_decoder::PointCloudDecoder;
use crate::prediction_scheme::EntryToPointIdMap;
use crate::sequential_attribute_decoder::SequentialAttributeDecoder;
use crate::status::{DracoError, Status};

pub struct SequentialGenericAttributeDecoder {
    base: SequentialAttributeDecoder,
}

impl Default for SequentialGenericAttributeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialGenericAttributeDecoder {
    pub fn new() -> Self {
        Self {
            base: SequentialAttributeDecoder::new(),
        }
    }

    pub fn init(&mut self, decoder: &PointCloudDecoder, attribute_id: i32) -> bool {
        self.base.init(decoder, attribute_id)
    }

    pub fn decode_values(
        &mut self,
        point_cloud: &mut PointCloud,
        point_ids: EntryToPointIdMap<'_>,
        buffer: &mut DecoderBuffer,
    ) -> Status {
        let attribute_id = self.base.attribute_id();
        let attribute = point_cloud.try_attribute_mut(attribute_id)?;

        let num_components = attribute.num_components() as usize;
        let num_points = point_ids.len();
        let data_type_size = attribute.data_type().byte_length();

        let total_size = num_points
            .checked_mul(num_components)
            .and_then(|size| size.checked_mul(data_type_size))
            .ok_or_else(|| DracoError::general("Generic attribute size overflow".to_string()))?;
        attribute
            .buffer_mut()
            .try_resize(total_size)
            .map_err(|_| DracoError::general("Failed to allocate generic attribute".to_string()))?;

        let bytes = buffer
            .decode_slice(total_size)
            .map_err(|_| DracoError::general("Failed to decode generic attribute".to_string()))?;
        attribute.buffer_mut().data_mut().copy_from_slice(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point_cloud_decoder::PointCloudDecoder;

    #[test]
    fn decode_values_rejects_invalid_attribute_id() {
        let mut decoder = SequentialGenericAttributeDecoder::new();
        let point_cloud_decoder = PointCloudDecoder::new();
        assert!(decoder.init(&point_cloud_decoder, 0));

        let mut point_cloud = PointCloud::new();
        let mut buffer = DecoderBuffer::new(&[]);

        assert!(decoder
            .decode_values(
                &mut point_cloud,
                EntryToPointIdMap::identity(0),
                &mut buffer
            )
            .is_err());
    }
}
