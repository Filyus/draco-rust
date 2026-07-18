use crate::{Document, Error, ResourceStore, Result};
use draco_core::draco_types::DataType;
use draco_io::{AccessorSource, DecodedAccessor, GltfError};

pub struct DocumentAccessorSource<'a> {
    document: &'a Document,
    resources: &'a ResourceStore,
}

/// Tightly packed accessor payload for compact consumers.
#[derive(Clone, Debug)]
pub struct AccessorData {
    pub count: usize,
    pub components: u8,
    pub component_type: u32,
    pub data_type: DataType,
    pub normalized: bool,
    pub bytes: Vec<u8>,
}
impl<'a> DocumentAccessorSource<'a> {
    pub fn new(document: &'a Document, resources: &'a ResourceStore) -> Self {
        Self {
            document,
            resources,
        }
    }
    fn read(&self, index: usize) -> Result<(usize, u8, u32, DataType, bool, Vec<u8>)> {
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
            // `DataType` has no f16 variant. Its on-disk layout is identical
            // to u16; `component` is preserved separately for packed output.
            5131 => DataType::Uint16,
            5132 => DataType::Float64,
            5133 => DataType::Int64,
            5134 => DataType::Uint64,
            _ => {
                return Err(Error::Extension(
                    "unsupported accessor component type".into(),
                ))
            }
        };
        let width = components as usize * data_type.byte_length();
        let byte_len = count
            .checked_mul(width)
            .ok_or_else(|| Error::ResourceLimit("accessor byte size overflow".into()))?;
        let mut bytes = if let Some(view) = accessor
            .get("bufferView")
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
        {
            let accessor_offset = accessor
                .get("byteOffset")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let (buffer, offset, stride) = self.buffer_view_layout(view, accessor_offset, width)?;
            let mut dense = Vec::new();
            dense.try_reserve_exact(byte_len).map_err(|_| {
                Error::ResourceLimit("accessor materialization allocation failed".into())
            })?;
            for row in 0..count {
                let start =
                    offset
                        .checked_add(row.checked_mul(stride).ok_or_else(|| {
                            Error::ResourceLimit("accessor stride overflow".into())
                        })?)
                        .ok_or_else(|| Error::ResourceLimit("accessor offset overflow".into()))?;
                let end = start
                    .checked_add(width)
                    .filter(|end| *end <= buffer.len())
                    .ok_or_else(|| Error::Extension("accessor range is out of bounds".into()))?;
                dense.extend_from_slice(&buffer[start..end]);
            }
            dense
        } else if accessor.get("sparse").is_some() {
            vec![0; byte_len]
        } else {
            return Err(Error::Extension(
                "accessor has neither bufferView nor sparse values".into(),
            ));
        };
        if let Some(sparse) = accessor.get("sparse") {
            self.apply_sparse(sparse, count, width, &mut bytes)?;
        }
        Ok((
            count,
            components,
            component as u32,
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

    fn buffer_view_layout(
        &self,
        view_index: usize,
        additional_offset: u64,
        default_stride: usize,
    ) -> Result<(&[u8], usize, usize)> {
        let view = self
            .document
            .as_value()
            .get("bufferViews")
            .and_then(|value| value.as_array())
            .and_then(|values| values.get(view_index))
            .ok_or_else(|| Error::Extension("bufferView out of range".into()))?;
        let buffer = view
            .get("buffer")
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
            .and_then(|index| self.resources.buffers.get(index))
            .ok_or_else(|| Error::Extension("buffer is not resolved".into()))?;
        let offset = view
            .get("byteOffset")
            .and_then(|value| value.as_u64())
            .unwrap_or(0)
            .checked_add(additional_offset)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| Error::ResourceLimit("bufferView offset is invalid".into()))?;
        let stride = view
            .get("byteStride")
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(default_stride);
        if stride < default_stride {
            return Err(Error::Extension(
                "bufferView byteStride is too small".into(),
            ));
        }
        Ok((buffer, offset, stride))
    }

    fn apply_sparse(
        &self,
        sparse: &crate::JsonValue,
        count: usize,
        width: usize,
        bytes: &mut [u8],
    ) -> Result<()> {
        let sparse_count = sparse
            .get("count")
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value <= count)
            .ok_or_else(|| Error::Extension("sparse accessor count is invalid".into()))?;
        let indices = sparse
            .get("indices")
            .ok_or_else(|| Error::Extension("sparse accessor indices are missing".into()))?;
        let index_view = indices
            .get("bufferView")
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| Error::Extension("sparse indices bufferView is invalid".into()))?;
        let index_type = indices
            .get("componentType")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| Error::Extension("sparse indices componentType is invalid".into()))?;
        let index_width = match index_type {
            5121 => 1,
            5123 => 2,
            5125 => 4,
            _ => {
                return Err(Error::Extension(
                    "sparse indices componentType is invalid".into(),
                ))
            }
        };
        let index_offset = indices
            .get("byteOffset")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let (index_buffer, index_start, _) =
            self.buffer_view_layout(index_view, index_offset, index_width)?;
        let values = sparse
            .get("values")
            .ok_or_else(|| Error::Extension("sparse accessor values are missing".into()))?;
        let value_view = values
            .get("bufferView")
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| Error::Extension("sparse values bufferView is invalid".into()))?;
        let value_offset = values
            .get("byteOffset")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let (value_buffer, value_start, _) =
            self.buffer_view_layout(value_view, value_offset, width)?;
        let mut previous = None;
        for entry in 0..sparse_count {
            let index_start =
                index_start
                    .checked_add(entry.checked_mul(index_width).ok_or_else(|| {
                        Error::ResourceLimit("sparse index offset overflow".into())
                    })?)
                    .ok_or_else(|| Error::ResourceLimit("sparse index offset overflow".into()))?;
            let index_end = index_start
                .checked_add(index_width)
                .filter(|end| *end <= index_buffer.len())
                .ok_or_else(|| Error::Extension("sparse indices are out of bounds".into()))?;
            let index = match index_type {
                5121 => index_buffer[index_start] as usize,
                5123 => {
                    u16::from_le_bytes(index_buffer[index_start..index_end].try_into().unwrap())
                        as usize
                }
                5125 => {
                    u32::from_le_bytes(index_buffer[index_start..index_end].try_into().unwrap())
                        as usize
                }
                _ => unreachable!(),
            };
            if index >= count || previous.is_some_and(|previous| index <= previous) {
                return Err(Error::Extension(
                    "sparse indices must be strictly increasing".into(),
                ));
            }
            previous = Some(index);
            let value_start =
                value_start
                    .checked_add(entry.checked_mul(width).ok_or_else(|| {
                        Error::ResourceLimit("sparse value offset overflow".into())
                    })?)
                    .ok_or_else(|| Error::ResourceLimit("sparse value offset overflow".into()))?;
            let value_end = value_start
                .checked_add(width)
                .filter(|end| *end <= value_buffer.len())
                .ok_or_else(|| Error::Extension("sparse values are out of bounds".into()))?;
            bytes[index * width..(index + 1) * width]
                .copy_from_slice(&value_buffer[value_start..value_end]);
        }
        Ok(())
    }

    pub fn read_accessor(&self, index: usize) -> Result<AccessorData> {
        let (count, components, component_type, data_type, normalized, bytes) = self.read(index)?;
        Ok(AccessorData {
            count,
            components,
            component_type,
            data_type,
            normalized,
            bytes,
        })
    }
}
impl AccessorSource for DocumentAccessorSource<'_> {
    fn read_attribute(
        &self,
        index: usize,
        expected: &[&str],
        allowed: &[u32],
    ) -> std::result::Result<DecodedAccessor, GltfError> {
        let (count, c, _, t, n, b) = self
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
        let (count, c, component_type, t, _, b) = self
            .read(index)
            .map_err(|e| GltfError::InvalidGltf(e.to_string()))?;
        if c != 1 {
            return Err(GltfError::InvalidGltf("indices must be SCALAR".into()));
        }
        if !matches!(component_type, 5121 | 5123 | 5125) {
            return Err(GltfError::Unsupported("index component type".into()));
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
