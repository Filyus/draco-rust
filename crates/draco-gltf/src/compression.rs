use draco_io::{decode_geometry, AccessorSource, DecodedAccessor};

use super::*;

/// Full-scene compression implementation behind [`Import::compress`].
pub(super) fn compress_document(
    import: &Import,
    options: &GltfCompressionOptions,
) -> Result<CompressionOutput<Vec<u8>>> {
    validate(&import.document)?;
    validate_resolved_buffers(&import.document, &import.buffers)?;
    let doc_value = import.canonical_document()?;

    // Pre-extract each primitive's (mode, semantic -> accessor index, indices)
    // from the JSON before `doc_value` is moved into the compressor. The JSON
    // carries the complete attribute set, including custom `_*` semantics that
    // the gltf-rs typed model would hide without the `extras` feature.
    let descriptors = primitive_descriptors(&doc_value)?;

    let source_document = doc_value.clone();
    let source = JsonAccessorSource {
        document: &source_document,
        buffers: &import.buffers,
    };
    let output = draco_io::compress_gltf_value(
        doc_value,
        &import.buffers,
        options,
        |mesh_idx, prim_idx| {
            let (mode, attributes, indices) = descriptors
                .get(&(mesh_idx, prim_idx))
                .ok_or_else(|| GltfError::InvalidGltf("primitive descriptor missing".into()))?;
            decode_geometry(&source, *mode, attributes, *indices)
        },
    )?;
    let (document, bin) = output.data;
    let data = draco_io::serialize_gltf_document(
        &document,
        &bin,
        import.input_format,
        options.output_format,
    )?;
    Ok(CompressionOutput {
        data,
        report: output.report,
    })
}

/// One primitive's geometry: `(mode, [(semantic, accessor index)], indices accessor)`.
type PrimitiveDescriptor = (u32, Vec<(String, usize)>, Option<usize>);

/// Collects each primitive's [`PrimitiveDescriptor`] from the glTF JSON, keyed by
/// `(mesh index, primitive index)`.
fn primitive_descriptors(
    doc: &Value,
) -> Result<std::collections::HashMap<(usize, usize), PrimitiveDescriptor>> {
    let mut out = std::collections::HashMap::new();
    let Some(meshes) = doc.get("meshes").and_then(Value::as_array) else {
        return Ok(out);
    };
    let mut primitive_count = 0usize;
    for mesh in meshes {
        primitive_count = primitive_count
            .checked_add(
                mesh.get("primitives")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len),
            )
            .ok_or_else(|| Error::ResourceLimit("primitive count overflow".into()))?;
    }
    out.try_reserve(primitive_count)
        .map_err(|_| Error::ResourceLimit("failed to allocate primitive descriptors".into()))?;
    for (mesh_idx, mesh) in meshes.iter().enumerate() {
        let Some(primitives) = mesh.get("primitives").and_then(Value::as_array) else {
            continue;
        };
        for (prim_idx, prim) in primitives.iter().enumerate() {
            let mode = u32::try_from(prim.get("mode").and_then(Value::as_u64).unwrap_or(4))
                .map_err(|_| GltfError::InvalidGltf("primitive mode exceeds u32".into()))?;
            let attribute_map = prim
                .get("attributes")
                .and_then(Value::as_object)
                .ok_or_else(|| GltfError::InvalidGltf("primitive attributes are missing".into()))?;
            let mut attributes = Vec::new();
            attributes
                .try_reserve_exact(attribute_map.len())
                .map_err(|_| {
                    Error::ResourceLimit(
                        "failed to allocate primitive attribute descriptors".into(),
                    )
                })?;
            for (semantic, accessor) in attribute_map {
                let accessor = accessor
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| {
                        GltfError::InvalidGltf(format!("{semantic} accessor index is invalid"))
                    })?;
                let mut semantic_copy = String::new();
                semantic_copy
                    .try_reserve_exact(semantic.len())
                    .map_err(|_| {
                        Error::ResourceLimit("failed to allocate attribute semantic".into())
                    })?;
                semantic_copy.push_str(semantic);
                attributes.push((semantic_copy, accessor));
            }
            let indices = prim
                .get("indices")
                .and_then(Value::as_u64)
                .map(usize::try_from)
                .transpose()
                .map_err(|_| GltfError::InvalidGltf("indices accessor exceeds usize".into()))?;
            out.insert((mesh_idx, prim_idx), (mode, attributes, indices));
        }
    }
    Ok(out)
}

