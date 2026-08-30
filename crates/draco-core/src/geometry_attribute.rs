use crate::attribute_transform_data::AttributeTransformData;
use crate::data_buffer::DataBuffer;
use crate::draco_types::DataType;
use crate::geometry_indices::{AttributeValueIndex, PointIndex, INVALID_ATTRIBUTE_VALUE_INDEX};
use crate::status::DracoError;
use std::convert::TryFrom;

/// Widen one stored scalar to `f32`, whatever Draco declared its type to be.
///
/// `bytes` must be exactly the declared type's width; callers slice it from the
/// attribute buffer. A type with no numeric reading -- `Bool` and the 64-bit
/// integers, which do not survive an `f32` anyway -- reads as zero.
fn scalar_as_f32(data_type: DataType, bytes: &[u8]) -> f32 {
    match data_type {
        DataType::Float32 => f32::from_le_bytes(bytes.try_into().unwrap()),
        DataType::Float64 => f64::from_le_bytes(bytes.try_into().unwrap()) as f32,
        DataType::Int8 => bytes[0] as i8 as f32,
        DataType::Uint8 => bytes[0] as f32,
        DataType::Int16 => i16::from_le_bytes(bytes.try_into().unwrap()) as f32,
        DataType::Uint16 => u16::from_le_bytes(bytes.try_into().unwrap()) as f32,
        DataType::Int32 => i32::from_le_bytes(bytes.try_into().unwrap()) as f32,
        DataType::Uint32 => u32::from_le_bytes(bytes.try_into().unwrap()) as f32,
        _ => 0.0,
    }
}

/// Semantic role of a geometry attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryAttributeType {
    /// Invalid or unset attribute type.
    Invalid = -1,
    /// Vertex or point positions.
    Position = 0,
    /// Vertex or point normals.
    Normal,
    /// Vertex or point colors.
    Color,
    /// Texture coordinates.
    TexCoord,
    /// Application-defined attribute data.
    Generic,
}

impl TryFrom<u8> for GeometryAttributeType {
    type Error = DracoError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Position),
            1 => Ok(Self::Normal),
            2 => Ok(Self::Color),
            3 => Ok(Self::TexCoord),
            4 => Ok(Self::Generic),
            _ => Err(DracoError::general(format!(
                "Invalid geometry attribute type: {value}"
            ))),
        }
    }
}

/// Format descriptor shared by point and mesh attributes.
#[derive(Debug, Clone)]
pub struct GeometryAttribute {
    attribute_type: GeometryAttributeType,
    data_type: DataType,
    num_components: u8,
    normalized: bool,
    byte_stride: i64,
    byte_offset: i64,
    unique_id: u32,
}

impl Default for GeometryAttribute {
    fn default() -> Self {
        Self {
            attribute_type: GeometryAttributeType::Invalid,
            data_type: DataType::Invalid,
            num_components: 0,
            normalized: false,
            byte_stride: 0,
            byte_offset: 0,
            unique_id: 0,
        }
    }
}

impl GeometryAttribute {
    // Attribute initialization requires 7 parameters to fully specify metadata:
    // type, components, data_type, normalized flag, num_values, byte_stride, byte_offset.
    // This matches the C++ PointAttribute::Init() signature and cannot be simplified
    // without breaking API compatibility or making attribute setup less explicit.
    /// Initializes the attribute format descriptor.
    #[allow(clippy::too_many_arguments)]
    pub fn init(
        &mut self,
        attribute_type: GeometryAttributeType,
        _buffer: Option<&DataBuffer>,
        num_components: u8,
        data_type: DataType,
        normalized: bool,
        byte_stride: i64,
        byte_offset: i64,
    ) {
        self.attribute_type = attribute_type;
        self.num_components = num_components;
        self.data_type = data_type;
        self.normalized = normalized;
        self.byte_stride = byte_stride;
        self.byte_offset = byte_offset;
    }

    /// Returns the semantic attribute type.
    pub fn attribute_type(&self) -> GeometryAttributeType {
        self.attribute_type
    }

    /// Returns the scalar data type used by each component.
    pub fn data_type(&self) -> DataType {
        self.data_type
    }

    /// Returns the number of scalar components per attribute value.
    pub fn num_components(&self) -> u8 {
        self.num_components
    }

    /// Returns whether integer data should be interpreted as normalized.
    pub fn normalized(&self) -> bool {
        self.normalized
    }

