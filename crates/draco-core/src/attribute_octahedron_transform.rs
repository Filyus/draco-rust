//! Octahedral normal attribute transform.
//!
//! [`AttributeOctahedronTransform`] projects unit normal vectors onto an
//! octahedron and quantizes the 2D result, trading three float components for a
//! compact integer pair; decode inverts the projection. Port of Draco's
//! `attribute_octahedron_transform.h`.

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
use crate::prediction_scheme::EntryToPointIdMap;
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

    pub fn set_parameters(&mut self, quantization_bits: i32) -> Status {
        if !Self::is_valid_quantization_bits(quantization_bits) {
            return Err(Self::invalid_quantization_bits(quantization_bits));
        }
        self.quantization_bits = quantization_bits;
        Ok(())
    }

    fn invalid_quantization_bits(bits: i32) -> DracoError {
        DracoError::invalid_parameter(format!(
            "Octahedral quantization bits {bits} outside the supported range 2..=30"
        ))
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
        point_ids: EntryToPointIdMap<'_>,
        num_points: usize,
        target_attribute: &mut PointAttribute,
    ) -> Status {
        if !self.is_initialized() {
            return Err(DracoError::invalid_parameter("Not initialized".to_string()));
        }

        let mut converter = OctahedronToolBox::new();
        if !converter.set_quantization_bits(self.quantization_bits) {
            return Err(Self::invalid_quantization_bits(self.quantization_bits));
        }

        let portable_data_size = num_points
            .checked_mul(2)
            .and_then(|v| v.checked_mul(4))
            .ok_or_else(|| {
                DracoError::general("Portable octahedron buffer size overflow".to_string())
            })?;
        let mut portable_data = Vec::new();
        portable_data
            .try_reserve_exact(portable_data_size)
            .map_err(|_| {
                DracoError::general("Failed to allocate portable octahedron buffer".to_string())
            })?;
        let byte_stride = usize::try_from(attribute.byte_stride())
            .map_err(|_| DracoError::general("Negative attribute byte stride".to_string()))?;
        let source_data = attribute.buffer().data();
        let read_normal = |att_val_id: usize| -> Result<[f32; 3], DracoError> {
            let offset = att_val_id
                .checked_mul(byte_stride)
                .ok_or_else(|| DracoError::general("Attribute byte offset overflow".to_string()))?;
            let end = offset
                .checked_add(12)
                .ok_or_else(|| DracoError::general("Attribute byte range overflow".to_string()))?;
            let bytes = source_data.get(offset..end).ok_or_else(|| {
                DracoError::general("Attribute normal source data is truncated".to_string())
            })?;
            Ok([
                bytemuck::pod_read_unaligned::<f32>(&bytes[0..4]),
                bytemuck::pod_read_unaligned::<f32>(&bytes[4..8]),
                bytemuck::pod_read_unaligned::<f32>(&bytes[8..12]),
            ])
        };

        if !point_ids.is_empty() {
            for entry in 0..point_ids.len() {
                let point_id = PointIndex(point_ids.get(entry).unwrap_or(u32::MAX));
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
                DracoError::general("Failed to allocate portable octahedron output".to_string())
            })?;
        target_attribute.buffer_mut().write(0, &portable_data);

        Ok(())
    }

    pub fn inverse_transform_attribute_with_legacy_octahedron(
        &self,
        attribute: &PointAttribute,
        target_attribute: &mut PointAttribute,
        legacy_octahedron_to_vector: bool,
    ) -> Status {
        self.inverse_transform_attribute_impl(
            attribute,
            target_attribute,
            legacy_octahedron_to_vector,
        )
    }

    fn inverse_transform_attribute_impl(
        &self,
        attribute: &PointAttribute,
        target_attribute: &mut PointAttribute,
        legacy_octahedron_to_vector: bool,
    ) -> Status {
        if target_attribute.data_type() != DataType::Float32 {
            return Err(DracoError::invalid_parameter(format!(
                "Octahedral decode needs a float32 target, got {:?}",
                target_attribute.data_type()
            )));
        }
        if target_attribute.num_components() != 3 {
            return Err(DracoError::invalid_parameter(format!(
                "Octahedral decode needs 3 target components, got {}",
                target_attribute.num_components()
            )));
        }

        let num_points = target_attribute.size();
        let mut converter = OctahedronToolBox::new();
        if !converter.set_quantization_bits(self.quantization_bits) {
            return Err(Self::invalid_quantization_bits(self.quantization_bits));
        }

        let source_buffer = attribute.buffer();
        let target_buffer = target_attribute.buffer_mut();

        // Ensure target buffer has enough space
        let Some(target_byte_size) = num_points.checked_mul(3).and_then(|v| v.checked_mul(4))
        else {
            return Err(DracoError::general(
                "Octahedral target buffer size overflow".to_string(),
            ));
        };
        if target_buffer.try_resize(target_byte_size).is_err() {
            return Err(DracoError::general(
                "Failed to allocate the octahedral target buffer".to_string(),
            ));
        }

        let source_data = source_buffer.data();
        // Source data is int32 (s, t) pairs.
        let Some(source_byte_size) = num_points.checked_mul(2).and_then(|v| v.checked_mul(4))
        else {
            return Err(DracoError::general(
                "Octahedral source buffer size overflow".to_string(),
            ));
        };
        if source_data.len() < source_byte_size {
            return Err(DracoError::general(
                "Octahedral portable data is truncated".to_string(),
            ));
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

            let att_val = if legacy_octahedron_to_vector {
                converter.quantized_octahedral_coords_to_unit_vector_legacy(s, t)
            } else {
                converter.quantized_octahedral_coords_to_unit_vector(s, t)
            };

            let target_offset = i * 12;
            // Write floats using bytemuck
            let bytes = &mut target_buffer.data_mut()[target_offset..target_offset + 12];
            bytes[0..4].copy_from_slice(bytemuck::bytes_of(&att_val[0]));
            bytes[4..8].copy_from_slice(bytemuck::bytes_of(&att_val[1]));
            bytes[8..12].copy_from_slice(bytemuck::bytes_of(&att_val[2]));
        }

        Ok(())
    }
}

