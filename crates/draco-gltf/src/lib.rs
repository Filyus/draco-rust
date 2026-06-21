//! Load and save full glTF scenes with Draco-compressed geometry.
//!
//! `draco-gltf` is a thin bridge between [`gltf`](https://docs.rs/gltf) (which
//! models the whole glTF scene: materials, textures, nodes, animations, skins,
//! and arbitrary extensions) and the Draco crates:
//!
//! - **decode** uses [`draco_core`] to decompress `KHR_draco_mesh_compression`
//!   geometry,
//! - **encode** delegates to [`draco_io`]'s document-preserving compressor, so
//!   the compression logic lives in exactly one place.
//!
//! It exists because neither side does the whole job alone: `gltf-rs` does not
//! decode Draco (and its validator even *rejects* a Draco asset, since
//! `KHR_draco_mesh_compression` is a required extension it does not implement),
//! while the Draco crates intentionally do not model the rest of a glTF scene.
//!
//! # Example
//!
//! ```no_run
//! // Load a glTF/GLB that uses Draco (and anything else), then decode geometry.
//! let scene = draco_gltf::import("model.glb")?;
//! println!("{} materials", scene.document.materials().count());
//! for (mesh, prim) in scene.draco_primitives() {
//!     let geometry = scene.decode_primitive(&prim)?; // draco_core::Mesh
//!     println!("mesh {:?}: {} faces", mesh.name(), geometry.num_faces());
//! }
//!
//! // Compress a full scene back to a self-contained Draco glTF.
//! let bytes = draco_gltf::compress(&scene.document, &scene.buffers)?;
//! std::fs::write("model.draco.gltf", bytes)?;
//! # Ok::<(), draco_gltf::Error>(())
//! ```
//!
//! # Validation
//!
//! `gltf-rs`'s own validator rejects Draco assets outright (it treats
//! `KHR_draco_mesh_compression` as an unsupported required extension). [`import`]
//! therefore runs *Draco-aware* validation: full gltf-rs validation with only
//! that one expected error filtered out, so structurally invalid assets are
//! still rejected. Use [`validate`] to check a document you built yourself.

use std::path::Path;

use draco_core::{DecoderBuffer, FaceIndex, Mesh, MeshDecoder, PointIndex};
#[cfg(not(feature = "image"))]
use gltf::buffer;
use serde_json::Value;

/// Re-export so callers can use the scene model without depending on `gltf`
/// directly.
pub use gltf;

const KHR_DRACO: &str = "KHR_draco_mesh_compression";

