use crate::compression_config::EncodedGeometryType;
use crate::draco_types::DataType;
use crate::encoder_buffer::EncoderBuffer;
use crate::encoder_options::EncoderOptions;
use crate::geometry_attribute::PointAttribute;
use crate::geometry_indices::PointIndex;
use crate::kd_tree_attributes_encoder::KdTreeAttributesEncoder;
use crate::mesh::Mesh;
use crate::mesh_encoder::EncodedAttributeInfo;
use crate::metadata::METADATA_FLAG_MASK;
use crate::point_cloud::PointCloud;
use crate::sequential_attribute_encoder::{
    select_sequential_encoder, SequentialAttributeEncoderType,
};
use crate::sequential_integer_attribute_encoder::SequentialIntegerAttributeEncoder;
use crate::sequential_normal_attribute_encoder::SequentialNormalAttributeEncoder;
use crate::status::{DracoError, Status};
use crate::version::{
    has_header_flags, uses_varint_encoding, uses_varint_unique_id, DEFAULT_POINT_CLOUD_VERSION,
};

use crate::corner_table::CornerTable;

/// Rejects attributes no encoder can represent, before any of them tries.
///
/// An attribute is a typed array, and both halves of its element type come from
/// the caller: a component count and a scalar type. Neither is validated where
/// it is set, because `PointAttribute::init` is a data-model API that mirrors
/// C++ Draco and stores what it is given. So a geometry assembled from a file
/// some other library parsed can reach the encoder with a zero-component or
/// untyped attribute, and every encoder path then derives a stride, a
/// dimension, or an axis count from it. The KD-tree coder takes the position
/// attribute's component count as its dimension and indexes a per-axis array
/// with it, which for zero components is an empty array indexed at 0.
///
/// This is the one place both encoders can share, so the refusal is stated once
/// here rather than defended at each derivation.
pub(crate) fn validate_encodable_attributes(point_cloud: &PointCloud) -> Status {
    for att_id in 0..point_cloud.num_attributes() {
        let attribute = point_cloud.attribute(att_id);
        if attribute.num_components() == 0 {
            return Err(DracoError::DracoError(format!(
                "Attribute {att_id} has zero components and cannot be encoded"
            )));
        }
        if attribute.data_type() == DataType::Invalid {
            return Err(DracoError::DracoError(format!(
                "Attribute {att_id} has an invalid data type and cannot be encoded"
            )));
        }

        // Every point must land on a value the attribute actually holds. The
        // encoders read attribute data by mapped index without re-checking it,
        // which is correct for geometry they built themselves and wrong for
        // geometry handed in: an identity-mapped attribute shorter than the
        // point count, or an explicit map with an entry past the value array
        // (including the invalid index a fresh map is filled with), otherwise
        // reads past the value buffer.
        //
        // Identity mapping is the common case and answers in one comparison,
        // since point i reads value i. Only an explicit map has to be walked,
        // and that walk is the same order as the encode that follows it.
        let num_values = attribute.size();
        let num_points = point_cloud.num_points();
        if attribute.is_mapping_identity() {
            if num_points > num_values {
                return Err(DracoError::DracoError(format!(
                    "Attribute {att_id} holds {num_values} values for {num_points} points"
                )));
            }
        } else {
            for point in 0..num_points {
                let value = attribute.mapped_index(PointIndex(point as u32));
                if (value.0 as usize) >= num_values {
                    return Err(DracoError::DracoError(format!(
                        "Attribute {att_id} maps point {point} to value {} but holds \
                         {num_values} values",
                        value.0
                    )));
                }
            }
        }

        validate_attribute_storage(att_id, attribute)?;
    }
    Ok(())
}