/// [`AccessorSource`] over the lossless JSON document.
struct JsonAccessorSource<'a> {
    document: &'a Value,
    buffers: &'a [Vec<u8>],
}

impl JsonAccessorSource<'_> {
    fn accessor(&self, index: usize) -> std::result::Result<&Value, GltfError> {
        self.document["accessors"]
            .as_array()
            .and_then(|accessors| accessors.get(index))
            .ok_or_else(|| GltfError::InvalidGltf(format!("accessor {index} out of range")))
    }

    /// Copies `row`-sized elements (stride removed) out of the accessor's buffer
    /// view into a tightly packed block, `accessor.count()` rows total.
    fn extract(&self, accessor: &Value, row: usize) -> std::result::Result<Vec<u8>, GltfError> {
        let overflow = || GltfError::InvalidGltf("accessor range overflow".into());
        if accessor.get("sparse").is_some() {
            return Err(GltfError::Unsupported(
                "sparse accessors are not supported".into(),
            ));
        }
        let view_index = accessor
            .get("bufferView")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| GltfError::InvalidGltf("accessor has no bufferView".into()))?;
        let view = self.document["bufferViews"]
            .as_array()
            .and_then(|views| views.get(view_index))
            .ok_or_else(|| GltfError::InvalidGltf("accessor bufferView out of range".into()))?;
        let buffer_index = view
            .get("buffer")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| GltfError::InvalidGltf("bufferView buffer is invalid".into()))?;
        let buffer = self
            .buffers
            .get(buffer_index)
            .ok_or_else(|| GltfError::InvalidGltf("buffer not resolved".into()))?;
        let view_offset = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let view_length = view
            .get("byteLength")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| GltfError::InvalidGltf("bufferView byteLength is invalid".into()))?;
        let view_end = view_offset.checked_add(view_length).ok_or_else(overflow)?;
        if view_end > buffer.len() {
            return Err(GltfError::InvalidGltf(
                "bufferView exceeds its declared or resolved buffer".into(),
            ));
        }
        let stride = view
            .get("byteStride")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .unwrap_or(row);
        if stride < row {
            return Err(GltfError::InvalidGltf(format!(
                "accessor stride {stride} is smaller than element size {row}"
            )));
        }
        let offset = accessor
            .get("byteOffset")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        let base = view_offset.checked_add(offset).ok_or_else(overflow)?;
        let count = accessor
            .get("count")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| GltfError::InvalidGltf("accessor count is invalid".into()))?;
        let output_len = count.checked_mul(row).ok_or_else(overflow)?;
        if count > 0 {
            let last = base
                .checked_add((count - 1).checked_mul(stride).ok_or_else(overflow)?)
                .and_then(|start| start.checked_add(row))
                .ok_or_else(overflow)?;
            if last > view_end || last > buffer.len() {
                return Err(GltfError::InvalidGltf("accessor out of bounds".into()));
            }
        } else if base > view_end {
            return Err(GltfError::InvalidGltf("accessor out of bounds".into()));
        }

        let mut out = Vec::new();
        out.try_reserve_exact(output_len)
            .map_err(|_| GltfError::ResourceLimitExceeded("accessor allocation failed".into()))?;
        for i in 0..count {
            let start = base
                .checked_add(i.checked_mul(stride).ok_or_else(overflow)?)
                .ok_or_else(overflow)?;
            let end = start.checked_add(row).ok_or_else(overflow)?;
            if end > view_end || end > buffer.len() {
                return Err(GltfError::InvalidGltf("accessor out of bounds".into()));
            }
            out.extend_from_slice(&buffer[start..end]);
        }
        Ok(out)
    }
}

