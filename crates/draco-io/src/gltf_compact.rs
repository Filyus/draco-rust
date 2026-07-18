//! Serde-free compact glTF reader front end.
//!
//! A size-constrained glTF reader that shares the strict GLB container
//! validation and the [`GltfError`] type with the rest of `draco-io` but parses
//! the JSON document with [`nanoserde`] instead of `serde_json`. This keeps the
//! serde-backed document model (and its dependency) out of the binary, which is
//! what browser/WASM builds need to stay under a tight size budget.
//!
//! For the full document model (materials, textures, skins, animations, ...)
//! use the serde-backed [`crate::gltf_reader::GltfReader`] behind the
//! `gltf-reader` feature instead.
//!
//! # Contract & limitations
//!
//! The compact reader deliberately handles a subset of glTF 2.0. Anything
//! outside this contract is reported via [`GltfError::Unsupported`] rather than
//! silently dropped:
//!
//! - **Geometry primitives** with draw mode `4` (`TRIANGLES`) or `5`
//!   (`TRIANGLE_STRIP`) only.
//! - **POSITION/NORMAL** must be `VEC3`, `componentType` `5126` (FLOAT),
//!   unnormalized — matches the native `GltfReader` geometry contract.
//! - **TEXCOORD_0** (`VEC2`) and **COLOR_0** (`VEC3`/`VEC4`) accept FLOAT
//!   (`5126`), UNSIGNED_BYTE (`5121`), and UNSIGNED_SHORT (`5123`). Integer
//!   accessors must be `normalized: true`; FLOAT must not be (the glTF
//!   `RequiredForInteger` policy). Integer components are expanded to `f32`
//!   via the standard normalize formula.
//! - **Index accessors** (`SCALAR`, componentType `5121`/`5123`/`5125`).
//! - **Multiple buffers** are supported: each `bufferView` may reference any
//!   declared buffer (GLB BIN chunk for buffer 0, external/data-URI buffers
//!   for the rest).
//! - **Multiple primitives per mesh** are supported: a mesh with `N` primitives
//!   flattens into `N` entries of [`CompactMeshData`]. A node's `mesh` index
//!   points at the first primitive of that mesh; callers that need the full
//!   extent can derive it from the document's primitive counts.
//! - **No sparse accessors.**
//! - **`extensionsRequired`** may contain only `KHR_draco_mesh_compression`.
//!   `KHR_draco_mesh_compression` primitives are decoded through `draco-core`
//!   and require FLOAT attributes (POSITION/NORMAL VEC3, TEXCOORD VEC2,
//!   COLOR VEC3/VEC4).
//! - Image URIs are **validated** (data-URIs decoded-and-discarded, external
//!   URIs must be supplied via the resource map) but image bytes are never
//!   materialized.
//! - Node TRS and scene indices are bounds-checked; finite floats are enforced.

use std::collections::HashMap;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::FaceIndex;
use draco_core::geometry_indices::PointIndex;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
use nanoserde::DeJson;

use crate::gltf_container::decode_data_uri;
use crate::gltf_geometry::{GltfError, Result};

/// Optional limits for compact glTF parsing. `None` means unlimited.
///
/// The baseline [`parse_compact_document`] API remains unlimited for backward
/// compatibility. Front ends that accept untrusted documents should use
/// [`parse_compact_document_with_limits`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompactLimits {
    /// Maximum UTF-8 JSON document size.
    pub max_json_bytes: Option<usize>,
    /// Maximum decoded size of one buffer or image resource.
    pub max_resource_bytes: Option<usize>,
    /// Maximum decoded size of all glTF buffers.
    pub max_total_buffer_bytes: Option<usize>,
    /// Maximum aggregate size of decoded geometry arrays.
    pub max_result_bytes: Option<usize>,
}

impl CompactLimits {
    /// Conservative limits for browser/WASM entry points.
    pub const WASM_DEFAULT: Self = Self {
        max_json_bytes: Some(16 * 1024 * 1024),
        max_resource_bytes: Some(64 * 1024 * 1024),
        max_total_buffer_bytes: Some(128 * 1024 * 1024),
        max_result_bytes: Some(256 * 1024 * 1024),
    };
}

#[derive(Default)]
struct ResultBudget {
    limit: Option<usize>,
    used: usize,
}

impl ResultBudget {
    fn new(limit: Option<usize>) -> Self {
        Self { limit, used: 0 }
    }

    fn reserve<T>(&mut self, count: usize, what: &str) -> Result<()> {
        let bytes = count
            .checked_mul(std::mem::size_of::<T>())
            .ok_or_else(|| GltfError::ResourceLimitExceeded(format!("{what} size overflow")))?;
        self.used = self
            .used
            .checked_add(bytes)
            .ok_or_else(|| GltfError::ResourceLimitExceeded(format!("{what} size overflow")))?;
        if self.limit.is_some_and(|limit| self.used > limit) {
            return Err(GltfError::ResourceLimitExceeded(format!(
                "decoded geometry exceeds the {what} budget"
            )));
        }
        Ok(())
    }
}

/// Decoded geometry for a single glTF primitive.
///
/// Geometry-neutral DTO: no `wasm_bindgen`, no `nanoserde` serialization. A
/// front end (e.g. a WASM crate) maps this into whatever shape its platform
/// expects.
#[derive(Debug, Clone, Default)]
pub struct CompactMeshData {
    /// Mesh name from the glTF `meshes[].name`.
    pub name: Option<String>,
    /// Vertex positions as `[x0, y0, z0, ...]`.
    pub positions: Vec<f32>,
    /// Triangle indices as `[i0, i1, i2, ...]`.
    pub indices: Vec<u32>,
    /// Vertex normals, when present.
    pub normals: Vec<f32>,
    /// First texture-coordinate set, when present.
    pub uvs: Vec<f32>,
    /// First color set, when present.
    pub colors: Vec<f32>,
}

/// Minimal scene-graph node (TRS + hierarchy), as carried by the document.
#[derive(Debug, Clone, Default)]
pub struct CompactNode {
    pub name: Option<String>,
    pub mesh: Option<usize>,
    pub translation: Option<Vec<f32>>,
    pub rotation: Option<Vec<f32>>,
    pub scale: Option<Vec<f32>>,
    pub children: Vec<usize>,
}

/// Minimal scene: a named set of root node indices.
#[derive(Debug, Clone, Default)]
pub struct CompactScene {
    pub name: Option<String>,
    pub nodes: Vec<usize>,
}

/// Fully decoded compact document: geometry + scene metadata.
#[derive(Debug, Clone, Default)]
pub struct CompactDocument {
    pub meshes: Vec<CompactMeshData>,
    pub nodes: Vec<CompactNode>,
    pub scenes: Vec<CompactScene>,
    pub default_scene: Option<usize>,
    pub uses_draco: bool,
}

#[derive(DeJson, Default)]
struct CompactRoot {
    #[nserde(default)]
    asset: Option<CompactAsset>,
    #[nserde(default)]
    accessors: Vec<CompactAccessor>,
    #[nserde(default, rename = "bufferViews")]
    buffer_views: Vec<CompactBufferView>,
    #[nserde(default)]
    buffers: Vec<CompactBuffer>,
    #[nserde(default)]
    images: Vec<CompactImage>,
    #[nserde(default)]
    meshes: Vec<CompactMesh>,
    #[nserde(default)]
    nodes: Vec<CompactNodeJson>,
    #[nserde(default)]
    scenes: Vec<CompactSceneJson>,
    #[nserde(default)]
    scene: Option<usize>,
    #[nserde(default, rename = "extensionsUsed")]
    extensions_used: Vec<String>,
    #[nserde(default, rename = "extensionsRequired")]
    extensions_required: Vec<String>,
}

#[derive(DeJson, Default)]
struct CompactAsset {
    #[nserde(default)]
    version: String,
}

#[derive(DeJson, Default)]
struct CompactAccessor {
    #[nserde(default, rename = "bufferView")]
    buffer_view: Option<usize>,
    #[nserde(default, rename = "byteOffset")]
    byte_offset: Option<usize>,
    #[nserde(default, rename = "componentType")]
    component_type: u32,
    #[nserde(default)]
    count: usize,
    #[nserde(default, rename = "type")]
    accessor_type: String,
    #[nserde(default)]
    normalized: bool,
    #[nserde(default)]
    sparse: Option<CompactSparse>,
}

