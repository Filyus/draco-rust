use crate::{Document, Error, ResourceStore, Result};
use draco_core::draco_types::DataType;
use draco_io::{AccessorSource, DecodedAccessor, GltfError};

/// Accessor source backed by a [`Document`] and its resolved resources.
pub struct DocumentAccessorSource<'a> {
    document: &'a Document,
    resources: &'a ResourceStore,
}

/// Tightly packed accessor payload for geometry consumers.
#[derive(Clone, Debug)]
pub struct AccessorData {
    /// Number of elements in the accessor.
    pub count: usize,
    /// Original glTF accessor shape (`SCALAR`, `VEC*`, or `MAT*`).
    pub accessor_type: String,
    /// Number of scalar components per element.
    pub components: u8,
    /// Original glTF component type code.
    pub component_type: u32,
    /// Draco storage type used for materialization.
    pub data_type: DataType,
    /// Whether integer values use normalized interpretation.
    pub normalized: bool,
    /// Tightly packed accessor bytes in glTF component order.
    ///
    /// Matrix columns retain glTF's column-major order, with on-disk alignment
    /// padding removed.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
struct AccessorLayout {
    tight_width: usize,
    source_width: usize,
    columns: usize,
    column_width: usize,
    column_stride: usize,
}

impl<'a> DocumentAccessorSource<'a> {
    /// Creates an accessor source over a document and resolved buffers.
    pub fn new(document: &'a Document, resources: &'a ResourceStore) -> Self {
        Self {
            document,
            resources,
        }
    }

    /// Copies one complete buffer view from the resolved resource store.
    ///
    /// The returned bytes retain the buffer view's original layout, including
    /// accessor stride or padding. Use [`Self::read_accessor`] when a tightly
    /// packed, sparse-materialized accessor payload is needed.
    pub fn read_buffer_view(&self, index: usize) -> Result<Vec<u8>> {
        let view = self
            .document
            .as_value()
            .get("bufferViews")
            .and_then(|value| value.as_array())
            .and_then(|values| values.get(index))
            .ok_or_else(|| Error::Extension("bufferView out of range".into()))?;
        let buffer = view
            .get("buffer")
            .and_then(|value| value.as_u64())
            .and_then(|value| usize::try_from(value).ok())
            .and_then(|index| self.resources.buffers.get(index))
            .ok_or_else(|| Error::Extension("buffer is not resolved".into()))?;
        let start = view
            .get("byteOffset")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        let length = view
            .get("byteLength")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| Error::Extension("bufferView byteLength is invalid".into()))?;
        let end = start
            .checked_add(length)
            .ok_or_else(|| Error::ResourceLimit("bufferView range overflow".into()))?;
        let start = usize::try_from(start).map_err(|_| {
            Error::ResourceLimit("bufferView offset exceeds platform limits".into())
        })?;
        let end = usize::try_from(end)
            .map_err(|_| Error::ResourceLimit("bufferView end exceeds platform limits".into()))?;
        let bytes = buffer
            .get(start..end)
            .ok_or_else(|| Error::Extension("bufferView range is out of bounds".into()))?;
        let mut output = Vec::new();
        output.try_reserve_exact(bytes.len()).map_err(|_| {
            Error::ResourceLimit("bufferView materialization allocation failed".into())
        })?;
        output.extend_from_slice(bytes);
        Ok(output)
    }

