use crate::attribute_transform::{AttributeTransform, AttributeTransformType};
use crate::attribute_transform_data::AttributeTransformData;
#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
use crate::geometry_attribute::PointAttribute;
use crate::geometry_indices::PointIndex;
use crate::quantization_utils::{Dequantizer, Quantizer};

pub struct AttributeQuantizationTransform {
    quantization_bits: i32,
    min_values: Vec<f32>,
    range: f32,
}

impl Default for AttributeQuantizationTransform {
    fn default() -> Self {
        Self {
            quantization_bits: -1,
            min_values: Vec::new(),
            range: 0.0,
        }
    }
}

impl AttributeQuantizationTransform {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_parameters(
        &mut self,
        quantization_bits: i32,
        min_values: &[f32],
        range: f32,
    ) -> bool {
        if !(1..=31).contains(&quantization_bits) {
            return false;
        }
        self.quantization_bits = quantization_bits;
        self.min_values = min_values.to_vec();
        self.range = range;
        true
    }

    pub fn compute_parameters(
        &mut self,
        attribute: &PointAttribute,
        quantization_bits: i32,
    ) -> bool {
        if !(1..=31).contains(&quantization_bits) {
            return false;
        }
        self.quantization_bits = quantization_bits;
        let num_components = attribute.num_components() as usize;

        let num_entries = attribute.size();
        if num_entries == 0 {
            return false;
        }

        if attribute.data_type() != DataType::Float32 {
            return false;
        }

        let buffer = attribute.buffer();
        let byte_stride = attribute.byte_stride() as usize;

        // Initialize min/max from first entry (matching C++ behavior exactly)
        self.min_values = vec![0.0f32; num_components];
        let mut max_values = vec![0.0f32; num_components];

        // Read the first entry to initialize min/max
        for c in 0..num_components {
            let val = bytemuck::pod_read_unaligned::<f32>(&buffer.data()[c * 4..c * 4 + 4]);
            self.min_values[c] = val;
            max_values[c] = val;
        }

        // Process remaining entries starting from index 1 (matching C++ loop)
        for i in 1..num_entries {
            let offset = i * byte_stride;
            // Read num_components floats
            for c in 0..num_components {
                let val = bytemuck::pod_read_unaligned::<f32>(
                    &buffer.data()[offset + c * 4..offset + c * 4 + 4],
                );

                if val.is_nan() {
                    return false;
                }
                if self.min_values[c] > val {
                    self.min_values[c] = val;
                }
                if max_values[c] < val {
                    max_values[c] = val;
                }
            }
        }

        // Check for NaN/Inf and compute range (matching C++)
        self.range = 0.0;
        for c in 0..num_components {
            if self.min_values[c].is_nan()
                || self.min_values[c].is_infinite()
                || max_values[c].is_nan()
                || max_values[c].is_infinite()
            {
                return false;
            }
            let diff = max_values[c] - self.min_values[c];
            if diff > self.range {
                self.range = diff;
            }
        }

        // Adjust range if it is 0?
        if self.range == 0.0 {
            self.range = 1.0;
        }

        true
    }