#[derive(DeJson, Default)]
struct CompactSparse {}

#[derive(DeJson, Default)]
struct CompactBufferView {
    #[nserde(default)]
    buffer: usize,
    #[nserde(default, rename = "byteOffset")]
    byte_offset: Option<usize>,
    #[nserde(default, rename = "byteLength")]
    byte_length: usize,
    #[nserde(default, rename = "byteStride")]
    byte_stride: Option<usize>,
}

#[derive(DeJson, Default)]
struct CompactBuffer {
    #[nserde(default, rename = "byteLength")]
    byte_length: usize,
    #[nserde(default)]
    uri: Option<String>,
}

#[derive(DeJson, Default)]
struct CompactImage {
    #[nserde(default)]
    uri: Option<String>,
    #[nserde(default, rename = "bufferView")]
    buffer_view: Option<usize>,
}

#[derive(DeJson, Default)]
struct CompactMesh {
    #[nserde(default)]
    name: Option<String>,
    #[nserde(default)]
    primitives: Vec<CompactPrimitive>,
}

#[derive(DeJson, Default)]
struct CompactPrimitive {
    #[nserde(default)]
    attributes: HashMap<String, u32>,
    #[nserde(default)]
    indices: Option<u32>,
    #[nserde(default)]
    mode: Option<u32>,
    #[nserde(default)]
    extensions: Option<CompactPrimitiveExtensions>,
}

#[derive(DeJson, Default)]
struct CompactPrimitiveExtensions {
    #[nserde(default, rename = "KHR_draco_mesh_compression")]
    khr_draco: Option<CompactDracoExtension>,
}

#[derive(DeJson, Default)]
struct CompactDracoExtension {
    #[nserde(default, rename = "bufferView")]
    buffer_view: Option<usize>,
    #[nserde(default)]
    attributes: HashMap<String, u32>,
}

#[derive(DeJson, Default)]
struct CompactNodeJson {
    #[nserde(default)]
    name: Option<String>,
    #[nserde(default)]
    mesh: Option<usize>,
    #[nserde(default)]
    translation: Option<Vec<f32>>,
    #[nserde(default)]
    rotation: Option<Vec<f32>>,
    #[nserde(default)]
    scale: Option<Vec<f32>>,
    #[nserde(default)]
    children: Vec<usize>,
}

#[derive(DeJson, Default)]
struct CompactSceneJson {
    #[nserde(default)]
    name: Option<String>,
    #[nserde(default)]
    nodes: Vec<usize>,
}

/// Parse a checked glTF JSON document against the compact contract.
///
/// `bin_buffer` is the GLB BIN chunk (or `None` for `.gltf` + external/data-URI
/// buffers). `resources` is a flat list of `(uri, bytes)` for external buffers
/// and images; a missing external buffer URI is surfaced as a controlled
/// [`GltfError::InvalidGltf`] carrying the URI.
///
/// GLB container splitting is owned by [`crate::parse_glb_json_and_bin`]; this
/// function takes already-separated JSON.
pub fn parse_compact_document(
    json: &str,
    bin_buffer: Option<&[u8]>,
    resources: &[(String, Vec<u8>)],
) -> Result<CompactDocument> {
    parse_compact_document_with_limits(json, bin_buffer, resources, &CompactLimits::default())
}

/// Parse a compact glTF document while enforcing explicit input and output
/// limits.
pub fn parse_compact_document_with_limits(
    json: &str,
    bin_buffer: Option<&[u8]>,
    resources: &[(String, Vec<u8>)],
    limits: &CompactLimits,
) -> Result<CompactDocument> {
    check_limit(json.len(), limits.max_json_bytes, "glTF JSON")?;
    let root: CompactRoot = DeJson::deserialize_json(json)
        .map_err(|_| GltfError::InvalidGltf("failed to parse glTF JSON".into()))?;
    let is_glb = bin_buffer.is_some();
    let mut buffers = resolve_document_buffers(&root, bin_buffer, resources, limits)?;
    // A GLB BIN chunk with no declared buffer still supplies buffer 0 for
    // bufferViews; keep the old lenient fallback for documents that omit it.
    if buffers.is_empty() {
        if let Some(bin) = bin_buffer {
            check_limit(bin.len(), limits.max_resource_bytes, "GLB BIN chunk")?;
            check_limit(
                bin.len(),
                limits.max_total_buffer_bytes,
                "glTF buffers total",
            )?;
            buffers.push(bin.to_vec());
        }
    }
    validate_images(&root.images, resources, limits)?;
    validate_document(&root, &buffers, is_glb)?;

    let uses_draco = document_uses_draco(&root);
    let mut meshes = Vec::new();
    let total_primitives: usize = root.meshes.iter().map(|mesh| mesh.primitives.len()).sum();
    meshes
        .try_reserve_exact(total_primitives)
        .map_err(|_| GltfError::ResourceLimitExceeded("mesh result is too large".into()))?;
    let mut budget = ResultBudget::new(limits.max_result_bytes);
    for gltf_mesh in &root.meshes {
        for primitive in &gltf_mesh.primitives {
            meshes.push(decode_primitive(
                &root,
                primitive,
                &buffers,
                gltf_mesh.name.clone(),
                &mut budget,
            )?);
        }
    }
    if meshes.is_empty() {
        return Err(GltfError::InvalidGltf(
            "document contains no mesh primitives".into(),
        ));
    }
    let nodes = root
        .nodes
        .iter()
        .map(|node| CompactNode {
            name: node.name.clone(),
            mesh: node.mesh,
            translation: node.translation.clone(),
            rotation: node.rotation.clone(),
            scale: node.scale.clone(),
            children: node.children.clone(),
        })
        .collect();
    let scenes = root
        .scenes
        .iter()
        .map(|scene| CompactScene {
            name: scene.name.clone(),
            nodes: scene.nodes.clone(),
        })
        .collect();
    Ok(CompactDocument {
        meshes,
        nodes,
        scenes,
        default_scene: root.scene,
        uses_draco,
    })
}

fn check_limit(length: usize, limit: Option<usize>, resource: &str) -> Result<()> {
    if limit.is_some_and(|limit| length > limit) {
        return Err(GltfError::ResourceLimitExceeded(format!(
            "{resource} is {length} bytes, exceeding the configured limit"
        )));
    }
    Ok(())
}

fn resolve_document_buffers(
    root: &CompactRoot,
    bin_buffer: Option<&[u8]>,
    resources: &[(String, Vec<u8>)],
    limits: &CompactLimits,
) -> Result<Vec<Vec<u8>>> {
    let mut buffers = Vec::new();
    buffers
        .try_reserve_exact(root.buffers.len())
        .map_err(|_| GltfError::ResourceLimitExceeded("buffer table allocation failed".into()))?;
    for (index, buffer) in root.buffers.iter().enumerate() {
        let bytes = if let Some(uri) = buffer.uri.as_deref() {
            resolve_buffer_uri(uri, resources, limits)?
        } else if index == 0 {
            // Buffer 0 without a URI is the GLB BIN chunk.
            bin_buffer
                .ok_or_else(|| {
                    GltfError::InvalidGltf(
                        "buffer 0 has no URI and no GLB BIN chunk was supplied".into(),
                    )
                })?
                .to_vec()
        } else {
            return Err(GltfError::InvalidGltf(format!(
                "buffer {index} has no URI; only buffer 0 may be the GLB BIN chunk"
            )));
        };
        check_limit(bytes.len(), limits.max_resource_bytes, "glTF buffer")?;
        buffers.push(bytes);
    }
    // The GLB BIN chunk must be the only buffer 0 source.
    if bin_buffer.is_some()
        && root
            .buffers
            .first()
            .is_some_and(|buffer| buffer.uri.is_some())
    {
        return Err(GltfError::InvalidGlb(
            "GLB BIN chunk requires buffer 0 without a URI".into(),
        ));
    }
    let total: usize = buffers.iter().map(Vec::len).sum();
    check_limit(total, limits.max_total_buffer_bytes, "glTF buffers total")?;
    Ok(buffers)
}

