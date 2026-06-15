//! Document-preserving glTF Draco compression.
//!
//! [`compress_gltf_bytes`] takes a self-contained glTF or GLB document and
//! returns a copy whose triangle-mesh geometry is compressed with
//! `KHR_draco_mesh_compression`, while **everything else in the document is
//! carried through untouched**: materials, textures, images, samplers, cameras,
//! nodes, animations, skins, `extras`, and unknown extensions.
//!
//! This is the key difference from the `read meshes -> write fresh glTF` path,
//! which only models geometry and therefore drops materials and other content.
//! Here we mutate the original JSON document in place and only touch the parts
//! that change: the compressed primitives, their geometry accessors, the
//! buffer, and the buffer views.
//!
//! # What gets compressed
//!
//! A primitive is compressed only when it can be reproduced losslessly:
//!
//! - triangle list (`mode` 4 or absent) with an `indices` accessor,
//! - not already Draco-compressed,
//! - every attribute semantic is `POSITION`, `NORMAL`, `TEXCOORD_n`, or
//!   `COLOR_n` (other semantics such as `TANGENT`, `JOINTS_n`, `WEIGHTS_n`, or
//!   custom `_*` attributes are not yet round-trippable through the Draco
//!   attribute model, so those primitives are left uncompressed),
//! - its geometry accessors are not shared with any other primitive,
//! - decoding, re-encoding, and the semantic mapping all succeed exactly.
//!
//! Any primitive that fails these checks is left uncompressed but fully
//! preserved. Skinned/animated assets therefore round-trip losslessly even when
//! their geometry cannot be compressed yet.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use serde_json::{Map, Value};

use crate::gltf_reader::{GltfError, GltfReader};
use crate::gltf_writer::{draco_semantic_map, encode_draco_mesh_with_info, QuantizationBits};

type Result<T> = std::result::Result<T, GltfError>;

const KHR_DRACO: &str = "KHR_draco_mesh_compression";
const GLB_MAGIC: u32 = 0x4654_6C67;
const GLB_VERSION: u32 = 2;
const GLB_CHUNK_JSON: u32 = 0x4E4F_534A;
const GLB_CHUNK_BIN: u32 = 0x004E_4942;
const MODE_TRIANGLES: u64 = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    Glb,
    Gltf,
}

/// Compress the geometry of a self-contained glTF/GLB document with Draco,
/// preserving all other document content.
///
/// `input` may be GLB bytes or glTF JSON whose buffers/images are embedded as
/// data URIs (use [`compress_gltf_bytes_with_base_path`] for external files).
/// The output container matches the input (GLB in -> GLB out, glTF in -> glTF
/// out with an embedded buffer).
pub fn compress_gltf_bytes(
    input: &[u8],
    quantization: Option<QuantizationBits>,
) -> Result<Vec<u8>> {
    compress_gltf_bytes_with_base_path(input, None, quantization)
}

/// Like [`compress_gltf_bytes`], but resolves external buffers/`.bin` files
/// relative to `base_path`.
pub fn compress_gltf_bytes_with_base_path(
    input: &[u8],
    base_path: Option<&Path>,
    quantization: Option<QuantizationBits>,
) -> Result<Vec<u8>> {
    let quant = quantization.unwrap_or_default();

    // Parse the full document as an opaque value we can mutate surgically.
    let is_glb = input.len() >= 4 && read_u32_le(&input[0..4]) == GLB_MAGIC;
    let (mut doc, container) = if is_glb {
        let (json_bytes, _) = split_glb(input)?;
        (serde_json::from_slice::<Value>(json_bytes)?, Container::Glb)
    } else {
        (serde_json::from_slice::<Value>(input)?, Container::Gltf)
    };
    if !doc.is_object() {
        return Err(GltfError::InvalidGltf("glTF root is not an object".into()));
    }

    // Reuse the reader (lenient: do not reject skins/animations/morph targets,
    // we only preserve them) for geometry decoding and resolved buffer bytes.
    let reader = GltfReader::from_bytes_lenient(input, base_path)?;
    let source_buffers: Vec<Vec<u8>> = reader.buffers().to_vec();

    // Reference-count accessor usage across every primitive so we only mutate
    // accessors that belong exclusively to a single primitive we compress.
    let accessor_users = count_accessor_users(&doc);

    // --- decide + encode ---
    let plans = build_plans(&doc, &reader, &accessor_users, &quant)?;

    // Mutate accessors of compressed primitives: drop their buffer view and set
    // the count to the Draco-encoded value. Done before scanning for orphans so
    // the now-unreferenced geometry buffer views fall out naturally.
    apply_accessor_mutations(&mut doc, &plans)?;

    // Repack the binary: keep only buffer views still referenced by the JSON,
    // append one Draco buffer view per compressed primitive, and reindex every
    // buffer-view reference in the document.
    let repack = repack_buffers(&mut doc, &source_buffers, &plans)?;

    // Write the Draco extension onto each compressed primitive (after reindex,
    // so the freshly appended buffer-view indices are not remapped).
    for (i, plan) in plans.iter().enumerate() {
        let draco_bv = repack.draco_buffer_views[i];
        set_primitive_draco_extension(&mut doc, plan, draco_bv)?;
    }

    if !plans.is_empty() {
        ensure_extension_listed(&mut doc, "extensionsUsed");
        ensure_extension_listed(&mut doc, "extensionsRequired");
    }

    set_single_buffer(&mut doc, repack.bin.len(), container);

    serialize(&doc, &repack.bin, container)
}