    /// Returns the byte stride between consecutive values.
    pub fn byte_stride(&self) -> i64 {
        self.byte_stride
    }

    /// Returns the byte offset of the first value.
    pub fn byte_offset(&self) -> i64 {
        self.byte_offset
    }

    /// Returns the stable Draco unique id for this attribute.
    pub fn unique_id(&self) -> u32 {
        self.unique_id
    }

    /// Sets the stable Draco unique id for this attribute.
    pub fn set_unique_id(&mut self, id: u32) {
        self.unique_id = id;
    }

    /// Sets the semantic attribute type.
    pub fn set_attribute_type(&mut self, attribute_type: GeometryAttributeType) {
        self.attribute_type = attribute_type;
    }

    /// Sets the scalar data type.
    pub fn set_data_type(&mut self, data_type: DataType) {
        self.data_type = data_type;
    }

    /// Sets the number of scalar components per value.
    pub fn set_num_components(&mut self, num_components: u8) {
        self.num_components = num_components;
    }
}

/// Typed attribute values attached to points in a point cloud or mesh.
///
/// Attribute data is stored in a contiguous byte buffer. Point ids either map
/// directly to attribute value ids, or through an explicit point-to-value map
/// when multiple points share or reorder attribute entries.
#[derive(Debug, Clone)]
pub struct PointAttribute {
    base: GeometryAttribute,
    buffer: DataBuffer,
    indices_map: Vec<AttributeValueIndex>,
    identity_mapping: bool,
    num_unique_entries: usize,
    attribute_transform_data: Option<Box<AttributeTransformData>>,
}

impl Default for PointAttribute {
    fn default() -> Self {
        Self {
            base: GeometryAttribute::default(),
            buffer: DataBuffer::new(),
            indices_map: Vec::new(),
            identity_mapping: true,
            num_unique_entries: 0,
            attribute_transform_data: None,
        }
    }
}

impl PointAttribute {
    /// Creates an empty attribute with an invalid semantic type.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initializes the attribute and allocates storage for its values.
    pub fn init(
        &mut self,
        attribute_type: GeometryAttributeType,
        num_components: u8,
        data_type: DataType,
        normalized: bool,
        num_attribute_values: usize,
    ) {
        let byte_stride = (num_components as usize * data_type.byte_length()) as i64;
        self.base.init(
            attribute_type,
            None,
            num_components,
            data_type,
            normalized,
            byte_stride,
            0,
        );
        self.buffer
            .resize(num_attribute_values * byte_stride as usize);
        self.num_unique_entries = num_attribute_values;
        self.identity_mapping = true;
    }

    /// Fallibly initializes the attribute and allocates storage for its values.
    pub fn try_init(
        &mut self,
        attribute_type: GeometryAttributeType,
        num_components: u8,
        data_type: DataType,
        normalized: bool,
        num_attribute_values: usize,
    ) -> Result<(), DracoError> {
        let byte_stride = num_components as usize * data_type.byte_length();
        let buffer_size = num_attribute_values
            .checked_mul(byte_stride)
            .ok_or_else(|| {
                DracoError::general("Point attribute buffer size overflow".to_string())
            })?;
        self.base.init(
            attribute_type,
            None,
            num_components,
            data_type,
            normalized,
            byte_stride as i64,
            0,
        );
        self.buffer.try_resize(buffer_size).map_err(|_| {
            DracoError::general("Failed to allocate point attribute buffer".to_string())
        })?;
        self.num_unique_entries = num_attribute_values;
        self.identity_mapping = true;
        Ok(())
    }