fn validate_images(
    images: &[CompactImage],
    resources: &[(String, Vec<u8>)],
    limits: &CompactLimits,
) -> Result<()> {
    for image in images {
        if image.buffer_view.is_some() {
            return Err(GltfError::Unsupported(
                "bufferView images are not supported by the compact reader".into(),
            ));
        }
        if let Some(uri) = image.uri.as_deref() {
            if uri.starts_with("data:") {
                decode_data_uri(uri, limits.max_resource_bytes).map_err(|error| match error {
                    GltfError::ResourceLimitExceeded(_) => error,
                    _ => GltfError::InvalidGltf("invalid image data URI".into()),
                })?;
            } else {
                let bytes = resources
                    .iter()
                    .find(|(candidate, _)| candidate == uri)
                    .map(|(_, bytes)| bytes)
                    .ok_or_else(|| GltfError::InvalidGltf(uri.to_string()))?;
                check_limit(bytes.len(), limits.max_resource_bytes, uri)?;
            }
        }
    }
    Ok(())
}

fn document_uses_draco(root: &CompactRoot) -> bool {
    root.meshes.iter().any(|mesh| {
        mesh.primitives.iter().any(|primitive| {
            primitive
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.khr_draco.as_ref())
                .is_some()
        })
    })
}

fn decode_primitive(
    root: &CompactRoot,
    primitive: &CompactPrimitive,
    buffers: &[Vec<u8>],
    name: Option<String>,
    budget: &mut ResultBudget,
) -> Result<CompactMeshData> {
    if let Some(draco) = primitive
        .extensions
        .as_ref()
        .and_then(|extensions| extensions.khr_draco.as_ref())
    {
        let view = draco
            .buffer_view
            .ok_or_else(|| GltfError::InvalidGltf("KHR Draco bufferView is missing".into()))?;
        let data = buffer_view_slice(&root.buffer_views, buffers, view)
            .ok_or_else(|| GltfError::InvalidGltf("KHR Draco bufferView is out of range".into()))?;
        let mut decoded = decode_draco_mesh(data, &draco.attributes, budget)?;
        decoded.name = name;
        return Ok(decoded);
    }

    let mut mesh = CompactMeshData {
        name,
        ..Default::default()
    };
    if let Some(&index) = primitive.attributes.get("POSITION") {
        mesh.positions = read_vec3(&root.accessors, &root.buffer_views, buffers, index, budget)?;
    }
    if let Some(&index) = primitive.attributes.get("NORMAL") {
        mesh.normals = read_vec3(&root.accessors, &root.buffer_views, buffers, index, budget)?;
    }
    if let Some(&index) = primitive.attributes.get("TEXCOORD_0") {
        mesh.uvs = read_vec2(&root.accessors, &root.buffer_views, buffers, index, budget)?;
    }
    if let Some(&index) = primitive.attributes.get("COLOR_0") {
        mesh.colors = read_color(&root.accessors, &root.buffer_views, buffers, index, budget)?;
    }
    if let Some(index) = primitive.indices {
        mesh.indices = read_indices(&root.accessors, &root.buffer_views, buffers, index, budget)?;
    }
    if primitive.mode.unwrap_or(4) == 5 {
        if mesh.indices.is_empty() {
            let count = mesh.positions.len() / 3;
            mesh.indices = generate_sequential_indices(count, budget)?;
        }
        mesh.indices = triangulate_strip(&mesh.indices, budget)?;
    } else if mesh.indices.is_empty() {
        let count = mesh.positions.len() / 3;
        if count % 3 != 0 {
            return Err(GltfError::InvalidGltf(
                "non-indexed TRIANGLES count is not divisible by three".into(),
            ));
        }
        mesh.indices = generate_sequential_indices(count, budget)?;
    }
    if mesh.indices.len() % 3 != 0
        || mesh
            .indices
            .iter()
            .any(|&index| index as usize >= mesh.positions.len() / 3)
    {
        return Err(GltfError::InvalidGltf("invalid triangle indices".into()));
    }
    Ok(mesh)
}

fn generate_sequential_indices(count: usize, budget: &mut ResultBudget) -> Result<Vec<u32>> {
    budget.reserve::<u32>(count, "index result")?;
    (0..count)
        .map(u32::try_from)
        .collect::<std::result::Result<Vec<u32>, _>>()
        .map_err(|_| GltfError::ResourceLimitExceeded("vertex count exceeds u32".into()))
}

fn resolve_buffer_uri(
    uri: &str,
    resources: &[(String, Vec<u8>)],
    limits: &CompactLimits,
) -> Result<Vec<u8>> {
    if uri.starts_with("data:") {
        let data = decode_data_uri(uri, limits.max_resource_bytes).map_err(|error| match error {
            GltfError::ResourceLimitExceeded(_) => error,
            _ => GltfError::InvalidGltf("invalid buffer data URI".into()),
        });
        if let Ok(data) = &data {
            check_limit(
                data.len(),
                limits.max_total_buffer_bytes,
                "glTF buffers total",
            )?;
        }
        return data;
    }
    let resource = resources
        .iter()
        .find(|(candidate, _)| candidate == uri)
        .ok_or_else(|| GltfError::InvalidGltf(uri.to_string()))?;
    let bytes = &resource.1;
    check_limit(bytes.len(), limits.max_resource_bytes, uri)?;
    check_limit(
        bytes.len(),
        limits.max_total_buffer_bytes,
        "glTF buffers total",
    )?;
    Ok(bytes.clone())
}

fn buffer_view_slice<'a>(
    views: &[CompactBufferView],
    buffers: &'a [Vec<u8>],
    index: usize,
) -> Option<&'a [u8]> {
    let view = views.get(index)?;
    let data = buffers.get(view.buffer)?;
    let start = view.byte_offset.unwrap_or(0);
    let end = start.checked_add(view.byte_length)?;
    data.get(start..end)
}

