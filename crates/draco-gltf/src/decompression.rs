use draco_core::{FaceIndex, Mesh, PointIndex};
use serde_json::Value;

use crate::{Error, Result};

/// Extracts an attribute's values as a tightly packed per-point byte array.
pub(super) fn attribute_bytes(mesh: &Mesh, draco_id: u32) -> Result<Vec<u8>> {
    let att = mesh.attribute_by_unique_id(draco_id).ok_or_else(|| {
        Error::Extension(format!("Draco attribute unique id {draco_id} is missing"))
    })?;
    let stride = usize::try_from(att.byte_stride())
        .ok()
        .filter(|stride| *stride > 0)
        .ok_or_else(|| Error::Extension("decoded attribute has an invalid stride".into()))?;
    let num_points = mesh.num_points();
    let output_len = num_points
        .checked_mul(stride)
        .ok_or_else(|| Error::Extension("decoded attribute size overflow".into()))?;
    let mut out = Vec::new();
    out.try_reserve_exact(output_len)
        .map_err(|_| Error::ResourceLimit("failed to allocate decoded attribute".into()))?;
    let mut tmp = Vec::new();
    tmp.try_reserve_exact(stride)
        .map_err(|_| Error::ResourceLimit("failed to allocate attribute row".into()))?;
    tmp.resize(stride, 0);
    for p in 0..num_points {
        let point = u32::try_from(p)
            .map(PointIndex)
            .map_err(|_| Error::Extension("decoded point id exceeds u32".into()))?;
        let value_index = att.mapped_index(point);
        let offset = (value_index.0 as usize)
            .checked_mul(stride)
            .ok_or_else(|| Error::Extension("decoded attribute offset overflow".into()))?;
        if !att.buffer().try_read(offset, &mut tmp) {
            return Err(Error::Extension(
                "decoded attribute value is out of bounds".into(),
            ));
        }
        out.extend_from_slice(&tmp);
    }
    Ok(out)
}

/// Flattens the mesh faces into a tightly packed `UNSIGNED_INT` index array.
pub(super) fn index_bytes(mesh: &Mesh) -> Result<Vec<u8>> {
    let len = mesh
        .num_faces()
        .checked_mul(3)
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(|| Error::Extension("decoded index size overflow".into()))?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| Error::ResourceLimit("failed to allocate decoded indices".into()))?;
    for f in 0..mesh.num_faces() {
        let face = u32::try_from(f)
            .map(FaceIndex)
            .map_err(|_| Error::Extension("decoded face id exceeds u32".into()))?;
        for point in mesh.face(face) {
            if point.0 as usize >= mesh.num_points() {
                return Err(Error::Extension(format!(
                    "decoded index {} exceeds point count {}",
                    point.0,
                    mesh.num_points()
                )));
            }
            out.extend_from_slice(&point.0.to_le_bytes());
        }
    }
    Ok(out)
}

/// Appends `bytes` (4-byte aligned) to `bin` and pushes a buffer view for it,
/// returning the new buffer-view index.
pub(super) fn push_view(
    doc: &mut Value,
    buffer_index: usize,
    bin: &mut Vec<u8>,
    bytes: &[u8],
) -> Result<usize> {
    let padding = (4 - (bin.len() % 4)) % 4;
    let additional = padding
        .checked_add(bytes.len())
        .ok_or_else(|| Error::Extension("output buffer size overflow".into()))?;
    bin.try_reserve_exact(additional)
        .map_err(|_| Error::ResourceLimit("failed to allocate output buffer".into()))?;
    let padded_len = bin
        .len()
        .checked_add(padding)
        .ok_or_else(|| Error::Extension("output buffer size overflow".into()))?;
    bin.resize(padded_len, 0);
    let offset = bin.len();
    bin.extend_from_slice(bytes);
    let views = doc["bufferViews"]
        .as_array_mut()
        .ok_or_else(|| Error::Extension("bufferViews is not an array".into()))?;
    let index = views.len();
    views
        .try_reserve_exact(1)
        .map_err(|_| Error::ResourceLimit("failed to allocate output buffer view".into()))?;
    views.push(serde_json::json!({
        "buffer": buffer_index as u64,
        "byteOffset": offset as u64,
        "byteLength": bytes.len() as u64,
    }));
    Ok(index)
}