/// Rejects an attribute whose value buffer cannot hold the values it reports.
///
/// The mapping check above answers "is this value index one of ours"; this one
/// answers "is that value actually in the buffer". They are different
/// questions, because both the element size and the buffer length are settable
/// after the fact: `PointAttribute::buffer_mut` hands out a `DataBuffer` whose
/// `resize` is public, and `set_num_components` / `set_data_type` change the
/// element size without recomputing the separately stored `byte_stride`. A
/// loader that truncates the buffer, or that widens the component count after
/// `init`, produces an attribute that satisfies every other rule here and still
/// overruns its storage.
///
/// The overrun lands in `DataBuffer::read`, which slices unchecked; each
/// encoder path reaches it through its own reader, so guarding it at each
/// reader would mean finding them all and finding each new one. The
/// quantization transform does bounds-check its own reads, but only float
/// attributes enter it - integer attributes go straight to the sequential and
/// KD-tree readers. One statement of the requirement here covers every reader,
/// present and future.
fn validate_attribute_storage(att_id: i32, attribute: &PointAttribute) -> Status {
    let num_values = attribute.size();
    if num_values == 0 {
        return Ok(());
    }

    let component_size = attribute.data_type().byte_length();
    let element_size = (attribute.num_components() as usize).saturating_mul(component_size);
    let byte_stride = attribute.byte_stride().max(0) as usize;
    if byte_stride < element_size {
        return Err(DracoError::DracoError(format!(
            "Attribute {att_id} declares a {byte_stride}-byte stride for {element_size}-byte \
             values"
        )));
    }

    // The last value starts at `(num_values - 1) * byte_stride` and is
    // `element_size` long, so the buffer needs that much and no more: a
    // trailing gap the stride would imply is never read.
    let required = (num_values - 1)
        .checked_mul(byte_stride)
        .and_then(|last_offset| last_offset.checked_add(element_size))
        .ok_or_else(|| {
            DracoError::DracoError(format!("Attribute {att_id} value extent overflows"))
        })?;
    let available = attribute.buffer().data_size();
    if available < required {
        return Err(DracoError::DracoError(format!(
            "Attribute {att_id} needs {required} bytes for {num_values} values but its buffer \
             holds {available}"
        )));
    }
    Ok(())
}

/// Picks sequential or KD-tree encoding, as C++ `ExpertEncoder::EncodeToBuffer`
/// does for a point cloud.
///
/// The default matters: with no explicit method and the default speed of 5, a
/// point cloud whose attributes are all eligible is encoded with the **KD-tree**
/// method, not the sequential one. Defaulting to sequential produces a different
/// method byte and an entirely different payload from the reference encoder for
/// the same input.
///
/// Note the asymmetry upstream has and this keeps: the `speed == 10` shortcut is
/// guarded on the method being unset, so an explicitly requested KD-tree encode
/// still takes that path at speed 10.
fn select_encoding_method(
    point_cloud: &PointCloud,
    options: &EncoderOptions,
) -> Result<i32, DracoError> {
    const SEQUENTIAL: i32 = 0;
    const KD_TREE: i32 = 1;

    let requested = options.get_encoding_method();
    if requested == Some(SEQUENTIAL) {
        return Ok(SEQUENTIAL);
    }
    if requested.is_none() && options.get_speed() == 10 {
        return Ok(SEQUENTIAL);
    }

    // Every attribute must be an integer type, or a float that something has
    // asked to quantize -- the KD-tree coder works on integers alone.
    let mut kd_tree_possible = true;
    for att_id in 0..point_cloud.num_attributes() {
        let attribute = point_cloud.attribute(att_id);
        let data_type = attribute.data_type();
        if !matches!(
            data_type,
            DataType::Float32
                | DataType::Uint32
                | DataType::Uint16
                | DataType::Uint8
                | DataType::Int32
                | DataType::Int16
                | DataType::Int8
        ) {
            kd_tree_possible = false;
        }
        if kd_tree_possible
            && data_type == DataType::Float32
            && options.get_attribute_int(att_id, "quantization_bits", -1) <= 0
        {
            kd_tree_possible = false; // Quantization not enabled.
        }
        if !kd_tree_possible {
            break;
        }
    }

    if kd_tree_possible {
        return Ok(KD_TREE);
    }
    if requested == Some(KD_TREE) {
        return Err(DracoError::DracoError(
            "Invalid encoding method.".to_string(),
        ));
    }
    Ok(SEQUENTIAL)
}

