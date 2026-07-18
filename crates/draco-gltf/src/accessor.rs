use crate::{Document, Error, ResourceStore, Result};
use draco_core::draco_types::DataType;
use draco_io::{AccessorSource, DecodedAccessor, GltfError};

pub struct NativeAccessorSource<'a> {
    document: &'a Document,
    resources: &'a ResourceStore,
}

/// Tightly packed accessor payload for compact consumers.
#[derive(Clone, Debug)]
pub struct AccessorData {
    pub count: usize,
    pub components: u8,
    pub data_type: DataType,
    pub normalized: bool,
    pub bytes: Vec<u8>,
}
impl<'a> NativeAccessorSource<'a> {
    pub fn new(document: &'a Document, resources: &'a ResourceStore) -> Self {
        Self {
            document,
            resources,
        }
    }
    fn read(&self, index: usize) -> Result<(usize, u8, DataType, bool, Vec<u8>)> {
        let accessor = self
            .document
            .as_value()
            .get("accessors")
            .and_then(|v| v.as_array())
            .and_then(|v| v.get(index))
            .ok_or_else(|| Error::Extension("accessor out of range".into()))?;
        let count = accessor
            .get("count")
            .and_then(|v| v.as_u64())
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| Error::Extension("accessor count is invalid".into()))?;
        let components = match accessor.get("type").and_then(|v| v.as_str()) {
            Some("SCALAR") => 1,
            Some("VEC2") => 2,
            Some("VEC3") => 3,
            Some("VEC4") => 4,
            Some("MAT2") => 4,
            Some("MAT3") => 9,
            Some("MAT4") => 16,
            _ => return Err(Error::Extension("accessor type is invalid".into())),
        };
        let component = accessor
            .get("componentType")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Extension("accessor componentType is invalid".into()))?;
        let data_type = match component {
            5120 => DataType::Int8,
            5121 => DataType::Uint8,
            5122 => DataType::Int16,
            5123 => DataType::Uint16,
            5125 => DataType::Uint32,
            5126 => DataType::Float32,
            5127 => DataType::Int32,
            5132 => DataType::Float64,
            5133 => DataType::Int64,
            5134 => DataType::Uint64,
            _ => {
                return Err(Error::Extension(
                    "unsupported accessor component type".into(),
                ))
            }
        };
        let view = accessor
            .get("bufferView")
            .and_then(|v| v.as_u64())
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| {
                Error::Extension("sparse and detached accessors are not supported".into())
            })?;
        let view = self
            .document
            .as_value()
            .get("bufferViews")
            .and_then(|v| v.as_array())
            .and_then(|v| v.get(view))
            .ok_or_else(|| Error::Extension("bufferView out of range".into()))?;
        let buffer = view
            .get("buffer")
            .and_then(|v| v.as_u64())
            .and_then(|v| usize::try_from(v).ok())
            .and_then(|v| self.resources.buffers.get(v))
            .ok_or_else(|| Error::Extension("buffer is not resolved".into()))?;
        let offset = view.get("byteOffset").and_then(|v| v.as_u64()).unwrap_or(0) as usize
            + accessor
                .get("byteOffset")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
        let width = components as usize * data_type.byte_length();
        let stride = view
            .get("byteStride")
            .and_then(|v| v.as_u64())
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(width);
        let mut bytes = Vec::with_capacity(count * width);
        for row in 0..count {
            let start = offset + row * stride;
            let end = start + width;
            bytes.extend_from_slice(
                buffer
                    .get(start..end)
                    .ok_or_else(|| Error::Extension("accessor range is out of bounds".into()))?,
            );
        }
        Ok((
            count,
            components,
            data_type,
            accessor
                .get("normalized")
                .and_then(|v| {
                    if let crate::JsonValue::Bool(v) = v {
                        Some(*v)
                    } else {
                        None
                    }
                })
                .unwrap_or(false),
            bytes,
        ))
    }

    pub fn read_accessor(&self, index: usize) -> Result<AccessorData> {
        let (count, components, data_type, normalized, bytes) = self.read(index)?;
        Ok(AccessorData {
            count,
            components,
            data_type,
            normalized,
            bytes,
        })
    }
}
impl AccessorSource for NativeAccessorSource<'_> {
    fn read_attribute(
        &self,
        index: usize,
        expected: &[&str],
        allowed: &[u32],
    ) -> std::result::Result<DecodedAccessor, GltfError> {
        let (count, c, t, n, b) = self
            .read(index)
            .map_err(|e| GltfError::InvalidGltf(e.to_string()))?;
        let a = &self.document.as_value()["accessors"][index];
        if !expected.contains(&a.get("type").and_then(|v| v.as_str()).unwrap_or(""))
            || !allowed
                .contains(&(a.get("componentType").and_then(|v| v.as_u64()).unwrap_or(0) as u32))
        {
            return Err(GltfError::Unsupported(
                "accessor layout is not permitted".into(),
            ));
        };
        DecodedAccessor::new(count, c, t, n, b)
    }
    fn read_indices(&self, index: usize) -> std::result::Result<Vec<u32>, GltfError> {
        let (count, c, t, _, b) = self
            .read(index)
            .map_err(|e| GltfError::InvalidGltf(e.to_string()))?;
        if c != 1 {
            return Err(GltfError::InvalidGltf("indices must be SCALAR".into()));
        }
        let w = t.byte_length();
        (0..count)
            .map(|i| {
                let s = &b[i * w..(i + 1) * w];
                Ok(match t {
                    DataType::Uint8 => s[0] as u32,
                    DataType::Uint16 => u16::from_le_bytes([s[0], s[1]]) as u32,
                    DataType::Uint32 => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                    _ => return Err(GltfError::Unsupported("index component type".into())),
                })
            })
            .collect()
    }
}
