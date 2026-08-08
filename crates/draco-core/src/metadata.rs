#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
use crate::status::DracoError;
use std::collections::BTreeMap;
use std::mem::size_of;
use std::str;

/// Header flag bit indicating that geometry metadata follows the payload.
pub const METADATA_FLAG_MASK: u16 = 0x8000;

#[cfg(feature = "decoder")]
const MAX_METADATA_NESTING_DEPTH: usize = 1000;
const MAX_METADATA_NAME_LEN: usize = u8::MAX as usize;

/// Raw Draco metadata map.
///
/// Entries store the exact byte payload written to the Draco bitstream. Typed
/// helpers encode values using the same little-endian layout as C++ Draco.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    entries: BTreeMap<String, Vec<u8>>,
    sub_metadata: BTreeMap<String, Metadata>,
}

/// Metadata associated with one geometry attribute unique id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeMetadata {
    attribute_unique_id: u32,
    metadata: Metadata,
}

/// Geometry-level metadata plus per-attribute metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeometryMetadata {
    metadata: Metadata,
    attribute_metadata: Vec<AttributeMetadata>,
}

impl Metadata {
    /// Creates an empty metadata map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when there are no entries or nested metadata blocks.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.sub_metadata.is_empty()
    }

    /// Returns all raw metadata entries.
    pub fn entries(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.entries
    }

    /// Returns all nested metadata blocks.
    pub fn sub_metadata(&self) -> &BTreeMap<String, Metadata> {
        &self.sub_metadata
    }

    /// Returns the raw byte value for an entry.
    pub fn get_raw(&self, name: &str) -> Option<&[u8]> {
        self.entries.get(name).map(Vec::as_slice)
    }

    /// Sets a raw byte metadata entry.
    pub fn set_raw(
        &mut self,
        name: impl Into<String>,
        value: impl Into<Vec<u8>>,
    ) -> Result<(), DracoError> {
        let name = name.into();
        validate_metadata_name(&name)?;
        let value = value.into();
        if value.is_empty() {
            return Err(DracoError::invalid_parameter(
                "Metadata entry value must not be empty".to_string(),
            ));
        }
        self.entries.insert(name, value);
        Ok(())
    }

    /// Stores an `i32` metadata entry.
    pub fn set_i32(&mut self, name: impl Into<String>, value: i32) -> Result<(), DracoError> {
        self.set_raw(name, value.to_le_bytes().to_vec())
    }

    /// Reads an `i32` metadata entry.
    pub fn get_i32(&self, name: &str) -> Option<i32> {
        let bytes = self.get_raw(name)?;
        let bytes: [u8; size_of::<i32>()] = bytes.try_into().ok()?;
        Some(i32::from_le_bytes(bytes))
    }

    /// Stores an array of `i32` metadata values.
    pub fn set_i32_array(
        &mut self,
        name: impl Into<String>,
        values: &[i32],
    ) -> Result<(), DracoError> {
        let mut bytes = Vec::new();
        let byte_len = values
            .len()
            .checked_mul(size_of::<i32>())
            .ok_or_else(|| DracoError::invalid_parameter("Metadata array too large".to_string()))?;
        bytes
            .try_reserve_exact(byte_len)
            .map_err(|_| DracoError::invalid_parameter("Metadata array too large".to_string()))?;
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        self.set_raw(name, bytes)
    }

    /// Reads an array of `i32` metadata values.
    pub fn get_i32_array(&self, name: &str) -> Option<Vec<i32>> {
        let bytes = self.get_raw(name)?;
        decode_le_array::<{ size_of::<i32>() }, i32>(bytes, i32::from_le_bytes)
    }

    /// Stores an `f64` metadata entry.
    pub fn set_f64(&mut self, name: impl Into<String>, value: f64) -> Result<(), DracoError> {
        self.set_raw(name, value.to_le_bytes().to_vec())
    }

    /// Reads an `f64` metadata entry.
    pub fn get_f64(&self, name: &str) -> Option<f64> {
        let bytes = self.get_raw(name)?;
        let bytes: [u8; size_of::<f64>()] = bytes.try_into().ok()?;
        Some(f64::from_le_bytes(bytes))
    }

    /// Stores an array of `f64` metadata values.
    pub fn set_f64_array(
        &mut self,
        name: impl Into<String>,
        values: &[f64],
    ) -> Result<(), DracoError> {
        let mut bytes = Vec::new();
        let byte_len = values
            .len()
            .checked_mul(size_of::<f64>())
            .ok_or_else(|| DracoError::invalid_parameter("Metadata array too large".to_string()))?;
        bytes
            .try_reserve_exact(byte_len)
            .map_err(|_| DracoError::invalid_parameter("Metadata array too large".to_string()))?;
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        self.set_raw(name, bytes)
    }

    /// Reads an array of `f64` metadata values.
    pub fn get_f64_array(&self, name: &str) -> Option<Vec<f64>> {
        let bytes = self.get_raw(name)?;
        decode_le_array::<{ size_of::<f64>() }, f64>(bytes, f64::from_le_bytes)
    }

    /// Stores a UTF-8 string metadata entry.
    pub fn set_string(
        &mut self,
        name: impl Into<String>,
        value: impl AsRef<str>,
    ) -> Result<(), DracoError> {
        self.set_raw(name, value.as_ref().as_bytes().to_vec())
    }

    /// Reads a UTF-8 string metadata entry.
    pub fn get_string(&self, name: &str) -> Option<&str> {
        str::from_utf8(self.get_raw(name)?).ok()
    }

    /// Removes and returns a raw metadata entry.
    pub fn remove_entry(&mut self, name: &str) -> Option<Vec<u8>> {
        self.entries.remove(name)
    }

    /// Returns a nested metadata block by name.
    pub fn get_sub_metadata(&self, name: &str) -> Option<&Metadata> {
        self.sub_metadata.get(name)
    }

    /// Returns a mutable nested metadata block by name.
    pub fn get_sub_metadata_mut(&mut self, name: &str) -> Option<&mut Metadata> {
        self.sub_metadata.get_mut(name)
    }

    /// Inserts a nested metadata block.
    pub fn insert_sub_metadata(
        &mut self,
        name: impl Into<String>,
        metadata: Metadata,
    ) -> Result<(), DracoError> {
        let name = name.into();
        validate_metadata_name(&name)?;
        if self.sub_metadata.contains_key(&name) {
            return Err(DracoError::general(format!(
                "Duplicate sub-metadata name: {name}"
            )));
        }
        self.sub_metadata.insert(name, metadata);
        Ok(())
    }

    #[cfg(feature = "decoder")]
    /// Decodes a metadata block from a Draco bitstream buffer.
    pub fn decode(buffer: &mut DecoderBuffer<'_>) -> Result<Self, DracoError> {
        Self::decode_with_depth(buffer, 0)
    }

    #[cfg(feature = "decoder")]
    fn decode_with_depth(buffer: &mut DecoderBuffer<'_>, depth: usize) -> Result<Self, DracoError> {
        if depth > MAX_METADATA_NESTING_DEPTH {
            return Err(DracoError::general(
                "Metadata nesting depth exceeded".to_string(),
            ));
        }

        let num_entries = decode_bounded_count(buffer, "metadata entries count")?;
        let mut metadata = Metadata::new();
        for _ in 0..num_entries {
            let name = decode_name(buffer)?;
            let data_size = usize::try_from(buffer.decode_varint()?).map_err(|_| {
                DracoError::general("Metadata entry data size too large".to_string())
            })?;
            if data_size == 0 {
                return Err(DracoError::general(
                    "Invalid metadata entry data size".to_string(),
                ));
            }
            if data_size > buffer.remaining_size() {
                return Err(DracoError::general(
                    "Failed to read metadata entry value".to_string(),
                ));
            }
            let value = buffer.decode_slice(data_size)?.to_vec();
            metadata.entries.insert(name, value);
        }

        // `decode_bounded_count` already refuses a count above the remaining
        // size, and nothing is read between there and here, so the check that
        // used to repeat it could not fire. The compiler had worked that out
        // and dropped its message from the binary; this removes the source it
        // was dropped from.
        let num_sub_metadata = decode_bounded_count(buffer, "sub-metadata count")?;
        for _ in 0..num_sub_metadata {
            let name = decode_name(buffer)?;
            if metadata.sub_metadata.contains_key(&name) {
                return Err(DracoError::general(format!(
                    "Duplicate sub-metadata name: {name}"
                )));
            }
            let sub_metadata = Self::decode_with_depth(buffer, depth + 1)?;
            metadata.sub_metadata.insert(name, sub_metadata);
        }

        Ok(metadata)
    }

    #[cfg(feature = "encoder")]
    /// Encodes this metadata block to a Draco bitstream buffer.
    pub fn encode(&self, buffer: &mut EncoderBuffer) -> Result<(), DracoError> {
        buffer.encode_varint(self.entries.len() as u64);
        for (name, value) in &self.entries {
            encode_name(buffer, name)?;
            if value.is_empty() {
                return Err(DracoError::invalid_parameter(
                    "Metadata entry value must not be empty".to_string(),
                ));
            }
            buffer.encode_varint(value.len() as u64);
            buffer.encode_data(value);
        }

        buffer.encode_varint(self.sub_metadata.len() as u64);
        for (name, metadata) in &self.sub_metadata {
            encode_name(buffer, name)?;
            metadata.encode(buffer)?;
        }

        Ok(())
    }
}