    fn generate_portable_attribute(
        &self,
        attribute: &PointAttribute,
        point_ids: &[PointIndex],
        target_attribute: &mut PointAttribute,
    ) {
        if self.quantization_bits < 1 || self.quantization_bits > 31 {
            // Invalid state; caller should have initialized parameters.
            return;
        }
        let num_points = if point_ids.is_empty() {
            attribute.size()
        } else {
            point_ids.len()
        };
        let num_components = attribute.num_components() as usize;

        target_attribute.init(
            attribute.attribute_type(),
            num_components as u8,
            DataType::Uint32, // Quantized values are usually stored as integers
            false,
            num_points,
        );

        // quantization_bits is allowed up to 31. Use a wider type to avoid
        // overflowing signed shifts (e.g. 1 << 31 on i32).
        let max_quantized_value: i32 = ((1u64 << (self.quantization_bits as u32)) - 1) as i32;
        let mut quantizer = Quantizer::new();
        quantizer.init(self.range, max_quantized_value);

        let src_buffer = attribute.buffer();
        let src_stride = attribute.byte_stride() as usize;
        let dst_stride = target_attribute.byte_stride() as usize;
        let dst_buffer = target_attribute.buffer_mut();
        let src_data = src_buffer.data();
        let dst_data = dst_buffer.data_mut();

        // Pre-allocate qvals outside the loop for debug printing.
        #[cfg(feature = "debug_logs")]
        let mut qvals = vec![0i32; num_components];
        #[cfg(feature = "debug_logs")]
        let debug_cmp_cpp = crate::debug_env_enabled("DRACO_DEBUG_CMP_CPP");
        #[cfg(feature = "debug_logs")]
        let debug_cmp_cpp_max_print = std::env::var("DRACO_DEBUG_CMP_MAX_PRINT")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(20);
        #[cfg(feature = "debug_logs")]
        let debug_cmp_cpp_file = std::env::var("DRACO_DEBUG_CMP_CPP_FILE").ok();

        // Fast path for common case: 3-component float -> 3-component uint32
        // with identity mapping (sequential encoding)
        if num_components == 3 && point_ids.is_empty() {
            for i in 0..num_points {
                let src_offset = i * src_stride;
                let dst_offset = i * dst_stride;

                // Read 3 floats
                let raw_x = f32::from_le_bytes([
                    src_data[src_offset],
                    src_data[src_offset + 1],
                    src_data[src_offset + 2],
                    src_data[src_offset + 3],
                ]);
                let raw_y = f32::from_le_bytes([
                    src_data[src_offset + 4],
                    src_data[src_offset + 5],
                    src_data[src_offset + 6],
                    src_data[src_offset + 7],
                ]);
                let raw_z = f32::from_le_bytes([
                    src_data[src_offset + 8],
                    src_data[src_offset + 9],
                    src_data[src_offset + 10],
                    src_data[src_offset + 11],
                ]);

                // Quantize
                let q_x = quantizer.quantize_float(raw_x - self.min_values[0]) as u32;
                let q_y = quantizer.quantize_float(raw_y - self.min_values[1]) as u32;
                let q_z = quantizer.quantize_float(raw_z - self.min_values[2]) as u32;

                // Write 3 uint32s
                dst_data[dst_offset..dst_offset + 4].copy_from_slice(&q_x.to_le_bytes());
                dst_data[dst_offset + 4..dst_offset + 8].copy_from_slice(&q_y.to_le_bytes());
                dst_data[dst_offset + 8..dst_offset + 12].copy_from_slice(&q_z.to_le_bytes());
            }
        } else {
            // Generic path
            for i in 0..num_points {
                // Use mapped_index to get the correct AttributeValueIndex, matching C++ behavior
                let point_idx = if point_ids.is_empty() {
                    PointIndex(i as u32)
                } else {
                    point_ids[i]
                };
                let att_val_idx = attribute.mapped_index(point_idx);
                let src_offset = att_val_idx.0 as usize * src_stride;
                let dst_offset = i * dst_stride;

                for c in 0..num_components {
                    // Read raw component then subtract min to match C++ ordering
                    let raw_val = bytemuck::pod_read_unaligned::<f32>(
                        &src_data[src_offset + c * 4..src_offset + c * 4 + 4],
                    );
                    let val = raw_val - self.min_values[c];
                    let q_val = quantizer.quantize_float(val);

                    #[cfg(feature = "debug_logs")]
                    {
                        qvals[c] = q_val;
                    }

                    let q_val_u32 = q_val as u32;
                    let bytes = bytemuck::bytes_of(&q_val_u32);
                    dst_data[dst_offset + c * 4..dst_offset + c * 4 + 4].copy_from_slice(bytes);
                }

                // Allow limiting how many points are printed via env var.
                #[cfg(feature = "debug_logs")]
                {
                    if debug_cmp_cpp && i < debug_cmp_cpp_max_print {
                        let orig_pt = point_idx.0;
                        eprintln!("RUST QT orig_pt={} P{}: {:?}", orig_pt, i, qvals);
                        if let Some(fname) = debug_cmp_cpp_file.as_deref() {
                            use std::io::Write;
                            if let Ok(mut f) = std::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(&fname)
                            {
                                let _ =
                                    writeln!(f, "RUST QT orig_pt={} P{}: {:?}", orig_pt, i, qvals);
                            }
                        }
                    }
                }
            }
        }
    }
}

