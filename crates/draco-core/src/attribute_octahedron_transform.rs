use crate::attribute_transform::{AttributeTransform, AttributeTransformType};
use crate::attribute_transform_data::AttributeTransformData;
#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
use crate::geometry_attribute::PointAttribute;
use crate::geometry_indices::PointIndex;
use crate::normal_compression_utils::OctahedronToolBox;
use crate::status::{DracoError, Status};

pub struct AttributeOctahedronTransform {
    quantization_bits: i32,
}

impl AttributeOctahedronTransform {
    pub fn new(quantization_bits: i32) -> Self {
        Self { quantization_bits }
    }

    pub fn is_valid_quantization_bits(quantization_bits: i32) -> bool {
        (2..=30).contains(&quantization_bits)
    }

    pub fn set_parameters(&mut self, quantization_bits: i32) -> bool {
        if !Self::is_valid_quantization_bits(quantization_bits) {
            return false;
        }
        self.quantization_bits = quantization_bits;
        true
    }

    pub fn is_initialized(&self) -> bool {
        self.quantization_bits != -1
    }

    pub fn quantization_bits(&self) -> i32 {
        self.quantization_bits
    }

    pub fn generate_portable_attribute(
        &self,
        attribute: &PointAttribute,
        point_ids: &[PointIndex],
        num_points: usize,
        target_attribute: &mut PointAttribute,
    ) -> Status {
        if !self.is_initialized() {
            return Err(DracoError::InvalidParameter("Not initialized".to_string()));
        }

        let mut converter = OctahedronToolBox::new();
        if !converter.set_quantization_bits(self.quantization_bits) {
            return Err(DracoError::InvalidParameter(
                "Invalid quantization bits".to_string(),
            ));
        }

        let portable_data_size = num_points
            .checked_mul(2)
            .and_then(|v| v.checked_mul(4))
            .ok_or_else(|| {
                DracoError::DracoError("Portable octahedron buffer size overflow".to_string())
            })?;
        let mut portable_data = Vec::new();
        portable_data
            .try_reserve_exact(portable_data_size)
            .map_err(|_| {
                DracoError::DracoError("Failed to allocate portable octahedron buffer".to_string())
            })?;
        let byte_stride = usize::try_from(attribute.byte_stride())
            .map_err(|_| DracoError::DracoError("Negative attribute byte stride".to_string()))?;
        let source_data = attribute.buffer().data();
        let read_normal = |att_val_id: usize| -> Result<[f32; 3], DracoError> {
            let offset = att_val_id.checked_mul(byte_stride).ok_or_else(|| {
                DracoError::DracoError("Attribute byte offset overflow".to_string())
            })?;
            let end = offset.checked_add(12).ok_or_else(|| {
                DracoError::DracoError("Attribute byte range overflow".to_string())
            })?;
            let bytes = source_data.get(offset..end).ok_or_else(|| {
                DracoError::DracoError("Attribute normal source data is truncated".to_string())
            })?;
            Ok([
                bytemuck::pod_read_unaligned::<f32>(&bytes[0..4]),
                bytemuck::pod_read_unaligned::<f32>(&bytes[4..8]),
                bytemuck::pod_read_unaligned::<f32>(&bytes[8..12]),
            ])
        };

        if !point_ids.is_empty() {
            for &point_id in point_ids {
                let att_val_id = attribute.mapped_index(point_id);
                let att_val = read_normal(att_val_id.0 as usize)?;

                let (s, t) = converter.float_vector_to_quantized_octahedral_coords(&att_val);
                portable_data.extend_from_slice(&s.to_le_bytes());
                portable_data.extend_from_slice(&t.to_le_bytes());
            }
        } else {
            for i in 0..num_points {
                let att_val_id = attribute.mapped_index(PointIndex(i as u32));
                let att_val = read_normal(att_val_id.0 as usize)?;

                let (s, t) = converter.float_vector_to_quantized_octahedral_coords(&att_val);
                portable_data.extend_from_slice(&s.to_le_bytes());
                portable_data.extend_from_slice(&t.to_le_bytes());
            }
        }

        target_attribute
            .buffer_mut()
            .try_resize(portable_data.len())
            .map_err(|_| {
                DracoError::DracoError("Failed to allocate portable octahedron output".to_string())
            })?;
        target_attribute.buffer_mut().write(0, &portable_data);

        Ok(())
    }
}

impl AttributeTransform for AttributeOctahedronTransform {
    fn transform_type(&self) -> AttributeTransformType {
        AttributeTransformType::OctahedronTransform
    }

    fn init_from_attribute(&mut self, attribute: &PointAttribute) -> bool {
        if let Some(transform_data) = attribute.attribute_transform_data() {
            if transform_data.transform_type() != AttributeTransformType::OctahedronTransform {
                return false;
            }
            if let Some(bits) = transform_data.get_parameter_value(0) {
                return self.set_parameters(bits);
            }
        }
        false
    }

    fn copy_to_attribute_transform_data(&self, out_data: &mut AttributeTransformData) {
        out_data.set_transform_type(AttributeTransformType::OctahedronTransform);
        out_data.append_parameter_value(self.quantization_bits);
    }

    fn transform_attribute(
        &self,
        attribute: &PointAttribute,
        point_ids: &[PointIndex],
        target_attribute: &mut PointAttribute,
    ) -> bool {
        self.generate_portable_attribute(
            attribute,
            point_ids,
            target_attribute.size(),
            target_attribute,
        )
        .is_ok()
    }

