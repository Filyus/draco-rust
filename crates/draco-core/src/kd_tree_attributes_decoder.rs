use crate::attribute_quantization_transform::AttributeQuantizationTransform;
use crate::attribute_transform::AttributeTransform;
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
use crate::dynamic_integer_points_kd_tree::DynamicIntegerPointsKdTreeDecoder;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::PointIndex;
use crate::point_cloud::PointCloud;

pub struct KdTreeAttributesDecoder {
    attribute_ids: Vec<i32>,
    quantized_portable_attributes: Vec<PointAttribute>,
    attribute_quantization_transforms: Vec<AttributeQuantizationTransform>,
    min_signed_values: Vec<i32>,
    attribute_specs: Vec<AttributeSpec>,
    signed_attribute_specs: Vec<SignedAttributeSpec>,
    cached_decoded: Option<CachedDecoded>,
}

#[derive(Clone)]
struct AttributeSpec {
    att_id: i32,
    offset: usize,
    num_components: usize,
    data_type: DataType,
}

#[derive(Clone)]
struct SignedAttributeSpec {
    att_id: i32,
    offset: usize,
    num_components: usize,
    data_type: DataType,
}

impl KdTreeAttributesDecoder {
    pub fn new(first_att_id: i32) -> Self {
        Self {
            attribute_ids: vec![first_att_id],
            quantized_portable_attributes: Vec::new(),
            attribute_quantization_transforms: Vec::new(),
            min_signed_values: Vec::new(),
            attribute_specs: Vec::new(),
            signed_attribute_specs: Vec::new(),
            cached_decoded: None,
        }
    }

    pub fn add_attribute_id(&mut self, att_id: i32) {
        self.attribute_ids.push(att_id);
    }

    pub fn decode_attributes_decoder_data(
        &mut self,
        point_cloud: &mut PointCloud,
        in_buffer: &mut DecoderBuffer,
    ) -> bool {
        self.attribute_ids.clear();
        let num_attributes = match in_buffer.decode_varint() {
            Ok(v) => v as usize,
            Err(_) => return false,
        };
        // Attribute descriptor minimum is 5 bytes: four one-byte fields
        // (type, data_type, num_components, normalized) plus at least one
        // byte for the unique_id varint, even when the id is zero.
        const MIN_ATTRIBUTE_DESCRIPTOR_BYTES: usize = 5;
        if num_attributes == 0
            || num_attributes > in_buffer.remaining_size() / MIN_ATTRIBUTE_DESCRIPTOR_BYTES
        {
            return false;
        }

        for _ in 0..num_attributes {
            let att_type_val = match in_buffer.decode_u8() {
                Ok(v) => v,
                Err(_) => return false,
            };
            let att_type = match GeometryAttributeType::try_from(att_type_val) {
                Ok(v) => v,
                Err(_) => return false,
            };

            let data_type_val = match in_buffer.decode_u8() {
                Ok(v) => v,
                Err(_) => return false,
            };
            let data_type = match DataType::try_from(data_type_val) {
                Ok(v) => v,
                Err(_) => return false,
            };

            let num_components = match in_buffer.decode_u8() {
                Ok(v) => v,
                Err(_) => return false,
            };
            if num_components == 0 {
                return false;
            }
            let normalized = match in_buffer.decode_u8() {
                Ok(v) => v != 0,
                Err(_) => return false,
            };
            let unique_id = match in_buffer.decode_varint() {
                Ok(v) => v as u32,
                Err(_) => return false,
            };

            let mut att = PointAttribute::new();
            if att
                .try_init(
                    att_type,
                    num_components,
                    data_type,
                    normalized,
                    point_cloud.num_points(),
                )
                .is_err()
            {
                return false;
            }
            att.set_unique_id(unique_id);

            let att_id = point_cloud.add_attribute(att);
            self.attribute_ids.push(att_id);
        }
        true
    }

    pub fn decode_attributes(
        &mut self,
        point_cloud: &mut PointCloud,
        in_buffer: &mut DecoderBuffer,
    ) -> bool {
        if !self.decode_portable_attributes(point_cloud, in_buffer) {
            return false;
        }
        if !self.decode_data_needed_by_portable_transforms(point_cloud, in_buffer) {
            return false;
        }
        if !self.transform_attributes_to_original_format(point_cloud) {
            return false;
        }
        true
    }