/// Geometry context used by attribute encoders and prediction selection.
pub trait GeometryEncoder {
    /// Returns point-cloud geometry when available.
    fn point_cloud(&self) -> Option<&PointCloud>;
    /// Returns mesh geometry when available.
    fn mesh(&self) -> Option<&Mesh>;
    /// Returns mesh corner-table topology when available.
    fn corner_table(&self) -> Option<&CornerTable>;
    /// Returns the active encoder options.
    fn options(&self) -> &EncoderOptions;
    /// Returns the encoded geometry type.
    fn get_geometry_type(&self) -> EncodedGeometryType;
    /// Returns the forced encoding method, if one is active.
    fn get_encoding_method(&self) -> Option<i32> {
        None
    }
    /// Returns a data-to-corner map for mesh attribute prediction, if present.
    fn get_data_to_corner_map(&self) -> Option<&[u32]> {
        None
    }
    /// Returns a vertex-to-data map for mesh attribute prediction, if present.
    fn get_vertex_to_data_map(&self) -> Option<&[i32]> {
        None
    }
    /// Returns the portable (quantized) form of an attribute, once the encoder
    /// has transformed it. Prediction schemes that read a parent attribute --
    /// tex coords and geometric normals both predict from the position -- must
    /// use this and not the original floats, because the decoder only ever has
    /// the portable values to predict from. Counterpart of C++
    /// `PointCloudEncoder::GetPortableAttribute`.
    fn get_portable_attribute(
        &self,
        _att_id: i32,
    ) -> Option<&crate::geometry_attribute::PointAttribute> {
        None
    }
}

/// Encoder for Draco point cloud bitstreams.
///
/// A `PointCloudEncoder` takes a [`PointCloud`] plus [`EncoderOptions`] and
/// writes a `.drc` bitstream into an [`EncoderBuffer`]. Depending on the options it uses
/// either KD-tree or sequential attribute encoding, matching C++ Draco's
/// `PointCloudEncoder` selection.
///
/// # Examples
///
/// ```
/// use draco_core::{
///     DataType, DecoderBuffer, EncoderBuffer, EncoderOptions, GeometryAttributeType,
///     PointAttribute, PointCloud, PointCloudDecoder, PointCloudEncoder,
/// };
///
/// // Three points with float32 positions.
/// let mut pc = PointCloud::new();
/// let mut position = PointAttribute::new();
/// position.init(GeometryAttributeType::Position, 3, DataType::Float32, false, 3);
/// let coords: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
/// for (i, value) in coords.iter().enumerate() {
///     position.buffer_mut().write(i * 4, &value.to_le_bytes());
/// }
/// pc.add_attribute(position);
///
/// // Encode, then decode it back.
/// let mut encoder = PointCloudEncoder::new();
/// encoder.set_point_cloud(pc);
/// let mut buffer = EncoderBuffer::new();
/// encoder.encode(&EncoderOptions::new(), &mut buffer)?;
///
/// let mut decoded = PointCloud::new();
/// PointCloudDecoder::new().decode(&mut DecoderBuffer::new(buffer.data()), &mut decoded)?;
/// assert_eq!(decoded.num_points(), 3);
/// # Ok::<(), draco_core::DracoError>(())
/// ```
pub struct PointCloudEncoder {
    point_cloud: Option<PointCloud>,
    options: EncoderOptions,
    encoded_point_cloud_info: Option<EncodedPointCloudInfo>,
}

/// Encoder choices and attribute metadata from a successful point-cloud encode.
///
/// The counterpart of [`EncodedMeshInfo`], and it exists for the same reason:
/// the encoder picks the KD-tree coder over the sequential one whenever every
/// attribute is eligible, and a caller had no way to find out which one ran.
///
/// [`EncodedMeshInfo`]: crate::mesh_encoder::EncodedMeshInfo
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EncodedPointCloudInfo {
    /// Numeric Draco point-cloud encoding method used: 0 sequential, 1 KD-tree.
    pub encoding_method: i32,
    /// Bitstream version written, after the default was substituted for an
    /// unset one.
    pub bitstream_version: (u8, u8),
    /// Speed the choices above were made at, after `encoding_speed` and
    /// `decoding_speed` were resolved into one value.
    pub speed: i32,
    /// Number of points encoded into the bitstream.
    pub num_encoded_points: usize,
    /// Per-attribute information captured during encoding. Empty for the
    /// KD-tree method, which encodes every attribute through one coder with no
    /// per-attribute choice to report.
    pub attributes: Vec<EncodedAttributeInfo>,
}