impl AttributeTransform for AttributeQuantizationTransform {
    fn transform_type(&self) -> AttributeTransformType {
        AttributeTransformType::QuantizationTransform
    }

    fn init_from_attribute(&mut self, attribute: &PointAttribute) -> bool {
        if let Some(data) = attribute.attribute_transform_data() {
            if data.transform_type() != AttributeTransformType::QuantizationTransform {
                return false;
            }
            let mut byte_offset = 0;
            if let Some(bits) = data.get_parameter_value::<i32>(byte_offset) {
                self.quantization_bits = bits;
                byte_offset += 4;
            } else {
                return false;
            }

            let num_components = attribute.num_components() as usize;
            self.min_values.resize(num_components, 0.0);
            for i in 0..num_components {
                if let Some(val) = data.get_parameter_value::<f32>(byte_offset) {
                    self.min_values[i] = val;
                    byte_offset += 4;
                } else {
                    return false;
                }
            }

            if let Some(range) = data.get_parameter_value::<f32>(byte_offset) {
                self.range = range;
            } else {
                return false;
            }

            true
        } else {
            false
        }
    }

    fn copy_to_attribute_transform_data(&self, out_data: &mut AttributeTransformData) {
        out_data.set_transform_type(AttributeTransformType::QuantizationTransform);
        out_data.append_parameter_value(self.quantization_bits);
        for &val in &self.min_values {
            out_data.append_parameter_value(val);
        }
        out_data.append_parameter_value(self.range);
    }

    fn transform_attribute(
        &self,
        attribute: &PointAttribute,
        point_ids: &[PointIndex],
        target_attribute: &mut PointAttribute,
    ) -> bool {
        self.generate_portable_attribute(attribute, point_ids, target_attribute);
        true
    }