impl AttributeMetadata {
    /// Creates metadata for one attribute unique id.
    pub fn new(attribute_unique_id: u32, metadata: Metadata) -> Self {
        Self {
            attribute_unique_id,
            metadata,
        }
    }

    /// Returns the Draco attribute unique id.
    pub fn attribute_unique_id(&self) -> u32 {
        self.attribute_unique_id
    }

    /// Returns the metadata payload.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns the mutable metadata payload.
    pub fn metadata_mut(&mut self) -> &mut Metadata {
        &mut self.metadata
    }
}

impl GeometryMetadata {
    /// Creates empty geometry metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true when no geometry or attribute metadata is present.
    pub fn is_empty(&self) -> bool {
        self.metadata.is_empty() && self.attribute_metadata.is_empty()
    }

    /// Returns geometry-level metadata.
    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns mutable geometry-level metadata.
    pub fn metadata_mut(&mut self) -> &mut Metadata {
        &mut self.metadata
    }

    /// Returns all per-attribute metadata blocks.
    pub fn attribute_metadata(&self) -> &[AttributeMetadata] {
        &self.attribute_metadata
    }

    /// Finds per-attribute metadata by Draco attribute unique id.
    pub fn attribute_metadata_by_unique_id(
        &self,
        attribute_unique_id: u32,
    ) -> Option<&AttributeMetadata> {
        self.attribute_metadata
            .iter()
            .find(|metadata| metadata.attribute_unique_id == attribute_unique_id)
    }