/// A primitive that will be compressed, with everything needed to rewrite it.
struct CompressPlan {
    mesh_idx: usize,
    prim_idx: usize,
    draco_bytes: Vec<u8>,
    /// `(glTF semantic, Draco attribute id)` for the extension's attribute map.
    semantic_to_id: Vec<(String, usize)>,
    /// Accessor indices for each attribute, plus the indices accessor.
    attribute_accessors: Vec<usize>,
    indices_accessor: usize,
    num_points: usize,
    num_indices: usize,
}

fn build_plans(
    doc: &Value,
    reader: &GltfReader,
    accessor_users: &HashMap<usize, usize>,
    quant: &QuantizationBits,
) -> Result<Vec<CompressPlan>> {
    let mut plans = Vec::new();
    let Some(meshes) = doc.get("meshes").and_then(Value::as_array) else {
        return Ok(plans);
    };

    for (mesh_idx, mesh) in meshes.iter().enumerate() {
        let Some(primitives) = mesh.get("primitives").and_then(Value::as_array) else {
            continue;
        };
        for (prim_idx, prim) in primitives.iter().enumerate() {
            if let Some(plan) =
                plan_for_primitive(prim, mesh_idx, prim_idx, reader, accessor_users, quant)?
            {
                plans.push(plan);
            }
        }
    }
    Ok(plans)
}

fn plan_for_primitive(
    prim: &Value,
    mesh_idx: usize,
    prim_idx: usize,
    reader: &GltfReader,
    accessor_users: &HashMap<usize, usize>,
    quant: &QuantizationBits,
) -> Result<Option<CompressPlan>> {
    // Triangle list only.
    let mode = prim
        .get("mode")
        .and_then(Value::as_u64)
        .unwrap_or(MODE_TRIANGLES);
    if mode != MODE_TRIANGLES {
        return Ok(None);
    }
    // Skip primitives already using Draco.
    if prim
        .get("extensions")
        .and_then(|e| e.get(KHR_DRACO))
        .is_some()
    {
        return Ok(None);
    }
    // Need an indices accessor (Draco glTF primitives are indexed).
    let Some(indices_accessor) = prim.get("indices").and_then(Value::as_u64) else {
        return Ok(None);
    };
    let indices_accessor = indices_accessor as usize;

    // Collect attribute semantics + accessors; require a round-trippable set.
    let Some(attributes) = prim.get("attributes").and_then(Value::as_object) else {
        return Ok(None);
    };
    if attributes.is_empty() || !attributes.contains_key("POSITION") {
        return Ok(None);
    }
    let mut attribute_accessors = Vec::new();
    for (semantic, accessor) in attributes {
        if !is_round_trippable_semantic(semantic) {
            return Ok(None);
        }
        let Some(acc) = accessor.as_u64() else {
            return Ok(None);
        };
        attribute_accessors.push(acc as usize);
    }

    // All geometry accessors must be used by exactly this primitive, so that
    // dropping their buffer view / changing their count cannot corrupt another
    // primitive that shares them.
    let exclusive = |acc: usize| accessor_users.get(&acc).copied().unwrap_or(0) == 1;
    if !exclusive(indices_accessor) || !attribute_accessors.iter().copied().all(exclusive) {
        return Ok(None);
    }

    // Decode geometry; an unsupported attribute/layout means "leave it alone".
    let mesh = match reader.decode_primitive(mesh_idx, prim_idx) {
        Ok(mesh) => mesh,
        Err(_) => return Ok(None),
    };
    let (draco_bytes, info) = match encode_draco_mesh_with_info(&mesh, quant) {
        Ok(out) => out,
        Err(_) => return Ok(None),
    };
    let semantic_to_id = match draco_semantic_map(&info) {
        Ok(map) => map,
        Err(_) => return Ok(None),
    };

    // The reproduced semantics must match the source primitive exactly. This
    // rejects any case where re-encoding would rename or reorder attributes.
    let produced: BTreeSet<&str> = semantic_to_id.iter().map(|(s, _)| s.as_str()).collect();
    let original: BTreeSet<&str> = attributes.keys().map(String::as_str).collect();
    if produced != original {
        return Ok(None);
    }

    Ok(Some(CompressPlan {
        mesh_idx,
        prim_idx,
        draco_bytes,
        semantic_to_id,
        attribute_accessors,
        indices_accessor,
        num_points: info.num_encoded_points,
        num_indices: info.num_encoded_faces * 3,
    }))
}