fn validate_document(root: &CompactRoot, buffers: &[Vec<u8>], is_glb: bool) -> Result<()> {
    let asset = root
        .asset
        .as_ref()
        .ok_or_else(|| GltfError::InvalidGltf("asset.version is missing".into()))?;
    if asset.version != "2.0" {
        return Err(GltfError::Unsupported(format!(
            "glTF asset version {} is not supported",
            asset.version
        )));
    }
    let draco_declared = root
        .extensions_used
        .iter()
        .any(|name| name == "KHR_draco_mesh_compression");
    let draco_used = document_uses_draco(root);
    if root
        .extensions_required
        .iter()
        .any(|name| name != "KHR_draco_mesh_compression")
    {
        return Err(GltfError::Unsupported(
            "extensionsRequired declares an unsupported extension".into(),
        ));
    }
    if root
        .extensions_required
        .iter()
        .any(|name| name == "KHR_draco_mesh_compression")
        && !draco_used
    {
        return Err(GltfError::InvalidGltf(
            "KHR_draco_mesh_compression is required but not used".into(),
        ));
    }
    if draco_used && !draco_declared {
        return Err(GltfError::InvalidGltf(
            "KHR_draco_mesh_compression is used but missing from extensionsUsed".into(),
        ));
    }
    if is_glb
        && root
            .buffers
            .first()
            .is_some_and(|buffer| buffer.uri.is_some())
    {
        return Err(GltfError::InvalidGlb(
            "GLB BIN chunk requires buffer 0 without a URI".into(),
        ));
    }
    if buffers.is_empty()
        && root.buffer_views.is_empty()
        && root.meshes.iter().all(|mesh| mesh.primitives.is_empty())
    {
        return Ok(());
    }
    if buffers.is_empty() {
        return Err(GltfError::InvalidGltf(
            "document references a buffer but none was supplied".into(),
        ));
    }
    // Each declared buffer's `byteLength` must fit its resolved bytes.
    for (index, buffer) in root.buffers.iter().enumerate() {
        let resolved = buffers.get(index).ok_or_else(|| {
            GltfError::InvalidGltf(format!("buffer {index} is declared but not resolved"))
        })?;
        if buffer.byte_length > resolved.len() {
            return Err(GltfError::InvalidGltf(format!(
                "declared buffer {index} length {} exceeds the resolved {} bytes",
                buffer.byte_length,
                resolved.len()
            )));
        }
    }
    for view in &root.buffer_views {
        let data = buffers.get(view.buffer).ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "bufferView references buffer {} which is not declared",
                view.buffer
            ))
        })?;
        if view
            .byte_offset
            .unwrap_or(0)
            .checked_add(view.byte_length)
            .is_none_or(|end| end > data.len())
        {
            return Err(GltfError::InvalidGltf("bufferView is out of range".into()));
        }
        if let Some(stride) = view.byte_stride {
            if !(4..=252).contains(&stride) || stride % 4 != 0 {
                return Err(GltfError::Unsupported(
                    "bufferView byteStride is outside 4..=252 or not divisible by 4".into(),
                ));
            }
        }
    }
    for (node_index, node) in root.nodes.iter().enumerate() {
        if node.mesh.is_some_and(|mesh| mesh >= root.meshes.len())
            || node
                .children
                .iter()
                .any(|&child| child >= root.nodes.len() || child == node_index)
        {
            return Err(GltfError::InvalidGltf(
                "node mesh or child reference is out of range".into(),
            ));
        }
        for value in [&node.translation, &node.scale] {
            if value.as_ref().is_some_and(|values| {
                values.len() != 3 || values.iter().any(|value| !value.is_finite())
            }) {
                return Err(GltfError::InvalidGltf(
                    "node translation/scale must be a finite VEC3".into(),
                ));
            }
        }
        if node.rotation.as_ref().is_some_and(|values| {
            values.len() != 4 || values.iter().any(|value| !value.is_finite())
        }) {
            return Err(GltfError::InvalidGltf(
                "node rotation must be a finite VEC4".into(),
            ));
        }
    }
    for scene in &root.scenes {
        if scene.nodes.iter().any(|&node| node >= root.nodes.len()) {
            return Err(GltfError::InvalidGltf(
                "scene node reference is out of range".into(),
            ));
        }
    }
    if root.scene.is_some_and(|scene| scene >= root.scenes.len()) {
        return Err(GltfError::InvalidGltf(
            "default scene reference is out of range".into(),
        ));
    }
    for mesh in &root.meshes {
        for primitive in &mesh.primitives {
            if !primitive.attributes.contains_key("POSITION")
                || !matches!(primitive.mode.unwrap_or(4), 4 | 5)
            {
                return Err(GltfError::Unsupported(
                    "primitive lacks POSITION or uses an unsupported draw mode".into(),
                ));
            }
            let has_draco = primitive
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.khr_draco.as_ref())
                .is_some();
            if primitive.attributes.keys().any(|semantic| {
                !matches!(
                    semantic.as_str(),
                    "POSITION" | "NORMAL" | "TEXCOORD_0" | "COLOR_0"
                )
            }) {
                return Err(GltfError::Unsupported(
                    "primitive has an unsupported vertex attribute semantic".into(),
                ));
            }
            let position = primitive.attributes["POSITION"];
            let position_count =
                validate_attribute_accessor(root, buffers, position, "POSITION", has_draco)?;
            for (semantic, &accessor) in &primitive.attributes {
                if semantic == "POSITION" {
                    continue;
                }
                let count =
                    validate_attribute_accessor(root, buffers, accessor, semantic, has_draco)?;
                if count != position_count {
                    return Err(GltfError::InvalidGltf(
                        "vertex attribute counts do not match POSITION".into(),
                    ));
                }
            }
            for &accessor in primitive.attributes.values() {
                validate_accessor(root, buffers, accessor, has_draco)?;
            }
            if let Some(index) = primitive.indices {
                validate_accessor(root, buffers, index, has_draco)?;
            }
            if let Some(draco) = primitive
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.khr_draco.as_ref())
            {
                let view = draco.buffer_view.ok_or_else(|| {
                    GltfError::InvalidGltf("KHR Draco bufferView is missing".into())
                })?;
                if draco.attributes.is_empty()
                    || buffer_view_slice(&root.buffer_views, buffers, view).is_none()
                {
                    return Err(GltfError::InvalidGltf(
                        "KHR Draco extension is missing attributes or bufferView".into(),
                    ));
                }
                if draco
                    .attributes
                    .keys()
                    .any(|semantic| !primitive.attributes.contains_key(semantic))
                {
                    return Err(GltfError::InvalidGltf(
                        "KHR Draco attributes must be a subset of primitive attributes".into(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_attribute_accessor(
    root: &CompactRoot,
    buffers: &[Vec<u8>],
    index: u32,
    semantic: &str,
    allow_missing_view: bool,
) -> Result<usize> {
    validate_accessor(root, buffers, index, allow_missing_view)?;
    let accessor = root
        .accessors
        .get(
            usize::try_from(index)
                .map_err(|_| GltfError::InvalidGltf("accessor index exceeds usize".into()))?,
        )
        .ok_or_else(|| GltfError::InvalidGltf("accessor index is out of range".into()))?;
    match semantic {
        "POSITION" | "NORMAL" => {
            if accessor.accessor_type != "VEC3"
                || accessor.component_type != 5126
                || accessor.normalized
            {
                return Err(GltfError::Unsupported(format!(
                    "{semantic} must be an unnormalized FLOAT (5126) VEC3"
                )));
            }
        }
        "TEXCOORD_0" => {
            validate_required_for_integer(accessor, semantic, 2, &[5126, 5121, 5123])?;
        }
        "COLOR_0" => {
            let components = match accessor.accessor_type.as_str() {
                "VEC3" => 3,
                "VEC4" => 4,
                _ => 0,
            };
            validate_required_for_integer(accessor, semantic, components, &[5126, 5121, 5123])?;
        }
        _ => {
            return Err(GltfError::Unsupported(format!(
                "{semantic} is not a supported vertex attribute semantic"
            )));
        }
    }
    Ok(accessor.count)
}

fn validate_accessor(
    root: &CompactRoot,
    buffers: &[Vec<u8>],
    index: u32,
    allow_missing_view: bool,
) -> Result<()> {
    let accessor = root
        .accessors
        .get(
            usize::try_from(index)
                .map_err(|_| GltfError::InvalidGltf("accessor index exceeds usize".into()))?,
        )
        .ok_or_else(|| GltfError::InvalidGltf("accessor index is out of range".into()))?;
    if accessor.sparse.is_some() {
        return Err(GltfError::Unsupported(
            "sparse accessors are not supported".into(),
        ));
    }
    let components: usize = match accessor.accessor_type.as_str() {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        other => {
            return Err(GltfError::Unsupported(format!(
                "accessor type {other} is not supported"
            )));
        }
    };
    let width = match accessor.component_type {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        other => {
            return Err(GltfError::Unsupported(format!(
                "componentType {other} is not supported"
            )));
        }
    };
    let view = match accessor.buffer_view {
        Some(index) => root
            .buffer_views
            .get(index)
            .ok_or_else(|| GltfError::InvalidGltf("bufferView index is out of range".into()))?,
        None if allow_missing_view => return Ok(()),
        None => {
            return Err(GltfError::InvalidGltf(
                "accessor is missing a bufferView".into(),
            ));
        }
    };
    let data = buffers.get(view.buffer).ok_or_else(|| {
        GltfError::InvalidGltf(format!(
            "accessor references buffer {} which is not declared",
            view.buffer
        ))
    })?;
    let row = components
        .checked_mul(width)
        .ok_or_else(|| GltfError::InvalidGltf("accessor row size overflow".into()))?;
    let stride = view.byte_stride.unwrap_or(row);
    if stride < row {
        return Err(GltfError::InvalidGltf(format!(
            "accessor stride {stride} is smaller than row size {row}"
        )));
    }
    let bytes = if accessor.count == 0 {
        0
    } else {
        accessor
            .count
            .checked_sub(1)
            .ok_or_else(|| GltfError::InvalidGltf("accessor count underflow".into()))?
            .checked_mul(stride)
            .ok_or_else(|| GltfError::InvalidGltf("accessor byte range overflow".into()))?
            .checked_add(row)
            .ok_or_else(|| GltfError::InvalidGltf("accessor byte range overflow".into()))?
    };
    let start = view
        .byte_offset
        .unwrap_or(0)
        .checked_add(accessor.byte_offset.unwrap_or(0))
        .ok_or_else(|| GltfError::InvalidGltf("accessor offset overflow".into()))?;
    if start.checked_add(bytes).is_none_or(|end| end > data.len()) {
        return Err(GltfError::InvalidGltf(
            "accessor byte range is outside the buffer".into(),
        ));
    }
    Ok(())
}

fn accessor_bounds<'a>(
    accessors: &'a [CompactAccessor],
    views: &'a [CompactBufferView],
    buffers: &'a [Vec<u8>],
    index: u32,
) -> Result<(&'a CompactAccessor, &'a CompactBufferView, &'a [u8], usize)> {
    let accessor = accessors
        .get(
            usize::try_from(index)
                .map_err(|_| GltfError::InvalidGltf("accessor index exceeds usize".into()))?,
        )
        .ok_or_else(|| GltfError::InvalidGltf("accessor index is out of range".into()))?;
    let view = accessor
        .buffer_view
        .and_then(|index| views.get(index))
        .ok_or_else(|| GltfError::InvalidGltf("accessor is missing a bufferView".into()))?;
    let data = buffers.get(view.buffer).ok_or_else(|| {
        GltfError::InvalidGltf(format!(
            "accessor references buffer {} which is not declared",
            view.buffer
        ))
    })?;
    let start = view
        .byte_offset
        .unwrap_or(0)
        .checked_add(accessor.byte_offset.unwrap_or(0))
        .ok_or_else(|| GltfError::InvalidGltf("accessor offset overflow".into()))?;
    Ok((accessor, view, data, start))
}

fn read_vec3(
    accessors: &[CompactAccessor],
    views: &[CompactBufferView],
    buffers: &[Vec<u8>],
    index: u32,
    budget: &mut ResultBudget,
) -> Result<Vec<f32>> {
    let (accessor, view, data, start) = accessor_bounds(accessors, views, buffers, index)?;
    // POSITION/NORMAL are FLOAT-only by the glTF geometry contract; they must
    // not be normalized. Integer POSITION/NORMAL attributes are out of spec.
    if accessor.accessor_type != "VEC3" || accessor.component_type != 5126 || accessor.normalized {
        return Err(GltfError::Unsupported(
            "POSITION/NORMAL must be an unnormalized FLOAT (5126) VEC3".into(),
        ));
    }
    let stride = view.byte_stride.unwrap_or(12);
    budget.reserve::<f32>(
        accessor
            .count
            .checked_mul(3)
            .ok_or_else(|| GltfError::ResourceLimitExceeded("VEC3 result size overflow".into()))?,
        "VEC3 result",
    )?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(
            accessor.count.checked_mul(3).ok_or_else(|| {
                GltfError::ResourceLimitExceeded("VEC3 result size overflow".into())
            })?,
        )
        .map_err(|_| GltfError::ResourceLimitExceeded("failed to allocate VEC3 result".into()))?;
    for row in 0..accessor.count {
        let offset = start
            .checked_add(
                row.checked_mul(stride)
                    .ok_or_else(|| GltfError::InvalidGltf("VEC3 row offset overflow".into()))?,
            )
            .ok_or_else(|| GltfError::InvalidGltf("VEC3 row offset overflow".into()))?;
        let end = offset
            .checked_add(12)
            .ok_or_else(|| GltfError::InvalidGltf("VEC3 row end overflow".into()))?;
        let bytes = data
            .get(offset..end)
            .ok_or_else(|| GltfError::InvalidGltf("VEC3 value extends past its buffer".into()))?;
        let x = scalar_to_f32(5126, false, &bytes[0..4])?;
        let y = scalar_to_f32(5126, false, &bytes[4..8])?;
        let z = scalar_to_f32(5126, false, &bytes[8..12])?;
        output.extend_from_slice(&[x, y, z]);
    }
    Ok(output)
}

fn read_vec2(
    accessors: &[CompactAccessor],
    views: &[CompactBufferView],
    buffers: &[Vec<u8>],
    index: u32,
    budget: &mut ResultBudget,
) -> Result<Vec<f32>> {
    let (accessor, view, data, start) = accessor_bounds(accessors, views, buffers, index)?;
    // TEXCOORD_0: FLOAT/UBYTE/USHORT. Integer accessors must be normalized;
    // FLOAT must not be.
    validate_required_for_integer(accessor, "TEXCOORD_0", 2, &[5126, 5121, 5123])?;
    let component_size = component_byte_size(accessor.component_type).ok_or_else(|| {
        GltfError::Unsupported("TEXCOORD_0 componentType is not supported".into())
    })?;
    let row_size = 2usize
        .checked_mul(component_size)
        .ok_or_else(|| GltfError::ResourceLimitExceeded("VEC2 row size overflow".into()))?;
    let stride = view.byte_stride.unwrap_or(row_size);
    budget.reserve::<f32>(
        accessor
            .count
            .checked_mul(2)
            .ok_or_else(|| GltfError::ResourceLimitExceeded("VEC2 result size overflow".into()))?,
        "VEC2 result",
    )?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(
            accessor.count.checked_mul(2).ok_or_else(|| {
                GltfError::ResourceLimitExceeded("VEC2 result size overflow".into())
            })?,
        )
        .map_err(|_| GltfError::ResourceLimitExceeded("failed to allocate VEC2 result".into()))?;
    for row in 0..accessor.count {
        let offset = start
            .checked_add(
                row.checked_mul(stride)
                    .ok_or_else(|| GltfError::InvalidGltf("VEC2 row offset overflow".into()))?,
            )
            .ok_or_else(|| GltfError::InvalidGltf("VEC2 row offset overflow".into()))?;
        let end = offset
            .checked_add(row_size)
            .ok_or_else(|| GltfError::InvalidGltf("VEC2 row end overflow".into()))?;
        let bytes = data
            .get(offset..end)
            .ok_or_else(|| GltfError::InvalidGltf("VEC2 value extends past its buffer".into()))?;
        for component in bytes.chunks_exact(component_size) {
            output.push(scalar_to_f32(
                accessor.component_type,
                accessor.normalized,
                component,
            )?);
        }
    }
    Ok(output)
}

fn read_color(
    accessors: &[CompactAccessor],
    views: &[CompactBufferView],
    buffers: &[Vec<u8>],
    index: u32,
    budget: &mut ResultBudget,
) -> Result<Vec<f32>> {
    let (accessor, view, data, start) = accessor_bounds(accessors, views, buffers, index)?;
    let components = match accessor.accessor_type.as_str() {
        "VEC3" => 3usize,
        "VEC4" => 4usize,
        _ => {
            return Err(GltfError::Unsupported(
                "COLOR_0 must be a VEC3 or VEC4".into(),
            ));
        }
    };
    // COLOR_0: FLOAT/UBYTE/USHORT. Integer accessors must be normalized;
    // FLOAT must not be.
    validate_required_for_integer(accessor, "COLOR_0", components, &[5126, 5121, 5123])?;
    let component_size = component_byte_size(accessor.component_type)
        .ok_or_else(|| GltfError::Unsupported("COLOR_0 componentType is not supported".into()))?;
    let row_size = components
        .checked_mul(component_size)
        .ok_or_else(|| GltfError::ResourceLimitExceeded("color row size overflow".into()))?;
    let stride = view.byte_stride.unwrap_or(row_size);
    budget.reserve::<f32>(
        accessor
            .count
            .checked_mul(components)
            .ok_or_else(|| GltfError::ResourceLimitExceeded("color result size overflow".into()))?,
        "color result",
    )?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(
            accessor.count.checked_mul(components).ok_or_else(|| {
                GltfError::ResourceLimitExceeded("color result size overflow".into())
            })?,
        )
        .map_err(|_| GltfError::ResourceLimitExceeded("failed to allocate colors".into()))?;
    for row in 0..accessor.count {
        let offset = start
            .checked_add(
                row.checked_mul(stride)
                    .ok_or_else(|| GltfError::InvalidGltf("color row offset overflow".into()))?,
            )
            .ok_or_else(|| GltfError::InvalidGltf("color row offset overflow".into()))?;
        let end = offset
            .checked_add(row_size)
            .ok_or_else(|| GltfError::InvalidGltf("color row end overflow".into()))?;
        let bytes = data
            .get(offset..end)
            .ok_or_else(|| GltfError::InvalidGltf("color value extends past its buffer".into()))?;
        for component in bytes.chunks_exact(component_size) {
            output.push(scalar_to_f32(
                accessor.component_type,
                accessor.normalized,
                component,
            )?);
        }
    }
    Ok(output)
}

/// `RequiredForInteger` policy: integer component types must carry
/// `normalized: true`, FLOAT must not. The component type must appear in
/// `allowed` and the accessor shape must match `expected_components`.
fn validate_required_for_integer(
    accessor: &CompactAccessor,
    semantic: &str,
    expected_components: usize,
    allowed: &[u32],
) -> Result<()> {
    let actual_components = match accessor.accessor_type.as_str() {
        "SCALAR" => 1,
        "VEC2" => 2,
        "VEC3" => 3,
        "VEC4" => 4,
        _ => 0,
    };
    if actual_components != expected_components {
        return Err(GltfError::Unsupported(format!(
            "{semantic} must be a {}-component accessor",
            component_count_name(expected_components)
        )));
    }
    if !allowed.contains(&accessor.component_type) {
        return Err(GltfError::Unsupported(format!(
            "{semantic} uses componentType {} which is not supported",
            accessor.component_type
        )));
    }
    let is_integer = accessor.component_type != 5126;
    if is_integer != accessor.normalized {
        return Err(GltfError::Unsupported(format!(
            "{semantic} integer accessors must be normalized; FLOAT accessors must not be"
        )));
    }
    Ok(())
}

fn component_count_name(count: usize) -> &'static str {
    match count {
        1 => "SCALAR",
        2 => "VEC2",
        3 => "VEC3",
        4 => "VEC4",
        _ => "vector",
    }
}

fn read_indices(
    accessors: &[CompactAccessor],
    views: &[CompactBufferView],
    buffers: &[Vec<u8>],
    index: u32,
    budget: &mut ResultBudget,
) -> Result<Vec<u32>> {
    let (accessor, view, data, start) = accessor_bounds(accessors, views, buffers, index)?;
    if accessor.accessor_type != "SCALAR" || accessor.normalized {
        return Err(GltfError::Unsupported(
            "index accessor must be an unnormalized SCALAR".into(),
        ));
    }
    let size = match accessor.component_type {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        other => {
            return Err(GltfError::Unsupported(format!(
                "index componentType {other} is not supported"
            )));
        }
    };
    let stride = view.byte_stride.unwrap_or(size);
    budget.reserve::<u32>(accessor.count, "index result")?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(accessor.count)
        .map_err(|_| GltfError::ResourceLimitExceeded("failed to allocate indices".into()))?;
    for row in 0..accessor.count {
        let row_offset = row
            .checked_mul(stride)
            .ok_or_else(|| GltfError::InvalidGltf("index row offset overflow".into()))?;
        let offset = start
            .checked_add(row_offset)
            .ok_or_else(|| GltfError::InvalidGltf("index row offset overflow".into()))?;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| GltfError::InvalidGltf("index row end overflow".into()))?;
        let bytes = data
            .get(offset..end)
            .ok_or_else(|| GltfError::InvalidGltf("index value extends past its buffer".into()))?;
        output.push(match size {
            1 => bytes[0] as u32,
            2 => u16::from_le_bytes(
                bytes
                    .try_into()
                    .map_err(|_| GltfError::InvalidGltf("u16 index is malformed".into()))?,
            ) as u32,
            _ => u32::from_le_bytes(
                bytes
                    .try_into()
                    .map_err(|_| GltfError::InvalidGltf("u32 index is malformed".into()))?,
            ),
        });
    }
    Ok(output)
}

fn triangulate_strip(indices: &[u32], budget: &mut ResultBudget) -> Result<Vec<u32>> {
    if indices.len() < 3 {
        return Ok(Vec::new());
    }
    let capacity = (indices.len() - 2)
        .checked_mul(3)
        .ok_or_else(|| GltfError::ResourceLimitExceeded("triangle strip index overflow".into()))?;
    budget.reserve::<u32>(capacity, "triangle strip index result")?;
    let mut output = Vec::new();
    output.try_reserve_exact(capacity).map_err(|_| {
        GltfError::ResourceLimitExceeded("failed to allocate triangle strip".into())
    })?;
    for index in 2..indices.len() {
        if index % 2 == 0 {
            output.extend_from_slice(&[indices[index - 2], indices[index - 1], indices[index]]);
        } else {
            output.extend_from_slice(&[indices[index - 1], indices[index - 2], indices[index]]);
        }
    }
    Ok(output)
}

fn decode_draco_mesh(
    data: &[u8],
    extension_attributes: &HashMap<String, u32>,
    budget: &mut ResultBudget,
) -> Result<CompactMeshData> {
    let mut buffer = DecoderBuffer::new(data);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder
        .decode(&mut buffer, &mut mesh)
        .map_err(GltfError::DracoDecode)?;
    if extension_attributes
        .values()
        .any(|&id| mesh.attribute_by_unique_id(id).is_none())
    {
        return Err(GltfError::InvalidGltf(
            "KHR Draco attribute id is not present in the decoded mesh".into(),
        ));
    }
    let positions = draco_attribute(&mesh, GeometryAttributeType::Position, 3, budget)?
        .ok_or_else(|| GltfError::InvalidGltf("decoded Draco mesh has no POSITION".into()))?;
    let normals =
        draco_attribute(&mesh, GeometryAttributeType::Normal, 3, budget)?.unwrap_or_default();
    let uvs =
        draco_attribute(&mesh, GeometryAttributeType::TexCoord, 2, budget)?.unwrap_or_default();
    let colors = draco_attribute_range(&mesh, GeometryAttributeType::Color, 3, 4, budget)?
        .unwrap_or_default();
    let count = mesh
        .num_faces()
        .checked_mul(3)
        .ok_or_else(|| GltfError::ResourceLimitExceeded("triangle index count overflow".into()))?;
    let mut indices = Vec::new();
    budget.reserve::<u32>(count, "Draco index result")?;
    indices
        .try_reserve_exact(count)
        .map_err(|_| GltfError::ResourceLimitExceeded("failed to allocate indices".into()))?;
    for face_index in 0..mesh.num_faces() {
        let face = mesh
            .face(FaceIndex(u32::try_from(face_index).map_err(|_| {
                GltfError::InvalidGltf("face index exceeds u32".into())
            })?));
        for point in face {
            if point.0 as usize >= mesh.num_points() {
                return Err(GltfError::InvalidGltf(
                    "decoded face references an out-of-range point".into(),
                ));
            }
            indices.push(point.0);
        }
    }
    Ok(CompactMeshData {
        name: None,
        positions,
        indices,
        normals,
        uvs,
        colors,
    })
}

fn draco_attribute(
    mesh: &Mesh,
    kind: GeometryAttributeType,
    components: u8,
    budget: &mut ResultBudget,
) -> Result<Option<Vec<f32>>> {
    draco_attribute_range(mesh, kind, components, components, budget)
}

fn draco_attribute_range(
    mesh: &Mesh,
    kind: GeometryAttributeType,
    min_components: u8,
    max_components: u8,
    budget: &mut ResultBudget,
) -> Result<Option<Vec<f32>>> {
    let id = mesh.named_attribute_id(kind);
    if id < 0 {
        return Ok(None);
    }
    let attribute = mesh
        .try_attribute(id)
        .map_err(|_| GltfError::InvalidGltf("invalid decoded Draco attribute".into()))?;
    let components = attribute.num_components();
    if components < min_components
        || components > max_components
        || attribute.data_type() != DataType::Float32
    {
        return Err(GltfError::Unsupported(
            "decoded Draco attribute uses an unsupported component count or type".into(),
        ));
    }
    let stride = usize::try_from(attribute.byte_stride())
        .map_err(|_| GltfError::InvalidGltf("decoded attribute has a negative stride".into()))?;
    let row_size = usize::from(components)
        .checked_mul(4)
        .ok_or_else(|| GltfError::InvalidGltf("decoded attribute row size overflow".into()))?;
    if stride < row_size {
        return Err(GltfError::InvalidGltf(format!(
            "decoded attribute stride {stride} is smaller than row size {row_size}"
        )));
    }
    let mut output = Vec::new();
    let count = mesh
        .num_points()
        .checked_mul(usize::from(components))
        .ok_or_else(|| GltfError::ResourceLimitExceeded("attribute result size overflow".into()))?;
    budget.reserve::<f32>(count, "Draco attribute result")?;
    output.try_reserve_exact(count).map_err(|_| {
        GltfError::ResourceLimitExceeded("failed to allocate attribute result".into())
    })?;
    for point in 0..mesh.num_points() {
        let value = attribute
            .mapped_index(PointIndex(u32::try_from(point).map_err(|_| {
                GltfError::InvalidGltf("point index exceeds u32".into())
            })?));
        if value.0 == u32::MAX {
            return Err(GltfError::InvalidGltf(format!(
                "decoded attribute has no value for point {point}"
            )));
        }
        let start = usize::try_from(value.0)
            .map_err(|_| GltfError::InvalidGltf("mapped index exceeds usize".into()))?
            .checked_mul(stride)
            .ok_or_else(|| GltfError::InvalidGltf("attribute offset overflow".into()))?;
        let end = start
            .checked_add(row_size)
            .ok_or_else(|| GltfError::InvalidGltf("attribute range overflow".into()))?;
        let bytes = attribute.buffer().data().get(start..end).ok_or_else(|| {
            GltfError::InvalidGltf("attribute value extends past its buffer".into())
        })?;
        for component in bytes.chunks_exact(4) {
            let value = f32::from_le_bytes(
                component
                    .try_into()
                    .map_err(|_| GltfError::InvalidGltf("decoded scalar is malformed".into()))?,
            );
            if !value.is_finite() {
                return Err(GltfError::InvalidGltf(
                    "decoded attribute contains a non-finite value".into(),
                ));
            }
            output.push(value);
        }
    }
    Ok(Some(output))
}

/// Byte width of a glTF vertex-attribute component type.
///
/// Returns `None` for types this reader never accepts on vertex attributes
/// (`INT`/`UINT` 5124/5125 carry 4 bytes but are out of spec for TEXCOORD/COLOR).
fn component_byte_size(component_type: u32) -> Option<usize> {
    match component_type {
        5120 | 5121 => Some(1),
        5122 | 5123 => Some(2),
        5126 => Some(4),
        _ => None,
    }
}

fn exact_bytes<const N: usize>(bytes: &[u8], what: &str) -> Result<[u8; N]> {
    bytes.try_into().map_err(|_| {
        GltfError::InvalidGltf(format!("{what} has {} bytes, expected {N}", bytes.len()))
    })
}

/// Decode one raw scalar component to `f32`, applying the glTF normalize
/// formula for integer types when `normalized` is set.
fn scalar_to_f32(component_type: u32, normalized: bool, bytes: &[u8]) -> Result<f32> {
    let value = match component_type {
        5120 => normalize_signed(
            i8::from_le_bytes(exact_bytes(bytes, "i8 component")?) as i32,
            i8::MAX as i32,
            normalized,
        ),
        5121 => normalize_unsigned(
            u8::from_le_bytes(exact_bytes(bytes, "u8 component")?) as u32,
            u8::MAX as u32,
            normalized,
        ),
        5122 => normalize_signed(
            i16::from_le_bytes(exact_bytes(bytes, "i16 component")?) as i32,
            i16::MAX as i32,
            normalized,
        ),
        5123 => normalize_unsigned(
            u16::from_le_bytes(exact_bytes(bytes, "u16 component")?) as u32,
            u16::MAX as u32,
            normalized,
        ),
        5126 => f32::from_le_bytes(exact_bytes(bytes, "f32 component")?),
        other => {
            return Err(GltfError::Unsupported(format!(
                "componentType {other} is not supported on a vertex attribute"
            )));
        }
    };
    if !value.is_finite() {
        return Err(GltfError::InvalidGltf(
            "decoded attribute contains a non-finite value".into(),
        ));
    }
    Ok(value)
}

/// Signed normalize: clamp the lower bound to exactly `-1.0` so that `-MAX`
/// (`-128`/`-32768`) maps to `-1.0` rather than underflowing past it.
fn normalize_signed(value: i32, max: i32, normalized: bool) -> f32 {
    if normalized {
        ((value as f32) / (max as f32)).max(-1.0)
    } else {
        value as f32
    }
}

/// Unsigned normalize: maps `[0, MAX]` linearly to `[0.0, 1.0]`.
fn normalize_unsigned(value: u32, max: u32, normalized: bool) -> f32 {
    if normalized {
        (value as f32) / (max as f32)
    } else {
        value as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle_resource() -> Vec<u8> {
        [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect()
    }

    fn external_triangle_json() -> Vec<u8> {
        br#"{
          "asset":{"version":"2.0"},
          "buffers":[{"uri":"triangle.bin","byteLength":36}],
          "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}],
          "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],
          "meshes":[{"name":"Triangle","primitives":[{"attributes":{"POSITION":0}}]}],
          "nodes":[{"mesh":0}],
          "scenes":[{"nodes":[0]}],
          "scene":0
        }"#
        .to_vec()
    }

    fn color_triangle_resource() -> Vec<u8> {
        let mut bytes = triangle_resource();
        bytes.extend(
            [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
                .into_iter()
                .flat_map(f32::to_le_bytes),
        );
        bytes
    }

    fn color_triangle_json() -> &'static str {
        r#"{
          "asset":{"version":"2.0"},
          "buffers":[{"uri":"triangle.bin","byteLength":72}],
          "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":36},
            {"buffer":0,"byteOffset":36,"byteLength":36}
          ],
          "accessors":[
            {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":1,"componentType":5126,"count":3,"type":"VEC3"}
          ],
          "meshes":[{"primitives":[{"attributes":{"POSITION":0,"COLOR_0":1}}]}]
        }"#
    }

    #[test]
    fn decodes_external_resource() {
        let resources = vec![("triangle.bin".to_string(), triangle_resource())];
        let input = external_triangle_json();
        let json = std::str::from_utf8(&input).unwrap();
        let document = parse_compact_document(json, None, &resources).unwrap();
        assert_eq!(document.meshes.len(), 1);
        assert_eq!(document.meshes[0].positions.len(), 9);
        assert_eq!(document.meshes[0].indices, vec![0, 1, 2]);
        assert_eq!(document.meshes[0].name.as_deref(), Some("Triangle"));
        assert_eq!(document.default_scene, Some(0));
    }

    #[test]
    fn missing_external_resource_reports_uri() {
        let input = external_triangle_json();
        let json = std::str::from_utf8(&input).unwrap();
        let error = parse_compact_document(json, None, &[]).unwrap_err();
        assert!(matches!(error, GltfError::InvalidGltf(ref message) if message == "triangle.bin"));
    }

    #[test]
    fn decodes_float_colors() {
        let resources = vec![("triangle.bin".to_string(), color_triangle_resource())];
        let document = parse_compact_document(color_triangle_json(), None, &resources).unwrap();
        assert_eq!(
            document.meshes[0].colors,
            vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
        );
    }

    #[test]
    fn rejects_unknown_vertex_semantics() {
        let resources = vec![("triangle.bin".to_string(), triangle_resource())];
        let json = String::from_utf8(external_triangle_json())
            .unwrap()
            .replace("\"POSITION\":0", "\"POSITION\":0,\"TANGENT\":0");
        let result = parse_compact_document(&json, None, &resources);
        assert!(matches!(result, Err(GltfError::Unsupported(_))));
    }

    #[test]
    fn rejects_mismatched_vertex_attribute_counts() {
        let resources = vec![("triangle.bin".to_string(), color_triangle_resource())];
        let json = color_triangle_json().replace(
            "\"count\":3,\"type\":\"VEC3\"}\n          ],",
            "\"count\":2,\"type\":\"VEC3\"}\n          ],",
        );
        let result = parse_compact_document(&json, None, &resources);
        assert!(matches!(result, Err(GltfError::InvalidGltf(_))));
    }

    #[test]
    fn rejects_default_scene_out_of_range() {
        let resources = vec![("triangle.bin".to_string(), triangle_resource())];
        let json = String::from_utf8(external_triangle_json())
            .unwrap()
            .replace("\"scene\":0", "\"scene\":1");
        let result = parse_compact_document(&json, None, &resources);
        assert!(matches!(result, Err(GltfError::InvalidGltf(_))));
    }

    #[test]
    fn decodes_multi_primitive_mesh() {
        // A mesh with two primitives (e.g. LOD/material split) flattens into
        // two `CompactMeshData` entries; `node.mesh` indexes the first.
        let resources = vec![("triangle.bin".to_string(), triangle_resource())];
        let json = String::from_utf8(external_triangle_json())
            .unwrap()
            .replace(
                "[{\"attributes\":{\"POSITION\":0}}]",
                "[{\"attributes\":{\"POSITION\":0}},{\"attributes\":{\"POSITION\":0}}]",
            );
        let document = parse_compact_document(&json, None, &resources).unwrap();
        assert_eq!(document.meshes.len(), 2);
        assert_eq!(document.meshes[0].positions.len(), 9);
        assert_eq!(document.meshes[1].positions.len(), 9);
    }

    #[test]
    fn rejects_draco_extension_missing_extensions_used() {
        let json = r#"{
          "asset":{"version":"2.0"},
          "buffers":[{"byteLength":1}],
          "bufferViews":[{"buffer":0,"byteLength":1}],
          "accessors":[{"componentType":5126,"count":0,"type":"VEC3"}],
          "meshes":[{"primitives":[{
            "attributes":{"POSITION":0},
            "extensions":{"KHR_draco_mesh_compression":{"bufferView":0,"attributes":{"POSITION":0}}}
          }]}]
        }"#;
        let result = parse_compact_document(json, Some(&[0]), &[]);
        assert!(matches!(result, Err(GltfError::InvalidGltf(_))));
    }

    #[test]
    fn decodes_multi_buffer_document() {
        // Two external buffers; POSITION in buffer 0, COLOR_0 in buffer 1.
        let triangle = triangle_resource();
        let colors = [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<u8>>();
        let resources = vec![
            ("triangle.bin".to_string(), triangle),
            ("colors.bin".to_string(), colors),
        ];
        let json = r#"{
          "asset":{"version":"2.0"},
          "buffers":[{"uri":"triangle.bin","byteLength":36},{"uri":"colors.bin","byteLength":36}],
          "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":36},
            {"buffer":1,"byteOffset":0,"byteLength":36}
          ],
          "accessors":[
            {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":1,"componentType":5126,"count":3,"type":"VEC3"}
          ],
          "meshes":[{"primitives":[{"attributes":{"POSITION":0,"COLOR_0":1}}]}]
        }"#;
        let document = parse_compact_document(json, None, &resources).unwrap();
        assert_eq!(document.meshes.len(), 1);
        assert_eq!(document.meshes[0].positions.len(), 9);
        assert_eq!(document.meshes[0].colors.len(), 9);
    }

    #[test]
    fn decodes_normalized_ushort_texcoord() {
        // TEXCOORD_0 as UNSIGNED_SHORT (5123), normalized. Values [0, 65535]
        // map to [0.0, 1.0]; 0 -> 0.0, 65535 -> 1.0.
        let uvs = [0u16, 0, 65535u16, 65535, 32768u16, 32768]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<u8>>();
        let resources_with_uvs = vec![
            ("triangle.bin".to_string(), triangle_resource()),
            ("uvs.bin".to_string(), uvs),
        ];
        let json = r#"{
          "asset":{"version":"2.0"},
          "buffers":[{"uri":"triangle.bin","byteLength":36},{"uri":"uvs.bin","byteLength":12}],
          "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":36},
            {"buffer":1,"byteOffset":0,"byteLength":12}
          ],
          "accessors":[
            {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":1,"componentType":5123,"count":3,"type":"VEC2","normalized":true}
          ],
          "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1}}]}]
        }"#;
        let document = parse_compact_document(json, None, &resources_with_uvs).unwrap();
        let uvs = &document.meshes[0].uvs;
        assert_eq!(uvs.len(), 6);
        assert!((uvs[0] - 0.0).abs() < 1e-5);
        assert!((uvs[1] - 0.0).abs() < 1e-5);
        assert!((uvs[2] - 1.0).abs() < 1e-3);
        assert!((uvs[3] - 1.0).abs() < 1e-3);
        // 32768 / 65535 ~= 0.50001
        assert!((uvs[4] - 0.5).abs() < 1e-3);
        assert!((uvs[5] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn rejects_float_texcoord_with_normalized() {
        // RequiredForInteger: a FLOAT accessor must NOT be normalized.
        let resources = vec![("triangle.bin".to_string(), triangle_resource())];
        let mut bytes = triangle_resource();
        bytes.extend(
            [0.0f32, 0.0, 1.0, 1.0]
                .into_iter()
                .flat_map(f32::to_le_bytes),
        );
        let resources = vec![("triangle.bin".to_string(), bytes)];
        let json = r#"{
          "asset":{"version":"2.0"},
          "buffers":[{"uri":"triangle.bin","byteLength":52}],
          "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":36},
            {"buffer":0,"byteOffset":36,"byteLength":16}
          ],
          "accessors":[
            {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":1,"componentType":5126,"count":2,"type":"VEC2","normalized":true}
          ],
          "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1}}]}]
        }"#;
        let result = parse_compact_document(json, None, &resources);
        assert!(matches!(result, Err(GltfError::Unsupported(_))));
    }

    #[test]
    fn rejects_ubyte_texcoord_without_normalized() {
        // RequiredForInteger: an integer accessor MUST be normalized.
        let resources = vec![("triangle.bin".to_string(), triangle_resource())];
        let mut bytes = triangle_resource();
        bytes.extend([0u8, 0, 255, 255, 128, 128]);
        let resources = vec![("triangle.bin".to_string(), bytes)];
        let json = r#"{
          "asset":{"version":"2.0"},
          "buffers":[{"uri":"triangle.bin","byteLength":42}],
          "bufferViews":[
            {"buffer":0,"byteOffset":0,"byteLength":36},
            {"buffer":0,"byteOffset":36,"byteLength":6}
          ],
          "accessors":[
            {"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":1,"componentType":5121,"count":3,"type":"VEC2","normalized":false}
          ],
          "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1}}]}]
        }"#;
        let result = parse_compact_document(json, None, &resources);
        assert!(matches!(result, Err(GltfError::Unsupported(_))));
    }

    #[test]
    fn enforces_compact_limits() {
        let resources = vec![("triangle.bin".to_string(), triangle_resource())];
        let result = parse_compact_document_with_limits(
            std::str::from_utf8(&external_triangle_json()).unwrap(),
            None,
            &resources,
            &CompactLimits {
                max_result_bytes: Some(1),
                ..CompactLimits::default()
            },
        );
        assert!(matches!(result, Err(GltfError::ResourceLimitExceeded(_))));
    }

    #[test]
    fn rejects_malformed_accessor() {
        let json = r#"{
            "asset":{"version":"2.0"},
            "buffers":[{"byteLength":4}],
            "bufferViews":[{"buffer":0,"byteLength":4}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]
        }"#;
        let result = parse_compact_document(json, Some(&[0; 4]), &[]);
        assert!(matches!(result, Err(GltfError::InvalidGltf(_))));
    }

    #[test]
    fn empty_document_has_no_meshes() {
        let json = r#"{"asset":{"version":"2.0"},"meshes":[]}"#;
        let result = parse_compact_document(json, None, &[]);
        assert!(matches!(result, Err(GltfError::InvalidGltf(_))));
    }

    #[test]
    fn rejects_unsupported_required_extension() {
        let json = r#"{
            "asset":{"version":"2.0"},
            "extensionsRequired":["KHR_materials_unlit"],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]
        }"#;
        let result = parse_compact_document(json, None, &[]);
        assert!(matches!(result, Err(GltfError::Unsupported(_))));
    }

    #[test]
    fn rejects_sparse_accessor() {
        // The compact schema does not model sparse-accessor subfields, so any
        // sparse accessor is rejected (at JSON deserialization time, since
        // nanoserde rejects the unmodeled `count`/`indices`/`values` fields).
        let json = r#"{
            "asset":{"version":"2.0"},
            "buffers":[{"byteLength":12}],
            "bufferViews":[{"buffer":0,"byteLength":12}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3","sparse":{"count":0}}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]
        }"#;
        let result = parse_compact_document(json, Some(&[0; 12]), &[]);
        assert!(result.is_err());
    }

    #[test]
    fn data_uri_decoder_rejects_non_canonical_base64() {
        // `decode_data_uri` is shared with the writer/reader path; sanity-check
        // it stays strict on padding/canonical form.
        assert_eq!(decode_data_uri("data:;base64,YQ==", None).unwrap(), b"a");
        assert!(decode_data_uri("data:;base64,YQ", None).is_err());
        assert!(decode_data_uri("data:;base64,YR==", None).is_err());
        assert!(decode_data_uri("data:,a%2", None).is_err());
    }
}