    /// The same shape, without reserving for the values.
    ///
    /// For decode paths where the count comes from the header: it is what the
    /// stream *claims*, and reserving for a claim is how a nine-byte header
    /// names gigabytes. The decoders that fill this buffer size it themselves
    /// as the values arrive -- the integer path resizes before it writes, the
    /// generic path resizes only once the bytes are in the stream to be read --
    /// so nothing here needs the room before there is anything to put in it.
    ///
    /// `num_unique_entries` still reports the declared count, because that is
    /// the ceiling the decode works towards; what is deferred is the memory.
    /// Not for the KD-tree path, which writes at computed offsets and needs the
    /// buffer sized first; that path bounds its own allocation instead.
    ///
    /// Used by decoder paths where the count comes from the bitstream and the
    /// value payload has not been validated yet.
    #[cfg(feature = "decoder")]
    pub(crate) fn init_deferred(
        &mut self,
        attribute_type: GeometryAttributeType,
        num_components: u8,
        data_type: DataType,
        normalized: bool,
        num_attribute_values: usize,
    ) -> Result<(), DracoError> {
        let byte_stride = num_components as usize * data_type.byte_length();
        // Still checked, so an overflowing shape is refused here rather than
        // wrapping into a small buffer somewhere later.
        num_attribute_values
            .checked_mul(byte_stride)
            .ok_or_else(|| {
                DracoError::general("Point attribute buffer size overflow".to_string())
            })?;
        self.base.init(
            attribute_type,
            None,
            num_components,
            data_type,
            normalized,
            byte_stride as i64,
            0,
        );
        self.num_unique_entries = num_attribute_values;
        self.identity_mapping = true;
        Ok(())
    }

    /// Maps a point id to the corresponding attribute value id.
    pub fn mapped_index(&self, point_index: PointIndex) -> AttributeValueIndex {
        if self.identity_mapping {
            AttributeValueIndex(point_index.0)
        } else if (point_index.0 as usize) < self.indices_map.len() {
            self.indices_map[point_index.0 as usize]
        } else {
            INVALID_ATTRIBUTE_VALUE_INDEX
        }
    }

    /// Returns the number of unique attribute values.
    pub fn size(&self) -> usize {
        self.num_unique_entries
    }

    /// Read `components` scalars per point as `f32`, in point order.
    ///
    /// The inverse of [`DataBuffer::update_f32s_le`](crate::data_buffer::DataBuffer::update_f32s_le):
    /// whatever component type the attribute stores widens to `f32`, and the
    /// value index mapping is followed, so an attribute with fewer unique
    /// values than points still lands on the right one.
    ///
    /// The output is always `num_points * components` long, zero-filled where
    /// the attribute has nothing to give. That matters because the result is a
    /// channel addressed by vertex index: returning a short row for an
    /// attribute with fewer components than asked for would shift every vertex
    /// after it onto the wrong values, which is worse than a zero.
    ///
    /// The ordinary case -- float32, tightly packed, identity mapping, asking
    /// for exactly what the attribute has -- is converted in one pass rather
    /// than one bounds-checked read per point.
    pub fn read_f32s(&self, num_points: usize, components: usize) -> Vec<f32> {
        let mut values = vec![0.0f32; num_points * components];
        if components == 0 {
            return values;
        }
        let stride = self.byte_stride() as usize;
        let width = self.data_type().byte_length();
        let data = self.buffer.data();

        if self.identity_mapping
            && self.data_type() == DataType::Float32
            && components == self.num_components() as usize
            && stride == components * 4
            && data.len() >= num_points * stride
        {
            let packed = &data[..num_points * stride];
            for (out, bytes) in values.iter_mut().zip(packed.as_chunks::<4>().0) {
                *out = f32::from_le_bytes(*bytes);
            }
            return values;
        }

        let available = (self.num_components() as usize).min(components);
        for point in 0..num_points {
            let value_index = self.mapped_index(PointIndex(point as u32)).0 as usize;
            // Also catches INVALID_ATTRIBUTE_VALUE_INDEX, whose `usize` product
            // with the stride would overflow on a 32-bit target.
            if value_index >= self.num_unique_entries {
                continue;
            }
            let base = value_index * stride;
            for component in 0..available {
                let offset = base + component * width;
                if offset + width > data.len() {
                    continue;
                }
                values[point * components + component] =
                    scalar_as_f32(self.data_type(), &data[offset..offset + width]);
            }
        }
        values
    }

    /// Resizes the unique attribute value storage.
    pub fn resize_unique_entries(&mut self, num_attribute_values: usize) -> Result<(), DracoError> {
        let byte_stride = self.byte_stride() as usize;
        let buffer_size = num_attribute_values
            .checked_mul(byte_stride)
            .ok_or_else(|| {
                DracoError::general("Point attribute buffer size overflow".to_string())
            })?;
        self.buffer.try_resize(buffer_size).map_err(|_| {
            DracoError::general("Failed to allocate point attribute buffer".to_string())
        })?;
        self.num_unique_entries = num_attribute_values;
        if self.identity_mapping {
            self.indices_map.clear();
        }
        Ok(())
    }