/// Errors from loading, decoding, or compressing.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `gltf-rs` failed to parse the document.
    #[error("glTF error: {0}")]
    Gltf(#[from] gltf::Error),
    /// Filesystem or stream I/O failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// Serializing the document for compression failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// A Draco bitstream failed to decode.
    #[error("Draco decode error: {0}")]
    Decode(String),
    /// Compression (via `draco-io`) failed.
    #[error("compression error: {0}")]
    Compress(String),
    /// The `KHR_draco_mesh_compression` extension was malformed or absent.
    #[error("Draco extension error: {0}")]
    Extension(String),
    /// The document failed glTF validation (ignoring Draco-specific errors).
    #[error("glTF validation failed: {0:?}")]
    Validation(Vec<String>),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// A loaded glTF document with its decoded buffers and images.
///
/// The `document` is a full [`gltf::Document`]: materials, textures, nodes,
/// animations, skins, lights, and extensions are all available through the
/// `gltf-rs` API. Use [`Self::decode_primitive`] to decompress the geometry of
/// any Draco primitive.
pub struct Import {
    /// The parsed glTF document (full scene model).
    pub document: gltf::Document,
    /// Resolved buffer bytes, indexed by glTF buffer index.
    pub buffers: Vec<Vec<u8>>,
    /// Decoded image data, indexed by glTF image index.
    ///
    /// Only present with the `image` feature (enabled by default).
    #[cfg(feature = "image")]
    pub images: Vec<gltf::image::Data>,
}

impl Import {
    /// Iterates `(mesh, primitive)` pairs whose geometry is Draco-compressed.
    pub fn draco_primitives(&self) -> impl Iterator<Item = (gltf::Mesh<'_>, gltf::Primitive<'_>)> {
        self.document
            .meshes()
            .flat_map(|mesh| mesh.primitives().map(move |prim| (mesh.clone(), prim)))
            .filter(|(_, prim)| is_draco(prim))
    }

    /// Decodes a Draco primitive's geometry into a [`draco_core::Mesh`].
    ///
    /// Errors if the primitive is not Draco-compressed or the extension is
    /// malformed.
    pub fn decode_primitive(&self, primitive: &gltf::Primitive<'_>) -> Result<Mesh> {
        decode_primitive(&self.document, &self.buffers, primitive)
    }

    /// Replaces every Draco primitive with plain, uncompressed geometry, so the
    /// rest of the document can be read through the normal `gltf-rs` API
    /// (`primitive.reader(...)`) without any Draco awareness.
    ///
    /// Each Draco primitive's geometry is decoded and written into a new buffer;
    /// the existing geometry accessors gain buffer views pointing at it, the
    /// `KHR_draco_mesh_compression` extension is removed, and Draco is dropped
    /// from `extensionsUsed`/`extensionsRequired`. Materials, textures, images,
    /// nodes, animations, skins, and other content are untouched.
    ///
    /// The now-unreferenced Draco buffer views are left in place (not pruned);
    /// they are small and harmless. This is a no-op if there are no Draco
    /// primitives.
    pub fn decompress_in_place(&mut self) -> Result<()> {
        // Pass 1: decode geometry and record where each primitive's accessors
        // live, without mutating anything yet.
        let mut doc_json = serde_json::to_value(self.document.clone().into_json())?;
        let mut plans = Vec::new();
        for (mesh_idx, mesh) in self.document.meshes().enumerate() {
            for (prim_idx, primitive) in mesh.primitives().enumerate() {
                if !is_draco(&primitive) {
                    continue;
                }
                let decoded = decode_primitive(&self.document, &self.buffers, &primitive)?;
                let semantics = draco_attribute_map(&primitive)
                    .ok_or_else(|| Error::Extension("missing attribute map".into()))?;
                let attrs = semantics
                    .into_iter()
                    .map(|(sem, draco_id)| {
                        let acc = doc_json["meshes"][mesh_idx]["primitives"][prim_idx]
                            ["attributes"][&sem]
                            .as_u64()
                            .ok_or_else(|| {
                                Error::Extension(format!("attribute {sem} has no accessor"))
                            })?;
                        Ok((acc as usize, draco_id))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let indices_acc = doc_json["meshes"][mesh_idx]["primitives"][prim_idx]["indices"]
                    .as_u64()
                    .map(|i| i as usize);
                plans.push((mesh_idx, prim_idx, decoded, attrs, indices_acc));
            }
        }
        if plans.is_empty() {
            return Ok(());
        }

        // Pass 2: write decoded geometry into a fresh buffer and repoint the
        // accessors at it.
        let new_buffer_index = doc_json["buffers"].as_array().map_or(0, |b| b.len());
        let mut bin: Vec<u8> = Vec::new();

        for (mesh_idx, prim_idx, mesh, attrs, indices_acc) in &plans {
            for (acc_idx, draco_id) in attrs {
                let bytes = attribute_bytes(mesh, *draco_id);
                let view = push_view(&mut doc_json, new_buffer_index, &mut bin, &bytes);
                set_accessor_view(&mut doc_json, *acc_idx, view, mesh.num_points());
            }
            if let Some(acc_idx) = indices_acc {
                let bytes = index_bytes(mesh);
                let view = push_view(&mut doc_json, new_buffer_index, &mut bin, &bytes);
                set_accessor_view(&mut doc_json, *acc_idx, view, mesh.num_faces() * 3);
                // Indices were written as UNSIGNED_INT.
                doc_json["accessors"][*acc_idx]["componentType"] = Value::from(5125u64);
            }
            // Drop the Draco extension from the primitive.
            if let Some(ext) = doc_json["meshes"][*mesh_idx]["primitives"][*prim_idx]
                .get_mut("extensions")
                .and_then(Value::as_object_mut)
            {
                ext.remove(KHR_DRACO);
            }
        }

        // Append the new buffer (its data is provided via `self.buffers`, so no
        // URI is needed) and drop Draco from the extension lists.
        if let Some(buffers) = doc_json["buffers"].as_array_mut() {
            buffers.push(serde_json::json!({ "byteLength": bin.len() as u64 }));
        }
        for key in ["extensionsUsed", "extensionsRequired"] {
            if let Some(arr) = doc_json.get_mut(key).and_then(Value::as_array_mut) {
                arr.retain(|v| v.as_str() != Some(KHR_DRACO));
            }
        }

        // Rebuild the document and attach the new buffer's data.
        let root: gltf::json::Root = serde_json::from_value(doc_json)?;
        self.document = gltf::Document::from_json_without_validation(root);
        self.buffers.push(bin);
        Ok(())
    }
}

/// Extracts an attribute's values as a tightly packed per-point byte array.
fn attribute_bytes(mesh: &Mesh, draco_id: u32) -> Vec<u8> {
    let att = mesh.attribute(draco_id as i32);
    let stride = att.byte_stride() as usize;
    let num_points = mesh.num_points();
    let mut out = Vec::with_capacity(num_points * stride);
    let mut tmp = vec![0u8; stride];
    for p in 0..num_points {
        let value_index = att.mapped_index(PointIndex(p as u32));
        att.buffer().read(value_index.0 as usize * stride, &mut tmp);
        out.extend_from_slice(&tmp);
    }
    out
}

/// Flattens the mesh faces into a tightly packed `UNSIGNED_INT` index array.
fn index_bytes(mesh: &Mesh) -> Vec<u8> {
    let mut out = Vec::with_capacity(mesh.num_faces() * 3 * 4);
    for f in 0..mesh.num_faces() {
        for point in mesh.face(FaceIndex(f as u32)) {
            out.extend_from_slice(&point.0.to_le_bytes());
        }
    }
    out
}

/// Appends `bytes` (4-byte aligned) to `bin` and pushes a buffer view for it,
/// returning the new buffer-view index.
fn push_view(doc: &mut Value, buffer_index: usize, bin: &mut Vec<u8>, bytes: &[u8]) -> usize {
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let offset = bin.len();
    bin.extend_from_slice(bytes);
    let views = doc["bufferViews"]
        .as_array_mut()
        .expect("bufferViews array");
    let index = views.len();
    views.push(serde_json::json!({
        "buffer": buffer_index as u64,
        "byteOffset": offset as u64,
        "byteLength": bytes.len() as u64,
    }));
    index
}

/// Points an accessor at `view` with `count` elements and no byte offset.
fn set_accessor_view(doc: &mut Value, accessor: usize, view: usize, count: usize) {
    let acc = &mut doc["accessors"][accessor];
    acc["bufferView"] = Value::from(view as u64);
    acc["byteOffset"] = Value::from(0u64);
    acc["count"] = Value::from(count as u64);
}

/// Loads a glTF/GLB file that may use Draco (and any other extensions).
///
/// Filesystem-only (not available on `wasm32`); on the web use [`import_slice`]
/// with bytes you have already fetched.
#[cfg(not(target_arch = "wasm32"))]
pub fn import<P: AsRef<Path>>(path: P) -> Result<Import> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    import_slice(&bytes, path.parent())
}

/// Loads glTF JSON or GLB bytes, resolving external resources relative to
/// `base` when present.
///
/// The document is validated with [`validate`] (gltf-rs validation minus the
/// expected Draco "unsupported extension" error), so a structurally invalid
/// asset is rejected even though gltf-rs's own validator cannot be used on a
/// Draco file directly.
pub fn import_slice(bytes: &[u8], base: Option<&Path>) -> Result<Import> {
    let gltf::Gltf { document, blob } = gltf::Gltf::from_slice_without_validation(bytes)?;
    validate(&document)?;

    #[cfg(feature = "image")]
    {
        let resolved = gltf::import_buffers(&document, base, blob)?;
        let images = gltf::import_images(&document, base, &resolved)?;
        let buffers = resolved.into_iter().map(|d| d.0).collect();
        Ok(Import {
            document,
            buffers,
            images,
        })
    }
    #[cfg(not(feature = "image"))]
    {
        let buffers = load_buffers(&document, blob, base)?;
        Ok(Import { document, buffers })
    }
}

/// Built-in buffer loader used when the `image` feature is off (so gltf-rs's
/// `import_buffers` is unavailable). Resolves data URIs and the GLB BIN chunk;
/// external file URIs are read only when a base path is given (never on wasm).
#[cfg(not(feature = "image"))]
fn load_buffers(
    document: &gltf::Document,
    blob: Option<Vec<u8>>,
    base: Option<&Path>,
) -> Result<Vec<Vec<u8>>> {
    let mut blob = blob;
    let mut out = Vec::new();
    for buffer in document.buffers() {
        let data = match buffer.source() {
            buffer::Source::Bin => blob
                .take()
                .ok_or_else(|| Error::Extension("GLB buffer has no BIN chunk".into()))?,
            buffer::Source::Uri(uri) => load_uri(uri, base)?,
        };
        out.push(data);
    }
    Ok(out)
}

#[cfg(not(feature = "image"))]
fn load_uri(uri: &str, base: Option<&Path>) -> Result<Vec<u8>> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let comma = rest
            .find(',')
            .ok_or_else(|| Error::Extension("malformed data URI".into()))?;
        if rest[..comma].contains(";base64") {
            return base64_decode(&rest[comma + 1..])
                .ok_or_else(|| Error::Extension("invalid base64 in data URI".into()));
        }
        return Err(Error::Extension(
            "only base64 data URIs are supported without the `image` feature".into(),
        ));
    }
    match base {
        #[cfg(not(target_arch = "wasm32"))]
        Some(base) => Ok(std::fs::read(base.join(uri))?),
        #[cfg(target_arch = "wasm32")]
        Some(_) => Err(Error::Extension(
            "external file URIs are not available on wasm".into(),
        )),
        None => Err(Error::Extension(
            "external resource URI requires a base path".into(),
        )),
    }
}