    fn inverse_transform_attribute(
        &self,
        attribute: &PointAttribute,
        target_attribute: &mut PointAttribute,
    ) -> bool {
        if target_attribute.data_type() != DataType::Float32 {
            return false;
        }

        if self.quantization_bits < 1 || self.quantization_bits > 31 {
            return false;
        }

        // quantization_bits is allowed up to 31. Use a wider type to avoid
        // overflowing signed shifts (e.g. 1 << 31 on i32).
        let max_quantized_value: i32 = ((1u64 << (self.quantization_bits as u32)) - 1) as i32;
        let mut dequantizer = Dequantizer::new();
        if !dequantizer.init(self.range, max_quantized_value) {
            return false;
        }

        let num_components = target_attribute.num_components() as usize;
        if self.min_values.len() < num_components {
            return false;
        }
        let num_values = target_attribute.size();

        let Ok(dst_stride) = usize::try_from(target_attribute.byte_stride()) else {
            return false;
        };
        let Ok(src_stride) = usize::try_from(attribute.byte_stride()) else {
            return false;
        };
        let src_buffer = attribute.buffer();
        let dst_buffer = target_attribute.buffer_mut();
        let src_data = src_buffer.data();
        let dst_data = dst_buffer.data_mut();

        const COMPONENT_SIZE: usize = std::mem::size_of::<u32>();
        let Some(tight_stride) = num_components.checked_mul(COMPONENT_SIZE) else {
            return false;
        };
        if attribute.data_type() == DataType::Uint32
            && (1..=4).contains(&num_components)
            && src_stride == tight_stride
            && dst_stride == tight_stride
        {
            let Some(required_src) = num_values.checked_mul(src_stride) else {
                return false;
            };
            let Some(required_dst) = num_values.checked_mul(dst_stride) else {
                return false;
            };
            if src_data.len() < required_src || dst_data.len() < required_dst {
                return false;
            }

            match num_components {
                1 => {
                    for i in 0..num_values {
                        let offset = i * tight_stride;
                        let q_x = i32::from_le_bytes([
                            src_data[offset],
                            src_data[offset + 1],
                            src_data[offset + 2],
                            src_data[offset + 3],
                        ]);

                        let x = dequantizer.dequantize_float(q_x) + self.min_values[0];
                        dst_data[offset..offset + COMPONENT_SIZE].copy_from_slice(&x.to_le_bytes());
                    }
                }
                2 => {
                    for i in 0..num_values {
                        let offset = i * tight_stride;
                        let q_x = i32::from_le_bytes([
                            src_data[offset],
                            src_data[offset + 1],
                            src_data[offset + 2],
                            src_data[offset + 3],
                        ]);
                        let q_y = i32::from_le_bytes([
                            src_data[offset + 4],
                            src_data[offset + 5],
                            src_data[offset + 6],
                            src_data[offset + 7],
                        ]);

                        let x = dequantizer.dequantize_float(q_x) + self.min_values[0];
                        let y = dequantizer.dequantize_float(q_y) + self.min_values[1];

                        dst_data[offset..offset + COMPONENT_SIZE].copy_from_slice(&x.to_le_bytes());
                        dst_data[offset + 4..offset + 8].copy_from_slice(&y.to_le_bytes());
                    }
                }
                3 => {
                    for i in 0..num_values {
                        let offset = i * tight_stride;
                        let q_x = i32::from_le_bytes([
                            src_data[offset],
                            src_data[offset + 1],
                            src_data[offset + 2],
                            src_data[offset + 3],
                        ]);
                        let q_y = i32::from_le_bytes([
                            src_data[offset + 4],
                            src_data[offset + 5],
                            src_data[offset + 6],
                            src_data[offset + 7],
                        ]);
                        let q_z = i32::from_le_bytes([
                            src_data[offset + 8],
                            src_data[offset + 9],
                            src_data[offset + 10],
                            src_data[offset + 11],
                        ]);

                        let x = dequantizer.dequantize_float(q_x) + self.min_values[0];
                        let y = dequantizer.dequantize_float(q_y) + self.min_values[1];
                        let z = dequantizer.dequantize_float(q_z) + self.min_values[2];

                        dst_data[offset..offset + COMPONENT_SIZE].copy_from_slice(&x.to_le_bytes());
                        dst_data[offset + 4..offset + 8].copy_from_slice(&y.to_le_bytes());
                        dst_data[offset + 8..offset + 12].copy_from_slice(&z.to_le_bytes());
                    }
                }
                4 => {
                    for i in 0..num_values {
                        let offset = i * tight_stride;
                        let q_x = i32::from_le_bytes([
                            src_data[offset],
                            src_data[offset + 1],
                            src_data[offset + 2],
                            src_data[offset + 3],
                        ]);
                        let q_y = i32::from_le_bytes([
                            src_data[offset + 4],
                            src_data[offset + 5],
                            src_data[offset + 6],
                            src_data[offset + 7],
                        ]);
                        let q_z = i32::from_le_bytes([
                            src_data[offset + 8],
                            src_data[offset + 9],
                            src_data[offset + 10],
                            src_data[offset + 11],
                        ]);
                        let q_w = i32::from_le_bytes([
                            src_data[offset + 12],
                            src_data[offset + 13],
                            src_data[offset + 14],
                            src_data[offset + 15],
                        ]);

                        let x = dequantizer.dequantize_float(q_x) + self.min_values[0];
                        let y = dequantizer.dequantize_float(q_y) + self.min_values[1];
                        let z = dequantizer.dequantize_float(q_z) + self.min_values[2];
                        let w = dequantizer.dequantize_float(q_w) + self.min_values[3];

                        dst_data[offset..offset + COMPONENT_SIZE].copy_from_slice(&x.to_le_bytes());
                        dst_data[offset + 4..offset + 8].copy_from_slice(&y.to_le_bytes());
                        dst_data[offset + 8..offset + 12].copy_from_slice(&z.to_le_bytes());
                        dst_data[offset + 12..offset + 16].copy_from_slice(&w.to_le_bytes());
                    }
                }
                _ => return false,
            }

            return true;
        }

        for i in 0..num_values {
            let Some(src_offset) = i.checked_mul(src_stride) else {
                return false;
            };
            let Some(dst_offset) = i.checked_mul(dst_stride) else {
                return false;
            };

            for c in 0..num_components {
                let Some(component_offset) = c.checked_mul(4) else {
                    return false;
                };
                let Some(src_pos) = src_offset.checked_add(component_offset) else {
                    return false;
                };
                let Some(src_end) = src_pos.checked_add(4) else {
                    return false;
                };
                let Some(src_bytes) = src_data.get(src_pos..src_end) else {
                    return false;
                };
                let q_val =
                    i32::from_le_bytes([src_bytes[0], src_bytes[1], src_bytes[2], src_bytes[3]]);

                let val = dequantizer.dequantize_float(q_val) + self.min_values[c];
                let Some(dst_pos) = dst_offset.checked_add(component_offset) else {
                    return false;
                };
                let Some(dst_end) = dst_pos.checked_add(4) else {
                    return false;
                };
                let Some(dst_bytes) = dst_data.get_mut(dst_pos..dst_end) else {
                    return false;
                };
                dst_bytes.copy_from_slice(&val.to_le_bytes());
            }
        }

        true
    }