impl GeometryEncoder for PointCloudEncoder {
    fn point_cloud(&self) -> Option<&PointCloud> {
        self.point_cloud.as_ref()
    }

    fn mesh(&self) -> Option<&Mesh> {
        None
    }

    fn corner_table(&self) -> Option<&CornerTable> {
        None
    }

    fn options(&self) -> &EncoderOptions {
        &self.options
    }

    fn get_geometry_type(&self) -> EncodedGeometryType {
        EncodedGeometryType::PointCloud
    }
}

impl Default for PointCloudEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl PointCloudEncoder {
    /// Creates an encoder without an assigned point cloud.
    pub fn new() -> Self {
        Self {
            point_cloud: None,
            options: EncoderOptions::default(),
            encoded_point_cloud_info: None,
        }
    }

    /// Returns the point cloud assigned to this encoder, if any.
    pub fn point_cloud(&self) -> Option<&PointCloud> {
        self.point_cloud.as_ref()
    }

    /// Returns what the last successful encode chose, if one has run.
    pub fn encoded_point_cloud_info(&self) -> Option<&EncodedPointCloudInfo> {
        self.encoded_point_cloud_info.as_ref()
    }

    /// Assigns the point cloud to encode.
    pub fn set_point_cloud(&mut self, pc: PointCloud) {
        self.point_cloud = Some(pc);
    }

    /// Encodes the assigned point cloud into an output buffer.
    ///
    /// A point cloud must have been provided with
    /// [`set_point_cloud`](PointCloudEncoder::set_point_cloud) first.
    ///
    /// # Errors
    ///
    /// Returns an error if no point cloud was set, the options are
    /// unsupported, or attribute encoding fails.
    pub fn encode(&mut self, options: &EncoderOptions, out_buffer: &mut EncoderBuffer) -> Status {
        self.options = options.clone();
        self.encoded_point_cloud_info = None;

        if self.point_cloud.is_none() {
            return Err(DracoError::DracoError("Point cloud not set".to_string()));
        }
        let pc = self.point_cloud.as_ref().unwrap();
        validate_encodable_attributes(pc)?;
        let method = select_encoding_method(pc, &self.options)?;
        let (major, minor) = self.options.get_version();
        let target = if method == 1 {
            crate::version::EncodeTarget::PointCloudKdTree
        } else {
            crate::version::EncodeTarget::PointCloudSequential
        };
        crate::version::validate_encodable_version(major, minor, target)?;

        let attributes = self.encode_geometry(out_buffer, method)?;

        let (mut major, mut minor) = self.options.get_version();
        if major == 0 && minor == 0 {
            (major, minor) = DEFAULT_POINT_CLOUD_VERSION;
        }
        self.encoded_point_cloud_info = Some(EncodedPointCloudInfo {
            encoding_method: method,
            bitstream_version: (major, minor),
            speed: self.options.get_speed(),
            num_encoded_points: self
                .point_cloud
                .as_ref()
                .expect("point cloud set")
                .num_points(),
            attributes,
        });
        Ok(())
    }