#[cfg(not(feature = "image"))]
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &b in input.as_bytes() {
        if b == b'=' || b.is_ascii_whitespace() {
            continue;
        }
        let v = val(b)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Runs gltf-rs validation on a document, ignoring the expected
/// `KHR_draco_mesh_compression` "unsupported extension" error (which gltf-rs
/// reports because it does not implement Draco). All other validation errors —
/// out-of-range indices, malformed accessors, and so on — are reported.
///
/// [`import`]/[`import_slice`] call this for you; it is public so callers can
/// validate a document they have built or modified.
pub fn validate(document: &gltf::Document) -> Result<()> {
    use gltf::json::validation::Validate;
    let root = document.clone().into_json();

    // gltf-rs 1.4's validator has a single panic vector: it directly indexes
    // `root.accessors[n]` for a primitive's POSITION attribute (a known bug,
    // fixed upstream but unreleased). Pre-check every primitive's accessor
    // references so the validator never reaches the panic. This keeps validation
    // correct even on wasm targets built with `panic = "abort"`, where
    // `catch_unwind` cannot intercept a panic.
    let accessor_count = root.accessors.len();
    let mut errors = Vec::new();
    for (mi, mesh) in root.meshes.iter().enumerate() {
        for (pi, prim) in mesh.primitives.iter().enumerate() {
            for accessor in prim.attributes.values() {
                if accessor.value() >= accessor_count {
                    errors.push(format!(
                        "IndexOutOfBounds at meshes[{mi}].primitives[{pi}].attributes"
                    ));
                }
            }
            if let Some(indices) = &prim.indices {
                if indices.value() >= accessor_count {
                    errors.push(format!(
                        "IndexOutOfBounds at meshes[{mi}].primitives[{pi}].indices"
                    ));
                }
            }
        }
    }
    if !errors.is_empty() {
        return Err(Error::Validation(errors));
    }

    // Every primitive accessor reference is in range, so the validator will not
    // panic. Run it and filter the errors that are expected for a valid Draco
    // asset: the unsupported-extension error, and "missing bufferView" on the
    // geometry accessors whose data lives in the Draco stream.
    let draco_accessors = draco_accessor_indices(document);
    let mut errors = Vec::new();
    root.validate(&root, gltf::json::Path::new, &mut |path, error| {
        let location = path().to_string();
        if location.contains(KHR_DRACO) {
            return;
        }
        if format!("{error:?}") == "Missing" {
            if let Some(idx) = accessor_buffer_view_index(&location) {
                if draco_accessors.contains(&idx) {
                    return;
                }
            }
        }
        errors.push(format!("{error:?} at {location}"));
    });
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::Validation(errors))
    }
}