/// `POSITION`, `NORMAL`, `TEXCOORD_<n>`, `COLOR_<n>` are reproduced with their
/// original names by the encoder; everything else is not (yet).
fn is_round_trippable_semantic(semantic: &str) -> bool {
    match semantic {
        "POSITION" | "NORMAL" => true,
        _ => {
            let suffix = semantic
                .strip_prefix("TEXCOORD_")
                .or_else(|| semantic.strip_prefix("COLOR_"));
            matches!(suffix, Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        }
    }
}

/// Counts, for each accessor index, how many primitives reference it (via
/// attributes or indices) across the whole document.
fn count_accessor_users(doc: &Value) -> HashMap<usize, usize> {
    let mut users: HashMap<usize, usize> = HashMap::new();
    let Some(meshes) = doc.get("meshes").and_then(Value::as_array) else {
        return users;
    };
    for mesh in meshes {
        let Some(primitives) = mesh.get("primitives").and_then(Value::as_array) else {
            continue;
        };
        for prim in primitives {
            if let Some(attrs) = prim.get("attributes").and_then(Value::as_object) {
                for accessor in attrs.values() {
                    if let Some(a) = accessor.as_u64() {
                        *users.entry(a as usize).or_default() += 1;
                    }
                }
            }
            if let Some(a) = prim.get("indices").and_then(Value::as_u64) {
                *users.entry(a as usize).or_default() += 1;
            }
        }
    }
    users
}

fn apply_accessor_mutations(doc: &mut Value, plans: &[CompressPlan]) -> Result<()> {
    let accessors = doc
        .get_mut("accessors")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| GltfError::InvalidGltf("missing accessors array".into()))?;

    for plan in plans {
        for &acc in &plan.attribute_accessors {
            strip_geometry_accessor(accessors, acc, plan.num_points)?;
        }
        strip_geometry_accessor(accessors, plan.indices_accessor, plan.num_indices)?;
    }
    Ok(())
}

/// Removes an accessor's buffer view (its data now lives in Draco) and sets the
/// count to the Draco-encoded element count. Other fields (type, componentType,
/// min/max, normalized) are preserved.
fn strip_geometry_accessor(accessors: &mut [Value], idx: usize, count: usize) -> Result<()> {
    let accessor = accessors
        .get_mut(idx)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| GltfError::InvalidGltf(format!("accessor {} out of range", idx)))?;
    accessor.remove("bufferView");
    accessor.remove("byteOffset");
    accessor.insert("count".into(), Value::from(count));
    Ok(())
}

struct Repack {
    bin: Vec<u8>,
    /// New buffer-view index for each plan's Draco stream, in plan order.
    draco_buffer_views: Vec<usize>,
}