    fn decode_portable_attributes(
        &mut self,
        point_cloud: &mut PointCloud,
        in_buffer: &mut DecoderBuffer,
    ) -> bool {
        let num_expected_points = point_cloud.num_points();
        // Don't clear transforms/min_values here as they are decoded separately.
        self.quantized_portable_attributes.clear();
        self.attribute_specs.clear();
        self.signed_attribute_specs.clear();
        self.cached_decoded = None;

        let compression_level = match in_buffer.decode_u8() {
            Ok(v) => v,
            Err(_) => return false,
        };
        if compression_level > 6 {
            return false;
        }

        let mut total_dimensionality: usize = 0;
        let mut float_specs: Vec<(i32, usize, usize)> = Vec::new();

        for &att_id in &self.attribute_ids {
            let Ok(att) = point_cloud.try_attribute(att_id) else {
                return false;
            };
            let num_components = att.num_components() as usize;
            self.attribute_specs.push(AttributeSpec {
                att_id,
                offset: total_dimensionality,
                num_components,
                data_type: att.data_type(),
            });
            match att.data_type() {
                DataType::Uint32 | DataType::Uint16 | DataType::Uint8 => {}
                DataType::Int32 | DataType::Int16 | DataType::Int8 => {
                    self.signed_attribute_specs.push(SignedAttributeSpec {
                        att_id,
                        offset: total_dimensionality,
                        num_components,
                        data_type: att.data_type(),
                    });
                    self.min_signed_values
                        .resize(self.min_signed_values.len() + num_components, 0);
                }
                DataType::Float32 => {
                    float_specs.push((att_id, total_dimensionality, num_components));
                }
                _ => return false,
            }
            total_dimensionality = match total_dimensionality.checked_add(num_components) {
                Some(v) => v,
                None => return false,
            };
        }
        if total_dimensionality == 0 {
            return false;
        }

        let total_dimensionality_u32 = match u32::try_from(total_dimensionality) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let mut decoder =
            DynamicIntegerPointsKdTreeDecoder::new(compression_level, total_dimensionality_u32);
        let decoded = match decoder.decode_points(in_buffer, num_expected_points as u32) {
            Some(v) => v,
            None => return false,
        };
        if decoder.num_decoded_points() as usize != num_expected_points {
            return false;
        }
        let Some(expected_decoded_len) = num_expected_points.checked_mul(total_dimensionality)
        else {
            return false;
        };
        if decoded.len() != expected_decoded_len {
            return false;
        }

        // Fill non-float attributes directly, and create portable attributes for float.
        for (att_id, offset, num_components) in float_specs {
            let Ok(att) = point_cloud.try_attribute(att_id) else {
                return false;
            };
            let mut portable = PointAttribute::default();
            if portable
                .try_init(
                    att.attribute_type(),
                    att.num_components(),
                    DataType::Uint32,
                    false,
                    num_expected_points,
                )
                .is_err()
            {
                return false;
            }
            portable.set_identity_mapping();

            if !write_u32_components_from_decoded(
                &decoded,
                total_dimensionality,
                offset,
                num_components,
                num_expected_points,
                &mut portable,
                DataType::Uint32,
            ) {
                return false;
            }

            self.quantized_portable_attributes.push(portable);
        }

        for spec in &self.attribute_specs {
            if matches!(
                spec.data_type,
                DataType::Uint32 | DataType::Uint16 | DataType::Uint8
            ) {
                let Ok(att) = point_cloud.try_attribute_mut(spec.att_id) else {
                    return false;
                };
                if !write_u32_components_from_decoded(
                    &decoded,
                    total_dimensionality,
                    spec.offset,
                    spec.num_components,
                    num_expected_points,
                    att,
                    spec.data_type,
                ) {
                    return false;
                }
            }
        }

        // Store decoded stream for later transforms.
        // We keep it by re-decoding into attributes as needed using stored offsets.
        // (For now we stash it into a hidden field by reconstructing on demand is expensive,
        // so we compute signed values later by reading from decoded slice again.)
        self.cached_decoded = Some(CachedDecoded {
            decoded,
            total_dimensionality,
        });

        true
    }