/// Accessor indices used by Draco-compressed primitives (attributes + indices).
fn draco_accessor_indices(document: &gltf::Document) -> std::collections::HashSet<usize> {
    let mut set = std::collections::HashSet::new();
    for mesh in document.meshes() {
        for prim in mesh.primitives() {
            if !is_draco(&prim) {
                continue;
            }
            for (_, accessor) in prim.attributes() {
                set.insert(accessor.index());
            }
            if let Some(indices) = prim.indices() {
                set.insert(indices.index());
            }
        }
    }
    set
}

/// Parses `N` from a validation path of the form `accessors[N].bufferView`.
fn accessor_buffer_view_index(location: &str) -> Option<usize> {
    let rest = location.strip_prefix("accessors[")?;
    if !rest.ends_with("].bufferView") {
        return None;
    }
    rest[..rest.find(']')?].parse().ok()
}

/// Returns `true` if the primitive's geometry is Draco-compressed.
pub fn is_draco(primitive: &gltf::Primitive<'_>) -> bool {
    primitive.extension_value(KHR_DRACO).is_some()
}

/// Decodes a Draco primitive's geometry into a [`draco_core::Mesh`].
pub fn decode_primitive(
    document: &gltf::Document,
    buffers: &[Vec<u8>],
    primitive: &gltf::Primitive<'_>,
) -> Result<Mesh> {
    let ext = primitive
        .extension_value(KHR_DRACO)
        .ok_or_else(|| Error::Extension("primitive is not Draco-compressed".into()))?;
    let view_index =
        ext.get("bufferView")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| Error::Extension("missing bufferView".into()))? as usize;

    let view = document
        .views()
        .nth(view_index)
        .ok_or_else(|| Error::Extension(format!("bufferView {view_index} out of range")))?;
    let buffer = buffers
        .get(view.buffer().index())
        .ok_or_else(|| Error::Extension("buffer not resolved".into()))?;
    let start = view.offset();
    let end = start
        .checked_add(view.length())
        .filter(|&e| e <= buffer.len())
        .ok_or_else(|| Error::Extension("Draco bufferView out of range".into()))?;

    let mut mesh = Mesh::new();
    MeshDecoder::new()
        .decode(&mut DecoderBuffer::new(&buffer[start..end]), &mut mesh)
        .map_err(|e| Error::Decode(format!("{e:?}")))?;
    Ok(mesh)
}

