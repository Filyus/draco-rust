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
//! // Compress a full scene and inspect what was compressed or preserved.
//! let compressed = scene.compress()?;
//! std::fs::write("model.draco.gltf", compressed.data)?;
//! println!("{:?}", compressed.report);
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

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use draco_core::{DecoderBuffer, DracoError, Mesh, MeshDecoder};
use serde::Deserialize;
use serde_json::Value;

pub mod document;
pub use document::{
    AccessorIndex, AnimationIndex, BufferIndex, BufferViewIndex, CameraIndex, ComponentType,
    Document, FileIndex, ImageIndex, MaterialIndex, MeshIndex, NodeIndex, PrimitiveRef,
    SamplerIndex, SceneIndex, ShapeIndex, SkinIndex, TextureIndex, ValidationProfile,
};
pub mod extensions;
pub use extensions::{
    DracoExtension, ExtensionHandler, ExtensionRegistry, ExtensionValidationContext, ResourceStore,
    KHR_DRACO_MESH_COMPRESSION,
};
mod native_import;
#[cfg(not(target_arch = "wasm32"))]
pub use native_import::open_native;
pub use native_import::{parse_native, parse_native_with_options, NativeImport};

/// Re-export so callers can use the scene model without depending on `gltf`
/// directly.
pub use gltf;

