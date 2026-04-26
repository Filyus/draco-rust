use crate::attribute_octahedron_transform::AttributeOctahedronTransform;
use crate::attribute_transform::AttributeTransform;
use crate::corner_table::CornerTable;
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
use crate::geometry_indices::PointIndex;
use crate::point_cloud::PointCloud;
use crate::point_cloud_decoder::PointCloudDecoder;
use crate::prediction_scheme_normal_octahedron_canonicalized_decoding_transform::PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform;
use crate::sequential_integer_attribute_decoder::SequentialIntegerAttributeDecoder;
use crate::status::{DracoError, Status};

use crate::prediction_scheme_delta::PredictionSchemeDeltaDecoder;

fn validate_normal_quantization_bits(quantization_bits: u8) -> Status {
    if !AttributeOctahedronTransform::is_valid_quantization_bits(quantization_bits as i32) {
        return Err(DracoError::DracoError(
            "Invalid normal quantization bits".to_string(),
        ));
    }
    Ok(())
}

pub struct SequentialNormalAttributeDecoder {
    base: SequentialIntegerAttributeDecoder,
    attribute_octahedron_transform: AttributeOctahedronTransform,
}

impl Default for SequentialNormalAttributeDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SequentialNormalAttributeDecoder {
    pub fn new() -> Self {
        Self {
            base: SequentialIntegerAttributeDecoder::new(),
            attribute_octahedron_transform: AttributeOctahedronTransform::new(-1),
        }
    }

    pub fn init(
        &mut self,
        decoder: &PointCloudDecoder,
        point_cloud: &PointCloud,
        attribute_id: i32,
    ) -> Status {
        if !self.base.init(decoder, attribute_id) {
            return Err(DracoError::DracoError("Failed to init base".to_string()));
        }

        let attribute = point_cloud.try_attribute(attribute_id)?;
        if attribute.num_components() != 3 {
            return Err(DracoError::InvalidParameter(
                "Attribute must have 3 components".to_string(),
            ));
        }

        Ok(())
    }

    pub fn decode_data_needed_by_portable_transform(
        &mut self,
        _point_cloud: &mut PointCloud,
        buffer: &mut DecoderBuffer,
    ) -> Status {
        let quantization_bits: u8;
        if let Ok(val) = buffer.decode::<u8>() {
            quantization_bits = val;
        } else {
            return Err(DracoError::BitstreamVersionUnsupported);
        }
        validate_normal_quantization_bits(quantization_bits)?;
        self.attribute_octahedron_transform
            .set_parameters(quantization_bits as i32);
        Ok(())
    }

    pub fn decode_values(
        &mut self,
        point_cloud: &mut PointCloud,
        point_ids: &[PointIndex],
        buffer: &mut DecoderBuffer,
        corner_table: Option<&CornerTable>,
        data_to_corner_map: Option<&[u32]>,
    ) -> Status {
        // Decode quantization bits if not initialized
        if !self.attribute_octahedron_transform.is_initialized() {
            let quantization_bits: u8 = match buffer.decode_u8() {
                Ok(v) => v,
                Err(_) => {
                    return Err(DracoError::DracoError(
                        "Failed to decode quantization bits".to_string(),
                    ))
                }
            };
            validate_normal_quantization_bits(quantization_bits)?;
            self.attribute_octahedron_transform
                .set_parameters(quantization_bits as i32);
        }

        // Create portable attribute
        let mut portable_attribute = crate::geometry_attribute::PointAttribute::new();
        portable_attribute.init(
            crate::geometry_attribute::GeometryAttributeType::Generic,
            2,
            DataType::Uint32,
            false,
            point_ids.len(),
        );

        // 1. Create prediction scheme
        let transform = PredictionSchemeNormalOctahedronCanonicalizedDecodingTransform::new();
        let prediction_scheme = Box::new(PredictionSchemeDeltaDecoder::new(transform));

        self.base.set_prediction_scheme(prediction_scheme);

        if !self.base.decode_values(
            point_cloud,
            point_ids,
            buffer,
            corner_table,
            data_to_corner_map,
            None,
            Some(&mut portable_attribute),
            None,
            None,
        ) {
            return Err(DracoError::DracoError(
                "Failed to decode values".to_string(),
            ));
        }

        // 2. Convert portable attribute to original attribute

        // Transform back to original attribute
        let attribute_id = self.base.attribute_id();
        let attribute = point_cloud.try_attribute_mut(attribute_id)?;

        if !self
            .attribute_octahedron_transform
            .inverse_transform_attribute(&portable_attribute, attribute)
        {
            return Err(DracoError::DracoError(
                "Failed to inverse transform attribute".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::point_cloud::PointCloud;

    #[test]
    fn decode_portable_transform_rejects_invalid_normal_quantization_bits() {
        let mut decoder = SequentialNormalAttributeDecoder::new();
        let mut point_cloud = PointCloud::new();

        let mut zero_bits = DecoderBuffer::new(&[0]);
        assert!(decoder
            .decode_data_needed_by_portable_transform(&mut point_cloud, &mut zero_bits)
            .is_err());

        let mut too_many_bits = DecoderBuffer::new(&[31]);
        assert!(decoder
            .decode_data_needed_by_portable_transform(&mut point_cloud, &mut too_many_bits)
            .is_err());
    }

    #[test]
    fn decode_portable_transform_accepts_valid_normal_quantization_bits() {
        let mut decoder = SequentialNormalAttributeDecoder::new();
        let mut point_cloud = PointCloud::new();
        let mut buffer = DecoderBuffer::new(&[10]);

        assert!(decoder
            .decode_data_needed_by_portable_transform(&mut point_cloud, &mut buffer)
            .is_ok());
    }

    #[test]
    fn init_rejects_invalid_attribute_id() {
        let mut decoder = SequentialNormalAttributeDecoder::new();
        let point_cloud_decoder = PointCloudDecoder::new();
        let point_cloud = PointCloud::new();

        assert!(decoder.init(&point_cloud_decoder, &point_cloud, 0).is_err());
    }
}