    fn read<const MATRICES: bool>(
        &self,
        index: usize,
    ) -> Result<(usize, u8, u32, DataType, bool, Vec<u8>)> {
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
        let accessor_type = accessor
            .get("type")
            .and_then(|value| value.as_str())
            .ok_or_else(|| Error::Extension("accessor type is invalid".into()))?;
        let components = match accessor_type {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT2" if MATRICES => 4,
            "MAT3" if MATRICES => 9,
            "MAT4" if MATRICES => 16,
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
            5124 => DataType::Int32,
            // `DataType` has no f16 variant. Its on-disk layout is identical
            // to u16; `component` is preserved separately for packed output.
            5131 => DataType::Uint16,
            5130 => DataType::Float64,
            5134 => DataType::Int64,
            5135 => DataType::Uint64,
            _ => {
                return Err(Error::Extension(
                    "unsupported accessor component type".into(),
                ))
            }
        };
        let layout = accessor_layout::<MATRICES>(accessor_type, data_type.byte_length())?;
        let width = layout.tight_width;
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
            let (buffer, offset, stride) =
                self.buffer_view_layout(view, accessor_offset, layout.source_width)?;
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
                copy_accessor_element(buffer, start, layout, &mut dense)?;
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
            self.apply_sparse(sparse, count, layout, &mut bytes)?;
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
        layout: AccessorLayout,
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
        let (value_buffer, value_start, value_stride) =
            self.buffer_view_layout(value_view, value_offset, layout.source_width)?;
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
                    .checked_add(entry.checked_mul(value_stride).ok_or_else(|| {
                        Error::ResourceLimit("sparse value offset overflow".into())
                    })?)
                    .ok_or_else(|| Error::ResourceLimit("sparse value offset overflow".into()))?;
            let destination_start = index
                .checked_mul(layout.tight_width)
                .ok_or_else(|| Error::ResourceLimit("sparse value offset overflow".into()))?;
            let destination_end = destination_start
                .checked_add(layout.tight_width)
                .ok_or_else(|| Error::ResourceLimit("sparse value offset overflow".into()))?;
            copy_accessor_element_into(
                value_buffer,
                value_start,
                layout,
                &mut bytes[destination_start..destination_end],
            )?;
        }
        Ok(())
    }

    /// Reads and materializes one accessor by zero-based index.
    #[cfg(feature = "accessors")]
    pub fn read_accessor(&self, index: usize) -> Result<AccessorData> {
        self.read_accessor_inner::<true>(index)
    }

    pub(crate) fn read_geometry_accessor(&self, index: usize) -> Result<AccessorData> {
        self.read_accessor_inner::<false>(index)
    }

    fn read_accessor_inner<const MATRICES: bool>(&self, index: usize) -> Result<AccessorData> {
        let (count, components, component_type, data_type, normalized, bytes) =
            self.read::<MATRICES>(index)?;
        let accessor_type = self
            .document
            .as_value()
            .get("accessors")
            .and_then(|value| value.as_array())
            .and_then(|values| values.get(index))
            .and_then(|value| value.get("type"))
            .and_then(|value| value.as_str())
            .ok_or_else(|| Error::Extension("accessor type is invalid".into()))?
            .to_owned();
        Ok(AccessorData {
            count,
            accessor_type,
            components,
            component_type,
            data_type,
            normalized,
            bytes,
        })
    }
}

fn accessor_layout<const MATRICES: bool>(
    accessor_type: &str,
    component_width: usize,
) -> Result<AccessorLayout> {
    let (columns, rows) = match accessor_type {
        "SCALAR" => (1, 1),
        "VEC2" => (1, 2),
        "VEC3" => (1, 3),
        "VEC4" => (1, 4),
        "MAT2" if MATRICES => (2, 2),
        "MAT3" if MATRICES => (3, 3),
        "MAT4" if MATRICES => (4, 4),
        _ => return Err(Error::Extension("accessor type is invalid".into())),
    };
    let column_width = rows * component_width;
    let column_stride = if columns > 1 && component_width < 4 {
        column_width
            .checked_add(3)
            .map(|width| width & !3)
            .ok_or_else(|| Error::ResourceLimit("accessor column size overflow".into()))?
    } else {
        column_width
    };
    Ok(AccessorLayout {
        tight_width: columns * column_width,
        source_width: columns * column_stride,
        columns,
        column_width,
        column_stride,
    })
}

fn copy_accessor_element(
    source: &[u8],
    start: usize,
    layout: AccessorLayout,
    destination: &mut Vec<u8>,
) -> Result<()> {
    let destination_start = destination.len();
    destination
        .try_reserve_exact(layout.tight_width)
        .map_err(|_| Error::ResourceLimit("accessor materialization allocation failed".into()))?;
    let destination_end = destination_start
        .checked_add(layout.tight_width)
        .ok_or_else(|| Error::ResourceLimit("accessor materialization size overflow".into()))?;
    destination.resize(destination_end, 0);
    copy_accessor_element_into(source, start, layout, &mut destination[destination_start..])
}

fn copy_accessor_element_into(
    source: &[u8],
    start: usize,
    layout: AccessorLayout,
    destination: &mut [u8],
) -> Result<()> {
    if destination.len() != layout.tight_width {
        return Err(Error::ResourceLimit(
            "accessor materialization size mismatch".into(),
        ));
    }
    for column in 0..layout.columns {
        let column_start =
            start
                .checked_add(column.checked_mul(layout.column_stride).ok_or_else(|| {
                    Error::ResourceLimit("accessor column offset overflow".into())
                })?)
                .ok_or_else(|| Error::ResourceLimit("accessor column offset overflow".into()))?;
        let column_end = column_start
            .checked_add(layout.column_width)
            .filter(|end| *end <= source.len())
            .ok_or_else(|| Error::Extension("accessor range is out of bounds".into()))?;
        let destination_start = column * layout.column_width;
        let destination_end = destination_start + layout.column_width;
        destination[destination_start..destination_end]
            .copy_from_slice(&source[column_start..column_end]);
    }
    Ok(())
}
impl AccessorSource for DocumentAccessorSource<'_> {
    fn read_attribute(
        &self,
        index: usize,
        expected: &[&str],
        allowed: &[u32],
    ) -> std::result::Result<DecodedAccessor, GltfError> {
        let (count, c, _, t, n, b) = self
            .read::<false>(index)
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
            .read::<false>(index)
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