    /// Merges values that are bit-identical, and remaps the point map onto
    /// what survives. Returns how many values are left.
    ///
    /// Port of upstream's `PointAttribute::DeduplicateValues`, which every one
    /// of its readers runs before handing a mesh to the encoder. Two vertices
    /// carrying the same position arrive as two values here and as one there,
    /// and the difference is not cosmetic: with the values merged, triangles
    /// that shared only a vertex come to share an edge, so the encoder sees
    /// one connected component where it otherwise sees two.
    ///
    /// Comparison is over the raw bytes, which is upstream's rule and not an
    /// approximation of it: it bit-copies each value into an integer of the
    /// same width before hashing, so `-0.0` and `0.0` are distinct values on
    /// both sides and two `NaN`s merge exactly when their payloads match.
    ///
    /// Refuses the types upstream refuses -- everything 64 bits wide -- and
    /// component counts outside `1..=4`, rather than silently doing something
    /// upstream does not.
    pub(crate) fn deduplicate_values(&mut self) -> Result<usize, DracoError> {
        let stride = match self.data_type() {
            DataType::Int8 | DataType::Uint8 | DataType::Bool => 1,
            DataType::Int16 | DataType::Uint16 => 2,
            DataType::Int32 | DataType::Uint32 | DataType::Float32 => 4,
            other => {
                return Err(DracoError::unsupported_feature(format!(
                    "deduplicating {other:?} attribute values"
                )))
            }
        } * usize::from(self.num_components());
        if !(1..=4).contains(&self.num_components()) {
            return Err(DracoError::unsupported_feature(format!(
                "deduplicating a {}-component attribute",
                self.num_components()
            )));
        }

        let count = self.num_unique_entries;
        // Widest supported value is four 32-bit components, so the key is
        // inline and the map allocates nothing per entry.
        let mut seen: std::collections::HashMap<[u8; 16], u32> =
            std::collections::HashMap::with_capacity(count);
        let mut value_map: Vec<u32> = Vec::with_capacity(count);
        let mut unique = 0usize;
        let data = self.buffer.data_mut();
        for i in 0..count {
            let at = i * stride;
            let mut key = [0u8; 16];
            key[..stride].copy_from_slice(&data[at..at + stride]);
            match seen.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    value_map.push(*entry.get());
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(unique as u32);
                    // Survivors are packed forward as they are found, so the
                    // buffer needs no second pass and no second allocation.
                    data.copy_within(at..at + stride, unique * stride);
                    value_map.push(unique as u32);
                    unique += 1;
                }
            }
        }
        if unique == count {
            return Ok(unique);
        }

        if self.identity_mapping {
            // The map was point == value, so the old value count is the point
            // count this attribute has to keep answering for.
            self.set_explicit_mapping(count);
            for (point, value) in value_map.iter().enumerate() {
                self.indices_map[point] = AttributeValueIndex(*value);
            }
        } else {
            for entry in self.indices_map.iter_mut() {
                *entry = AttributeValueIndex(value_map[entry.0 as usize]);
            }
        }
        self.num_unique_entries = unique;
        Ok(unique)
    }

    /// Drops values no point maps to, and remaps the point map onto what is
    /// left. Returns how many values survive.
    ///
    /// Port of upstream's `PointAttribute::RemoveUnusedValues`, which upstream
    /// compiles only into its transcoder and never runs from a reader. Under
    /// identity mapping every value is used by definition, so there is nothing
    /// to do.
    ///
    /// A value nothing points at still costs: the encoder quantizes over the
    /// range of the values it is given, so one stray entry spends bits on
    /// empty space. Measured on a unit triangle carrying an unreferenced
    /// vertex at `1000, 1000, 1000`: same encoded size, and the surviving
    /// coordinates came back as `1.007095` instead of `1.0`.
    pub(crate) fn remove_unused_values(&mut self) -> usize {
        if self.identity_mapping {
            return self.num_unique_entries;
        }
        let mut used = vec![false; self.num_unique_entries];
        let mut num_used = 0usize;
        for value in &self.indices_map {
            let index = value.0 as usize;
            if index < used.len() && !used[index] {
                used[index] = true;
                num_used += 1;
            }
        }
        if num_used == self.num_unique_entries {
            return num_used;
        }

        let stride = usize::from(self.num_components()) * self.data_type().byte_length();
        let mut old_to_new = vec![INVALID_ATTRIBUTE_VALUE_INDEX; self.num_unique_entries];
        let mut next = 0usize;
        {
            let data = self.buffer.data_mut();
            for old in 0..used.len() {
                if !used[old] {
                    continue;
                }
                if old != next && stride > 0 {
                    data.copy_within(old * stride..old * stride + stride, next * stride);
                }
                old_to_new[old] = AttributeValueIndex(next as u32);
                next += 1;
            }
        }
        for entry in self.indices_map.iter_mut() {
            *entry = old_to_new[entry.0 as usize];
        }
        self.num_unique_entries = num_used;
        num_used
    }

    /// Returns the raw attribute value buffer.
    pub fn buffer(&self) -> &DataBuffer {
        &self.buffer
    }

    /// Returns the mutable raw attribute value buffer.
    pub fn buffer_mut(&mut self) -> &mut DataBuffer {
        &mut self.buffer
    }

    /// Returns the semantic attribute type.
    pub fn attribute_type(&self) -> GeometryAttributeType {
        self.base.attribute_type()
    }

    /// Returns the stable Draco unique id.
    pub fn unique_id(&self) -> u32 {
        self.base.unique_id()
    }

    /// Sets the stable Draco unique id.
    pub fn set_unique_id(&mut self, id: u32) {
        self.base.set_unique_id(id);
    }

    /// Sets the semantic attribute type.
    pub fn set_attribute_type(&mut self, attribute_type: GeometryAttributeType) {
        self.base.set_attribute_type(attribute_type);
    }

    /// Sets the scalar data type.
    pub fn set_data_type(&mut self, data_type: DataType) {
        self.base.set_data_type(data_type);
    }

    /// Sets the number of scalar components per value.
    pub fn set_num_components(&mut self, num_components: u8) {
        self.base.set_num_components(num_components);
    }

    /// Returns whether point ids are used directly as attribute value ids.
    ///
    /// The counterpart of [`set_identity_mapping`](Self::set_identity_mapping)
    /// and [`set_explicit_mapping`](Self::set_explicit_mapping): with identity
    /// mapping, point `i` reads value `i`, so a caller validating the mapping
    /// answers in one comparison rather than a call to
    /// [`mapped_index`](Self::mapped_index) per point.
    pub fn is_mapping_identity(&self) -> bool {
        self.identity_mapping
    }

    /// Uses point ids directly as attribute value ids.
    pub fn set_identity_mapping(&mut self) {
        self.identity_mapping = true;
        self.indices_map.clear();
    }

    /// Allocates an explicit point-to-attribute-value map.
    pub fn set_explicit_mapping(&mut self, num_points: usize) {
        self.identity_mapping = false;
        self.indices_map
            .resize(num_points, INVALID_ATTRIBUTE_VALUE_INDEX);
    }

    /// The explicit point-to-attribute-value map, if one is set.
    ///
    /// `None` under identity mapping. The bulk counterpart of calling
    /// [`mapped_index`](Self::mapped_index) per point, for callers that copy
    /// the map somewhere whole.
    pub fn explicit_mapping(&self) -> Option<&[AttributeValueIndex]> {
        if self.identity_mapping {
            None
        } else {
            Some(&self.indices_map)
        }
    }

    /// Replaces the whole point-to-attribute-value map in one copy.
    ///
    /// The bulk form of [`set_explicit_mapping`](Self::set_explicit_mapping)
    /// followed by one [`try_set_point_map_entry`](Self::try_set_point_map_entry)
    /// per point: a caller that has already assembled the full map hands it
    /// over as a slice copy instead of a fallible call per entry.
    pub fn set_explicit_mapping_from(&mut self, entries: &[AttributeValueIndex]) {
        self.identity_mapping = false;
        self.indices_map.clear();
        self.indices_map.extend_from_slice(entries);
    }

    /// Sets one point-to-attribute-value map entry.
    pub fn set_point_map_entry(
        &mut self,
        point_index: PointIndex,
        entry_index: AttributeValueIndex,
    ) {
        self.try_set_point_map_entry(point_index, entry_index)
            .expect("point map entry must be in range");
    }

    /// Fallibly sets one point-to-attribute-value map entry.
    pub fn try_set_point_map_entry(
        &mut self,
        point_index: PointIndex,
        entry_index: AttributeValueIndex,
    ) -> Result<(), DracoError> {
        if self.identity_mapping {
            return Ok(());
        }
        let Some(slot) = self.indices_map.get_mut(point_index.0 as usize) else {
            return Err(DracoError::general(
                "Point map entry index out of range".to_string(),
            ));
        };
        *slot = entry_index;
        Ok(())
    }

    /// Stores transform metadata associated with this attribute.
    pub fn set_attribute_transform_data(&mut self, data: AttributeTransformData) {
        self.attribute_transform_data = Some(Box::new(data));
    }

    /// Returns transform metadata associated with this attribute, if present.
    pub fn attribute_transform_data(&self) -> Option<&AttributeTransformData> {
        self.attribute_transform_data.as_deref()
    }

    /// Returns the scalar data type.
    pub fn data_type(&self) -> DataType {
        self.base.data_type()
    }

    /// Returns whether integer data should be interpreted as normalized.
    pub fn normalized(&self) -> bool {
        self.base.normalized()
    }

    /// Returns the number of scalar components per value.
    pub fn num_components(&self) -> u8 {
        self.base.num_components()
    }

    /// Returns the byte stride between consecutive values.
    pub fn byte_stride(&self) -> i64 {
        self.base.byte_stride()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn float_attribute(components: u8, values: &[f32]) -> PointAttribute {
        let mut attribute = PointAttribute::new();
        attribute.init(
            GeometryAttributeType::Position,
            components,
            DataType::Float32,
            false,
            values.len() / components as usize,
        );
        attribute.buffer_mut().update_f32s_le(0, values);
        attribute
    }

    /// The packed float path, which is what every WASM reader hits.
    #[test]
    fn read_f32s_reads_packed_floats() {
        let attribute = float_attribute(3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(
            attribute.read_f32s(2, 3),
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
        );
    }

    /// Asking for more components than the attribute has pads rather than
    /// shortening the row. A short row would shift every later vertex onto the
    /// wrong values, because the result is addressed by vertex index.
    #[test]
    fn read_f32s_pads_missing_components() {
        let attribute = float_attribute(2, &[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(
            attribute.read_f32s(2, 3),
            vec![1.0, 2.0, 0.0, 3.0, 4.0, 0.0]
        );
    }

    /// Asking for fewer takes the leading components and drops the rest.
    #[test]
    fn read_f32s_truncates_extra_components() {
        let attribute = float_attribute(3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(attribute.read_f32s(2, 2), vec![1.0, 2.0, 4.0, 5.0]);
    }

    /// A widening type takes the slow path and still comes back as f32.
    #[test]
    fn read_f32s_widens_integer_components() {
        let mut attribute = PointAttribute::new();
        attribute.init(GeometryAttributeType::Color, 2, DataType::Uint8, true, 2);
        attribute.buffer_mut().update(&[1, 2, 250, 255], None);
        assert_eq!(attribute.read_f32s(2, 2), vec![1.0, 2.0, 250.0, 255.0]);
    }

    /// Points beyond what the attribute stores read as zero rather than
    /// panicking or running off the buffer.
    #[test]
    fn read_f32s_zero_fills_points_past_the_end() {
        let attribute = float_attribute(3, &[1.0, 2.0, 3.0]);
        assert_eq!(
            attribute.read_f32s(2, 3),
            vec![1.0, 2.0, 3.0, 0.0, 0.0, 0.0]
        );
    }

    /// A non-identity mapping is followed, so two points sharing one value both
    /// read it. The fast path must not swallow this case.
    #[test]
    fn read_f32s_follows_the_value_mapping() {
        let mut attribute = float_attribute(3, &[7.0, 8.0, 9.0]);
        attribute.set_explicit_mapping(2);
        attribute
            .try_set_point_map_entry(PointIndex(0), AttributeValueIndex(0))
            .unwrap();
        attribute
            .try_set_point_map_entry(PointIndex(1), AttributeValueIndex(0))
            .unwrap();
        assert_eq!(
            attribute.read_f32s(2, 3),
            vec![7.0, 8.0, 9.0, 7.0, 8.0, 9.0]
        );
    }

    #[test]
    fn try_set_point_map_entry_rejects_out_of_range_point() {
        let mut attribute = PointAttribute::new();
        attribute.set_explicit_mapping(1);

        assert!(attribute
            .try_set_point_map_entry(PointIndex(1), AttributeValueIndex(0))
            .is_err());
    }
}
