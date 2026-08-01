//! Quantization attribute transform.
//!
//! [`AttributeQuantizationTransform`] maps floating-point attribute values onto
//! a fixed-point integer grid defined by an origin, range, and bit count, and
//! inverts it (dequantizes) on decode. This is the most common transform for
//! positions and texture coordinates. Port of Draco's
//! `attribute_quantization_transform.h`.

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
use crate::status::{DracoError, Status};

/// The quantization bit counts this transform can represent.
///
/// Deliberately wider than the octahedron transform's `2..=30`: the two carry
/// different payloads and upstream validates them separately.
const VALID_QUANTIZATION_BITS: std::ops::RangeInclusive<i32> = 1..=31;

fn invalid_quantization_bits(bits: i32) -> DracoError {
    DracoError::InvalidParameter(format!(
        "Quantization bits {bits} outside the supported range 1..=31"
    ))
}

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
    ) -> Status {
        if !VALID_QUANTIZATION_BITS.contains(&quantization_bits) {
            return Err(invalid_quantization_bits(quantization_bits));
        }
        self.quantization_bits = quantization_bits;
        self.min_values = min_values.to_vec();
        self.range = range;
        Ok(())
    }

    pub fn compute_parameters(
        &mut self,
        attribute: &PointAttribute,
        quantization_bits: i32,
    ) -> Status {
        if !VALID_QUANTIZATION_BITS.contains(&quantization_bits) {
            return Err(invalid_quantization_bits(quantization_bits));
        }
        self.quantization_bits = quantization_bits;
        let num_components = attribute.num_components() as usize;

        let num_entries = attribute.size();
        if num_entries == 0 {
            return Err(DracoError::InvalidParameter(
                "Cannot compute quantization parameters from an empty attribute".to_string(),
            ));
        }

        if attribute.data_type() != DataType::Float32 {
            return Err(DracoError::InvalidParameter(format!(
                "Quantization needs a float32 attribute, got {:?}",
                attribute.data_type()
            )));
        }

        let buffer = attribute.buffer();
        let data = buffer.data();
        let byte_stride = attribute.byte_stride() as usize;

        // The buffer is allocated alongside `size()` on the common path, but
        // `buffer_mut()` is public and geometry handed to the encoder may have
        // been assembled from a file whose loader truncated the buffer
        // independently of the value count it reports. Every read is therefore
        // bounds-checked, as the octahedron transform's counterpart already is,
        // rather than sliced unchecked.
        self.min_values = vec![0.0f32; num_components];
        let mut max_values = vec![0.0f32; num_components];

        let read_component = |start: usize| -> Option<f32> {
            let end = start.checked_add(4)?;
            Some(bytemuck::pod_read_unaligned::<f32>(data.get(start..end)?))
        };

        let truncated = || {
            DracoError::General(
                "Attribute source data is truncated relative to its value count".to_string(),
            )
        };

        // Read the first entry to initialize min/max (matching C++ behavior)
        for c in 0..num_components {
            let Some(val) = read_component(c * 4) else {
                return Err(truncated());
            };
            self.min_values[c] = val;
            max_values[c] = val;
        }

        // Process remaining entries starting from index 1 (matching C++ loop)
        for i in 1..num_entries {
            let Some(offset) = i.checked_mul(byte_stride) else {
                return Err(DracoError::General(
                    "Attribute byte offset overflow".to_string(),
                ));
            };
            // Read num_components floats
            for c in 0..num_components {
                let Some(val) = offset.checked_add(c * 4).and_then(read_component) else {
                    return Err(truncated());
                };

                if val.is_nan() {
                    return Err(DracoError::InvalidParameter(
                        "Attribute value is NaN and cannot be quantized".to_string(),
                    ));
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
                return Err(DracoError::InvalidParameter(format!(
                    "Attribute component {c} spans a non-finite range and cannot be quantized"
                )));
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

        Ok(())
    }

    /// Quantizes `attribute` into `target_attribute`, reporting what went wrong
    /// when the source cannot supply the values asked of it.
    ///
    /// The source offset is `mapped_index(point) * byte_stride`, and the mapped
    /// index is caller data: an attribute whose point map does not cover the
    /// points this encode walks yields the invalid index, whose offset is past
    /// the end of any buffer. The octahedron transform's counterpart already
    /// refuses a source too short for what it is asked to read; this one
    /// indexed and panicked.
    fn generate_portable_attribute(
        &self,
        attribute: &PointAttribute,
        point_ids: &[PointIndex],
        target_attribute: &mut PointAttribute,
    ) -> Status {
        if !VALID_QUANTIZATION_BITS.contains(&self.quantization_bits) {
            // Invalid state; caller should have initialized parameters.
            return Err(invalid_quantization_bits(self.quantization_bits));
        }
        let num_points = if point_ids.is_empty() {
            attribute.size()
        } else {
            point_ids.len()
        };
        let num_components = attribute.num_components() as usize;

        // `min_values` was sized by `compute_parameters` from whichever
        // attribute it was given, and nothing ties that to the attribute being
        // transformed here. The loops below index it per component -- the fast
        // path as `min_values[0..3]` literally -- so a transform computed for
        // two components and applied to three panicked, reachable through the
        // public `AttributeTransform::transform_attribute`. The inverse
        // direction has checked exactly this since it was written; this is the
        // forward half of the same check.
        if self.min_values.len() < num_components {
            return Err(DracoError::InvalidParameter(format!(
                "Quantization parameters cover {} components, attribute has {num_components}",
                self.min_values.len()
            )));
        }

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
                if src_offset + 12 > src_data.len() || dst_offset + 12 > dst_data.len() {
                    return Err(DracoError::General(
                        "Quantization source or target data is truncated".to_string(),
                    ));
                }

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
                let Some(src_offset) = (att_val_idx.0 as usize).checked_mul(src_stride) else {
                    return Err(DracoError::General(
                        "Attribute byte offset overflow".to_string(),
                    ));
                };
                let dst_offset = i * dst_stride;
                if src_offset + num_components * 4 > src_data.len()
                    || dst_offset + num_components * 4 > dst_data.len()
                {
                    return Err(DracoError::General(
                        "Quantization source or target data is truncated".to_string(),
                    ));
                }

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
                        debug_log!("RUST QT orig_pt={} P{}: {:?}", orig_pt, i, qvals);
                        if let Some(fname) = debug_cmp_cpp_file.as_deref() {
                            use std::io::Write;
                            if let Ok(mut f) = std::fs::OpenOptions::new()
                                .create(true)
                                .append(true)
                                .open(fname)
                            {
                                let _ =
                                    writeln!(f, "RUST QT orig_pt={} P{}: {:?}", orig_pt, i, qvals);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl AttributeTransform for AttributeQuantizationTransform {
    fn transform_type(&self) -> AttributeTransformType {
        AttributeTransformType::QuantizationTransform
    }

    fn init_from_attribute(&mut self, attribute: &PointAttribute) -> Status {
        let Some(data) = attribute.attribute_transform_data() else {
            return Err(DracoError::InvalidParameter(
                "Attribute carries no transform data".to_string(),
            ));
        };
        if data.transform_type() != AttributeTransformType::QuantizationTransform {
            return Err(DracoError::InvalidParameter(format!(
                "Attribute carries {:?}, not a quantization transform",
                data.transform_type()
            )));
        }
        let truncated = || {
            DracoError::InvalidParameter(
                "Attribute transform data is shorter than the quantization parameters".to_string(),
            )
        };

        let mut byte_offset = 0;
        let Some(bits) = data.get_parameter_value::<i32>(byte_offset) else {
            return Err(truncated());
        };
        self.quantization_bits = bits;
        byte_offset += 4;

        let num_components = attribute.num_components() as usize;
        self.min_values.resize(num_components, 0.0);
        for i in 0..num_components {
            let Some(val) = data.get_parameter_value::<f32>(byte_offset) else {
                return Err(truncated());
            };
            self.min_values[i] = val;
            byte_offset += 4;
        }

        let Some(range) = data.get_parameter_value::<f32>(byte_offset) else {
            return Err(truncated());
        };
        self.range = range;

        Ok(())
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
    ) -> Status {
        self.generate_portable_attribute(attribute, point_ids, target_attribute)
    }

    fn inverse_transform_attribute(
        &self,
        attribute: &PointAttribute,
        target_attribute: &mut PointAttribute,
    ) -> Status {
        if target_attribute.data_type() != DataType::Float32 {
            return Err(DracoError::InvalidParameter(format!(
                "Dequantization needs a float32 target, got {:?}",
                target_attribute.data_type()
            )));
        }

        if !VALID_QUANTIZATION_BITS.contains(&self.quantization_bits) {
            return Err(invalid_quantization_bits(self.quantization_bits));
        }

        // quantization_bits is allowed up to 31. Use a wider type to avoid
        // overflowing signed shifts (e.g. 1 << 31 on i32).
        let max_quantized_value: i32 = ((1u64 << (self.quantization_bits as u32)) - 1) as i32;
        let mut dequantizer = Dequantizer::new();
        if !dequantizer.init(self.range, max_quantized_value) {
            return Err(DracoError::InvalidParameter(format!(
                "Dequantizer rejects range {} over {max_quantized_value} steps",
                self.range
            )));
        }

        let num_components = target_attribute.num_components() as usize;
        if self.min_values.len() < num_components {
            return Err(DracoError::InvalidParameter(format!(
                "Quantization parameters cover {} components, attribute has {num_components}",
                self.min_values.len()
            )));
        }
        let num_values = target_attribute.size();

        let Ok(dst_stride) = usize::try_from(target_attribute.byte_stride()) else {
            return Err(DracoError::General(
                "Negative target byte stride".to_string(),
            ));
        };
        let Ok(src_stride) = usize::try_from(attribute.byte_stride()) else {
            return Err(DracoError::General(
                "Negative source byte stride".to_string(),
            ));
        };
        let src_buffer = attribute.buffer();
        let dst_buffer = target_attribute.buffer_mut();
        let src_data = src_buffer.data();
        let dst_data = dst_buffer.data_mut();

        let overflow = || DracoError::General("Attribute byte range overflow".to_string());
        let truncated =
            || DracoError::General("Dequantization source or target data is truncated".to_string());

        const COMPONENT_SIZE: usize = std::mem::size_of::<u32>();
        let Some(tight_stride) = num_components.checked_mul(COMPONENT_SIZE) else {
            return Err(overflow());
        };
        if attribute.data_type() == DataType::Uint32
            && (1..=4).contains(&num_components)
            && src_stride == tight_stride
            && dst_stride == tight_stride
        {
            let Some(required_src) = num_values.checked_mul(src_stride) else {
                return Err(overflow());
            };
            let Some(required_dst) = num_values.checked_mul(dst_stride) else {
                return Err(overflow());
            };
            if src_data.len() < required_src || dst_data.len() < required_dst {
                return Err(truncated());
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
                _ => {
                    return Err(DracoError::InvalidParameter(format!(
                        "Dequantization does not support {num_components} components"
                    )))
                }
            }

            return Ok(());
        }

        for i in 0..num_values {
            let Some(src_offset) = i.checked_mul(src_stride) else {
                return Err(overflow());
            };
            let Some(dst_offset) = i.checked_mul(dst_stride) else {
                return Err(overflow());
            };

            for c in 0..num_components {
                let Some(component_offset) = c.checked_mul(4) else {
                    return Err(overflow());
                };
                let Some(src_pos) = src_offset.checked_add(component_offset) else {
                    return Err(overflow());
                };
                let Some(src_end) = src_pos.checked_add(4) else {
                    return Err(overflow());
                };
                let Some(src_bytes) = src_data.get(src_pos..src_end) else {
                    return Err(truncated());
                };
                let q_val =
                    i32::from_le_bytes([src_bytes[0], src_bytes[1], src_bytes[2], src_bytes[3]]);

                let val = dequantizer.dequantize_float(q_val) + self.min_values[c];
                let Some(dst_pos) = dst_offset.checked_add(component_offset) else {
                    return Err(overflow());
                };
                let Some(dst_end) = dst_pos.checked_add(4) else {
                    return Err(overflow());
                };
                let Some(dst_bytes) = dst_data.get_mut(dst_pos..dst_end) else {
                    return Err(truncated());
                };
                dst_bytes.copy_from_slice(&val.to_le_bytes());
            }
        }

        Ok(())
    }

    #[cfg(feature = "encoder")]
    fn encode_parameters(&self, encoder_buffer: &mut EncoderBuffer) -> Status {
        // The sibling octahedron transform gates this on `is_initialized`;
        // this one wrote whatever it held. On a transform whose parameters were
        // never computed that is `-1`, which `as u8` truncates to `0xFF` -- a
        // quantization-bits byte no decoder accepts, written into the stream
        // and reported as success. Every in-tree caller computes parameters
        // first and checks that call, so this is the trait method's own
        // contract rather than a live defect.
        if !VALID_QUANTIZATION_BITS.contains(&self.quantization_bits) {
            return Err(invalid_quantization_bits(self.quantization_bits));
        }
        for &val in &self.min_values {
            encoder_buffer.encode(val);
        }
        encoder_buffer.encode(self.range);
        encoder_buffer.encode_u8(self.quantization_bits as u8);
        Ok(())
    }

    #[cfg(feature = "decoder")]
    fn decode_parameters(
        &mut self,
        attribute: &PointAttribute,
        decoder_buffer: &mut DecoderBuffer,
    ) -> Status {
        let num_components = attribute.num_components() as usize;
        let truncated = |what: &str| {
            DracoError::BufferError(format!(
                "Stream ends before the quantization {what} it declares"
            ))
        };

        self.min_values.resize(num_components, 0.0);
        for i in 0..num_components {
            let Ok(val) = decoder_buffer.decode::<f32>() else {
                return Err(truncated("minimum values"));
            };
            self.min_values[i] = val;
        }

        let Ok(range) = decoder_buffer.decode::<f32>() else {
            return Err(truncated("range"));
        };
        self.range = range;

        let Ok(bits) = decoder_buffer.decode_u8() else {
            return Err(truncated("bit count"));
        };
        self.quantization_bits = bits as i32;

        if !VALID_QUANTIZATION_BITS.contains(&self.quantization_bits) {
            return Err(invalid_quantization_bits(self.quantization_bits));
        }

        Ok(())
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

    /// A three-component attribute quantized with parameters computed from a
    /// two-component one is refused, not indexed past the end of `min_values`.
    ///
    /// `compute_parameters` sizes `min_values` from whatever attribute it was
    /// given, and nothing ties that to the attribute later transformed. The
    /// fast path indexes `min_values[0]`, `[1]`, `[2]` literally, so this
    /// panicked with "the len is 2 but the index is 2" -- through
    /// `AttributeTransform::transform_attribute`, which is public. The inverse
    /// direction has always checked the same length.
    #[test]
    fn quantizing_with_parameters_from_a_narrower_attribute_is_refused() {
        let mut narrow = PointAttribute::new();
        narrow.init(
            GeometryAttributeType::Generic,
            2,
            DataType::Float32,
            false,
            4,
        );
        for i in 0..8 {
            narrow.buffer_mut().write(i * 4, &(i as f32).to_le_bytes());
        }
        let mut transform = AttributeQuantizationTransform::new();
        assert!(transform.compute_parameters(&narrow, 12).is_ok());
        assert_eq!(transform.min_values.len(), 2);

        let mut wide = PointAttribute::new();
        wide.init(
            GeometryAttributeType::Generic,
            3,
            DataType::Float32,
            false,
            4,
        );
        for i in 0..12 {
            wide.buffer_mut().write(i * 4, &(i as f32).to_le_bytes());
        }

        let mut target = PointAttribute::default();
        let err = transform
            .transform_attribute(&wide, &[], &mut target)
            .expect_err("three components against two min_values must be refused");
        assert!(
            err.to_string().contains("cover 2 components"),
            "the error should name the mismatch, got: {err}"
        );
    }

    /// A transform whose parameters were never computed refuses to write them.
    ///
    /// It used to write `-1 as u8`, that is `0xFF`, as the quantization-bits
    /// byte and report success -- a value no decoder accepts. The sibling
    /// octahedron transform has always gated this on `is_initialized`.
    #[test]
    fn encoding_parameters_from_an_uninitialized_transform_is_refused() {
        let transform = AttributeQuantizationTransform::new();
        let mut buffer = crate::encoder_buffer::EncoderBuffer::new();
        let err = transform
            .encode_parameters(&mut buffer)
            .expect_err("uninitialized parameters must not be written");
        assert!(
            err.to_string().contains("-1"),
            "the error should name the bit count it refused, got: {err}"
        );
        assert!(
            buffer.data().is_empty(),
            "nothing should reach the stream on refusal"
        );
    }

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
        assert!(transform.set_parameters(10, &[0.0, 0.0, 0.0], 1.0).is_ok());
        let err = transform
            .inverse_transform_attribute(&source, &mut target)
            .expect_err("a source one component short must be refused");
        assert!(
            err.to_string().contains("truncated"),
            "the error should say the data is short, got: {err}"
        );
    }

    #[test]
    fn compute_parameters_rejects_truncated_value_buffer() {
        // `buffer_mut()` is public, so a loader can truncate the value buffer
        // without updating `size()`. `compute_parameters` used to slice the
        // buffer at `i * byte_stride` unchecked; it now reports a failure, as
        // the octahedron transform's counterpart already does.
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            4, // reports four values, so the loop below would index past 12 bytes
        );
        attribute.buffer_mut().resize(8); // one value short of two complete entries

        let mut transform = AttributeQuantizationTransform::new();
        let err = transform
            .compute_parameters(&attribute, 8)
            .expect_err("a buffer shorter than the value count must be refused");
        assert!(
            err.to_string().contains("truncated"),
            "the error should say the source is short, got: {err}"
        );
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
        assert!(transform.set_parameters(10, &[0.0, 0.0], 1.0).is_ok());
        let err = transform
            .inverse_transform_attribute(&source, &mut target)
            .expect_err("two min_values against three components must be refused");
        assert!(
            err.to_string().contains("cover 2 components"),
            "the error should name the mismatch, got: {err}"
        );
    }

    /// The three refusals the trait used to report as one indistinguishable
    /// `false` now name themselves.
    ///
    /// This is the whole point of the conversion: "the source is truncated",
    /// "the parameters were never computed" and "the parameters are for a
    /// different component count" are separate faults with separate fixes, and
    /// a caller could not previously tell them apart.
    #[test]
    fn the_three_quantization_refusals_are_distinguishable() {
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            4,
        );
        attribute.buffer_mut().resize(8);
        let mut truncated = AttributeQuantizationTransform::new();
        let truncated = truncated.compute_parameters(&attribute, 8).unwrap_err();

        let uninitialized = AttributeQuantizationTransform::new()
            .encode_parameters(&mut crate::encoder_buffer::EncoderBuffer::new())
            .unwrap_err();

        let mut narrow = AttributeQuantizationTransform::new();
        narrow.set_parameters(10, &[0.0, 0.0], 1.0).unwrap();
        let mut wide = PointAttribute::new();
        wide.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            1,
        );
        let mismatched = narrow
            .transform_attribute(&wide, &[], &mut PointAttribute::default())
            .unwrap_err();

        let messages = [
            truncated.to_string(),
            uninitialized.to_string(),
            mismatched.to_string(),
        ];
        for (i, a) in messages.iter().enumerate() {
            for b in &messages[i + 1..] {
                assert_ne!(a, b, "distinct faults must report distinct messages");
            }
        }
    }
}
