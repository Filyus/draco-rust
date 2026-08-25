//! Quantized sequential attribute decoder.
//!
//! [`SequentialQuantizationAttributeDecoder`] reads an attribute that was
//! quantized into integers: the integer decoder fills a portable attribute with
//! the quantized values, and the quantization transform this holds turns them
//! back into floats. Decode-side counterpart of the quantization branch of
//! `SequentialIntegerAttributeEncoder`, and port of Draco's
//! `sequential_quantization_attribute_decoder.h`.
//!
//! Where the parameters sit depends on the version, and that split is the
//! reason this type exists rather than a plain call to the integer decoder:
//! below 2.0 they are written between the prediction header and the values, so
//! they have to be read before the decode and the decode has to be told to step
//! over them; 2.0 and later carry them after the values, where the caller reads
//! them from the transform data.

use crate::attribute_quantization_transform::AttributeQuantizationTransform;
// Brings `decode_parameters` into scope, which only the pre-2.0 layout calls.
#[cfg(feature = "legacy_bitstream_decode")]
use crate::attribute_transform::AttributeTransform;
use crate::corner_table::CornerTable;
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
use crate::geometry_attribute::PointAttribute;
use crate::point_cloud::PointCloud;
use crate::point_cloud_decoder::PointCloudDecoder;
use crate::prediction_scheme::EntryToPointIdMap;
use crate::sequential_integer_attribute_decoder::{
    PortableExtent, SequentialIntegerAttributeDecoder,
};
use crate::status::{DracoError, Status};

pub struct SequentialQuantizationAttributeDecoder {
    base: SequentialIntegerAttributeDecoder,
    transform: AttributeQuantizationTransform,
}

impl Default for SequentialQuantizationAttributeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialQuantizationAttributeDecoder {
    pub fn new() -> Self {
        Self {
            base: SequentialIntegerAttributeDecoder::new(),
            transform: AttributeQuantizationTransform::new(),
        }
    }

    /// The transform, carrying whatever parameters have been read so far. Below
    /// 2.0 that is everything, because `decode_values` read them; at 2.0 and
    /// later it is still empty and the caller fills it from the transform data
    /// that follows the values.
    pub fn transform(&self) -> &AttributeQuantizationTransform {
        &self.transform
    }

    /// Hands the transform over, for the caller to keep alongside the portable
    /// attribute until the inverse pass runs.
    pub fn into_transform(self) -> AttributeQuantizationTransform {
        self.transform
    }

    /// Refuses an attribute quantization cannot describe, as upstream's `Init`
    /// does: only floating-point values are quantized.
    pub fn init(
        &mut self,
        decoder: &PointCloudDecoder,
        point_cloud: &PointCloud,
        attribute_id: i32,
    ) -> Status {
        self.base.init(decoder, attribute_id);

        let attribute = point_cloud.try_attribute(attribute_id)?;
        if attribute.data_type() != DataType::Float32 {
            return Err(DracoError::general(format!(
                "Quantized decoding needs float32 values, attribute {attribute_id} is {:?}",
                attribute.data_type()
            )));
        }
        Ok(())
    }

    /// Reads the quantization parameters out of a pre-2.0 stream and reports
    /// how many bytes the integer decode has to step over to reach the values.
    ///
    /// The buffer is left where it was found, because the integer decoder reads
    /// the prediction header in front of those parameters itself.
    #[cfg(feature = "legacy_bitstream_decode")]
    fn read_inline_parameters(
        &mut self,
        point_cloud: &PointCloud,
        attribute_id: i32,
        buffer: &mut DecoderBuffer,
    ) -> Result<usize, DracoError> {
        let saved_pos = buffer.position();
        let method_byte = buffer
            .decode_u8()
            .map_err(|_| DracoError::general("Failed to read prediction method".to_string()))?;
        let carries_transform = crate::point_cloud_decoder::carries_transform_byte(method_byte);
        if carries_transform {
            buffer
                .decode_u8()
                .map_err(|_| DracoError::general("Failed to read transform type".to_string()))?;
        }
        let original = point_cloud.try_attribute(attribute_id)?;
        self.transform
            .decode_parameters(original, buffer)
            .map_err(|e| {
                DracoError::general(format!(
                    "Failed to decode quantization parameters (v<2.0): {e}"
                ))
            })?;

        let bytes_consumed = buffer.position() - saved_pos;
        let header_bytes = if carries_transform { 2 } else { 1 };
        buffer
            .set_position(saved_pos)
            .map_err(|_| DracoError::general("Failed to reset buffer position".to_string()))?;
        Ok(bytes_consumed - header_bytes)
    }

    /// Decodes the quantized values into a portable attribute and hands it back
    /// for the caller's inverse transform.
    ///
    /// The mesh-only arguments are the corner table and its two maps, plus the
    /// portable position a 2.0+ prediction predicts from; a point cloud passes
    /// `None` for all of them.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_values(
        &mut self,
        point_cloud: &mut PointCloud,
        point_ids: EntryToPointIdMap<'_>,
        buffer: &mut DecoderBuffer,
        bitstream_version: u16,
        extent: PortableExtent,
        corner_table: Option<&CornerTable>,
        data_to_corner_map: Option<&[u32]>,
        vertex_to_data_map: Option<&[i32]>,
        portable_parent_attribute: Option<&PointAttribute>,
    ) -> Result<PointAttribute, DracoError> {
        let attribute_id = self.base.attribute_id();
        // The portable attribute keeps the original's type and width and swaps
        // only the value representation, which is what the inverse transform
        // reads back out of it.
        let (attribute_type, num_components) = {
            let original = point_cloud.try_attribute(attribute_id)?;
            (original.attribute_type(), original.num_components())
        };
        let mut portable = PointAttribute::default();
        extent.init(
            &mut portable,
            attribute_type,
            num_components,
            DataType::Uint32,
            false,
        )?;

        let skip_bytes = if bitstream_version < 0x0200 {
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            {
                return Err(DracoError::bitstream_version_unsupported());
            }
            #[cfg(feature = "legacy_bitstream_decode")]
            {
                self.read_inline_parameters(point_cloud, attribute_id, buffer)?
            }
        } else {
            0
        };

        let mut skip = move |buf: &mut DecoderBuffer<'_>| -> bool {
            skip_bytes == 0 || buf.try_advance(skip_bytes).is_ok()
        };
        let hook: Option<&mut dyn FnMut(&mut DecoderBuffer<'_>) -> bool> = if skip_bytes > 0 {
            Some(&mut skip)
        } else {
            None
        };

        self.base.decode_values(
            point_cloud,
            point_ids,
            buffer,
            corner_table,
            data_to_corner_map,
            vertex_to_data_map,
            Some(&mut portable),
            portable_parent_attribute,
            hook,
        )?;
        Ok(portable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry_attribute::GeometryAttributeType;

    /// Upstream's `Init` refuses this before reading a byte. The port used to
    /// reach the quantization transform with it and fail there instead.
    #[test]
    fn init_refuses_an_attribute_quantization_cannot_describe() {
        let mut point_cloud = PointCloud::new();
        point_cloud.set_num_points(1);
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Uint32,
            false,
            1,
        );
        point_cloud.add_attribute(attribute);

        let error = SequentialQuantizationAttributeDecoder::new()
            .init(&PointCloudDecoder::new(), &point_cloud, 0)
            .expect_err("integers are not quantized");
        assert!(
            error.to_string().contains("float32"),
            "unexpected message: {error}"
        );
    }
}
