#[cfg(feature = "decoder")]
use crate::decoder_buffer::DecoderBuffer;
#[cfg(feature = "encoder")]
use crate::encoder_buffer::EncoderBuffer;
use crate::status::DracoError;
use std::collections::BTreeMap;

pub const METADATA_FLAG_MASK: u16 = 0x8000;

#[cfg(feature = "decoder")]
const MAX_METADATA_NESTING_DEPTH: usize = 1000;
const MAX_METADATA_NAME_LEN: usize = u8::MAX as usize;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    entries: BTreeMap<String, Vec<u8>>,
    sub_metadata: BTreeMap<String, Metadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeMetadata {
    attribute_unique_id: u32,
    metadata: Metadata,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeometryMetadata {
    metadata: Metadata,
    attribute_metadata: Vec<AttributeMetadata>,
}

impl Metadata {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.sub_metadata.is_empty()
    }

    pub fn entries(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.entries
    }

    pub fn sub_metadata(&self) -> &BTreeMap<String, Metadata> {
        &self.sub_metadata
    }

    pub fn get_raw(&self, name: &str) -> Option<&[u8]> {
        self.entries.get(name).map(Vec::as_slice)
    }

    pub fn set_raw(
        &mut self,
        name: impl Into<String>,
        value: impl Into<Vec<u8>>,
    ) -> Result<(), DracoError> {
        let name = name.into();
        validate_metadata_name(&name)?;
        let value = value.into();
        if value.is_empty() {
            return Err(DracoError::InvalidParameter(
                "Metadata entry value must not be empty".to_string(),
            ));
        }
        self.entries.insert(name, value);
        Ok(())
    }

    pub fn remove_entry(&mut self, name: &str) -> Option<Vec<u8>> {
        self.entries.remove(name)
    }

    pub fn get_sub_metadata(&self, name: &str) -> Option<&Metadata> {
        self.sub_metadata.get(name)
    }

    pub fn get_sub_metadata_mut(&mut self, name: &str) -> Option<&mut Metadata> {
        self.sub_metadata.get_mut(name)
    }

    pub fn insert_sub_metadata(
        &mut self,
        name: impl Into<String>,
        metadata: Metadata,
    ) -> Result<(), DracoError> {
        let name = name.into();
        validate_metadata_name(&name)?;
        if self.sub_metadata.contains_key(&name) {
            return Err(DracoError::DracoError(format!(
                "Duplicate sub-metadata name: {name}"
            )));
        }
        self.sub_metadata.insert(name, metadata);
        Ok(())
    }

    #[cfg(feature = "decoder")]
    pub fn decode(buffer: &mut DecoderBuffer<'_>) -> Result<Self, DracoError> {
        Self::decode_with_depth(buffer, 0)
    }

    #[cfg(feature = "decoder")]
    fn decode_with_depth(buffer: &mut DecoderBuffer<'_>, depth: usize) -> Result<Self, DracoError> {
        if depth > MAX_METADATA_NESTING_DEPTH {
            return Err(DracoError::DracoError(
                "Metadata nesting depth exceeded".to_string(),
            ));
        }

        let num_entries = decode_bounded_count(buffer, "metadata entries count")?;
        let mut metadata = Metadata::new();
        for _ in 0..num_entries {
            let name = decode_name(buffer)?;
            let data_size = usize::try_from(buffer.decode_varint()?).map_err(|_| {
                DracoError::DracoError("Metadata entry data size too large".to_string())
            })?;
            if data_size == 0 {
                return Err(DracoError::DracoError(
                    "Invalid metadata entry data size".to_string(),
                ));
            }
            if data_size > buffer.remaining_size() {
                return Err(DracoError::DracoError(
                    "Failed to read metadata entry value".to_string(),
                ));
            }
            let value = buffer.decode_slice(data_size)?.to_vec();
            metadata.entries.insert(name, value);
        }

        let num_sub_metadata = decode_bounded_count(buffer, "sub-metadata count")?;
        if num_sub_metadata > buffer.remaining_size() {
            return Err(DracoError::DracoError(
                "Invalid sub-metadata count".to_string(),
            ));
        }
        for _ in 0..num_sub_metadata {
            let name = decode_name(buffer)?;
            if metadata.sub_metadata.contains_key(&name) {
                return Err(DracoError::DracoError(format!(
                    "Duplicate sub-metadata name: {name}"
                )));
            }
            let sub_metadata = Self::decode_with_depth(buffer, depth + 1)?;
            metadata.sub_metadata.insert(name, sub_metadata);
        }

        Ok(metadata)
    }

    #[cfg(feature = "encoder")]
    pub fn encode(&self, buffer: &mut EncoderBuffer) -> Result<(), DracoError> {
        buffer.encode_varint(self.entries.len() as u64);
        for (name, value) in &self.entries {
            encode_name(buffer, name)?;
            if value.is_empty() {
                return Err(DracoError::InvalidParameter(
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
    pub fn new(attribute_unique_id: u32, metadata: Metadata) -> Self {
        Self {
            attribute_unique_id,
            metadata,
        }
    }

    pub fn attribute_unique_id(&self) -> u32 {
        self.attribute_unique_id
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn metadata_mut(&mut self) -> &mut Metadata {
        &mut self.metadata
    }
}

impl GeometryMetadata {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.metadata.is_empty() && self.attribute_metadata.is_empty()
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    pub fn metadata_mut(&mut self) -> &mut Metadata {
        &mut self.metadata
    }

    pub fn attribute_metadata(&self) -> &[AttributeMetadata] {
        &self.attribute_metadata
    }

    pub fn attribute_metadata_by_unique_id(
        &self,
        attribute_unique_id: u32,
    ) -> Option<&AttributeMetadata> {
        self.attribute_metadata
            .iter()
            .find(|metadata| metadata.attribute_unique_id == attribute_unique_id)
    }

    pub fn attribute_metadata_by_unique_id_mut(
        &mut self,
        attribute_unique_id: u32,
    ) -> Option<&mut AttributeMetadata> {
        self.attribute_metadata
            .iter_mut()
            .find(|metadata| metadata.attribute_unique_id == attribute_unique_id)
    }

    pub fn set_attribute_metadata(&mut self, attribute_unique_id: u32, metadata: Metadata) {
        if let Some(existing) = self.attribute_metadata_by_unique_id_mut(attribute_unique_id) {
            existing.metadata = metadata;
        } else {
            self.attribute_metadata
                .push(AttributeMetadata::new(attribute_unique_id, metadata));
        }
    }

    #[cfg(feature = "decoder")]
    pub fn decode(buffer: &mut DecoderBuffer<'_>) -> Result<Self, DracoError> {
        let num_attribute_metadata = decode_bounded_count(buffer, "attribute metadata count")?;
        let mut geometry_metadata = GeometryMetadata::new();
        geometry_metadata
            .attribute_metadata
            .try_reserve_exact(num_attribute_metadata)
            .map_err(|_| DracoError::DracoError("Failed to allocate metadata".to_string()))?;

        for _ in 0..num_attribute_metadata {
            let attribute_unique_id = u32::try_from(buffer.decode_varint()?).map_err(|_| {
                DracoError::DracoError("Attribute metadata unique id too large".to_string())
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
    pub fn encode(&self, buffer: &mut EncoderBuffer) -> Result<(), DracoError> {
        buffer.encode_varint(self.attribute_metadata.len() as u64);
        for attribute_metadata in &self.attribute_metadata {
            buffer.encode_varint(attribute_metadata.attribute_unique_id as u64);
            attribute_metadata.metadata.encode(buffer)?;
        }
        self.metadata.encode(buffer)
    }
}

fn validate_metadata_name(name: &str) -> Result<(), DracoError> {
    if name.len() > MAX_METADATA_NAME_LEN {
        return Err(DracoError::InvalidParameter(format!(
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
        return Err(DracoError::DracoError(
            "Failed to read metadata name".to_string(),
        ));
    }
    let name = buffer.decode_slice(name_len)?.to_vec();
    String::from_utf8(name)
        .map_err(|_| DracoError::DracoError("Invalid UTF-8 metadata name".to_string()))
}

#[cfg(feature = "decoder")]
fn decode_bounded_count(
    buffer: &mut DecoderBuffer<'_>,
    label: &'static str,
) -> Result<usize, DracoError> {
    let count = usize::try_from(buffer.decode_varint()?)
        .map_err(|_| DracoError::DracoError(format!("{label} too large")))?;
    if count > buffer.remaining_size() {
        return Err(DracoError::DracoError(format!("Invalid {label}")));
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
