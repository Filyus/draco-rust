use crate::attribute_quantization_transform::AttributeQuantizationTransform;
use crate::attribute_transform::AttributeTransform;
use crate::draco_types::DataType;
use crate::dynamic_integer_points_kd_tree::{DynamicIntegerPointsKdTreeEncoder, PointDVector};
use crate::encoder_buffer::EncoderBuffer;
use crate::encoder_options::EncoderOptions;
use crate::geometry_indices::PointIndex;
use crate::point_cloud::PointCloud;

pub struct KdTreeAttributesEncoder {
    attribute_ids: Vec<i32>,
    num_components: u32,
    attribute_quantization_transforms: Vec<AttributeQuantizationTransform>,
    quantized_portable_attributes: Vec<crate::geometry_attribute::PointAttribute>,
    min_signed_values: Vec<i32>,
}

impl KdTreeAttributesEncoder {
    pub fn new(first_att_id: i32) -> Self {
        Self {
            attribute_ids: vec![first_att_id],
            num_components: 0,
            attribute_quantization_transforms: Vec::new(),
            quantized_portable_attributes: Vec::new(),
            min_signed_values: Vec::new(),
        }
    }

    pub fn add_attribute_id(&mut self, att_id: i32) {
        self.attribute_ids.push(att_id);
    }

    pub fn encode_attributes_encoder_data(
        &self,
        point_cloud: &PointCloud,
        out_buffer: &mut EncoderBuffer,
    ) -> bool {
        // Encode number of attributes
        out_buffer.encode_varint(self.attribute_ids.len() as u64);

        for &att_id in &self.attribute_ids {
            let att = point_cloud.attribute(att_id);
            // Encode attribute metadata
            out_buffer.encode_u8(att.attribute_type() as u8);
            out_buffer.encode_u8(att.data_type() as u8);
            out_buffer.encode_u8(att.num_components());
            out_buffer.encode_u8(if att.normalized() { 1 } else { 0 });
            out_buffer.encode_varint(att.unique_id() as u64);
        }
        true
    }

    pub fn attribute_ids(&self) -> &[i32] {
        &self.attribute_ids
    }

    pub fn transform_attributes_to_portable_format(
        &mut self,
        point_cloud: &PointCloud,
        options: &EncoderOptions,
    ) -> bool {
        self.attribute_quantization_transforms.clear();
        self.quantized_portable_attributes.clear();
        self.min_signed_values.clear();

        let num_points = point_cloud.num_points();
        let point_ids: Vec<PointIndex> = (0..num_points).map(|i| PointIndex(i as u32)).collect();

        let mut total_components: u32 = 0;
        for &att_id in &self.attribute_ids {
            let att = point_cloud.attribute(att_id);
            total_components += att.num_components() as u32;
        }
        self.num_components = total_components;

        for &att_id in &self.attribute_ids {
            let att = point_cloud.attribute(att_id);
            match att.data_type() {
                DataType::Float32 => {
                    let quantization_bits =
                        options.get_attribute_int(att_id, "quantization_bits", -1);
                    if quantization_bits < 1 {
                        return false;
                    }
                    let mut transform = AttributeQuantizationTransform::new();
                    if !transform.compute_parameters(att, quantization_bits) {
                        return false;
                    }

                    let mut portable_att = crate::geometry_attribute::PointAttribute::default();
                    portable_att.init(
                        att.attribute_type(),
                        att.num_components(),
                        DataType::Uint32,
                        false,
                        num_points,
                    );
                    portable_att.set_identity_mapping();

                    if !transform.transform_attribute(att, &point_ids, &mut portable_att) {
                        return false;
                    }

                    self.attribute_quantization_transforms.push(transform);
                    self.quantized_portable_attributes.push(portable_att);
                }
                DataType::Int32 | DataType::Int16 | DataType::Int8 => {
                    // Determine per-component minimum.
                    let num_components = att.num_components() as usize;
                    let mut min_vals = vec![i32::MAX; num_components];
                    let stride = att.byte_stride() as usize;
                    let component_size = att.data_type().byte_length();
                    for i in 0..num_points {
                        let avi = att.mapped_index(PointIndex(i as u32));
                        let base = avi.0 as usize * stride;
                        for c in 0..num_components {
                            let v = read_as_i32(
                                att.buffer(),
                                base + c * component_size,
                                att.data_type(),
                            );
                            if v < min_vals[c] {
                                min_vals[c] = v;
                            }
                        }
                    }
                    self.min_signed_values.extend_from_slice(&min_vals);
                }
                _ => {}
            }
        }

        true
    }