    fn inverse_transform_attribute(
        &self,
        attribute: &PointAttribute,
        target_attribute: &mut PointAttribute,
    ) -> bool {
        if target_attribute.data_type() != DataType::Float32 {
            return false;
        }
        if target_attribute.num_components() != 3 {
            return false;
        }

        let num_points = target_attribute.size();
        let mut converter = OctahedronToolBox::new();
        if !converter.set_quantization_bits(self.quantization_bits) {
            return false;
        }

        let source_buffer = attribute.buffer();
        let target_buffer = target_attribute.buffer_mut();

        // Ensure target buffer has enough space
        let Some(target_byte_size) = num_points.checked_mul(3).and_then(|v| v.checked_mul(4))
        else {
            return false;
        };
        if target_buffer.try_resize(target_byte_size).is_err() {
            return false;
        }

        let source_data = source_buffer.data();
        // Source data is int32 (s, t) pairs.
        let Some(source_byte_size) = num_points.checked_mul(2).and_then(|v| v.checked_mul(4))
        else {
            return false;
        };
        if source_data.len() < source_byte_size {
            return false;
        }

        for i in 0..num_points {
            let offset = i * 8; // 2 int32s.
            let s_bytes = &source_data[offset..offset + 4];
            let t_bytes = &source_data[offset + 4..offset + 8];
            let mut s_array = [0u8; 4];
            let mut t_array = [0u8; 4];
            s_array.copy_from_slice(s_bytes);
            t_array.copy_from_slice(t_bytes);
            let s = i32::from_le_bytes(s_array);
            let t = i32::from_le_bytes(t_array);

            let att_val = converter.quantized_octahedral_coords_to_unit_vector(s, t);

            let target_offset = i * 12;
            // Write floats using bytemuck
            let bytes = &mut target_buffer.data_mut()[target_offset..target_offset + 12];
            bytes[0..4].copy_from_slice(bytemuck::bytes_of(&att_val[0]));
            bytes[4..8].copy_from_slice(bytemuck::bytes_of(&att_val[1]));
            bytes[8..12].copy_from_slice(bytemuck::bytes_of(&att_val[2]));
        }

        true
    }

    #[cfg(feature = "encoder")]
    fn encode_parameters(&self, encoder_buffer: &mut EncoderBuffer) -> bool {
        if self.is_initialized() {
            encoder_buffer.encode(self.quantization_bits as u8);
            true
        } else {
            false
        }
    }

    #[cfg(feature = "decoder")]
    fn decode_parameters(
        &mut self,
        _attribute: &PointAttribute,
        decoder_buffer: &mut DecoderBuffer,
    ) -> bool {
        if let Ok(quantization_bits) = decoder_buffer.decode::<u8>() {
            self.set_parameters(quantization_bits as i32)
        } else {
            false
        }
    }

    fn get_transformed_data_type(&self, _attribute: &PointAttribute) -> DataType {
        DataType::Uint32
    }

    fn get_transformed_num_components(&self, _attribute: &PointAttribute) -> i32 {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::AttributeOctahedronTransform;
    use crate::attribute_transform::AttributeTransform;
    use crate::attribute_transform::AttributeTransformType;
    use crate::attribute_transform_data::AttributeTransformData;
    use crate::decoder_buffer::DecoderBuffer;
    use crate::draco_types::DataType;
    use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};

    #[test]
    fn inverse_transform_rejects_truncated_portable_data() {
        let transform = AttributeOctahedronTransform::new(10);
        let mut portable = PointAttribute::new();
        portable.init(GeometryAttributeType::Normal, 2, DataType::Uint32, false, 1);
        portable.buffer_mut().resize(4);

        let mut target = PointAttribute::new();
        target.init(
            GeometryAttributeType::Normal,
            3,
            DataType::Float32,
            false,
            1,
        );

        assert!(!transform.inverse_transform_attribute(&portable, &mut target));
    }

    #[test]
    fn generate_portable_attribute_rejects_truncated_source_data() {
        let transform = AttributeOctahedronTransform::new(10);
        let mut source = PointAttribute::new();
        source.init(
            GeometryAttributeType::Normal,
            3,
            DataType::Float32,
            false,
            1,
        );
        source.buffer_mut().resize(8);

        let mut target = PointAttribute::new();
        target.init(GeometryAttributeType::Normal, 2, DataType::Uint32, false, 1);

        assert!(transform
            .generate_portable_attribute(&source, &[], 1, &mut target)
            .is_err());
    }

    #[test]
    fn decode_parameters_rejects_invalid_quantization_bits() {
        let attribute = PointAttribute::new();
        let mut transform = AttributeOctahedronTransform::new(-1);

        let mut zero_bits = DecoderBuffer::new(&[0]);
        assert!(!transform.decode_parameters(&attribute, &mut zero_bits));

        let mut too_many_bits = DecoderBuffer::new(&[31]);
        assert!(!transform.decode_parameters(&attribute, &mut too_many_bits));
    }

    #[test]
    fn decode_parameters_accepts_valid_quantization_bits() {
        let attribute = PointAttribute::new();
        let mut transform = AttributeOctahedronTransform::new(-1);
        let mut buffer = DecoderBuffer::new(&[10]);

        assert!(transform.decode_parameters(&attribute, &mut buffer));
        assert_eq!(transform.quantization_bits(), 10);
    }

    #[test]
    fn init_from_attribute_rejects_invalid_quantization_bits() {
        let mut transform_data = AttributeTransformData::new();
        transform_data.set_transform_type(AttributeTransformType::OctahedronTransform);
        transform_data.append_parameter_value(31i32);

        let mut attribute = PointAttribute::new();
        attribute.set_attribute_transform_data(transform_data);

        let mut transform = AttributeOctahedronTransform::new(-1);
        assert!(!transform.init_from_attribute(&attribute));
    }
}