impl AccessorSource for JsonAccessorSource<'_> {
    fn read_attribute(
        &self,
        accessor_idx: usize,
        expected_types: &[&str],
        allowed_component_types: &[u32],
    ) -> std::result::Result<DecodedAccessor, GltfError> {
        let accessor = self.accessor(accessor_idx)?;

        let type_str = accessor
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| GltfError::InvalidGltf("accessor type is invalid".into()))?;
        if !expected_types.contains(&type_str) {
            return Err(GltfError::InvalidGltf(format!(
                "expected one of {expected_types:?} accessor, got {type_str}"
            )));
        }
        let gl_enum = accessor
            .get("componentType")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| GltfError::InvalidGltf("accessor componentType is invalid".into()))?;
        if !allowed_component_types.contains(&gl_enum) {
            return Err(GltfError::Unsupported(format!(
                "unsupported {type_str} component type {gl_enum}"
            )));
        }

        let num_components = match type_str {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => {
                return Err(GltfError::Unsupported(
                    "matrix accessor not supported".into(),
                ))
            }
        };
        let component_size = component_size(gl_enum)?;
        let row = (num_components as usize)
            .checked_mul(component_size)
            .ok_or_else(|| GltfError::InvalidGltf("accessor row size overflow".into()))?;
        let bytes = self.extract(&accessor, row)?;

        DecodedAccessor::new(
            accessor
                .get("count")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| GltfError::InvalidGltf("accessor count is invalid".into()))?,
            num_components,
            draco_data_type(gl_enum)?,
            accessor
                .get("normalized")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            bytes,
        )
    }

    fn read_indices(&self, accessor_idx: usize) -> std::result::Result<Vec<u32>, GltfError> {
        let accessor = self.accessor(accessor_idx)?;
        if accessor.get("type").and_then(Value::as_str) != Some("SCALAR") {
            return Err(GltfError::InvalidGltf(
                "indices accessor must be SCALAR".into(),
            ));
        }
        let component = accessor
            .get("componentType")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| GltfError::InvalidGltf("indices componentType is invalid".into()))?;
        let bytes = self.extract(accessor, component_size(component)?)?;
        let mut indices = Vec::new();
        let count = accessor
            .get("count")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| GltfError::InvalidGltf("indices count is invalid".into()))?;
        indices.try_reserve_exact(count).map_err(|_| {
            GltfError::ResourceLimitExceeded("index accessor allocation failed".into())
        })?;
        match component {
            5121 => indices.extend(bytes.iter().map(|&byte| u32::from(byte))),
            5123 => indices.extend(
                bytes
                    .chunks_exact(2)
                    .map(|chunk| u32::from(u16::from_le_bytes([chunk[0], chunk[1]]))),
            ),
            5125 => indices.extend(
                bytes
                    .chunks_exact(4)
                    .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])),
            ),
            other => {
                return Err(GltfError::Unsupported(format!(
                    "unsupported index component type {other:?}"
                )))
            }
        }
        Ok(indices)
    }
}

/// Maps a gltf-rs accessor dimension to its glTF type string, rejecting the
/// matrix types Draco geometry attributes never use.
fn component_size(component: u32) -> std::result::Result<usize, GltfError> {
    match component {
        5120 | 5121 => Ok(1),
        5122 | 5123 => Ok(2),
        5125 | 5126 => Ok(4),
        _ => Err(GltfError::Unsupported(format!(
            "unsupported Draco component type {component}"
        ))),
    }
}

fn draco_data_type(
    component: u32,
) -> std::result::Result<draco_core::draco_types::DataType, GltfError> {
    use draco_core::draco_types::DataType as D;
    Ok(match component {
        5120 => D::Int8,
        5121 => D::Uint8,
        5122 => D::Int16,
        5123 => D::Uint16,
        5125 => D::Uint32,
        5126 => D::Float32,
        _ => {
            return Err(GltfError::Unsupported(format!(
                "unsupported Draco component type {component}"
            )))
        }
    })
}