    pub fn decode_data_needed_by_portable_transforms(
        &mut self,
        point_cloud: &mut PointCloud,
        in_buffer: &mut DecoderBuffer,
    ) -> bool {
        // Float quantization parameters in attribute order.
        for &att_id in &self.attribute_ids {
            let Ok(att) = point_cloud.try_attribute(att_id) else {
                return false;
            };
            if att.data_type() == DataType::Float32 {
                let mut min_values = vec![0.0f32; att.num_components() as usize];
                for v in &mut min_values {
                    *v = match in_buffer.decode::<f32>() {
                        Ok(x) => x,
                        Err(_) => return false,
                    };
                }
                let range = match in_buffer.decode::<f32>() {
                    Ok(x) => x,
                    Err(_) => return false,
                };
                let bits = match in_buffer.decode_u8() {
                    Ok(x) => x,
                    Err(_) => return false,
                };
                if bits > 31 {
                    return false;
                }
                let mut t = AttributeQuantizationTransform::new();
                if !t.set_parameters(bits as i32, &min_values, range) {
                    return false;
                }
                self.attribute_quantization_transforms.push(t);
            }
        }

        // Signed min values.
        for i in 0..self.min_signed_values.len() {
            self.min_signed_values[i] = match in_buffer.decode_varint_signed_i32() {
                Ok(v) => v,
                Err(_) => return false,
            };
        }

        true
    }

    pub fn transform_attributes_to_original_format(
        &mut self,
        point_cloud: &mut PointCloud,
    ) -> bool {
        let cached = match self.cached_decoded.take() {
            Some(c) => c,
            None => return false,
        };

        // Floats.
        let mut float_attr_index = 0usize;
        for &att_id in &self.attribute_ids {
            let Ok(attribute) = point_cloud.try_attribute(att_id) else {
                return false;
            };
            let dt = attribute.data_type();
            if dt == DataType::Float32 {
                let Some(portable) = self.quantized_portable_attributes.get(float_attr_index)
                else {
                    return false;
                };
                let Some(transform) = self.attribute_quantization_transforms.get(float_attr_index)
                else {
                    return false;
                };

                let Ok(target) = point_cloud.try_attribute_mut(att_id) else {
                    return false;
                };
                if !transform.inverse_transform_attribute(portable, target) {
                    return false;
                }

                float_attr_index += 1;
            }
        }

        // Signed ints.
        let mut min_index = 0usize;
        for spec in &self.signed_attribute_specs {
            let Ok(att) = point_cloud.try_attribute_mut(spec.att_id) else {
                return false;
            };
            let num_points = att.size();
            if num_points == 0 {
                continue;
            }

            let stride = att.byte_stride() as usize;
            let component_size = att.data_type().byte_length();

            for p in 0..num_points {
                let avi = att.mapped_index(PointIndex(p as u32));
                let Some(base) = (avi.0 as usize).checked_mul(stride) else {
                    return false;
                };
                for c in 0..spec.num_components {
                    let Some(decoded_index) = p
                        .checked_mul(cached.total_dimensionality)
                        .and_then(|v| v.checked_add(spec.offset))
                        .and_then(|v| v.checked_add(c))
                    else {
                        return false;
                    };
                    let Some(&unsigned) = cached.decoded.get(decoded_index) else {
                        return false;
                    };
                    let Some(&min_value) = self.min_signed_values.get(min_index + c) else {
                        return false;
                    };
                    let signed = unsigned as i64 + min_value as i64;
                    let Some(component_delta) = c.checked_mul(component_size) else {
                        return false;
                    };
                    let Some(component_offset) = base.checked_add(component_delta) else {
                        return false;
                    };
                    if !write_signed_component(
                        att.buffer_mut(),
                        component_offset,
                        spec.data_type,
                        signed,
                    ) {
                        return false;
                    }
                }
            }
            min_index += spec.num_components;
        }

        true
    }
}

struct CachedDecoded {
    decoded: Vec<u32>,
    total_dimensionality: usize,
}