fn repack_buffers(
    doc: &mut Value,
    source_buffers: &[Vec<u8>],
    plans: &[CompressPlan],
) -> Result<Repack> {
    let old_views: Vec<Value> = doc
        .get("bufferViews")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Which buffer views are still referenced anywhere in the JSON (accessors,
    // images, surviving Draco extensions, and any unknown extension)? Scanning
    // by key name covers known and unknown referrers uniformly.
    let mut referenced = BTreeSet::new();
    collect_buffer_view_refs(doc, &mut referenced);

    // Build the new binary: kept views (remapped), then one view per Draco blob.
    let mut bin: Vec<u8> = Vec::new();
    let mut new_views: Vec<Value> = Vec::new();
    let mut remap: HashMap<usize, usize> = HashMap::new();

    for &old_idx in &referenced {
        let view = old_views
            .get(old_idx)
            .and_then(Value::as_object)
            .ok_or_else(|| GltfError::InvalidGltf(format!("buffer view {} invalid", old_idx)))?;
        let bytes = buffer_view_bytes(view, source_buffers)?;
        align_to_4(&mut bin);
        let offset = bin.len();
        bin.extend_from_slice(bytes);

        let mut new_view = view.clone();
        new_view.insert("buffer".into(), Value::from(0u64));
        new_view.insert("byteOffset".into(), Value::from(offset as u64));
        new_view.insert("byteLength".into(), Value::from(bytes.len() as u64));
        let new_idx = new_views.len();
        new_views.push(Value::Object(new_view));
        remap.insert(old_idx, new_idx);
    }

    // Reindex every buffer-view reference in the document to the kept set.
    remap_buffer_view_refs(doc, &remap);

    // Append the Draco buffer views (not present in the JSON yet, so they are
    // intentionally added after the remap pass).
    let mut draco_buffer_views = Vec::with_capacity(plans.len());
    for plan in plans {
        align_to_4(&mut bin);
        let offset = bin.len();
        bin.extend_from_slice(&plan.draco_bytes);
        let new_idx = new_views.len();
        new_views.push(serde_json::json!({
            "buffer": 0,
            "byteOffset": offset as u64,
            "byteLength": plan.draco_bytes.len() as u64,
        }));
        draco_buffer_views.push(new_idx);
    }

    if new_views.is_empty() {
        doc.as_object_mut().unwrap().remove("bufferViews");
    } else {
        doc.as_object_mut()
            .unwrap()
            .insert("bufferViews".into(), Value::Array(new_views));
    }

    Ok(Repack {
        bin,
        draco_buffer_views,
    })
}

fn buffer_view_bytes<'a>(view: &Map<String, Value>, buffers: &'a [Vec<u8>]) -> Result<&'a [u8]> {
    let buffer_idx = view
        .get("buffer")
        .and_then(Value::as_u64)
        .ok_or_else(|| GltfError::InvalidGltf("buffer view missing buffer index".into()))?
        as usize;
    let offset = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let length = view
        .get("byteLength")
        .and_then(Value::as_u64)
        .ok_or_else(|| GltfError::InvalidGltf("buffer view missing byteLength".into()))?
        as usize;
    let buffer = buffers
        .get(buffer_idx)
        .ok_or_else(|| GltfError::InvalidGltf(format!("buffer {} not resolved", buffer_idx)))?;
    let end = offset
        .checked_add(length)
        .filter(|&e| e <= buffer.len())
        .ok_or_else(|| GltfError::InvalidGltf("buffer view out of range".into()))?;
    Ok(&buffer[offset..end])
}

/// Recursively collect every integer found under a `"bufferView"` key.
fn collect_buffer_view_refs(value: &Value, out: &mut BTreeSet<usize>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if let Some(idx) = child.as_u64().filter(|_| key == "bufferView") {
                    out.insert(idx as usize);
                }
                collect_buffer_view_refs(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_buffer_view_refs(item, out);
            }
        }
        _ => {}
    }
}

/// Recursively remap every integer under a `"bufferView"` key using `remap`.
fn remap_buffer_view_refs(value: &mut Value, remap: &HashMap<usize, usize>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if let Some(old) = child.as_u64().filter(|_| key == "bufferView") {
                    if let Some(&new) = remap.get(&(old as usize)) {
                        *child = Value::from(new as u64);
                    }
                }
                remap_buffer_view_refs(child, remap);
            }
        }
        Value::Array(items) => {
            for item in items {
                remap_buffer_view_refs(item, remap);
            }
        }
        _ => {}
    }
}

fn set_primitive_draco_extension(
    doc: &mut Value,
    plan: &CompressPlan,
    draco_buffer_view: usize,
) -> Result<()> {
    let prim = doc
        .get_mut("meshes")
        .and_then(Value::as_array_mut)
        .and_then(|m| m.get_mut(plan.mesh_idx))
        .and_then(|m| m.get_mut("primitives"))
        .and_then(Value::as_array_mut)
        .and_then(|p| p.get_mut(plan.prim_idx))
        .and_then(Value::as_object_mut)
        .ok_or_else(|| GltfError::InvalidGltf("primitive vanished during rewrite".into()))?;

    let mut attributes = Map::new();
    for (semantic, id) in &plan.semantic_to_id {
        attributes.insert(semantic.clone(), Value::from(*id as u64));
    }
    let draco = serde_json::json!({
        "bufferView": draco_buffer_view as u64,
        "attributes": Value::Object(attributes),
    });

    let extensions = prim
        .entry("extensions")
        .or_insert_with(|| Value::Object(Map::new()));
    if !extensions.is_object() {
        return Err(GltfError::InvalidGltf(
            "primitive.extensions is not an object".into(),
        ));
    }
    extensions
        .as_object_mut()
        .unwrap()
        .insert(KHR_DRACO.into(), draco);
    Ok(())
}