/// The compression configuration and report types are owned by `draco-io` so
/// byte-oriented, full-scene, native, and WASM entry points share one contract.
pub use draco_io::{
    CompressionOutput, CompressionReport, EncodingMethod, ExternalFilePolicy, FileResourceResolver,
    GltfCompressionOptions, GltfContainerFormat, GltfError, OutputFormat, PreserveReason,
    QuantizationOptions, ResourceLimits, ResourceResolver,
};

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
    Decode(#[from] DracoError),
    /// Container, resource, geometry, or compression work in `draco-io` failed.
    #[error("draco-io error: {0}")]
    DracoIo(#[from] GltfError),
    /// The `KHR_draco_mesh_compression` extension was malformed or absent.
    #[error("Draco extension error: {0}")]
    Extension(String),
    /// The document failed glTF validation (ignoring Draco-specific errors).
    #[error("glTF validation failed: {0:?}")]
    Validation(Vec<String>),
    /// An import quota was exceeded.
    #[error("resource quota exceeded: {0}")]
    ResourceLimit(String),
    /// Image bytes could not be decoded.
    #[cfg(feature = "image")]
    #[error("image decode error: {0}")]
    Image(#[from] image::ImageError),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Import configuration for external resources and optional quotas.
pub struct ImportOptions<'a> {
    /// Base directory used by the built-in filesystem resolver.
    pub base_path: Option<&'a Path>,
    /// Policy applied by the built-in filesystem resolver.
    pub external_file_policy: ExternalFilePolicy,
    /// Optional application-provided synchronous resolver. When present it is
    /// used instead of the built-in filesystem resolver.
    pub resolver: Option<&'a dyn ResourceResolver>,
    /// Optional resource and image quotas. `None` means unlimited.
    pub limits: ResourceLimits,
}

impl Default for ImportOptions<'_> {
    fn default() -> Self {
        Self {
            base_path: None,
            external_file_policy: ExternalFilePolicy::Deny,
            resolver: None,
            limits: ResourceLimits::default(),
        }
    }
}

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
    input_format: GltfContainerFormat,
    // gltf-json intentionally ignores arbitrary object properties outside
    // `extensions`/`extras`. Keep the exact parsed JSON as the canonical scene
    // and apply typed Document changes to it before every transformation.
    raw_document: Value,
    document_snapshot: Value,
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

    /// Compresses eligible primitives with the canonical glTF defaults while
    /// preserving the input container kind.
    pub fn compress(&self) -> Result<CompressionOutput<Vec<u8>>> {
        self.compress_with_options(&GltfCompressionOptions::default())
    }

    /// Compresses eligible primitives and returns both bytes and a typed
    /// primitive-by-primitive report. Valid unsupported primitives are
    /// preserved; malformed input is returned as an error.
    pub fn compress_with_options(
        &self,
        options: &GltfCompressionOptions,
    ) -> Result<CompressionOutput<Vec<u8>>> {
        compress_document(self, options)
    }

    fn canonical_document(&self) -> Result<Value> {
        let current = serde_json::to_value(self.document.clone().into_json())?;
        let mut canonical = self.raw_document.clone();
        apply_typed_document_diff(&self.document_snapshot, &current, &mut canonical);
        Ok(canonical)
    }

    /// Serializes the current document without recompressing it. This is the
    /// normal save path after [`Self::decompress_in_place`].
    pub fn to_bytes(&self, output_format: OutputFormat) -> Result<Vec<u8>> {
        let document = self.canonical_document()?;
        let (document, bin) = draco_io::consolidate_gltf_buffers(document, &self.buffers)?;
        Ok(draco_io::serialize_gltf_document(
            &document,
            &bin,
            self.input_format,
            output_format,
        )?)
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
        // Build and validate the complete replacement before changing `self`.
        // This makes a failure in any primitive, allocation, or final glTF
        // validation leave the import byte-for-byte untouched.
        let mut doc_json = self.canonical_document()?;
        let usage = accessor_usage_counts(&doc_json)?;
        let mut plan_capacity = 0usize;
        for mesh in self.document.meshes() {
            plan_capacity = plan_capacity
                .checked_add(mesh.primitives().count())
                .ok_or_else(|| Error::ResourceLimit("primitive count overflow".into()))?;
        }
        let mut plans = Vec::new();
        plans
            .try_reserve_exact(plan_capacity)
            .map_err(|_| Error::ResourceLimit("failed to allocate decompression plan".into()))?;
        for (mesh_idx, mesh) in self.document.meshes().enumerate() {
            for (prim_idx, primitive) in mesh.primitives().enumerate() {
                if !is_draco(&primitive) {
                    continue;
                }
                let decoded = decode_primitive(&self.document, &self.buffers, &primitive)?;
                let semantics = draco_attribute_map(&primitive).and_then(|map| {
                    map.ok_or_else(|| Error::Extension("missing attribute map".into()))
                })?;
                let mut attrs = Vec::new();
                attrs.try_reserve_exact(semantics.len()).map_err(|_| {
                    Error::ResourceLimit("failed to allocate decompression attribute plan".into())
                })?;
                for (sem, draco_id) in semantics {
                    let acc = doc_json["meshes"][mesh_idx]["primitives"][prim_idx]["attributes"]
                        [&sem]
                        .as_u64()
                        .ok_or_else(|| {
                            Error::Extension(format!("attribute {sem} has no accessor"))
                        })?;
                    let acc = usize::try_from(acc).map_err(|_| {
                        Error::Extension(format!("attribute {sem} accessor is too large"))
                    })?;
                    attrs.push((sem, acc, draco_id));
                }
                let indices_acc = doc_json["meshes"][mesh_idx]["primitives"][prim_idx]["indices"]
                    .as_u64()
                    .map(usize::try_from)
                    .transpose()
                    .map_err(|_| Error::Extension("indices accessor is too large".into()))?;
                plans.push((mesh_idx, prim_idx, decoded, attrs, indices_acc));
            }
        }
        if plans.is_empty() {
            return Ok(());
        }

        // Materialize tightly packed attributes and triangle-list indices in a
        // fresh buffer. Only mapped accessors are replaced; ordinary side
        // attributes retain their original bytes and accessors.
        let new_buffer_index = doc_json["buffers"].as_array().map_or(0, |b| b.len());
        let mut bin: Vec<u8> = Vec::new();

        for (mesh_idx, prim_idx, mesh, attrs, indices_acc) in &plans {
            for (semantic, original_acc, draco_id) in attrs {
                let acc_idx = writable_accessor(
                    &mut doc_json,
                    &usage,
                    *mesh_idx,
                    *prim_idx,
                    Some(semantic),
                    *original_acc,
                )?;
                let bytes = attribute_bytes(mesh, *draco_id)?;
                let view = push_view(&mut doc_json, new_buffer_index, &mut bin, &bytes)?;
                set_accessor_view(&mut doc_json, acc_idx, view, mesh.num_points())?;
                if semantic == "POSITION" {
                    set_position_bounds(&mut doc_json, acc_idx, &bytes)?;
                }
            }
            let bytes = index_bytes(mesh)?;
            let view = push_view(&mut doc_json, new_buffer_index, &mut bin, &bytes)?;
            let count = mesh
                .num_faces()
                .checked_mul(3)
                .ok_or_else(|| Error::Extension("decoded index count overflow".into()))?;
            let acc_idx = match indices_acc {
                Some(original) => {
                    writable_accessor(&mut doc_json, &usage, *mesh_idx, *prim_idx, None, *original)?
                }
                None => append_index_accessor(&mut doc_json, *mesh_idx, *prim_idx)?,
            };
            set_accessor_view(&mut doc_json, acc_idx, view, count)?;
            doc_json["accessors"][acc_idx]["componentType"] = Value::from(5125u64);
            doc_json["accessors"][acc_idx]["type"] = Value::from("SCALAR");
            doc_json["accessors"][acc_idx]
                .as_object_mut()
                .ok_or_else(|| Error::Extension("indices accessor is not an object".into()))?
                .remove("normalized");

            // Draco always decodes to oriented triangle faces. Materialize the
            // result as indexed TRIANGLES regardless of whether the compressed
            // primitive used TRIANGLES, TRIANGLE_STRIP, or omitted `indices`.
            doc_json["meshes"][*mesh_idx]["primitives"][*prim_idx]["mode"] = Value::from(4u64);
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
        let buffers = doc_json["buffers"]
            .as_array_mut()
            .ok_or_else(|| Error::Extension("buffers is not an array".into()))?;
        buffers
            .try_reserve_exact(1)
            .map_err(|_| Error::ResourceLimit("failed to allocate output buffer entry".into()))?;
        buffers.push(serde_json::json!({ "byteLength": bin.len() as u64 }));
        for key in ["extensionsUsed", "extensionsRequired"] {
            if let Some(arr) = doc_json.get_mut(key).and_then(Value::as_array_mut) {
                arr.retain(|v| v.as_str() != Some(KHR_DRACO));
            }
        }

        // A plain document no longer needs the Draco-aware validation escape
        // hatch. Let gltf-rs validate the fully materialized replacement, then
        // verify its buffer declarations against the candidate buffer set.
        let root = gltf::json::Root::deserialize(&doc_json)?;
        let document = gltf::Document::from_json(root)?;
        let document_snapshot = serde_json::to_value(document.clone().into_json())?;
        let candidate_len = self
            .buffers
            .len()
            .checked_add(1)
            .ok_or_else(|| Error::ResourceLimit("output buffer count overflow".into()))?;
        let mut candidate: Vec<&[u8]> = Vec::new();
        candidate.try_reserve_exact(candidate_len).map_err(|_| {
            Error::ResourceLimit("failed to allocate buffer validation table".into())
        })?;
        candidate.extend(self.buffers.iter().map(Vec::as_slice));
        candidate.push(&bin);
        validate_resolved_buffers(&document, &candidate)?;
        drop(candidate);

        self.buffers
            .try_reserve_exact(1)
            .map_err(|_| Error::ResourceLimit("failed to allocate output buffer slot".into()))?;
        self.buffers.push(bin);
        self.document = document;
        self.raw_document = doc_json;
        self.document_snapshot = document_snapshot;
        Ok(())
    }
}

/// Applies changes made through the typed gltf-rs document to its canonical raw
/// JSON representation. Keys absent from both typed snapshots are arbitrary
/// JSON and remain untouched. Equal-length arrays are merged element-wise so
/// unknown fields on scene objects survive ordinary typed edits.
fn apply_typed_document_diff(previous: &Value, current: &Value, canonical: &mut Value) {
    if previous == current {
        return;
    }
    match (previous, current, canonical) {
        (Value::Object(previous), Value::Object(current), Value::Object(canonical)) => {
            for key in previous.keys() {
                if !current.contains_key(key) {
                    canonical.remove(key);
                }
            }
            for (key, current_value) in current {
                match (previous.get(key), canonical.get_mut(key)) {
                    (Some(previous_value), Some(canonical_value)) => {
                        apply_typed_document_diff(previous_value, current_value, canonical_value);
                    }
                    _ => {
                        canonical.insert(key.clone(), current_value.clone());
                    }
                }
            }
        }
        (Value::Array(previous), Value::Array(current), Value::Array(canonical))
            if previous.len() == current.len() && current.len() == canonical.len() =>
        {
            for ((previous, current), canonical) in
                previous.iter().zip(current).zip(canonical.iter_mut())
            {
                apply_typed_document_diff(previous, current, canonical);
            }
        }
        (_, current, canonical) => canonical.clone_from(current),
    }
}

mod decompression;
use decompression::*;

mod resources;
#[cfg(not(target_arch = "wasm32"))]
pub use resources::import;
pub use resources::{import_slice, import_slice_with_options};

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

    // Parse the extension before gltf-rs validation so malformed KHR JSON is a
    // controlled error and so only accessors whose fallback is legitimately
    // omitted are exempted from gltf-rs's generic `bufferView` requirement.
    let draco_accessors = validate_draco_schema(&root)?;

    // Every primitive accessor reference is in range, so the validator will not
    // panic. Run it and filter the errors that are expected for a valid Draco
    // asset: the unsupported-extension error, and "missing bufferView" on the
    // geometry accessors whose data lives in the Draco stream.
    let mut errors = Vec::new();
    root.validate(&root, gltf::json::Path::new, &mut |path, error| {
        let location = path().to_string();
        if location.contains(KHR_DRACO) {
            return;
        }
        if error == gltf::json::validation::Error::Missing {
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

fn parse_draco_extension(value: &Value) -> Result<draco_io::KhrDracoExtension> {
    Ok(draco_io::parse_khr_draco_extension_value(value)?)
}

/// Runs the common strict KHR parser and returns only the accessors for which a
/// missing fallback `bufferView` is permitted. Schema, mode, declarations, and
/// fallback semantics remain owned by `draco-io`.
fn validate_draco_schema(root: &gltf::json::Root) -> Result<HashSet<usize>> {
    let document = serde_json::to_value(root)?;
    draco_io::validate_khr_draco_document(&document)?;
    let mut allowed = HashSet::new();
    let Some(meshes) = document.get("meshes").and_then(Value::as_array) else {
        return Ok(allowed);
    };
    for mesh in meshes {
        let Some(primitives) = mesh.get("primitives").and_then(Value::as_array) else {
            continue;
        };
        for primitive in primitives {
            let Some(parsed) = draco_io::parse_khr_draco_mesh_compression(&document, primitive)?
            else {
                continue;
            };
            if !parsed.required {
                continue;
            }
            let attributes = primitive["attributes"]
                .as_object()
                .ok_or_else(|| Error::Extension("primitive attributes are not an object".into()))?;
            for semantic in parsed.attributes.keys() {
                let accessor = attributes[semantic]
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| Error::Extension("attribute accessor is invalid".into()))?;
                let definition = &document["accessors"][accessor];
                if definition.get("bufferView").is_none() && definition.get("sparse").is_none() {
                    allowed.try_reserve(1).map_err(|_| {
                        Error::ResourceLimit("failed to allocate Draco accessor set".into())
                    })?;
                    allowed.insert(accessor);
                }
            }
            if let Some(accessor) = primitive
                .get("indices")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
            {
                let definition = &document["accessors"][accessor];
                if definition.get("bufferView").is_none() && definition.get("sparse").is_none() {
                    allowed.try_reserve(1).map_err(|_| {
                        Error::ResourceLimit("failed to allocate Draco accessor set".into())
                    })?;
                    allowed.insert(accessor);
                }
            }
        }
    }
    Ok(allowed)
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
    validate(document)?;
    validate_resolved_buffers(document, buffers)?;
    let ext = primitive
        .extension_value(KHR_DRACO)
        .ok_or_else(|| Error::Extension("primitive is not Draco-compressed".into()))?;
    let parsed = parse_draco_extension(ext)?;
    let view_index = parsed.buffer_view;

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
        .map_err(Error::Decode)?;
    validate_decoded_contract(primitive, &parsed.attributes, &mesh)?;
    Ok(mesh)
}

/// Returns the extension's `glTF semantic -> Draco attribute id` map for a
/// Draco primitive, or `None` if it is not Draco-compressed.
///
/// The ids match the unique ids of the attributes in the decoded
/// [`draco_core::Mesh`], so callers can line up decoded data with glTF
/// semantics (`POSITION`, `JOINTS_0`, `TANGENT`, …).
pub fn draco_attribute_map(
    primitive: &gltf::Primitive<'_>,
) -> Result<Option<BTreeMap<String, u32>>> {
    let Some(ext) = primitive.extension_value(KHR_DRACO) else {
        return Ok(None);
    };
    Ok(Some(parse_draco_extension(ext)?.attributes))
}

fn validate_decoded_contract(
    primitive: &gltf::Primitive<'_>,
    map: &BTreeMap<String, u32>,
    mesh: &Mesh,
) -> Result<()> {
    use draco_core::draco_types::DataType as D;
    use gltf::accessor::{DataType as G, Dimensions};

    let mut matched = 0usize;
    for (gltf_semantic, accessor) in primitive.attributes() {
        let semantic_name = gltf_semantic_name(&gltf_semantic)?;
        let semantic = semantic_name.as_ref();
        let Some(unique_id) = map.get(semantic) else {
            continue;
        };
        matched = matched
            .checked_add(1)
            .ok_or_else(|| Error::Extension("mapped attribute count overflow".into()))?;
        let attribute = mesh.attribute_by_unique_id(*unique_id).ok_or_else(|| {
            Error::Extension(format!("Draco attribute unique id {unique_id} is missing"))
        })?;
        if accessor.count() != mesh.num_points() {
            return Err(Error::Extension(format!(
                "{semantic} accessor count {} does not match decoded point count {}",
                accessor.count(),
                mesh.num_points()
            )));
        }
        if accessor.dimensions().multiplicity() != attribute.num_components() as usize {
            return Err(Error::Extension(format!(
                "{semantic} accessor component count does not match decoded attribute"
            )));
        }
        let expected_type = match attribute.data_type() {
            D::Int8 => G::I8,
            D::Uint8 => G::U8,
            D::Int16 => G::I16,
            D::Uint16 => G::U16,
            D::Uint32 => G::U32,
            D::Float32 => G::F32,
            other => {
                return Err(Error::Extension(format!(
                    "{semantic} decoded data type {other:?} is not representable in glTF 2.0"
                )))
            }
        };
        if accessor.data_type() != expected_type {
            return Err(Error::Extension(format!(
                "{semantic} accessor component type does not match decoded attribute"
            )));
        }
        if accessor.normalized() != attribute.normalized() {
            return Err(Error::Extension(format!(
                "{semantic} accessor normalized={} does not match decoded normalized={}",
                accessor.normalized(),
                attribute.normalized()
            )));
        }

        let dimensions = accessor.dimensions();
        let data_type = accessor.data_type();
        let normalized = accessor.normalized();
        let semantic_ok = match semantic {
            "POSITION" | "NORMAL" => {
                dimensions == Dimensions::Vec3 && data_type == G::F32 && !normalized
            }
            "TANGENT" => dimensions == Dimensions::Vec4 && data_type == G::F32 && !normalized,
            value if value.starts_with("TEXCOORD_") => {
                dimensions == Dimensions::Vec2
                    && (data_type == G::F32 || (matches!(data_type, G::U8 | G::U16) && normalized))
            }
            value if value.starts_with("COLOR_") => {
                matches!(dimensions, Dimensions::Vec3 | Dimensions::Vec4)
                    && (data_type == G::F32 || (matches!(data_type, G::U8 | G::U16) && normalized))
            }
            value if value.starts_with("JOINTS_") => {
                dimensions == Dimensions::Vec4 && matches!(data_type, G::U8 | G::U16) && !normalized
            }
            value if value.starts_with("WEIGHTS_") => {
                dimensions == Dimensions::Vec4
                    && (data_type == G::F32 || (matches!(data_type, G::U8 | G::U16) && normalized))
            }
            value if value.starts_with('_') => true,
            _ => false,
        };
        if !semantic_ok {
            return Err(Error::Extension(format!(
                "{semantic} accessor layout is invalid for its semantic"
            )));
        }
    }
    if matched != map.len() {
        return Err(Error::Extension(
            "Draco extension semantic has no primitive accessor".into(),
        ));
    }

    if let Some(indices) = primitive.indices() {
        if indices.dimensions() != Dimensions::Scalar
            || !matches!(indices.data_type(), G::U8 | G::U16 | G::U32)
            || indices.normalized()
        {
            return Err(Error::Extension(
                "indices accessor must be non-normalized unsigned SCALAR".into(),
            ));
        }
        let expected = match primitive.mode() {
            gltf::mesh::Mode::Triangles => mesh
                .num_faces()
                .checked_mul(3)
                .ok_or_else(|| Error::Extension("decoded index count overflow".into()))?,
            gltf::mesh::Mode::TriangleStrip => mesh
                .num_faces()
                .checked_add(2)
                .ok_or_else(|| Error::Extension("decoded strip count overflow".into()))?,
            _ => {
                return Err(Error::Extension(
                    "Draco primitive mode must be TRIANGLES or TRIANGLE_STRIP".into(),
                ))
            }
        };
        if indices.count() != expected {
            return Err(Error::Extension(format!(
                "indices accessor count {} does not match decoded index count {expected}",
                indices.count()
            )));
        }
    } else {
        let expected_faces = match primitive.mode() {
            gltf::mesh::Mode::Triangles => {
                if !mesh.num_points().is_multiple_of(3) {
                    return Err(Error::Extension(
                        "non-indexed TRIANGLES point count is not divisible by 3".into(),
                    ));
                }
                mesh.num_points() / 3
            }
            gltf::mesh::Mode::TriangleStrip => mesh.num_points().saturating_sub(2),
            _ => {
                return Err(Error::Extension(
                    "Draco primitive mode must be TRIANGLES or TRIANGLE_STRIP".into(),
                ))
            }
        };
        if mesh.num_faces() != expected_faces {
            return Err(Error::Extension(format!(
                "non-indexed primitive implies {expected_faces} faces but Draco decoded {}",
                mesh.num_faces()
            )));
        }
    }
    Ok(())
}

fn gltf_semantic_name(semantic: &gltf::Semantic) -> Result<Cow<'_, str>> {
    use gltf::Semantic;
    match semantic {
        Semantic::Positions => Ok(Cow::Borrowed("POSITION")),
        Semantic::Normals => Ok(Cow::Borrowed("NORMAL")),
        Semantic::Tangents => Ok(Cow::Borrowed("TANGENT")),
        Semantic::Colors(set) => indexed_semantic("COLOR_", *set).map(Cow::Owned),
        Semantic::TexCoords(set) => indexed_semantic("TEXCOORD_", *set).map(Cow::Owned),
        Semantic::Joints(set) => indexed_semantic("JOINTS_", *set).map(Cow::Owned),
        Semantic::Weights(set) => indexed_semantic("WEIGHTS_", *set).map(Cow::Owned),
        Semantic::Extras(name) => {
            let capacity = name
                .len()
                .checked_add(1)
                .ok_or_else(|| Error::ResourceLimit("attribute semantic length overflow".into()))?;
            let mut output = String::new();
            output.try_reserve_exact(capacity).map_err(|_| {
                Error::ResourceLimit("failed to allocate attribute semantic".into())
            })?;
            output.push('_');
            output.push_str(name);
            Ok(Cow::Owned(output))
        }
    }
}

fn indexed_semantic(prefix: &str, set: u32) -> Result<String> {
    use std::fmt::Write as _;

    let capacity = prefix
        .len()
        .checked_add(10)
        .ok_or_else(|| Error::ResourceLimit("attribute semantic length overflow".into()))?;
    let mut output = String::new();
    output
        .try_reserve_exact(capacity)
        .map_err(|_| Error::ResourceLimit("failed to allocate attribute semantic".into()))?;
    output.push_str(prefix);
    write!(&mut output, "{set}")
        .map_err(|_| Error::Extension("failed to format attribute semantic".into()))?;
    Ok(output)
}

mod compression;
use compression::compress_document;

#[cfg(test)]
mod tests;