    #[cfg(feature = "encoder")]
    fn encode_parameters(&self, encoder_buffer: &mut EncoderBuffer) -> bool {
        for &val in &self.min_values {
            encoder_buffer.encode(val);
        }
        encoder_buffer.encode(self.range);
        encoder_buffer.encode_u8(self.quantization_bits as u8);
        true
    }

    #[cfg(feature = "decoder")]
    fn decode_parameters(
        &mut self,
        attribute: &PointAttribute,
        decoder_buffer: &mut DecoderBuffer,
    ) -> bool {
        let num_components = attribute.num_components() as usize;

        self.min_values.resize(num_components, 0.0);
        for i in 0..num_components {
            if let Ok(val) = decoder_buffer.decode::<f32>() {
                self.min_values[i] = val;
            } else {
                return false;
            }
        }

        if let Ok(range) = decoder_buffer.decode::<f32>() {
            self.range = range;
        } else {
            return false;
        }

        if let Ok(bits) = decoder_buffer.decode_u8() {
            self.quantization_bits = bits as i32;
        } else {
            return false;
        }

        if self.quantization_bits < 1 || self.quantization_bits > 31 {
            return false;
        }

        true
    }

    fn get_transformed_data_type(&self, _attribute: &PointAttribute) -> DataType {
        DataType::Uint32
    }

    fn get_transformed_num_components(&self, attribute: &PointAttribute) -> i32 {
        attribute.num_components() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};

    #[test]
    fn inverse_quantization_rejects_truncated_source_buffer() {
        let mut source = PointAttribute::new();
        source.init(
            GeometryAttributeType::Position,
            3,
            DataType::Uint32,
            false,
            1,
        );
        source.buffer_mut().write(0, &1u32.to_le_bytes());
        source.buffer_mut().write(4, &2u32.to_le_bytes());
        source.buffer_mut().resize(8);

        let mut target = PointAttribute::new();
        target.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            1,
        );

        let mut transform = AttributeQuantizationTransform::new();
        assert!(transform.set_parameters(10, &[0.0, 0.0, 0.0], 1.0));
        assert!(!transform.inverse_transform_attribute(&source, &mut target));
    }

    #[test]
    fn inverse_quantization_rejects_short_min_values() {
        let mut source = PointAttribute::new();
        source.init(
            GeometryAttributeType::Position,
            3,
            DataType::Uint32,
            false,
            1,
        );

        let mut target = PointAttribute::new();
        target.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            1,
        );

        let mut transform = AttributeQuantizationTransform::new();
        assert!(transform.set_parameters(10, &[0.0, 0.0], 1.0));
        assert!(!transform.inverse_transform_attribute(&source, &mut target));
    }
}