impl AttributeTransform for AttributeOctahedronTransform {
    fn transform_type(&self) -> AttributeTransformType {
        AttributeTransformType::OctahedronTransform
    }

    fn init_from_attribute(&mut self, attribute: &PointAttribute) -> Status {
        let Some(transform_data) = attribute.attribute_transform_data() else {
            return Err(DracoError::invalid_parameter(
                "Attribute carries no transform data".to_string(),
            ));
        };
        if transform_data.transform_type() != AttributeTransformType::OctahedronTransform {
            return Err(DracoError::invalid_parameter(format!(
                "Attribute carries {:?}, not an octahedron transform",
                transform_data.transform_type()
            )));
        }
        let Some(bits) = transform_data.get_parameter_value(0) else {
            return Err(DracoError::invalid_parameter(
                "Attribute transform data is shorter than the octahedral bit count".to_string(),
            ));
        };
        self.set_parameters(bits)
    }

    fn copy_to_attribute_transform_data(&self, out_data: &mut AttributeTransformData) {
        out_data.set_transform_type(AttributeTransformType::OctahedronTransform);
        out_data.append_parameter_value(self.quantization_bits);
    }

    fn transform_attribute(
        &self,
        attribute: &PointAttribute,
        point_ids: EntryToPointIdMap<'_>,
        target_attribute: &mut PointAttribute,
    ) -> Status {
        self.generate_portable_attribute(
            attribute,
            point_ids,
            target_attribute.size(),
            target_attribute,
        )
    }

    fn inverse_transform_attribute(
        &self,
        attribute: &PointAttribute,
        target_attribute: &mut PointAttribute,
    ) -> Status {
        self.inverse_transform_attribute_impl(attribute, target_attribute, false)
    }

    #[cfg(feature = "encoder")]
    fn encode_parameters(&self, encoder_buffer: &mut EncoderBuffer) -> Status {
        if !self.is_initialized() {
            return Err(DracoError::invalid_parameter(
                "Octahedron transform parameters were never set".to_string(),
            ));
        }
        encoder_buffer.encode(self.quantization_bits as u8);
        Ok(())
    }

    #[cfg(feature = "decoder")]
    fn decode_parameters(
        &mut self,
        _attribute: &PointAttribute,
        decoder_buffer: &mut DecoderBuffer,
    ) -> Status {
        let Ok(quantization_bits) = decoder_buffer.decode::<u8>() else {
            return Err(DracoError::buffer(
                "Stream ends before the octahedral bit count it declares".to_string(),
            ));
        };
        self.set_parameters(quantization_bits as i32)
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
    use crate::prediction_scheme::EntryToPointIdMap;

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

        assert!(transform
            .inverse_transform_attribute(&portable, &mut target)
            .is_err());
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
            .generate_portable_attribute(&source, EntryToPointIdMap::identity(0), 1, &mut target)
            .is_err());
    }

    #[test]
    fn decode_parameters_rejects_invalid_quantization_bits() {
        let attribute = PointAttribute::new();
        let mut transform = AttributeOctahedronTransform::new(-1);

        let mut zero_bits = DecoderBuffer::new(&[0]);
        assert!(transform
            .decode_parameters(&attribute, &mut zero_bits)
            .is_err());

        let mut too_many_bits = DecoderBuffer::new(&[31]);
        assert!(transform
            .decode_parameters(&attribute, &mut too_many_bits)
            .is_err());
    }

    #[test]
    fn decode_parameters_accepts_valid_quantization_bits() {
        let attribute = PointAttribute::new();
        let mut transform = AttributeOctahedronTransform::new(-1);
        let mut buffer = DecoderBuffer::new(&[10]);

        assert!(transform.decode_parameters(&attribute, &mut buffer).is_ok());
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
        assert!(transform.init_from_attribute(&attribute).is_err());
    }
}