/// Returns the extension's `glTF semantic -> Draco attribute id` map for a
/// Draco primitive, or `None` if it is not Draco-compressed.
///
/// The ids match the unique ids of the attributes in the decoded
/// [`draco_core::Mesh`], so callers can line up decoded data with glTF
/// semantics (`POSITION`, `JOINTS_0`, `TANGENT`, …).
pub fn draco_attribute_map(primitive: &gltf::Primitive<'_>) -> Option<Vec<(String, u32)>> {
    let ext = primitive.extension_value(KHR_DRACO)?;
    let map = ext.get("attributes")?.as_object()?;
    Some(
        map.iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_u64()? as u32)))
            .collect(),
    )
}

/// Compresses a full glTF scene to a self-contained Draco glTF (embedded
/// buffer), preserving materials, textures, nodes, animations, skins, and any
/// other content.
///
/// The compression itself runs in [`draco_io::compress_gltf_value`] — the same
/// document-preserving core `draco-io` uses for the byte API, so there is a
/// single compression implementation. The already-parsed `gltf-rs` `document`
/// and its resolved `buffers` are fed straight to that core: geometry is decoded
/// through `draco-io`'s reader built directly from the in-memory document (no
/// serialize-to-bytes / re-parse / re-resolve-buffers round trip).
pub fn compress(document: &gltf::Document, buffers: &[Vec<u8>]) -> Result<Vec<u8>> {
    let doc_value = serde_json::to_value(document.clone().into_json())?;

    // Build draco-io's reader from the in-memory document + resolved buffers,
    // then drive the shared compressor core with it as the geometry decoder.
    let reader = draco_io::GltfReader::from_value(&doc_value, buffers.to_vec())
        .map_err(|e| Error::Compress(e.to_string()))?;
    let (mut out_doc, bin) =
        draco_io::compress_gltf_value(doc_value, buffers, None, |mesh, prim| {
            reader.decode_primitive_with_semantics(mesh, prim)
        })
        .map_err(|e| Error::Compress(e.to_string()))?;

    // The core collapses everything to one buffer carrying `byteLength` only;
    // embed the bytes as a data URI to make the glTF self-contained.
    embed_single_buffer(&mut out_doc, &bin);
    Ok(serde_json::to_vec(&out_doc)?)
}

/// Fills the single output buffer's `uri` with `bin` as a base64 data URI.
///
/// [`draco_io::compress_gltf_value`] leaves `buffers[0]` with a `byteLength`
/// but no URI (so the caller can choose GLB or embedded glTF); we always emit
/// embedded glTF. A no-op when `bin` is empty (the core then emits no buffer).
fn embed_single_buffer(doc: &mut Value, bin: &[u8]) {
    if bin.is_empty() {
        return;
    }
    if let Some(buffer) = doc
        .get_mut("buffers")
        .and_then(Value::as_array_mut)
        .and_then(|b| b.get_mut(0))
        .and_then(Value::as_object_mut)
    {
        buffer.insert(
            "uri".into(),
            Value::from(format!(
                "data:application/octet-stream;base64,{}",
                base64_encode(bin)
            )),
        );
    }
}

fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}
