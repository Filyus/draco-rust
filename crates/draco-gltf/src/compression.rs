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

    let source = GltfRsSource {
        document: &import.document,
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

/// [`AccessorSource`] over a parsed gltf-rs document: it copies attribute and
/// index bytes straight out of the accessors' buffer views, reusing gltf-rs's
/// already-parsed accessor metadata instead of `draco-io`'s glTF reader.
struct GltfRsSource<'a> {
    document: &'a gltf::Document,
    buffers: &'a [Vec<u8>],
}

impl GltfRsSource<'_> {
    fn accessor(&self, index: usize) -> std::result::Result<gltf::Accessor<'_>, GltfError> {
        self.document
            .accessors()
            .nth(index)
            .ok_or_else(|| GltfError::InvalidGltf(format!("accessor {index} out of range")))
    }

    /// Copies `row`-sized elements (stride removed) out of the accessor's buffer
    /// view into a tightly packed block, `accessor.count()` rows total.
    fn extract(
        &self,
        accessor: &gltf::Accessor<'_>,
        row: usize,
    ) -> std::result::Result<Vec<u8>, GltfError> {
        let overflow = || GltfError::InvalidGltf("accessor range overflow".into());
        let view = accessor
            .view()
            .ok_or_else(|| GltfError::Unsupported("sparse accessors are not supported".into()))?;
        let buffer = self
            .buffers
            .get(view.buffer().index())
            .ok_or_else(|| GltfError::InvalidGltf("buffer not resolved".into()))?;
        let view_end = view
            .offset()
            .checked_add(view.length())
            .ok_or_else(overflow)?;
        if view_end > buffer.len() || view_end > view.buffer().length() {
            return Err(GltfError::InvalidGltf(
                "bufferView exceeds its declared or resolved buffer".into(),
            ));
        }
        let stride = view.stride().unwrap_or(row);
        if stride < row {
            return Err(GltfError::InvalidGltf(format!(
                "accessor stride {stride} is smaller than element size {row}"
            )));
        }
        let base = view
            .offset()
            .checked_add(accessor.offset())
            .ok_or_else(overflow)?;
        let output_len = accessor.count().checked_mul(row).ok_or_else(overflow)?;
        if accessor.count() > 0 {
            let last = base
                .checked_add(
                    (accessor.count() - 1)
                        .checked_mul(stride)
                        .ok_or_else(overflow)?,
                )
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
        for i in 0..accessor.count() {
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

impl AccessorSource for GltfRsSource<'_> {
    fn read_attribute(
        &self,
        accessor_idx: usize,
        expected_types: &[&str],
        allowed_component_types: &[u32],
    ) -> std::result::Result<DecodedAccessor, GltfError> {
        let accessor = self.accessor(accessor_idx)?;

        let type_str = dimensions_str(accessor.dimensions())?;
        if !expected_types.contains(&type_str) {
            return Err(GltfError::InvalidGltf(format!(
                "expected one of {expected_types:?} accessor, got {type_str}"
            )));
        }
        let gl_enum = accessor.data_type().as_gl_enum();
        if !allowed_component_types.contains(&gl_enum) {
            return Err(GltfError::Unsupported(format!(
                "unsupported {type_str} component type {gl_enum}"
            )));
        }

        let num_components = accessor.dimensions().multiplicity() as u8;
        let row = (num_components as usize)
            .checked_mul(accessor.data_type().size())
            .ok_or_else(|| GltfError::InvalidGltf("accessor row size overflow".into()))?;
        let bytes = self.extract(&accessor, row)?;

        DecodedAccessor::new(
            accessor.count(),
            num_components,
            draco_data_type(accessor.data_type()),
            accessor.normalized(),
            bytes,
        )
    }

    fn read_indices(&self, accessor_idx: usize) -> std::result::Result<Vec<u32>, GltfError> {
        use gltf::accessor::DataType as G;
        let accessor = self.accessor(accessor_idx)?;
        if !matches!(accessor.dimensions(), gltf::accessor::Dimensions::Scalar) {
            return Err(GltfError::InvalidGltf(
                "indices accessor must be SCALAR".into(),
            ));
        }
        let bytes = self.extract(&accessor, accessor.data_type().size())?;
        let mut indices = Vec::new();
        indices.try_reserve_exact(accessor.count()).map_err(|_| {
            GltfError::ResourceLimitExceeded("index accessor allocation failed".into())
        })?;
        match accessor.data_type() {
            G::U8 => indices.extend(bytes.iter().map(|&byte| u32::from(byte))),
            G::U16 => indices.extend(
                bytes
                    .chunks_exact(2)
                    .map(|chunk| u32::from(u16::from_le_bytes([chunk[0], chunk[1]]))),
            ),
            G::U32 => indices.extend(
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
fn dimensions_str(d: gltf::accessor::Dimensions) -> std::result::Result<&'static str, GltfError> {
    use gltf::accessor::Dimensions::*;
    Ok(match d {
        Scalar => "SCALAR",
        Vec2 => "VEC2",
        Vec3 => "VEC3",
        Vec4 => "VEC4",
        _ => {
            return Err(GltfError::Unsupported(
                "matrix accessor not supported".into(),
            ))
        }
    })
}

/// Maps a gltf-rs component type to the matching `draco-core` data type.
fn draco_data_type(d: gltf::accessor::DataType) -> draco_core::draco_types::DataType {
    use draco_core::draco_types::DataType as D;
    use gltf::accessor::DataType as G;
    match d {
        G::I8 => D::Int8,
        G::U8 => D::Uint8,
        G::I16 => D::Int16,
        G::U16 => D::Uint16,
        G::U32 => D::Uint32,
        G::F32 => D::Float32,
    }
}