/// Adds `KHR_draco_mesh_compression` to a root string array (creating it if
/// absent) without duplicating it.
fn ensure_extension_listed(doc: &mut Value, key: &str) {
    let root = doc.as_object_mut().unwrap();
    let list = root.entry(key).or_insert_with(|| Value::Array(Vec::new()));
    let Some(arr) = list.as_array_mut() else {
        return;
    };
    if !arr.iter().any(|v| v.as_str() == Some(KHR_DRACO)) {
        arr.push(Value::from(KHR_DRACO));
    }
}

/// Collapses the document to a single buffer of `bin_len` bytes. For glTF
/// output the data URI is filled in by [`serialize`] (which owns the bytes); for
/// GLB the buffer carries no URI (it is the BIN chunk).
fn set_single_buffer(doc: &mut Value, bin_len: usize, _container: Container) {
    let root = doc.as_object_mut().unwrap();
    if bin_len == 0 {
        root.remove("buffers");
        return;
    }
    let mut buffer = Map::new();
    buffer.insert("byteLength".into(), Value::from(bin_len as u64));
    root.insert("buffers".into(), Value::Array(vec![Value::Object(buffer)]));
}

fn serialize(doc: &Value, bin: &[u8], container: Container) -> Result<Vec<u8>> {
    match container {
        Container::Gltf => {
            // Re-emit with the buffer's data URI carrying the real bytes.
            let mut doc = doc.clone();
            if !bin.is_empty() {
                let uri = format!(
                    "data:application/octet-stream;base64,{}",
                    base64_encode(bin)
                );
                if let Some(buffers) = doc.get_mut("buffers").and_then(Value::as_array_mut) {
                    if let Some(buffer) = buffers.get_mut(0).and_then(Value::as_object_mut) {
                        buffer.insert("uri".into(), Value::from(uri));
                    }
                }
            }
            Ok(serde_json::to_vec(&doc)?)
        }
        Container::Glb => build_glb(doc, bin),
    }
}

fn build_glb(doc: &Value, bin: &[u8]) -> Result<Vec<u8>> {
    let mut json = serde_json::to_vec(doc)?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin_padded = bin.to_vec();
    while !bin_padded.len().is_multiple_of(4) {
        bin_padded.push(0);
    }

    let has_bin = !bin_padded.is_empty();
    let mut total = 12 + 8 + json.len();
    if has_bin {
        total += 8 + bin_padded.len();
    }

    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&GLB_VERSION.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());

    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&GLB_CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json);

    if has_bin {
        out.extend_from_slice(&(bin_padded.len() as u32).to_le_bytes());
        out.extend_from_slice(&GLB_CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(&bin_padded);
    }

    Ok(out)
}

/// Returns the GLB JSON chunk bytes and optional BIN chunk bytes.
fn split_glb(data: &[u8]) -> Result<(&[u8], Option<&[u8]>)> {
    if data.len() < 12 {
        return Err(GltfError::InvalidGlb(
            "file too small for GLB header".into(),
        ));
    }
    if read_u32_le(&data[0..4]) != GLB_MAGIC {
        return Err(GltfError::InvalidGlb("bad GLB magic".into()));
    }
    let total = read_u32_le(&data[8..12]) as usize;
    if total > data.len() {
        return Err(GltfError::InvalidGlb("GLB length exceeds data".into()));
    }

    let mut json: Option<&[u8]> = None;
    let mut bin: Option<&[u8]> = None;
    let mut pos = 12;
    while pos + 8 <= total {
        let len = read_u32_le(&data[pos..pos + 4]) as usize;
        let kind = read_u32_le(&data[pos + 4..pos + 8]);
        let start = pos + 8;
        let end = start
            .checked_add(len)
            .filter(|&e| e <= total)
            .ok_or_else(|| GltfError::InvalidGlb("GLB chunk out of range".into()))?;
        match kind {
            GLB_CHUNK_JSON => json = Some(&data[start..end]),
            GLB_CHUNK_BIN => bin = Some(&data[start..end]),
            _ => {}
        }
        pos = end;
    }
    let json = json.ok_or_else(|| GltfError::InvalidGlb("GLB has no JSON chunk".into()))?;
    Ok((json, bin))
}

fn read_u32_le(bytes: &[u8]) -> u32 {
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn align_to_4(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(n >> 18) as usize & 0x3F] as char);
        out.push(TABLE[(n >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}