/// Points an accessor at `view` with `count` elements and no byte offset.
pub(super) fn set_accessor_view(
    doc: &mut Value,
    accessor: usize,
    view: usize,
    count: usize,
) -> Result<()> {
    let acc = doc["accessors"]
        .as_array_mut()
        .and_then(|accessors| accessors.get_mut(accessor))
        .ok_or_else(|| Error::Extension(format!("accessor {accessor} out of range")))?;
    acc["bufferView"] = Value::from(view as u64);
    acc["byteOffset"] = Value::from(0u64);
    acc["count"] = Value::from(count as u64);
    let acc = acc
        .as_object_mut()
        .ok_or_else(|| Error::Extension(format!("accessor {accessor} is not an object")))?;
    acc.remove("sparse");
    // Compression can change mapped values. POSITION gets fresh bounds below;
    // other optional bounds must be removed rather than left stale.
    acc.remove("min");
    acc.remove("max");
    Ok(())
}

/// Counts standard accessor consumers. This deliberately understands only
/// specified accessor slots; arbitrary `extras` and unknown extension JSON are
/// never interpreted as references.
pub(super) fn accessor_usage_counts(doc: &Value) -> Result<Vec<usize>> {
    fn bump(usage: &mut [usize], value: &Value, label: &str) -> Result<()> {
        let index = value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                Error::Extension(format!("{label} accessor reference is not a valid index"))
            })?;
        let count = usage.get_mut(index).ok_or_else(|| {
            Error::Extension(format!("{label} references accessor {index} out of range"))
        })?;
        *count = count
            .checked_add(1)
            .ok_or_else(|| Error::ResourceLimit("accessor usage count overflow".into()))?;
        Ok(())
    }

    let accessor_count = doc["accessors"].as_array().map_or(0, Vec::len);
    let mut usage = Vec::new();
    usage
        .try_reserve_exact(accessor_count)
        .map_err(|_| Error::ResourceLimit("failed to allocate accessor usage table".into()))?;
    usage.resize(accessor_count, 0);
    if let Some(meshes) = doc["meshes"].as_array() {
        for mesh in meshes {
            let Some(primitives) = mesh["primitives"].as_array() else {
                continue;
            };
            for primitive in primitives {
                if let Some(attributes) = primitive["attributes"].as_object() {
                    for accessor in attributes.values() {
                        bump(&mut usage, accessor, "primitive attribute")?;
                    }
                }
                if let Some(indices) = primitive.get("indices") {
                    bump(&mut usage, indices, "primitive indices")?;
                }
                if let Some(targets) = primitive["targets"].as_array() {
                    for target in targets {
                        if let Some(attributes) = target.as_object() {
                            for accessor in attributes.values() {
                                bump(&mut usage, accessor, "morph target attribute")?;
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(animations) = doc["animations"].as_array() {
        for animation in animations {
            if let Some(samplers) = animation["samplers"].as_array() {
                for sampler in samplers {
                    bump(&mut usage, &sampler["input"], "animation sampler input")?;
                    bump(&mut usage, &sampler["output"], "animation sampler output")?;
                }
            }
        }
    }
    if let Some(skins) = doc["skins"].as_array() {
        for skin in skins {
            if let Some(accessor) = skin.get("inverseBindMatrices") {
                bump(&mut usage, accessor, "skin inverseBindMatrices")?;
            }
        }
    }
    if let Some(nodes) = doc["nodes"].as_array() {
        for node in nodes {
            let extension = node
                .get("extensions")
                .and_then(Value::as_object)
                .and_then(|extensions| extensions.get("EXT_mesh_gpu_instancing"));
            if let Some(extension) = extension {
                let attributes = extension
                    .get("attributes")
                    .and_then(Value::as_object)
                    .ok_or_else(|| {
                        Error::Extension(
                            "EXT_mesh_gpu_instancing attributes are not an object".into(),
                        )
                    })?;
                for accessor in attributes.values() {
                    bump(&mut usage, accessor, "EXT_mesh_gpu_instancing attribute")?;
                }
            }
        }
    }
    Ok(usage)
}

/// Returns an accessor that may be safely overwritten for one primitive slot,
/// cloning and repointing when any other primitive/animation/skin/morph/known
/// extension slot shares the original accessor.
pub(super) fn writable_accessor(
    doc: &mut Value,
    usage: &[usize],
    mesh: usize,
    primitive: usize,
    semantic: Option<&str>,
    original: usize,
) -> Result<usize> {
    let shared = usage.get(original).copied().ok_or_else(|| {
        Error::Extension(format!(
            "accessor {original} referenced by primitive is out of range"
        ))
    })? > 1;
    let index = if shared {
        let accessors = doc["accessors"]
            .as_array_mut()
            .ok_or_else(|| Error::Extension("accessors is not an array".into()))?;
        let cloned = accessors
            .get(original)
            .cloned()
            .ok_or_else(|| Error::Extension(format!("accessor {original} out of range")))?;
        let index = accessors.len();
        accessors
            .try_reserve_exact(1)
            .map_err(|_| Error::ResourceLimit("failed to clone shared accessor".into()))?;
        accessors.push(cloned);
        index
    } else {
        original
    };
    if shared {
        let slot = &mut doc["meshes"][mesh]["primitives"][primitive];
        if let Some(semantic) = semantic {
            slot["attributes"][semantic] = Value::from(index as u64);
        } else {
            slot["indices"] = Value::from(index as u64);
        }
    }
    Ok(index)
}

pub(super) fn append_index_accessor(
    doc: &mut Value,
    mesh: usize,
    primitive: usize,
) -> Result<usize> {
    let accessors = doc["accessors"]
        .as_array_mut()
        .ok_or_else(|| Error::Extension("accessors is not an array".into()))?;
    let index = accessors.len();
    accessors
        .try_reserve_exact(1)
        .map_err(|_| Error::ResourceLimit("failed to allocate index accessor".into()))?;
    accessors.push(serde_json::json!({
        "componentType": 5125,
        "count": 0,
        "type": "SCALAR"
    }));
    doc["meshes"][mesh]["primitives"][primitive]["indices"] = Value::from(index as u64);
    Ok(index)
}

pub(super) fn set_position_bounds(doc: &mut Value, accessor: usize, bytes: &[u8]) -> Result<()> {
    if !bytes.len().is_multiple_of(12) || bytes.is_empty() {
        return Err(Error::Extension(
            "decoded POSITION byte layout is not tightly packed VEC3/FLOAT".into(),
        ));
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for row in bytes.chunks_exact(12) {
        for component in 0..3 {
            let offset = component * 4;
            let value = f32::from_le_bytes([
                row[offset],
                row[offset + 1],
                row[offset + 2],
                row[offset + 3],
            ]);
            if !value.is_finite() {
                return Err(Error::Extension(
                    "decoded POSITION contains a non-finite coordinate".into(),
                ));
            }
            min[component] = min[component].min(value);
            max[component] = max[component].max(value);
        }
    }
    doc["accessors"][accessor]["min"] = serde_json::json!(min);
    doc["accessors"][accessor]["max"] = serde_json::json!(max);
    Ok(())
}

pub(super) fn validate_resolved_buffers<T: AsRef<[u8]>>(
    document: &gltf::Document,
    buffers: &[T],
) -> Result<()> {
    for buffer in document.buffers() {
        let data = buffers
            .get(buffer.index())
            .map(AsRef::as_ref)
            .ok_or_else(|| {
                Error::Extension(format!("buffer {} was not resolved", buffer.index()))
            })?;
        if data.len() < buffer.length() {
            return Err(Error::Extension(format!(
                "buffer {} declares {} bytes but only {} were resolved",
                buffer.index(),
                buffer.length(),
                data.len()
            )));
        }
    }
    for view in document.views() {
        let data = buffers
            .get(view.buffer().index())
            .map(AsRef::as_ref)
            .ok_or_else(|| {
                Error::Extension(format!("buffer {} was not resolved", view.buffer().index()))
            })?;
        let end = view
            .offset()
            .checked_add(view.length())
            .ok_or_else(|| Error::Extension("bufferView range overflow".into()))?;
        if end > view.buffer().length() || end > data.len() {
            return Err(Error::Extension(format!(
                "bufferView {} exceeds its declared or resolved buffer",
                view.index()
            )));
        }
    }
    Ok(())
}