    pub fn encode_attributes(
        &mut self,
        point_cloud: &PointCloud,
        options: &EncoderOptions,
        out_buffer: &mut EncoderBuffer,
    ) -> bool {
        // Draco C++: compression_level = min(10 - speed, 6).
        let speed = options.get_encoding_speed();
        let mut compression_level: u8 = (10 - speed).clamp(0, 6) as u8;
        if compression_level == 6 && self.num_components > 15 {
            compression_level = 5;
        }

        out_buffer.encode_u8(compression_level);

        let num_points = point_cloud.num_points();
        let mut point_vector = PointDVector::new(num_points, self.num_components as usize);

        let mut num_processed_components: usize = 0;
        let mut num_processed_quantized_attributes: usize = 0;
        let mut num_processed_signed_components: usize = 0;

        for &att_id in &self.attribute_ids {
            let att = point_cloud.attribute(att_id);
            let use_quantized;

            let src: &crate::geometry_attribute::PointAttribute = match att.data_type() {
                DataType::Uint32
                | DataType::Uint16
                | DataType::Uint8
                | DataType::Int32
                | DataType::Int16
                | DataType::Int8 => {
                    use_quantized = false;
                    att
                }
                DataType::Float32 => {
                    use_quantized = true;
                    let pa =
                        &self.quantized_portable_attributes[num_processed_quantized_attributes];
                    num_processed_quantized_attributes += 1;
                    pa
                }
                _ => {
                    return false;
                }
            };

            let num_att_components = src.num_components() as usize;
            let stride = src.byte_stride() as usize;
            let component_size = src.data_type().byte_length();

            match src.data_type() {
                DataType::Uint32 => {
                    for p in 0..num_points {
                        let avi = src.mapped_index(PointIndex(p as u32));
                        let base = avi.0 as usize * stride;
                        let dst = point_vector.point_mut(p);
                        for c in 0..num_att_components {
                            let v = read_as_u32(
                                src.buffer(),
                                base + c * component_size,
                                DataType::Uint32,
                            );
                            dst[num_processed_components + c] = v;
                        }
                    }
                }
                DataType::Int32 | DataType::Int16 | DataType::Int8 => {
                    for p in 0..num_points {
                        let avi = src.mapped_index(PointIndex(p as u32));
                        let base = avi.0 as usize * stride;
                        let dst = point_vector.point_mut(p);
                        for c in 0..num_att_components {
                            let signed = read_as_i32(
                                src.buffer(),
                                base + c * component_size,
                                src.data_type(),
                            );
                            let minv = self.min_signed_values[num_processed_signed_components + c];
                            dst[num_processed_components + c] = (signed - minv) as u32;
                        }
                    }
                    num_processed_signed_components += num_att_components;
                }
                DataType::Uint16 | DataType::Uint8 => {
                    for p in 0..num_points {
                        let avi = src.mapped_index(PointIndex(p as u32));
                        let base = avi.0 as usize * stride;
                        let dst = point_vector.point_mut(p);
                        for c in 0..num_att_components {
                            let v = read_as_u32(
                                src.buffer(),
                                base + c * component_size,
                                src.data_type(),
                            );
                            dst[num_processed_components + c] = v;
                        }
                    }
                }
                _ => {
                    // Should only happen for Float32 which gets converted to Uint32 portable.
                    if use_quantized {
                        return false;
                    }
                    return false;
                }
            }

            num_processed_components += num_att_components;
        }

        // Compute maximum bit length.
        let mut num_bits: u32 = 0;
        for &v in point_vector.as_slice() {
            if v != 0 {
                let msb = 32 - v.leading_zeros();
                if msb > num_bits {
                    num_bits = msb;
                }
            }
        }

        let mut encoder =
            DynamicIntegerPointsKdTreeEncoder::new(compression_level, self.num_components);
        encoder.encode_points(&mut point_vector, num_bits, out_buffer)
    }

    pub fn encode_data_needed_by_portable_transforms(
        &self,
        out_buffer: &mut EncoderBuffer,
    ) -> bool {
        for t in &self.attribute_quantization_transforms {
            if !t.encode_parameters(out_buffer) {
                return false;
            }
        }

        for &minv in &self.min_signed_values {
            out_buffer.encode_varint_signed_i32(minv);
        }

        true
    }
}

fn read_as_i32(buffer: &crate::data_buffer::DataBuffer, offset: usize, data_type: DataType) -> i32 {
    match data_type {
        DataType::Int8 => {
            let mut bytes = [0u8; 1];
            buffer.read(offset, &mut bytes);
            bytes[0] as i8 as i32
        }
        DataType::Int16 => {
            let mut bytes = [0u8; 2];
            buffer.read(offset, &mut bytes);
            i16::from_le_bytes(bytes) as i32
        }
        DataType::Int32 => {
            let mut bytes = [0u8; 4];
            buffer.read(offset, &mut bytes);
            i32::from_le_bytes(bytes)
        }
        _ => 0,
    }
}

fn read_as_u32(buffer: &crate::data_buffer::DataBuffer, offset: usize, data_type: DataType) -> u32 {
    match data_type {
        DataType::Uint8 => {
            let mut bytes = [0u8; 1];
            buffer.read(offset, &mut bytes);
            bytes[0] as u32
        }
        DataType::Uint16 => {
            let mut bytes = [0u8; 2];
            buffer.read(offset, &mut bytes);
            u16::from_le_bytes(bytes) as u32
        }
        DataType::Uint32 => {
            let mut bytes = [0u8; 4];
            buffer.read(offset, &mut bytes);
            u32::from_le_bytes(bytes)
        }
        _ => 0,
    }
}