    /// The encode itself, split out so [`encode`](Self::encode) has one place
    /// to record what it chose. Returns the per-attribute report, empty for the
    /// KD-tree method and for a cloud with no attributes.
    fn encode_geometry(
        &mut self,
        out_buffer: &mut EncoderBuffer,
        method: i32,
    ) -> Result<Vec<EncodedAttributeInfo>, DracoError> {
        let pc = self.point_cloud.as_ref().expect("point cloud set");

        // 1. Encode Header
        self.encode_header(out_buffer, method)?;
        self.encode_metadata(out_buffer)?;

        if method == 1 {
            // KD-Tree Encoding (Draco v2.3)

            // Encode Geometry Data (Num points)
            // Note: Draco point cloud encodes num_points as fixed u32 for both
            // sequential and KD-tree, NOT as varint (matching decoder).
            out_buffer.encode_u32(pc.num_points() as u32);

            // No attributes, no encoder. Upstream calls
            // GenerateAttributesEncoder once per attribute, so a cloud without
            // any never creates one and writes a count of zero. Building one
            // regardless would seed it with attribute id 0 and index a point
            // cloud that has none.
            if pc.num_attributes() == 0 {
                out_buffer.encode_u8(0);
                return Ok(Vec::new());
            }

            // Generate Attributes Encoders
            // For now, we put all attributes into a single KdTreeAttributesEncoder
            let mut att_encoder = KdTreeAttributesEncoder::new(0);
            for i in 1..pc.num_attributes() {
                att_encoder.add_attribute_id(i);
            }

            // Encode number of attribute encoders
            out_buffer.encode_u8(1); // We have only 1 encoder

            // Init (Transform attributes to portable format)
            if !att_encoder.transform_attributes_to_portable_format(pc, &self.options) {
                return Err(DracoError::DracoError(
                    "Failed to transform attributes".to_string(),
                ));
            }

            // Note: KD-tree encoding does NOT write an encoder type identifier byte.
            // This is different from sequential encoding where each attribute has a decoder type.
            // The decoder knows to use KdTreeAttributesDecoder because the encoding method
            // in the header is 1 (KD-tree).

            // Encode Attributes Encoder Data (Metadata)
            if !att_encoder.encode_attributes_encoder_data(pc, out_buffer) {
                return Err(DracoError::DracoError(
                    "Failed to encode attribute metadata".to_string(),
                ));
            }

            // Encode Attributes (Portable Data)
            if !att_encoder.encode_attributes(pc, &self.options, out_buffer) {
                return Err(DracoError::DracoError(
                    "Failed to encode attributes".to_string(),
                ));
            }

            // Encode Attributes Transform Data
            if !att_encoder.encode_data_needed_by_portable_transforms(out_buffer) {
                return Err(DracoError::DracoError(
                    "Failed to encode attribute transform data".to_string(),
                ));
            }
        } else {
            // Sequential Encoding (Draco v1.3)
            //
            // C++ Structure:
            // 1. num_points (u32)
            // 2. num_attribute_encoders (u8)
            // 3. For each encoder: encoder_identifier (none for sequential - skipped in v1.3)
            // 4. For each encoder: EncodeAttributesEncoderData
            //    - num_attributes_in_encoder (varint for v2+, u32 for v1.x)
            //    - for each attribute: type, data_type, num_components, normalized, unique_id
            // 5. For each attribute: decoder_type (u8)
            // 6. For each attribute: encoded data

            let num_points = pc.num_points();
            let num_attributes = pc.num_attributes();
            let point_ids: Vec<PointIndex> =
                (0..num_points).map(|i| PointIndex(i as u32)).collect();

            // Draco bitstream < 2.0 encodes number of points as a fixed u32.
            out_buffer.encode_u32(num_points as u32);

            // Number of attribute encoders
            // For empty point clouds (0 attributes), we write 0 encoders
            if num_attributes == 0 {
                out_buffer.encode_u8(0);
                return Ok(Vec::new());
            }

            // For non-empty point clouds, use 1 encoder for all attributes
            out_buffer.encode_u8(1);

            // Encode attributes encoder data:
            // Use the buffer's version (set in encode_header) for version checks
            let major = out_buffer.version_major();
            let minor = out_buffer.version_minor();
            if !uses_varint_encoding(major, minor) {
                out_buffer.encode_u32(num_attributes as u32);
            } else {
                out_buffer.encode_varint(num_attributes as u64);
            }

            // For each attribute, encode metadata
            for i in 0..num_attributes {
                let att = pc.attribute(i);
                out_buffer.encode_u8(att.attribute_type() as u8);
                out_buffer.encode_u8(att.data_type() as u8);
                out_buffer.encode_u8(att.num_components());
                out_buffer.encode_u8(if att.normalized() { 1 } else { 0 });

                if !uses_varint_unique_id(major, minor) {
                    out_buffer.encode_u16(att.unique_id() as u16);
                } else {
                    out_buffer.encode_varint(att.unique_id() as u64);
                }
            }

            // One identifier byte per attribute, naming the encoder that writes
            // it. Picked once here and dispatched on below, so the byte cannot
            // disagree with the encoder that actually runs.
            let encoder_types: Vec<SequentialAttributeEncoderType> = (0..num_attributes)
                .map(|i| {
                    let quantization_bits =
                        self.options.get_attribute_int(i, "quantization_bits", -1);
                    select_sequential_encoder(pc.attribute(i), quantization_bits)
                })
                .collect();
            for &encoder_type in &encoder_types {
                out_buffer.encode_u8(encoder_type as u8);
            }

            // Encoding follows C++ order:
            // 1. EncodePortableAttributes (encode_values for each attribute)
            // 2. EncodeDataNeededByPortableTransforms (transform params for each attribute)

            // Store encoders so we can call encode_data_needed_by_portable_transform later
            let mut integer_encoders: Vec<Option<SequentialIntegerAttributeEncoder>> =
                Vec::with_capacity(num_attributes as usize);
            let mut normal_encoders: Vec<Option<SequentialNormalAttributeEncoder>> =
                Vec::with_capacity(num_attributes as usize);

            // First pass: encode all values
            for i in 0..num_attributes {
                let att = pc.attribute(i);

                match encoder_types[i as usize] {
                    SequentialAttributeEncoderType::Normals => {
                        let mut att_encoder = SequentialNormalAttributeEncoder::new();
                        if !att_encoder.init(pc, i, &self.options) {
                            return Err(DracoError::DracoError(format!(
                                "Failed to init normal attribute encoder {}",
                                i
                            )));
                        }

                        if !att_encoder.encode_values(
                            pc,
                            &point_ids,
                            out_buffer,
                            &self.options,
                            self,
                        ) {
                            return Err(DracoError::DracoError(format!(
                                "Failed to encode attribute {}",
                                i
                            )));
                        }

                        integer_encoders.push(None);
                        normal_encoders.push(Some(att_encoder));
                        continue;
                    }
                    SequentialAttributeEncoderType::Quantization
                    | SequentialAttributeEncoderType::Integer => {
                        let mut att_encoder = SequentialIntegerAttributeEncoder::new();
                        att_encoder.init(i);

                        if !att_encoder.encode_values(
                            pc,
                            &point_ids,
                            out_buffer,
                            &self.options,
                            self,
                            None,
                            false,
                        ) {
                            return Err(DracoError::DracoError(format!(
                                "Failed to encode attribute {}",
                                i
                            )));
                        }

                        integer_encoders.push(Some(att_encoder));
                    }
                    SequentialAttributeEncoderType::Generic => {
                        let entry_size = att.byte_stride() as usize;
                        let data = att.buffer().data();
                        for &point_id in &point_ids {
                            let value_index = att.mapped_index(point_id).0 as usize;
                            let offset = value_index.checked_mul(entry_size).ok_or_else(|| {
                                DracoError::DracoError(
                                    "Point cloud raw attribute offset overflow".to_string(),
                                )
                            })?;
                            let end = offset.checked_add(entry_size).ok_or_else(|| {
                                DracoError::DracoError(
                                    "Point cloud raw attribute byte range overflow".to_string(),
                                )
                            })?;
                            if end > data.len() {
                                return Err(DracoError::DracoError(
                                    "Point cloud raw attribute data out of bounds".to_string(),
                                ));
                            }
                            out_buffer.encode_data(&data[offset..end]);
                        }

                        integer_encoders.push(None);
                    }
                }

                normal_encoders.push(None);
            }

            // Second pass: encode transform parameters (EncodeDataNeededByPortableTransforms)
            for i in 0..num_attributes as usize {
                if encoder_types[i] == SequentialAttributeEncoderType::Normals {
                    if let Some(ref att_encoder) = normal_encoders[i] {
                        let (major, minor) = self.options.get_version();
                        let bitstream_version = crate::version::bitstream_version(major, minor);
                        if bitstream_version != 0 && bitstream_version < 0x0102 {
                            continue;
                        }
                        if !att_encoder.encode_data_needed_by_portable_transform(out_buffer) {
                            return Err(DracoError::DracoError(format!(
                                "Failed to encode normal attribute transform data {}",
                                i
                            )));
                        }
                    }
                } else if let Some(ref att_encoder) = integer_encoders[i] {
                    if !att_encoder.encode_data_needed_by_portable_transform(out_buffer) {
                        return Err(DracoError::DracoError(format!(
                            "Failed to encode quantization transform data {}",
                            i
                        )));
                    }
                }
            }

            // Read back off the encoders that just ran: the prediction scheme
            // is theirs to choose, and a request for one is only honoured when
            // the attribute supports it.
            let mut attributes = Vec::with_capacity(num_attributes as usize);
            for i in 0..num_attributes {
                let att = pc.attribute(i);
                let encoder_type = encoder_types[i as usize];
                let prediction = match encoder_type {
                    SequentialAttributeEncoderType::Normals => normal_encoders[i as usize]
                        .as_ref()
                        .and_then(|encoder| encoder.selected_prediction()),
                    _ => integer_encoders[i as usize]
                        .as_ref()
                        .and_then(|encoder| encoder.selected_prediction()),
                };
                let quantization_bits = match encoder_type {
                    SequentialAttributeEncoderType::Quantization
                    | SequentialAttributeEncoderType::Normals => {
                        Some(self.options.get_attribute_int(i, "quantization_bits", -1))
                    }
                    SequentialAttributeEncoderType::Integer
                    | SequentialAttributeEncoderType::Generic => None,
                };
                attributes.push(EncodedAttributeInfo {
                    source_attribute_id: i,
                    attribute_type: att.attribute_type(),
                    data_type: att.data_type(),
                    num_components: att.num_components(),
                    normalized: att.normalized(),
                    unique_id: att.unique_id(),
                    num_encoded_values: att.size(),
                    encoder_type,
                    quantization_bits,
                    prediction,
                    position_min: None,
                    position_max: None,
                });
            }
            return Ok(attributes);
        }

        Ok(Vec::new())
    }