    /// Finds per-attribute metadata by a string metadata entry.
    pub fn attribute_metadata_by_string_entry(
        &self,
        entry_name: &str,
        entry_value: &str,
    ) -> Option<&AttributeMetadata> {
        self.attribute_metadata
            .iter()
            .find(|metadata| metadata.metadata.get_string(entry_name) == Some(entry_value))
    }

    /// Finds mutable per-attribute metadata by Draco attribute unique id.
    pub fn attribute_metadata_by_unique_id_mut(
        &mut self,
        attribute_unique_id: u32,
    ) -> Option<&mut AttributeMetadata> {
        self.attribute_metadata
            .iter_mut()
            .find(|metadata| metadata.attribute_unique_id == attribute_unique_id)
    }

    /// Inserts or replaces metadata for one Draco attribute unique id.
    pub fn set_attribute_metadata(&mut self, attribute_unique_id: u32, metadata: Metadata) {
        if let Some(existing) = self.attribute_metadata_by_unique_id_mut(attribute_unique_id) {
            existing.metadata = metadata;
        } else {
            self.attribute_metadata
                .push(AttributeMetadata::new(attribute_unique_id, metadata));
        }
    }

    #[cfg(feature = "decoder")]
    /// Decodes geometry metadata from a Draco bitstream buffer.
    pub fn decode(buffer: &mut DecoderBuffer<'_>) -> Result<Self, DracoError> {
        let num_attribute_metadata = decode_bounded_count(buffer, "attribute metadata count")?;
        let mut geometry_metadata = GeometryMetadata::new();
        geometry_metadata
            .attribute_metadata
            .try_reserve_exact(num_attribute_metadata)
            .map_err(|_| DracoError::general("Failed to allocate metadata".to_string()))?;

        for _ in 0..num_attribute_metadata {
            let attribute_unique_id = u32::try_from(buffer.decode_varint()?).map_err(|_| {
                DracoError::general("Attribute metadata unique id too large".to_string())
            })?;
            let metadata = Metadata::decode(buffer)?;
            geometry_metadata
                .attribute_metadata
                .push(AttributeMetadata::new(attribute_unique_id, metadata));
        }
        geometry_metadata.metadata = Metadata::decode(buffer)?;
        Ok(geometry_metadata)
    }

