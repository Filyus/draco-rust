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

    pub fn init(&mut self, decoder: &PointCloudDecoder, attribute_id: i32) {
        self.base.init(decoder, attribute_id);
    }

    pub fn decode_values(
        &mut self,
        point_cloud: &mut PointCloud,
        point_ids: EntryToPointIdMap<'_>,
        buffer: &mut DecoderBuffer,
    ) -> Status {
        let attribute_id = self.base.attribute_id();
        let attribute = point_cloud.try_attribute(attribute_id)?;

        let num_components = attribute.num_components() as usize;
        let num_points = point_ids.len();
        let data_type_size = attribute.data_type().byte_length();

        let total_size = num_points
            .checked_mul(num_components)
            .and_then(|size| size.checked_mul(data_type_size))
            .ok_or_else(|| DracoError::general("Generic attribute size overflow".to_string()))?;

        // The bytes first, the buffer after. Generic values are copied verbatim
        // out of the stream, so `decode_slice` is an exact bound on how many
        // there can be: it borrows from the input, allocates nothing, and
        // refuses a count the remaining bytes cannot cover. Sizing the
        // destination from the slice that came back makes the allocation
        // data-backed by construction, with no budget to clear -- the ratio
        // that stood here allowed a million bytes per input byte on a path
        // where the honest bound is one.
        let bytes = buffer
            .decode_slice(total_size)
            .map_err(|_| DracoError::general("Failed to decode generic attribute".to_string()))?;

        let attribute = point_cloud.try_attribute_mut(attribute_id)?;
        attribute
            .buffer_mut()
            .try_resize(bytes.len())
            .map_err(|_| DracoError::general("Failed to allocate generic attribute".to_string()))?;
        attribute.buffer_mut().data_mut().copy_from_slice(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point_cloud_decoder::PointCloudDecoder;

    use crate::draco_types::DataType;
    use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};

    fn cloud_with_one_generic_value() -> PointCloud {
        let mut point_cloud = PointCloud::new();
        point_cloud.set_num_points(1);
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Generic,
            1,
            DataType::Uint32,
            false,
            1,
        );
        point_cloud.add_attribute(attribute);
        point_cloud
    }

    /// Both cases the point-cloud decoder used to pin on its own copy of this
    /// path, kept here now that the copy is gone.
    #[test]
    fn a_value_count_that_overflows_its_byte_size_is_refused() {
        let mut point_cloud = cloud_with_one_generic_value();
        let mut buffer = DecoderBuffer::new(&[]);
        let mut decoder = SequentialGenericAttributeDecoder::new();
        decoder.init(&PointCloudDecoder::new(), 0);

        assert!(decoder
            .decode_values(
                &mut point_cloud,
                EntryToPointIdMap::identity(usize::MAX),
                &mut buffer
            )
            .is_err());
    }

    #[test]
    fn a_stream_too_short_for_the_values_is_refused() {
        let mut point_cloud = cloud_with_one_generic_value();
        let bytes = [1u8, 2, 3];
        let mut buffer = DecoderBuffer::new(&bytes);
        let mut decoder = SequentialGenericAttributeDecoder::new();
        decoder.init(&PointCloudDecoder::new(), 0);

        assert!(decoder
            .decode_values(
                &mut point_cloud,
                EntryToPointIdMap::identity(1),
                &mut buffer
            )
            .is_err());
    }

    #[test]
    fn decode_values_rejects_invalid_attribute_id() {
        let mut decoder = SequentialGenericAttributeDecoder::new();
        let point_cloud_decoder = PointCloudDecoder::new();
        decoder.init(&point_cloud_decoder, 0);

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
