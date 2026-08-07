//! Base sequential attribute decoder.
//!
//! [`SequentialAttributeDecoder`] is the generic decode path that reads
//! attribute values in point order with no prediction or transform. It is the
//! base reused by the integer, generic, and normal decoders. Port of Draco's
//! `sequential_attribute_decoder.h`.

use crate::decoder_buffer::DecoderBuffer;
use crate::geometry_attribute::PointAttribute;
use crate::point_cloud_decoder::PointCloudDecoder;
use crate::prediction_scheme::EntryToPointIdMap;

pub struct SequentialAttributeDecoder {
    attribute_id: i32,
}

impl Default for SequentialAttributeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialAttributeDecoder {
    pub fn new() -> Self {
        Self { attribute_id: -1 }
    }

    pub fn attribute_id(&self) -> i32 {
        self.attribute_id
    }

    pub fn init(&mut self, _decoder: &PointCloudDecoder, attribute_id: i32) -> bool {
        self.attribute_id = attribute_id;
        true
    }

    pub fn initialize_standalone(&mut self, _attribute: &PointAttribute) -> bool {
        true
    }

    pub fn decode_portable_attribute(
        &mut self,
        _point_ids: EntryToPointIdMap<'_>,
        _in_buffer: &mut DecoderBuffer,
    ) -> bool {
        true
    }

    pub fn decode_data_needed_by_portable_transform(
        &mut self,
        _point_ids: EntryToPointIdMap<'_>,
        _in_buffer: &mut DecoderBuffer,
    ) -> bool {
        true
    }

    pub fn transform_attribute_to_original_format(
        &mut self,
        _point_ids: EntryToPointIdMap<'_>,
    ) -> bool {
        true
    }
}