    #[cfg(feature = "encoder")]
    /// Encodes geometry metadata to a Draco bitstream buffer.
    pub fn encode(&self, buffer: &mut EncoderBuffer) -> Result<(), DracoError> {
        buffer.encode_varint(self.attribute_metadata.len() as u64);
        for attribute_metadata in &self.attribute_metadata {
            buffer.encode_varint(attribute_metadata.attribute_unique_id as u64);
            attribute_metadata.metadata.encode(buffer)?;
        }
        self.metadata.encode(buffer)
    }
}

fn decode_le_array<const N: usize, T>(
    bytes: &[u8],
    decode: impl Fn([u8; N]) -> T,
) -> Option<Vec<T>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(N) {
        return None;
    }

    let mut values = Vec::with_capacity(bytes.len() / N);
    for chunk in bytes.chunks_exact(N) {
        let mut item = [0u8; N];
        item.copy_from_slice(chunk);
        values.push(decode(item));
    }
    Some(values)
}

fn validate_metadata_name(name: &str) -> Result<(), DracoError> {
    if name.len() > MAX_METADATA_NAME_LEN {
        return Err(DracoError::invalid_parameter(format!(
            "Metadata name too long: {} bytes",
            name.len()
        )));
    }
    Ok(())
}

#[cfg(feature = "encoder")]
fn encode_name(buffer: &mut EncoderBuffer, name: &str) -> Result<(), DracoError> {
    validate_metadata_name(name)?;
    buffer.encode_u8(name.len() as u8);
    if !name.is_empty() {
        buffer.encode_data(name.as_bytes());
    }
    Ok(())
}

#[cfg(feature = "decoder")]
fn decode_name(buffer: &mut DecoderBuffer<'_>) -> Result<String, DracoError> {
    let name_len = buffer.decode_u8()? as usize;
    if name_len > buffer.remaining_size() {
        return Err(DracoError::general(
            "Failed to read metadata name".to_string(),
        ));
    }
    let name = buffer.decode_slice(name_len)?.to_vec();
    String::from_utf8(name)
        .map_err(|_| DracoError::general("Invalid UTF-8 metadata name".to_string()))
}