fn write_u32_components_from_decoded(
    decoded: &[u32],
    total_dimensionality: usize,
    offset: usize,
    num_components: usize,
    num_points: usize,
    target_attribute: &mut PointAttribute,
    target_type: DataType,
) -> bool {
    let stride = target_attribute.byte_stride() as usize;
    let component_size = target_type.byte_length();
    for p in 0..num_points {
        let avi = target_attribute.mapped_index(PointIndex(p as u32));
        let Some(base) = (avi.0 as usize).checked_mul(stride) else {
            return false;
        };
        for c in 0..num_components {
            let Some(decoded_index) = p
                .checked_mul(total_dimensionality)
                .and_then(|v| v.checked_add(offset))
                .and_then(|v| v.checked_add(c))
            else {
                return false;
            };
            let Some(&v) = decoded.get(decoded_index) else {
                return false;
            };
            let Some(component_delta) = c.checked_mul(component_size) else {
                return false;
            };
            let Some(component_offset) = base.checked_add(component_delta) else {
                return false;
            };
            if !write_unsigned_component(
                target_attribute.buffer_mut(),
                component_offset,
                target_type,
                v,
            ) {
                return false;
            }
        }
    }
    true
}

fn write_unsigned_component(
    buffer: &mut crate::data_buffer::DataBuffer,
    offset: usize,
    data_type: DataType,
    value: u32,
) -> bool {
    match data_type {
        DataType::Uint8 => buffer.try_write(offset, &[value as u8]),
        DataType::Uint16 => buffer.try_write(offset, &(value as u16).to_le_bytes()),
        DataType::Uint32 => buffer.try_write(offset, &value.to_le_bytes()),
        _ => true,
    }
}

fn write_signed_component(
    buffer: &mut crate::data_buffer::DataBuffer,
    offset: usize,
    data_type: DataType,
    value: i64,
) -> bool {
    match data_type {
        DataType::Int8 => buffer.try_write(offset, &[(value as i8) as u8]),
        DataType::Int16 => buffer.try_write(offset, &(value as i16).to_le_bytes()),
        DataType::Int32 => buffer.try_write(offset, &(value as i32).to_le_bytes()),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        write_u32_components_from_decoded, write_unsigned_component, CachedDecoded,
        KdTreeAttributesDecoder,
    };
    use crate::data_buffer::DataBuffer;
    use crate::decoder_buffer::DecoderBuffer;
    use crate::draco_types::DataType;
    use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
    use crate::point_cloud::PointCloud;

    #[test]
    fn kd_tree_component_write_rejects_out_of_bounds_buffer() {
        let mut buffer = DataBuffer::new();
        buffer.resize(1);

        assert!(!write_unsigned_component(
            &mut buffer,
            0,
            DataType::Uint32,
            7
        ));
    }

    #[test]
    fn kd_tree_decoded_component_write_rejects_short_decoded_stream() {
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Uint32,
            false,
            1,
        );

        assert!(!write_u32_components_from_decoded(
            &[1, 2],
            3,
            0,
            3,
            1,
            &mut attribute,
            DataType::Uint32,
        ));
    }

    #[test]
    fn kd_tree_portable_decode_rejects_invalid_attribute_id() {
        let mut decoder = KdTreeAttributesDecoder::new(-1);
        let mut point_cloud = PointCloud::new();
        let bytes = [0u8];
        let mut buffer = DecoderBuffer::new(&bytes);

        assert!(!decoder.decode_portable_attributes(&mut point_cloud, &mut buffer));
    }

    #[test]
    fn kd_tree_transform_data_rejects_invalid_attribute_id() {
        let mut decoder = KdTreeAttributesDecoder::new(-1);
        let mut point_cloud = PointCloud::new();
        let bytes = [];
        let mut buffer = DecoderBuffer::new(&bytes);

        assert!(!decoder.decode_data_needed_by_portable_transforms(&mut point_cloud, &mut buffer,));
    }

    #[test]
    fn kd_tree_original_transform_rejects_invalid_attribute_id() {
        let mut decoder = KdTreeAttributesDecoder::new(-1);
        decoder.cached_decoded = Some(CachedDecoded {
            decoded: Vec::new(),
            total_dimensionality: 1,
        });
        let mut point_cloud = PointCloud::new();

        assert!(!decoder.transform_attributes_to_original_format(&mut point_cloud));
    }
}
