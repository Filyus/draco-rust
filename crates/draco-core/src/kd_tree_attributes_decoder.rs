//! KD-tree point-cloud attribute decoder.
//!
//! [`KdTreeAttributesDecoder`] decodes point-cloud attributes compressed with
//! Draco's KD-tree scheme, which exploits spatial coherence instead of explicit
//! connectivity. Used for point clouds (no mesh faces). Port of Draco's
//! `kd_tree_attributes_decoder.h`.

use crate::attribute_quantization_transform::AttributeQuantizationTransform;
use crate::attribute_transform::AttributeTransform;
use crate::decoder_buffer::DecoderBuffer;
use crate::draco_types::DataType;
use crate::dynamic_integer_points_kd_tree::DynamicIntegerPointsKdTreeDecoder;
use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
use crate::geometry_indices::PointIndex;
use crate::point_cloud::PointCloud;
use crate::status::{DracoError, Status};

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
    ) -> Status {
        self.attribute_ids.clear();
        let num_attributes = in_buffer
            .decode_varint()
            .map_err(|_| DracoError::buffer("Buffer ran out reading the KD-tree attribute count"))?
            as usize;
        // Attribute descriptor minimum is 5 bytes: four one-byte fields
        // (type, data_type, num_components, normalized) plus at least one
        // byte for the unique_id varint, even when the id is zero.
        const MIN_ATTRIBUTE_DESCRIPTOR_BYTES: usize = 5;
        if num_attributes == 0 {
            return Err(DracoError::general(
                "A KD-tree attributes decoder must carry at least one attribute",
            ));
        }
        if num_attributes > in_buffer.remaining_size() / MIN_ATTRIBUTE_DESCRIPTOR_BYTES {
            return Err(DracoError::general(format!(
                "{num_attributes} attribute descriptors need at least \
                 {MIN_ATTRIBUTE_DESCRIPTOR_BYTES} bytes each, and {} remain",
                in_buffer.remaining_size()
            )));
        }

        for index in 0..num_attributes {
            let att_type_val = in_buffer.decode_u8().map_err(|_| {
                DracoError::buffer(format!("Buffer ran out reading attribute {index}'s type"))
            })?;
            let att_type = GeometryAttributeType::try_from(att_type_val).map_err(|_| {
                DracoError::general(format!(
                    "Attribute {index} names geometry attribute type {att_type_val}, which is not defined"
                ))
            })?;

            let data_type_val = in_buffer.decode_u8().map_err(|_| {
                DracoError::buffer(format!(
                    "Buffer ran out reading attribute {index}'s data type"
                ))
            })?;
            let data_type = DataType::try_from(data_type_val).map_err(|_| {
                DracoError::general(format!(
                    "Attribute {index} names data type {data_type_val}, which is not defined"
                ))
            })?;

            let num_components = in_buffer.decode_u8().map_err(|_| {
                DracoError::buffer(format!(
                    "Buffer ran out reading attribute {index}'s component count"
                ))
            })?;
            if num_components == 0 {
                return Err(DracoError::general(format!(
                    "Attribute {index} declares zero components"
                )));
            }
            let normalized = in_buffer.decode_u8().map_err(|_| {
                DracoError::buffer(format!(
                    "Buffer ran out reading attribute {index}'s normalized flag"
                ))
            })? != 0;
            let unique_id = in_buffer.decode_varint().map_err(|_| {
                DracoError::buffer(format!(
                    "Buffer ran out reading attribute {index}'s unique id"
                ))
            })? as u32;

            // Uncharged, because untaken: the point count came out of the
            // header and this decoder runs before a single point has been read,
            // so the buffer is left unreserved and sized in
            // `decode_portable_attributes`, once the KD-tree has produced the
            // values and their length has been checked against the count.
            // The caller's ceiling, unlike the budget above, counts what a
            // legitimate file decodes as well as what a hostile one claims, so
            // it is charged here where the size is first known.
            in_buffer.admit_attribute(
                point_cloud.num_points(),
                num_components as usize,
                data_type.byte_length(),
            )?;
            let mut att = PointAttribute::new();
            att.init_deferred(
                att_type,
                num_components,
                data_type,
                normalized,
                point_cloud.num_points(),
            )?;
            att.set_unique_id(unique_id);

            let att_id = point_cloud.add_attribute_preserve_unique_id(att);
            self.attribute_ids.push(att_id);
        }
        Ok(())
    }

    pub fn decode_attributes(
        &mut self,
        point_cloud: &mut PointCloud,
        in_buffer: &mut DecoderBuffer,
    ) -> Status {
        if in_buffer.bitstream_version() < 0x0203 {
            #[cfg(feature = "legacy_bitstream_decode")]
            return self.decode_pre_2_3_attributes(point_cloud, in_buffer);
            #[cfg(not(feature = "legacy_bitstream_decode"))]
            return Err(DracoError::bitstream_version_unsupported());
        }
        self.decode_portable_attributes(point_cloud, in_buffer)?;
        self.decode_data_needed_by_portable_transforms(point_cloud, in_buffer)?;
        self.transform_attributes_to_original_format(point_cloud)
    }

    fn decode_portable_attributes(
        &mut self,
        point_cloud: &mut PointCloud,
        in_buffer: &mut DecoderBuffer,
    ) -> Status {
        let num_expected_points = point_cloud.num_points();
        // Don't clear transforms/min_values here as they are decoded separately.
        self.quantized_portable_attributes.clear();
        self.attribute_specs.clear();
        self.signed_attribute_specs.clear();
        self.cached_decoded = None;

        let compression_level = in_buffer.decode_u8().map_err(|_| {
            DracoError::buffer("Buffer ran out reading the KD-tree compression level")
        })?;
        if compression_level > 6 {
            return Err(DracoError::general(format!(
                "KD-tree compression level {compression_level} outside the supported range 0..=6"
            )));
        }

        let mut total_dimensionality: usize = 0;
        let mut float_specs: Vec<(i32, usize, usize)> = Vec::new();

        for &att_id in &self.attribute_ids {
            let att = point_cloud.try_attribute(att_id)?;
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
                other => {
                    return Err(DracoError::unsupported_feature(format!(
                        "The KD-tree coder does not carry {other:?} attributes"
                    )))
                }
            }
            total_dimensionality = total_dimensionality
                .checked_add(num_components)
                .ok_or_else(|| {
                    DracoError::general("The attributes' components overflow a usize in total")
                })?;
        }
        if total_dimensionality == 0 {
            return Err(DracoError::general(
                "The KD-tree attributes have no components between them",
            ));
        }

        let total_dimensionality_u32 = u32::try_from(total_dimensionality).map_err(|_| {
            DracoError::general(format!(
                "Total dimensionality {total_dimensionality} does not fit the coder's u32"
            ))
        })?;
        let mut decoder =
            DynamicIntegerPointsKdTreeDecoder::new(compression_level, total_dimensionality_u32);
        let decoded = decoder.decode_points(in_buffer, num_expected_points as u32)?;
        if decoder.num_decoded_points() as usize != num_expected_points {
            return Err(DracoError::general(format!(
                "KD-tree produced {} points against the {num_expected_points} the header declares",
                decoder.num_decoded_points()
            )));
        }
        let expected_decoded_len = num_expected_points
            .checked_mul(total_dimensionality)
            .ok_or_else(|| {
                DracoError::general("Point count times dimensionality overflows a usize")
            })?;
        if decoded.len() != expected_decoded_len {
            return Err(DracoError::general(format!(
                "KD-tree produced {} values against the {expected_decoded_len} expected",
                decoded.len()
            )));
        }

        self.take_attribute_buffers(point_cloud, decoded.len())?;

        // Fill non-float attributes directly, and create portable attributes for float.
        for (att_id, offset, num_components) in float_specs {
            let att = point_cloud.try_attribute(att_id)?;
            let mut portable = PointAttribute::default();
            portable.try_init(
                att.attribute_type(),
                att.num_components(),
                DataType::Uint32,
                false,
                num_expected_points,
            )?;
            portable.set_identity_mapping();

            write_u32_components_from_decoded(
                &decoded,
                total_dimensionality,
                offset,
                num_components,
                num_expected_points,
                &mut portable,
                DataType::Uint32,
            )
            .map_err(|err| err.context(format!("Portable copy of attribute {att_id}")))?;

            self.quantized_portable_attributes.push(portable);
        }

        for spec in &self.attribute_specs {
            if matches!(
                spec.data_type,
                DataType::Uint32 | DataType::Uint16 | DataType::Uint8
            ) {
                let att_id = spec.att_id;
                let att = point_cloud.try_attribute_mut(att_id)?;
                write_u32_components_from_decoded(
                    &decoded,
                    total_dimensionality,
                    spec.offset,
                    spec.num_components,
                    num_expected_points,
                    att,
                    spec.data_type,
                )
                .map_err(|err| err.context(format!("Attribute {att_id}")))?;
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

        Ok(())
    }

    /// Sizes every attribute's buffer for its declared points.
    ///
    /// Taken once the KD-tree has produced the values rather than when the
    /// attributes were declared: `decoded_values` exist by then and their
    /// count has been checked against the declared one, so this is memory
    /// backed by data instead of by a header. The fills write at computed
    /// offsets and need the room in one piece, which is why the values are not
    /// pushed instead.
    fn take_attribute_buffers(
        &self,
        point_cloud: &mut PointCloud,
        decoded_values: usize,
    ) -> Status {
        for &att_id in &self.attribute_ids {
            let att = point_cloud.try_attribute_mut(att_id)?;
            let stride = usize::try_from(att.byte_stride()).map_err(|_| {
                DracoError::general(format!("Attribute {att_id} has a negative byte stride"))
            })?;
            let required = att.size().checked_mul(stride).ok_or_else(|| {
                DracoError::general(format!(
                    "Attribute {att_id}'s buffer size overflows a usize"
                ))
            })?;
            if att.buffer().data_size() < required {
                att.buffer_mut().try_resize(required).map_err(|_| {
                    DracoError::allocation_exceeds_input(required, decoded_values * 4)
                })?;
            }
        }
        Ok(())
    }

    /// The layout bitstreams before 2.3 use, which upstream still reads in
    /// `DecodeDataNeededByPortableTransforms`.
    ///
    /// The tree itself is the same; what differs is the header in front of it
    /// and what comes after. A method byte picks between two encodings: `0`
    /// for one float position quantized as a whole (`FloatPointsTreeDecoder`,
    /// with a quantization header of its own and one range for all three
    /// axes), `1` for integer attributes written as decoded. Each repeats the
    /// point count, and neither has a transform to undo afterwards. Draco 1.0
    /// to 1.2 write the first for any point cloud of positions alone.
    ///
    /// Two checks are stricter than upstream's. The repeated count must equal
    /// the geometry header's, where upstream sizes the attributes by the
    /// repeated one and leaves the cloud disagreeing with itself; and the
    /// quantized encoding needs a `Float32` attribute and at least one bit,
    /// where upstream writes floats into whatever it is given and divides by
    /// zero.
    #[cfg(feature = "legacy_bitstream_decode")]
    fn decode_pre_2_3_attributes(
        &mut self,
        point_cloud: &mut PointCloud,
        in_buffer: &mut DecoderBuffer,
    ) -> Status {
        let mut total_dimensionality = 0usize;
        for &att_id in &self.attribute_ids {
            let att = point_cloud.try_attribute(att_id)?;
            if att.data_type().byte_length() > 4 {
                return Err(DracoError::unsupported_feature(format!(
                    "Attribute {att_id} is {:?}, wider than the 32 bits a pre-2.3 KD-tree holds",
                    att.data_type()
                )));
            }
            total_dimensionality += att.num_components() as usize;
        }
        let method = in_buffer
            .decode_u8()
            .map_err(|_| DracoError::buffer("Buffer ran out reading the KD-tree method"))?;
        match method {
            0 => self.decode_pre_2_3_quantized_positions(point_cloud, in_buffer),
            1 => self.decode_pre_2_3_integers(point_cloud, in_buffer, total_dimensionality),
            other => Err(DracoError::general(format!(
                "Pre-2.3 KD-tree method {other} is neither quantized (0) nor integer (1)"
            ))),
        }
    }

    /// The point count a pre-2.3 KD-tree repeats after its header.
    #[cfg(feature = "legacy_bitstream_decode")]
    fn decode_pre_2_3_point_count(
        point_cloud: &PointCloud,
        in_buffer: &mut DecoderBuffer,
    ) -> Result<u32, DracoError> {
        let num_points = in_buffer
            .decode_u32()
            .map_err(|_| DracoError::buffer("Buffer ran out reading the KD-tree point count"))?;
        if num_points as usize != point_cloud.num_points() {
            return Err(DracoError::general(format!(
                "KD-tree header repeats {num_points} points against the {} the geometry declares",
                point_cloud.num_points()
            )));
        }
        Ok(num_points)
    }

    #[cfg(feature = "legacy_bitstream_decode")]
    fn decode_pre_2_3_integers(
        &mut self,
        point_cloud: &mut PointCloud,
        in_buffer: &mut DecoderBuffer,
        total_dimensionality: usize,
    ) -> Status {
        let compression_level = in_buffer.decode_u8().map_err(|_| {
            DracoError::buffer("Buffer ran out reading the KD-tree compression level")
        })?;
        if compression_level > 6 {
            return Err(DracoError::general(format!(
                "KD-tree compression level {compression_level} outside the supported range 0..=6"
            )));
        }
        let num_points = Self::decode_pre_2_3_point_count(point_cloud, in_buffer)?;
        let dimension = u32::try_from(total_dimensionality).map_err(|_| {
            DracoError::general(format!(
                "Total dimensionality {total_dimensionality} does not fit the coder's u32"
            ))
        })?;
        let mut decoder = DynamicIntegerPointsKdTreeDecoder::new(compression_level, dimension);
        let decoded = decoder.decode_points(in_buffer, num_points)?;
        // A tree that yields fewer points than the header repeats would leave
        // the rest of every attribute unwritten; upstream refuses it too.
        if decoder.num_decoded_points() != num_points
            || decoded.len() != num_points as usize * total_dimensionality
        {
            return Err(DracoError::general(format!(
                "KD-tree produced {} points against the {num_points} its header repeats",
                decoder.num_decoded_points()
            )));
        }
        self.take_attribute_buffers(point_cloud, decoded.len())?;

        // Each value is written as the low bytes of its 32 bits, whatever the
        // attribute's type, as upstream's output iterator copies them.
        let mut offset = 0usize;
        for &att_id in &self.attribute_ids {
            let att = point_cloud.try_attribute_mut(att_id)?;
            let num_components = att.num_components() as usize;
            let width = att.data_type().byte_length();
            let stride = att.byte_stride() as usize;
            for point in 0..num_points as usize {
                let base = att.mapped_index(PointIndex(point as u32)).0 as usize * stride;
                let row = &decoded[point * total_dimensionality + offset..][..num_components];
                for (component, value) in row.iter().enumerate() {
                    att.buffer_mut().update(
                        &value.to_le_bytes()[..width],
                        Some(base + component * width),
                    );
                }
            }
            offset += num_components;
        }
        Ok(())
    }

    #[cfg(feature = "legacy_bitstream_decode")]
    fn decode_pre_2_3_quantized_positions(
        &mut self,
        point_cloud: &mut PointCloud,
        in_buffer: &mut DecoderBuffer,
    ) -> Status {
        let att_id = match self.attribute_ids.as_slice() {
            &[att_id] => att_id,
            ids => {
                return Err(DracoError::general(format!(
                    "The quantized pre-2.3 KD-tree carries one attribute, not {}",
                    ids.len()
                )))
            }
        };
        let att = point_cloud.try_attribute(att_id)?;
        if att.num_components() != 3 || att.data_type() != DataType::Float32 {
            return Err(DracoError::general(format!(
                "The quantized pre-2.3 KD-tree carries three Float32 components, not {} {:?}",
                att.num_components(),
                att.data_type()
            )));
        }
        // Upstream reads this level and never uses it; the quantization header
        // below carries the one the tree was written at.
        in_buffer.decode_u8().map_err(|_| {
            DracoError::buffer("Buffer ran out reading the KD-tree compression level")
        })?;
        let num_points = Self::decode_pre_2_3_point_count(point_cloud, in_buffer)?;

        let read_u32 = |in_buffer: &mut DecoderBuffer, what: &str| {
            in_buffer
                .decode_u32()
                .map_err(|_| DracoError::buffer(format!("Buffer ran out reading the {what}")))
        };
        match read_u32(in_buffer, "quantized KD-tree version")? {
            3 => {
                let method = in_buffer.decode_u8().map_err(|_| {
                    DracoError::buffer("Buffer ran out reading the quantized KD-tree method")
                })?;
                // `KDTREE` in upstream's `PointCloudCompressionMethod`, the
                // only method there is.
                if method != 1 {
                    return Err(DracoError::general(format!(
                        "Quantized KD-tree method {method} is not the KD-tree (1)"
                    )));
                }
            }
            // Version 2 has no method field: the KD-tree was all there was.
            2 => {}
            other => {
                return Err(DracoError::general(format!(
                    "Quantized KD-tree version {other} is neither 2 nor 3"
                )))
            }
        }
        let bits = read_u32(in_buffer, "quantized KD-tree's quantization bits")?;
        if bits == 0 || bits > 31 {
            return Err(DracoError::general(format!(
                "Quantized KD-tree declares {bits} quantization bits, outside 1..=31"
            )));
        }
        let range = in_buffer.decode_f32().map_err(|_| {
            DracoError::buffer("Buffer ran out reading the quantized KD-tree range")
        })?;
        let tree_points = read_u32(in_buffer, "quantized KD-tree's point count")?;
        if tree_points != num_points {
            return Err(DracoError::general(format!(
                "Quantized KD-tree declares {tree_points} points against the {num_points} before it"
            )));
        }
        let compression_level = read_u32(in_buffer, "quantized KD-tree's compression level")?;
        if compression_level > 6 {
            return Err(DracoError::general(format!(
                "KD-tree compression level {compression_level} outside the supported range 0..=6"
            )));
        }

        let decoded = if num_points == 0 {
            Vec::new()
        } else {
            let mut decoder = DynamicIntegerPointsKdTreeDecoder::new(compression_level as u8, 3);
            let decoded = decoder.decode_points(in_buffer, num_points)?;
            if decoder.num_decoded_points() != num_points
                || decoded.len() != num_points as usize * 3
            {
                return Err(DracoError::general(format!(
                    "KD-tree produced {} points against the {num_points} its header declares",
                    decoder.num_decoded_points()
                )));
            }
            decoded
        };
        self.take_attribute_buffers(point_cloud, decoded.len())?;

        // Upstream's `DequantizePoints3`: the values are offset by the largest
        // quantized value to make them unsigned, and share one step for all
        // three axes. The subtraction wraps in `uint32_t` and is then read as
        // `int32_t`, which the wrapping cast below reproduces.
        let max_quantized_value = (1u32 << bits) - 1;
        let delta = range / max_quantized_value as f32;
        let att = point_cloud.try_attribute_mut(att_id)?;
        let stride = att.byte_stride() as usize;
        for (point, row) in decoded.as_chunks::<3>().0.iter().enumerate() {
            let base = att.mapped_index(PointIndex(point as u32)).0 as usize * stride;
            for (component, &value) in row.iter().enumerate() {
                let coordinate = value.wrapping_sub(max_quantized_value) as i32 as f32 * delta;
                att.buffer_mut()
                    .update(&coordinate.to_le_bytes(), Some(base + component * 4));
            }
        }
        Ok(())
    }

    pub fn decode_data_needed_by_portable_transforms(
        &mut self,
        point_cloud: &mut PointCloud,
        in_buffer: &mut DecoderBuffer,
    ) -> Status {
        // Float quantization parameters in attribute order.
        for &att_id in &self.attribute_ids {
            let att = point_cloud.try_attribute(att_id)?;
            if att.data_type() == DataType::Float32 {
                let mut min_values = vec![0.0f32; att.num_components() as usize];
                for (component, value) in min_values.iter_mut().enumerate() {
                    *value = in_buffer.decode::<f32>().map_err(|_| {
                        DracoError::buffer(format!(
                            "Buffer ran out reading minimum {component} of attribute {att_id}"
                        ))
                    })?;
                }
                let range = in_buffer.decode::<f32>().map_err(|_| {
                    DracoError::buffer(format!(
                        "Buffer ran out reading the quantization range of attribute {att_id}"
                    ))
                })?;
                let bits = in_buffer.decode_u8().map_err(|_| {
                    DracoError::buffer(format!(
                        "Buffer ran out reading the quantization bits of attribute {att_id}"
                    ))
                })?;
                if bits > 31 {
                    return Err(DracoError::general(format!(
                        "Attribute {att_id} declares {bits} quantization bits, above the 31 a u32 holds"
                    )));
                }
                let mut t = AttributeQuantizationTransform::new();
                t.set_parameters(bits as i32, &min_values, range)?;
                self.attribute_quantization_transforms.push(t);
            }
        }

        // Signed min values.
        for i in 0..self.min_signed_values.len() {
            self.min_signed_values[i] = in_buffer.decode_varint_signed_i32().map_err(|_| {
                DracoError::buffer(format!(
                    "Buffer ran out reading signed minimum {i} of {}",
                    self.min_signed_values.len()
                ))
            })?;
        }

        Ok(())
    }

    pub fn transform_attributes_to_original_format(
        &mut self,
        point_cloud: &mut PointCloud,
    ) -> Status {
        let cached = self.cached_decoded.take().ok_or_else(|| {
            DracoError::general(
                "No decoded KD-tree points to transform: the portable decode did not run",
            )
        })?;

        // Floats.
        let mut float_attr_index = 0usize;
        for &att_id in &self.attribute_ids {
            let attribute = point_cloud.try_attribute(att_id)?;
            let dt = attribute.data_type();
            if dt == DataType::Float32 {
                let portable = self
                    .quantized_portable_attributes
                    .get(float_attr_index)
                    .ok_or_else(|| {
                        DracoError::general(format!(
                            "Attribute {att_id} has no portable copy at index {float_attr_index}"
                        ))
                    })?;
                let transform = self
                    .attribute_quantization_transforms
                    .get(float_attr_index)
                    .ok_or_else(|| {
                        DracoError::general(format!(
                            "Attribute {att_id} has no quantization parameters at index {float_attr_index}"
                        ))
                    })?;

                let target = point_cloud.try_attribute_mut(att_id)?;
                transform.inverse_transform_attribute(portable, target)?;

                float_attr_index += 1;
            }
        }

        // Signed ints.
        let mut min_index = 0usize;
        for spec in &self.signed_attribute_specs {
            let att = point_cloud.try_attribute_mut(spec.att_id)?;
            let num_points = att.size();
            if num_points == 0 {
                continue;
            }

            let stride = att.byte_stride() as usize;
            let component_size = att.data_type().byte_length();

            for p in 0..num_points {
                let avi = att.mapped_index(PointIndex(p as u32));
                let base = (avi.0 as usize).checked_mul(stride).ok_or_else(|| {
                    DracoError::general("A signed attribute's value offset overflows a usize")
                })?;
                for c in 0..spec.num_components {
                    let decoded_index = p
                        .checked_mul(cached.total_dimensionality)
                        .and_then(|v| v.checked_add(spec.offset))
                        .and_then(|v| v.checked_add(c))
                        .ok_or_else(|| {
                            DracoError::general("A decoded value index overflows a usize")
                        })?;
                    let &unsigned = cached.decoded.get(decoded_index).ok_or_else(|| {
                        DracoError::general(format!(
                            "Point {p} component {c} reads past the {} decoded values",
                            cached.decoded.len()
                        ))
                    })?;
                    let &min_value =
                        self.min_signed_values.get(min_index + c).ok_or_else(|| {
                            DracoError::general(format!(
                                "No signed minimum at index {} of {}",
                                min_index + c,
                                self.min_signed_values.len()
                            ))
                        })?;
                    let signed = unsigned as i64 + min_value as i64;
                    let component_offset = c
                        .checked_mul(component_size)
                        .and_then(|delta| base.checked_add(delta))
                        .ok_or_else(|| {
                            DracoError::general("A component offset overflows a usize")
                        })?;
                    write_signed_component(
                        att.buffer_mut(),
                        component_offset,
                        spec.data_type,
                        signed,
                    )
                    .map_err(|err| err.context(format!("Point {p} component {c}")))?;
                }
            }
            min_index += spec.num_components;
        }

        Ok(())
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
) -> Status {
    let stride = target_attribute.byte_stride() as usize;
    let component_size = target_type.byte_length();
    // One point's worth of destination bytes. Bounding the row up front lets the
    // components below be written straight into a slice of exactly that size:
    // the per-component form checked and copied each scalar separately, which on
    // a kd-tree point cloud is a memcpy call per component per point and was 19%
    // of decode.
    let row_bytes = num_components
        .checked_mul(component_size)
        .ok_or_else(|| DracoError::general("Attribute row size overflows a usize".to_string()))?;

    for p in 0..num_points {
        let avi = target_attribute.mapped_index(PointIndex(p as u32));
        let base = (avi.0 as usize).checked_mul(stride).ok_or_else(|| {
            DracoError::general(format!("Point {p}'s value offset overflows a usize"))
        })?;
        let row_end = base
            .checked_add(row_bytes)
            .ok_or_else(|| DracoError::general(format!("Point {p}'s row end overflows a usize")))?;

        let src_start = p
            .checked_mul(total_dimensionality)
            .and_then(|v| v.checked_add(offset))
            .ok_or_else(|| {
                DracoError::general(format!("Point {p} indexes past what a usize holds"))
            })?;
        let src_end = src_start.checked_add(num_components).ok_or_else(|| {
            DracoError::general(format!("Point {p}'s component run overflows a usize"))
        })?;
        let src = decoded.get(src_start..src_end).ok_or_else(|| {
            DracoError::general(format!(
                "Point {p} reads {src_start}..{src_end} of {} decoded values",
                decoded.len()
            ))
        })?;

        let dst = target_attribute
            .buffer_mut()
            .data_mut()
            .get_mut(base..row_end)
            .ok_or_else(|| {
                DracoError::general(format!(
                    "Point {p} writes {base}..{row_end} past the attribute buffer"
                ))
            })?;

        match target_type {
            DataType::Uint8 => {
                for (d, &v) in dst.iter_mut().zip(src) {
                    *d = v as u8;
                }
            }
            DataType::Uint16 => {
                for (d, &v) in dst.as_chunks_mut::<2>().0.iter_mut().zip(src) {
                    d.copy_from_slice(&(v as u16).to_le_bytes());
                }
            }
            DataType::Uint32 => {
                for (d, &v) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src) {
                    d.copy_from_slice(&v.to_le_bytes());
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn write_signed_component(
    buffer: &mut crate::data_buffer::DataBuffer,
    offset: usize,
    data_type: DataType,
    value: i64,
) -> Status {
    let written = match data_type {
        DataType::Int8 => buffer.try_write(offset, &[(value as i8) as u8]),
        DataType::Int16 => buffer.try_write(offset, &(value as i16).to_le_bytes()),
        DataType::Int32 => buffer.try_write(offset, &(value as i32).to_le_bytes()),
        _ => true,
    };
    if written {
        Ok(())
    } else {
        Err(DracoError::buffer(format!(
            "Write of {data_type:?} at byte {offset} lands past the attribute buffer's {} bytes",
            buffer.data_size()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{write_u32_components_from_decoded, CachedDecoded, KdTreeAttributesDecoder};
    use crate::decoder_buffer::DecoderBuffer;
    use crate::draco_types::DataType;
    use crate::geometry_attribute::{GeometryAttributeType, PointAttribute};
    use crate::point_cloud::PointCloud;

    #[test]
    fn kd_tree_component_write_rejects_out_of_bounds_buffer() {
        // An attribute whose buffer is shorter than the row it is asked to
        // write: the writer bounds a whole point's row at once, so this is
        // where a short buffer has to be refused rather than panicked on.
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            3,
            DataType::Uint32,
            false,
            1,
        );
        attribute.buffer_mut().resize(4);

        let err = write_u32_components_from_decoded(
            &[1, 2, 3],
            3,
            0,
            3,
            1,
            &mut attribute,
            DataType::Uint32,
        )
        .unwrap_err();
        assert!(err.message().contains("past the attribute buffer"), "{err}");
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

        let err = write_u32_components_from_decoded(
            &[1, 2],
            3,
            0,
            3,
            1,
            &mut attribute,
            DataType::Uint32,
        )
        .unwrap_err();
        assert!(err.message().contains("2 decoded values"), "{err}");
    }

    #[test]
    fn kd_tree_portable_decode_rejects_invalid_attribute_id() {
        let mut decoder = KdTreeAttributesDecoder::new(-1);
        let mut point_cloud = PointCloud::new();
        let bytes = [0u8];
        let mut buffer = DecoderBuffer::new(&bytes);

        assert!(decoder
            .decode_portable_attributes(&mut point_cloud, &mut buffer)
            .is_err());
    }

    #[test]
    fn kd_tree_transform_data_rejects_invalid_attribute_id() {
        let mut decoder = KdTreeAttributesDecoder::new(-1);
        let mut point_cloud = PointCloud::new();
        let bytes = [];
        let mut buffer = DecoderBuffer::new(&bytes);

        assert!(decoder
            .decode_data_needed_by_portable_transforms(&mut point_cloud, &mut buffer)
            .is_err());
    }

    #[test]
    fn kd_tree_original_transform_rejects_invalid_attribute_id() {
        let mut decoder = KdTreeAttributesDecoder::new(-1);
        decoder.cached_decoded = Some(CachedDecoded {
            decoded: Vec::new(),
            total_dimensionality: 1,
        });
        let mut point_cloud = PointCloud::new();

        assert!(decoder
            .transform_attributes_to_original_format(&mut point_cloud)
            .is_err());
    }
}