#[cfg(feature = "decoder")]
fn decode_bounded_count(
    buffer: &mut DecoderBuffer<'_>,
    label: &'static str,
) -> Result<usize, DracoError> {
    let count = usize::try_from(buffer.decode_varint()?)
        .map_err(|_| DracoError::general(format!("{label} too large")))?;
    if count > buffer.remaining_size() {
        return Err(DracoError::general(format!("Invalid {label}")));
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_metadata(metadata: &Metadata) -> Vec<u8> {
        let mut buffer = EncoderBuffer::new();
        metadata.encode(&mut buffer).expect("metadata encode");
        buffer.data().to_vec()
    }

    #[test]
    fn metadata_codec_accepts_empty_metadata() {
        let metadata = Metadata::new();
        let bytes = encode_metadata(&metadata);
        assert_eq!(bytes, vec![0, 0]);

        let mut buffer = DecoderBuffer::new(&bytes);
        assert_eq!(Metadata::decode(&mut buffer).unwrap(), metadata);
    }

    #[test]
    fn metadata_codec_roundtrips_raw_entry() {
        let mut metadata = Metadata::new();
        metadata.set_raw("name", b"value".to_vec()).unwrap();

        let bytes = encode_metadata(&metadata);
        let mut buffer = DecoderBuffer::new(&bytes);
        let decoded = Metadata::decode(&mut buffer).unwrap();

        assert_eq!(decoded.get_raw("name"), Some(b"value".as_slice()));
    }

    #[test]
    fn metadata_typed_helpers_encode_cpp_compatible_bytes() {
        let mut metadata = Metadata::new();
        metadata.set_i32("i32", -123).unwrap();
        metadata.set_i32_array("i32_array", &[1, -2, 3]).unwrap();
        metadata.set_f64("f64", 1.25).unwrap();
        metadata.set_f64_array("f64_array", &[0.5, -2.0]).unwrap();
        metadata.set_string("string", "metadata").unwrap();

        assert_eq!(
            metadata.get_raw("i32"),
            Some((-123i32).to_le_bytes().as_slice())
        );
        assert_eq!(
            metadata.get_raw("i32_array"),
            Some(
                [
                    1i32.to_le_bytes(),
                    (-2i32).to_le_bytes(),
                    3i32.to_le_bytes()
                ]
                .concat()
                .as_slice()
            )
        );
        assert_eq!(
            metadata.get_raw("f64"),
            Some(1.25f64.to_le_bytes().as_slice())
        );
        assert_eq!(
            metadata.get_raw("f64_array"),
            Some(
                [0.5f64.to_le_bytes(), (-2.0f64).to_le_bytes()]
                    .concat()
                    .as_slice()
            )
        );
        assert_eq!(metadata.get_raw("string"), Some(b"metadata".as_slice()));

        assert_eq!(metadata.get_i32("i32"), Some(-123));
        assert_eq!(metadata.get_i32_array("i32_array"), Some(vec![1, -2, 3]));
        assert_eq!(metadata.get_f64("f64"), Some(1.25));
        assert_eq!(metadata.get_f64_array("f64_array"), Some(vec![0.5, -2.0]));
        assert_eq!(metadata.get_string("string"), Some("metadata"));
    }

    #[test]
    fn metadata_typed_getters_decode_raw_bytes() {
        let mut metadata = Metadata::new();
        metadata
            .set_raw("i32", 42i32.to_le_bytes().to_vec())
            .unwrap();
        metadata
            .set_raw(
                "i32_array",
                [7i32.to_le_bytes(), (-8i32).to_le_bytes()].concat(),
            )
            .unwrap();
        metadata
            .set_raw("f64", 3.5f64.to_le_bytes().to_vec())
            .unwrap();
        metadata
            .set_raw(
                "f64_array",
                [1.0f64.to_le_bytes(), 2.25f64.to_le_bytes()].concat(),
            )
            .unwrap();
        metadata.set_raw("string", b"from raw".to_vec()).unwrap();

        assert_eq!(metadata.get_i32("i32"), Some(42));
        assert_eq!(metadata.get_i32_array("i32_array"), Some(vec![7, -8]));
        assert_eq!(metadata.get_f64("f64"), Some(3.5));
        assert_eq!(metadata.get_f64_array("f64_array"), Some(vec![1.0, 2.25]));
        assert_eq!(metadata.get_string("string"), Some("from raw"));
    }

    #[test]
    fn metadata_typed_getters_reject_wrong_raw_shapes() {
        let mut metadata = Metadata::new();
        metadata.set_raw("i32", vec![1, 2, 3]).unwrap();
        metadata.set_raw("i32_array", vec![1, 2, 3]).unwrap();
        metadata.set_raw("f64", vec![1, 2, 3, 4]).unwrap();
        metadata.set_raw("f64_array", vec![1, 2, 3, 4]).unwrap();
        metadata.set_raw("string", vec![0xff]).unwrap();

        assert_eq!(metadata.get_i32("i32"), None);
        assert_eq!(metadata.get_i32_array("i32_array"), None);
        assert_eq!(metadata.get_f64("f64"), None);
        assert_eq!(metadata.get_f64_array("f64_array"), None);
        assert_eq!(metadata.get_string("string"), None);
    }

    #[test]
    fn metadata_typed_setters_reject_empty_values() {
        let mut metadata = Metadata::new();

        assert!(metadata.set_string("string", "").is_err());
        assert!(metadata.set_i32_array("i32_array", &[]).is_err());
        assert!(metadata.set_f64_array("f64_array", &[]).is_err());
    }

    #[test]
    fn metadata_typed_setters_overwrite_existing_entry() {
        let mut metadata = Metadata::new();
        metadata.set_i32("key", 5).unwrap();
        metadata.set_string("key", "value").unwrap();

        assert_eq!(metadata.get_i32("key"), None);
        assert_eq!(metadata.get_string("key"), Some("value"));
        assert_eq!(metadata.get_raw("key"), Some(b"value".as_slice()));
    }

    #[test]
    fn metadata_codec_encodes_entries_in_sorted_order() {
        let mut metadata = Metadata::new();
        metadata.set_raw("z", b"last".to_vec()).unwrap();
        metadata.set_raw("a", b"first".to_vec()).unwrap();

        let bytes = encode_metadata(&metadata);

        assert_eq!(bytes[0], 2);
        assert_eq!(bytes[1], 1);
        assert_eq!(bytes[2], b'a');
    }

    #[test]
    fn metadata_codec_roundtrips_nested_metadata() {
        let mut child = Metadata::new();
        child.set_raw("child", b"value".to_vec()).unwrap();

        let mut metadata = Metadata::new();
        metadata.insert_sub_metadata("node", child).unwrap();

        let bytes = encode_metadata(&metadata);
        let mut buffer = DecoderBuffer::new(&bytes);
        let decoded = Metadata::decode(&mut buffer).unwrap();

        assert_eq!(
            decoded
                .get_sub_metadata("node")
                .and_then(|metadata| metadata.get_raw("child")),
            Some(b"value".as_slice())
        );
    }

    #[test]
    fn geometry_metadata_roundtrips_attribute_metadata() {
        let mut attribute = Metadata::new();
        attribute.set_raw("semantic", b"POSITION".to_vec()).unwrap();

        let mut geometry = GeometryMetadata::new();
        geometry.set_attribute_metadata(7, attribute);
        geometry
            .metadata_mut()
            .set_raw("asset", b"fixture".to_vec())
            .unwrap();

        let mut bytes = EncoderBuffer::new();
        geometry.encode(&mut bytes).unwrap();

        let mut buffer = DecoderBuffer::new(bytes.data());
        let decoded = GeometryMetadata::decode(&mut buffer).unwrap();

        assert_eq!(
            decoded.metadata().get_raw("asset"),
            Some(b"fixture".as_slice())
        );
        assert_eq!(
            decoded
                .attribute_metadata_by_unique_id(7)
                .and_then(|metadata| metadata.metadata().get_raw("semantic")),
            Some(b"POSITION".as_slice())
        );
    }

    #[test]
    fn geometry_metadata_finds_attribute_metadata_by_string_entry() {
        let mut invalid_string = Metadata::new();
        invalid_string.set_raw("name", vec![0xff]).unwrap();

        let mut position = Metadata::new();
        position.set_string("name", "position").unwrap();

        let mut geometry = GeometryMetadata::new();
        geometry.set_attribute_metadata(3, invalid_string);
        geometry.set_attribute_metadata(7, position);

        assert_eq!(
            geometry
                .attribute_metadata_by_string_entry("name", "position")
                .map(AttributeMetadata::attribute_unique_id),
            Some(7)
        );
        assert!(geometry
            .attribute_metadata_by_string_entry("name", "normal")
            .is_none());
    }

    #[test]
    fn metadata_set_raw_overwrites_existing_entry() {
        let mut metadata = Metadata::new();
        metadata.set_raw("key", b"old".to_vec()).unwrap();
        metadata.set_raw("key", b"new".to_vec()).unwrap();

        assert_eq!(metadata.get_raw("key"), Some(b"new".as_slice()));
    }

    #[test]
    fn metadata_decode_rejects_duplicate_sub_metadata() {
        let bytes = [
            0, 2, // root: entries=0, sub_metadata=2
            1, b'a', 0, 0, // sub a: entries=0, sub=0
            1, b'a', 0, 0, // duplicate sub a
        ];
        let mut buffer = DecoderBuffer::new(&bytes);

        assert!(Metadata::decode(&mut buffer).is_err());
    }

    #[test]
    fn metadata_decode_rejects_zero_length_entry_value() {
        let bytes = [1, 1, b'a', 0, 0];
        let mut buffer = DecoderBuffer::new(&bytes);

        assert!(Metadata::decode(&mut buffer).is_err());
    }

    #[test]
    fn metadata_decode_rejects_truncated_name() {
        let bytes = [1, 5, b'a'];
        let mut buffer = DecoderBuffer::new(&bytes);

        assert!(Metadata::decode(&mut buffer).is_err());
    }

    #[test]
    fn metadata_decode_rejects_truncated_value() {
        let bytes = [1, 1, b'a', 4, 1, 2];
        let mut buffer = DecoderBuffer::new(&bytes);

        assert!(Metadata::decode(&mut buffer).is_err());
    }

    #[test]
    fn metadata_decode_rejects_unreasonable_sub_metadata_count() {
        let bytes = [0, 5];
        let mut buffer = DecoderBuffer::new(&bytes);

        assert!(Metadata::decode(&mut buffer).is_err());
    }

    #[test]
    fn metadata_decode_rejects_excessive_nesting() {
        let mut bytes = Vec::new();
        for _ in 0..=MAX_METADATA_NESTING_DEPTH {
            bytes.extend_from_slice(&[0, 1, 1, b'a']);
        }
        bytes.extend_from_slice(&[0, 0]);

        let mut buffer = DecoderBuffer::new(&bytes);
        assert!(Metadata::decode(&mut buffer).is_err());
    }
}