    fn encode_metadata(&self, buffer: &mut EncoderBuffer) -> Status {
        if let Some(metadata) = self
            .point_cloud
            .as_ref()
            .and_then(|point_cloud| point_cloud.metadata())
            .filter(|metadata| !metadata.is_empty())
        {
            metadata.encode(buffer)?;
        }
        Ok(())
    }

    fn encode_header(&self, buffer: &mut EncoderBuffer, method: i32) -> Status {
        let (mut major, mut minor) = self.options.get_version();
        if major == 0 && minor == 0 {
            (major, minor) = DEFAULT_POINT_CLOUD_VERSION;
        }
        let has_metadata = self
            .point_cloud
            .as_ref()
            .and_then(|point_cloud| point_cloud.metadata())
            .is_some_and(|metadata| !metadata.is_empty());

        if has_metadata && !has_header_flags(major, minor) {
            return Err(DracoError::UnsupportedVersion(
                "Metadata requires Draco bitstream version 1.3 or newer".to_string(),
            ));
        }

        #[cfg(not(feature = "legacy_bitstream_encode"))]
        match self.options.get_prediction_scheme() {
            2 | 3 => {
                return Err(DracoError::UnsupportedFeature(
                    "legacy prediction schemes require the legacy_bitstream_encode feature"
                        .to_string(),
                ));
            }
            _ => {}
        }

        buffer.encode_data(b"DRACO");

        buffer.encode_u8(major);
        buffer.encode_u8(minor);
        buffer.set_version(major, minor);

        buffer.encode_u8(self.get_geometry_type() as u8);
        buffer.encode_u8(method as u8);

        // The flags field is part of the header for every version this crate
        // encodes: upstream `PointCloudEncoder::EncodeHeader` writes it
        // unconditionally as far back as 1.0.0, and both decoders read it
        // unconditionally. Writing it only from 1.3 left a stream that was two
        // bytes short of what its own decoder expects, so an explicit
        // `set_version(1, 0)` produced a `.drc` nothing could read.
        let flags = if has_metadata { METADATA_FLAG_MASK } else { 0 };
        buffer.encode_u16(flags);
        Ok(())
    }

    /// Returns the geometry type produced by this encoder.
    pub fn get_geometry_type(&self) -> EncodedGeometryType {
        EncodedGeometryType::PointCloud
    }
}
