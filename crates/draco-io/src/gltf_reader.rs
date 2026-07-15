//! glTF/GLB reader with full scene graph and mesh decoding support.
//!
//! This module provides support for reading glTF 2.0 files. It supports:
//! - Draco-compressed primitives via `KHR_draco_mesh_compression`
//! - Standard (non-Draco) primitives with accessor-based geometry
//! - Full scene graph parsing (scenes, nodes, transforms, hierarchy)
//! - Both `.gltf` (JSON + separate `.bin`) and `.glb` (binary container) formats
//!
//! # Example
//!
//! ```no_run
//! use draco_io::gltf_reader::GltfReader;
//! use draco_io::SceneReader;
//!
//! let mut reader = GltfReader::open("model.glb")?;
//!
//! // Read all meshes (Draco and non-Draco)
//! let meshes = reader.decode_all_meshes()?;
//!
//! // Or read the full scene graph with transforms
//! let scene = reader.read_scene()?;
//! for node in &scene.root_nodes {
//!     println!(
//!         "Node: {:?}, mesh instances: {}",
//!         node.name,
//!         node.mesh_instances.len()
//!     );
//! }
//! # Ok::<(), draco_io::GltfError>(())
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::Path;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::PointAttribute;
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
#[cfg(feature = "point_cloud_decode")]
use draco_core::point_cloud::PointCloud;
#[cfg(feature = "point_cloud_decode")]
use draco_core::point_cloud_decoder::PointCloudDecoder;
use serde::{Deserialize, Deserializer};

use crate::gltf_container::{
    parse_gltf_container, resolve_gltf_buffers, resolve_resource_uri, ExternalFilePolicy,
    FileResourceResolver, GltfBufferReference, GltfContainerFormat, ResourceLimits,
    ResourceResolver,
};
use crate::traits::ReadFromBytes;

// The error type, the reader-agnostic geometry decoder, and the glTF numeric
// constants live in `gltf_geometry` so they are available with only the writer
// feature (the compressor reuses them without linking this reader).
use crate::gltf_geometry::{
    add_named_attribute, component_type_for_data_type, decode_geometry,
    gltf_type_for_num_components, supported_semantic_spec, validate_semantic_accessor,
    AccessorSource, DecodedAccessor, GltfError, Result, GLTF_COMPONENT_BYTE, GLTF_COMPONENT_FLOAT,
    GLTF_COMPONENT_SHORT, GLTF_COMPONENT_UNSIGNED_BYTE, GLTF_COMPONENT_UNSIGNED_INT,
    GLTF_COMPONENT_UNSIGNED_SHORT, GLTF_MODE_TRIANGLES,
};
use crate::gltf_khr_draco::{
    validate_khr_draco_contract, validate_khr_draco_document, KhrDracoExtensionContract,
    KhrDracoPrimitiveContract, KHR_DRACO_MESH_COMPRESSION,
};

// ============================================================================
// glTF JSON Schema (full scene graph support)
// ============================================================================

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GltfRoot {
    asset: Asset,
    #[serde(default)]
    accessors: Vec<Accessor>,
    #[serde(default)]
    buffer_views: Vec<BufferView>,
    #[serde(default)]
    buffers: Vec<Buffer>,
    #[serde(default)]
    images: Vec<Image>,
    #[serde(default)]
    meshes: Vec<GltfMesh>,
    #[serde(default)]
    nodes: Vec<GltfNode>,
    #[serde(default)]
    scenes: Vec<GltfScene>,
    #[serde(default)]
    skins: Vec<Skin>,
    #[serde(default)]
    animations: Vec<Animation>,
    /// Default scene index (if present).
    #[serde(default, deserialize_with = "deserialize_present_option")]
    scene: Option<usize>,
    #[serde(default)]
    extensions_used: Vec<String>,
    #[serde(default)]
    extensions_required: Vec<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Asset {
    version: String,
    min_version: Option<String>,
}

fn deserialize_present_option<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Skin {
    #[serde(default, deserialize_with = "deserialize_present_option")]
    inverse_bind_matrices: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    skeleton: Option<usize>,
    joints: Vec<usize>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Animation {
    channels: Vec<AnimationChannel>,
    samplers: Vec<AnimationSampler>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct AnimationChannel {
    sampler: usize,
    target: AnimationTarget,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct AnimationTarget {
    #[serde(default, deserialize_with = "deserialize_present_option")]
    node: Option<usize>,
    path: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct AnimationSampler {
    input: usize,
    output: usize,
    #[serde(default)]
    interpolation: Option<String>,
}

/// A glTF scene containing root node indices.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GltfScene {
    name: Option<String>,
    #[serde(default)]
    nodes: Vec<usize>,
}

/// A glTF node in the scene graph.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GltfNode {
    name: Option<String>,
    /// Index into meshes array.
    #[serde(default, deserialize_with = "deserialize_present_option")]
    mesh: Option<usize>,
    /// Child node indices.
    #[serde(default)]
    children: Vec<usize>,
    /// 4x4 transformation matrix (column-major).
    matrix: Option<[f32; 16]>,
    /// Translation (T in TRS).
    translation: Option<[f32; 3]>,
    /// Rotation quaternion [x, y, z, w] (R in TRS).
    rotation: Option<[f32; 4]>,
    /// Scale (S in TRS).
    scale: Option<[f32; 3]>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    skin: Option<usize>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Accessor {
    #[serde(default, deserialize_with = "deserialize_present_option")]
    buffer_view: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    byte_offset: Option<usize>,
    component_type: u32,
    #[serde(default)]
    normalized: bool,
    count: usize,
    #[serde(rename = "type")]
    accessor_type: String,
    #[serde(default)]
    min: Vec<f64>,
    #[serde(default)]
    max: Vec<f64>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    sparse: Option<SparseAccessor>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SparseAccessor {
    count: usize,
    indices: SparseIndices,
    values: SparseValues,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SparseIndices {
    buffer_view: usize,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    byte_offset: Option<usize>,
    component_type: u32,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SparseValues {
    buffer_view: usize,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    byte_offset: Option<usize>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BufferView {
    buffer: usize,
    byte_offset: Option<usize>,
    byte_length: usize,
    byte_stride: Option<usize>,
    target: Option<u32>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Buffer {
    byte_length: usize,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    uri: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Image {
    #[serde(default, deserialize_with = "deserialize_present_option")]
    uri: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    buffer_view: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GltfMesh {
    name: Option<String>,
    primitives: Vec<Primitive>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Primitive {
    #[serde(default)]
    attributes: BTreeMap<String, usize>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    indices: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    mode: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    material: Option<usize>,
    #[serde(default)]
    targets: Vec<BTreeMap<String, usize>>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    extensions: Option<PrimitiveExtensions>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrimitiveExtensions {
    #[serde(
        rename = "KHR_draco_mesh_compression",
        default,
        deserialize_with = "deserialize_present_option"
    )]
    khr_draco_mesh_compression: Option<DracoExtension>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DracoExtension {
    buffer_view: u32,
    #[serde(default)]
    attributes: BTreeMap<String, u32>,
}

// ============================================================================
// GltfReader
// ============================================================================

/// A reader for glTF/GLB files with Draco mesh decompression support.
pub struct GltfReader {
    root: GltfRoot,
    buffers: Vec<Vec<u8>>,
}

/// Lightweight scene metadata exposed without reparsing the JSON document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfDocumentMetadata {
    /// Mesh names repeated once per primitive, in mesh-major order.
    pub primitive_names: Vec<Option<String>>,
    pub nodes: Vec<GltfNodeMetadata>,
    pub scenes: Vec<GltfSceneMetadata>,
    pub default_scene: Option<usize>,
    pub uses_draco: bool,
    /// Non-data companion URIs referenced by buffers or images, deduplicated.
    pub external_resource_uris: Vec<String>,
}

/// Node fields used by lightweight front ends.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GltfNodeMetadata {
    pub name: Option<String>,
    pub mesh: Option<usize>,
    pub translation: Option<[f32; 3]>,
    pub rotation: Option<[f32; 4]>,
    pub scale: Option<[f32; 3]>,
    pub children: Vec<usize>,
}

/// Scene name and root-node indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GltfSceneMetadata {
    pub name: Option<String>,
    pub nodes: Vec<usize>,
}

// `DecodedAccessor` and the `AccessorSource` trait now live in `gltf_geometry`;
// `GltfAccessorReader` is this crate's implementation over a parsed `GltfRoot`.
struct GltfAccessorReader<'a> {
    accessors: &'a [Accessor],
    buffer_views: &'a [BufferView],
    buffers: &'a [Vec<u8>],
}

impl AccessorSource for GltfAccessorReader<'_> {
    fn read_attribute(
        &self,
        accessor: usize,
        expected_types: &[&str],
        allowed_component_types: &[u32],
    ) -> Result<DecodedAccessor> {
        GltfAccessorReader::read_attribute(self, accessor, expected_types, allowed_component_types)
    }

    fn read_indices(&self, accessor: usize) -> Result<Vec<u32>> {
        GltfAccessorReader::read_indices(self, accessor)
    }
}

impl<'a> GltfAccessorReader<'a> {
    fn new(root: &'a GltfRoot, buffers: &'a [Vec<u8>]) -> Self {
        Self {
            accessors: &root.accessors,
            buffer_views: &root.buffer_views,
            buffers,
        }
    }

    fn read_attribute(
        &self,
        accessor_idx: usize,
        expected_types: &[&str],
        allowed_component_types: &[u32],
    ) -> Result<DecodedAccessor> {
        let accessor = self.accessor(accessor_idx)?;

        if !expected_types
            .iter()
            .any(|expected| accessor.accessor_type == *expected)
        {
            return Err(GltfError::InvalidGltf(format!(
                "Expected one of {:?} accessor, got {}",
                expected_types, accessor.accessor_type
            )));
        }

        if !allowed_component_types.contains(&accessor.component_type) {
            return Err(GltfError::Unsupported(format!(
                "Unsupported {} component type: {}",
                accessor.accessor_type, accessor.component_type
            )));
        }

        let num_components = accessor_num_components(&accessor.accessor_type)?;
        let data_type = data_type_for_component_type(accessor.component_type)?;
        let component_size = data_type.byte_length();
        let row_size = (num_components as usize)
            .checked_mul(component_size)
            .ok_or_else(|| GltfError::InvalidGltf("Accessor row size overflow".into()))?;
        let layout = self.accessor_layout(accessor, row_size, component_size, true, "Accessor")?;

        let byte_len = accessor
            .count
            .checked_mul(row_size)
            .ok_or_else(|| GltfError::InvalidGltf("Accessor byte size overflow".into()))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(byte_len)
            .map_err(|_| GltfError::ResourceLimitExceeded("Accessor allocation failed".into()))?;
        for i in 0..accessor.count {
            let relative = i
                .checked_mul(layout.stride)
                .ok_or_else(|| GltfError::InvalidGltf("Accessor range overflow".into()))?;
            let offset = layout
                .start
                .checked_add(relative)
                .ok_or_else(|| GltfError::InvalidGltf("Accessor range overflow".into()))?;
            let end = offset
                .checked_add(row_size)
                .filter(|end| *end <= layout.view_end)
                .ok_or_else(|| {
                    GltfError::InvalidGltf(format!(
                        "{} accessor out of bounds",
                        accessor.accessor_type
                    ))
                })?;
            if end > layout.buffer.len() {
                return Err(GltfError::InvalidGltf(format!(
                    "{} accessor out of bounds",
                    accessor.accessor_type
                )));
            }
            bytes.extend_from_slice(&layout.buffer[offset..end]);
        }

        DecodedAccessor::new(
            accessor.count,
            num_components,
            data_type,
            accessor.normalized,
            bytes,
        )
    }

    fn read_indices(&self, accessor_idx: usize) -> Result<Vec<u32>> {
        let accessor = self.accessor(accessor_idx)?;

        if accessor.accessor_type != "SCALAR" {
            return Err(GltfError::InvalidGltf(format!(
                "Expected SCALAR accessor for indices, got {}",
                accessor.accessor_type
            )));
        }
        if accessor.normalized {
            return Err(GltfError::InvalidGltf(
                "Index accessor must not be normalized".into(),
            ));
        }

        let component_size = match accessor.component_type {
            GLTF_COMPONENT_UNSIGNED_BYTE => 1,
            GLTF_COMPONENT_UNSIGNED_SHORT => 2,
            GLTF_COMPONENT_UNSIGNED_INT => 4,
            _ => {
                return Err(GltfError::Unsupported(format!(
                    "Unsupported index component type: {}",
                    accessor.component_type
                )));
            }
        };
        let layout = self.accessor_layout(
            accessor,
            component_size,
            component_size,
            false,
            "Index accessor",
        )?;
        let mut result = Vec::new();
        result.try_reserve_exact(accessor.count).map_err(|_| {
            GltfError::ResourceLimitExceeded("Index accessor allocation failed".into())
        })?;

        for index in 0..accessor.count {
            let bytes = layout.element(index, component_size, "Index accessor")?;
            let value = match accessor.component_type {
                GLTF_COMPONENT_UNSIGNED_BYTE => bytes[0] as u32,
                GLTF_COMPONENT_UNSIGNED_SHORT => u16::from_le_bytes([bytes[0], bytes[1]]) as u32,
                GLTF_COMPONENT_UNSIGNED_INT => {
                    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
                }
                component_type => {
                    return Err(GltfError::Unsupported(format!(
                        "Unsupported index component type: {component_type}"
                    )));
                }
            };
            result.push(value);
        }

        Ok(result)
    }

    fn accessor(&self, accessor_idx: usize) -> Result<&Accessor> {
        self.accessors.get(accessor_idx).ok_or_else(|| {
            GltfError::InvalidGltf(format!("Invalid accessor index: {}", accessor_idx))
        })
    }

    fn accessor_layout(
        &self,
        accessor: &Accessor,
        element_size: usize,
        component_size: usize,
        vertex_attribute: bool,
        label: &str,
    ) -> Result<AccessorLayout<'a>> {
        if accessor.sparse.is_some() {
            return Err(GltfError::Unsupported(
                "Sparse accessors are not supported".into(),
            ));
        }
        if accessor.count == 0 {
            return Err(GltfError::InvalidGltf(format!(
                "{} count must be greater than zero",
                label
            )));
        }

        let buffer_view_idx = accessor
            .buffer_view
            .ok_or_else(|| GltfError::InvalidGltf(format!("{} has no bufferView", label)))?;

        let buffer_view = self.buffer_views.get(buffer_view_idx).ok_or_else(|| {
            GltfError::InvalidGltf(format!("Invalid bufferView index: {}", buffer_view_idx))
        })?;

        let buffer = self.buffers.get(buffer_view.buffer).ok_or_else(|| {
            GltfError::InvalidGltf(format!("Invalid buffer index: {}", buffer_view.buffer))
        })?;

        let view_offset = buffer_view.byte_offset.unwrap_or(0);
        let accessor_offset = accessor.byte_offset.unwrap_or(0);
        if !accessor_offset.is_multiple_of(component_size) {
            return Err(GltfError::InvalidGltf(format!(
                "{} byteOffset is not aligned to component size {}",
                label, component_size
            )));
        }

        let start = view_offset
            .checked_add(accessor_offset)
            .ok_or_else(|| GltfError::InvalidGltf("Accessor start overflow".into()))?;
        if start % component_size != 0 {
            return Err(GltfError::InvalidGltf(format!(
                "{} absolute byte offset is not aligned to component size {}",
                label, component_size
            )));
        }
        if !vertex_attribute && buffer_view.byte_stride.is_some() {
            return Err(GltfError::InvalidGltf(format!(
                "{} bufferView must not define byteStride",
                label
            )));
        }

        let stride = buffer_view.byte_stride.unwrap_or(element_size);

        if stride < element_size {
            return Err(GltfError::InvalidGltf(format!(
                "{} byteStride {} is smaller than element size {}",
                label, stride, element_size
            )));
        }
        if stride % component_size != 0 {
            return Err(GltfError::InvalidGltf(format!(
                "{} byteStride {} is not aligned to component size {}",
                label, stride, component_size
            )));
        }
        if let Some(byte_stride) = buffer_view.byte_stride {
            if !(4..=252).contains(&byte_stride) {
                return Err(GltfError::InvalidGltf(format!(
                    "{} byteStride {} is outside glTF range 4..=252",
                    label, byte_stride
                )));
            }
            if vertex_attribute && byte_stride % 4 != 0 {
                return Err(GltfError::InvalidGltf(format!(
                    "{} byteStride {} is not 4-byte aligned",
                    label, byte_stride
                )));
            }
        }

        let view_end = view_offset
            .checked_add(buffer_view.byte_length)
            .ok_or_else(|| GltfError::InvalidGltf("Buffer view range overflow".into()))?;
        if start > view_end {
            return Err(GltfError::InvalidGltf(format!(
                "{} starts past bufferView end",
                label
            )));
        }
        let byte_len = stride
            .checked_mul(accessor.count - 1)
            .and_then(|prefix| prefix.checked_add(element_size))
            .ok_or_else(|| GltfError::InvalidGltf("Accessor byte range overflow".into()))?;
        let accessor_end = start
            .checked_add(byte_len)
            .ok_or_else(|| GltfError::InvalidGltf("Accessor byte range overflow".into()))?;
        if accessor_end > view_end {
            return Err(GltfError::InvalidGltf(format!(
                "{} accessor does not fit its bufferView",
                label
            )));
        }
        if view_end > buffer.len() {
            return Err(GltfError::InvalidGltf(
                "Buffer view extends past buffer end".into(),
            ));
        }

        Ok(AccessorLayout {
            buffer,
            start,
            stride,
            view_end,
        })
    }
}

struct AccessorLayout<'a> {
    buffer: &'a [u8],
    start: usize,
    stride: usize,
    view_end: usize,
}

impl<'a> AccessorLayout<'a> {
    fn element(&self, index: usize, size: usize, label: &str) -> Result<&'a [u8]> {
        let relative = index
            .checked_mul(self.stride)
            .ok_or_else(|| GltfError::InvalidGltf(format!("{label} range overflow")))?;
        let start = self
            .start
            .checked_add(relative)
            .ok_or_else(|| GltfError::InvalidGltf(format!("{label} range overflow")))?;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= self.view_end)
            .ok_or_else(|| GltfError::InvalidGltf(format!("{label} out of bounds")))?;
        self.buffer
            .get(start..end)
            .ok_or_else(|| GltfError::InvalidGltf(format!("{label} out of bounds")))
    }
}

/// Information about a Draco-compressed primitive within a glTF mesh.
#[derive(Debug, Clone)]
pub struct DracoPrimitiveInfo {
    /// Index of the mesh in the glTF file.
    pub mesh_index: usize,
    /// Name of the mesh (if available).
    pub mesh_name: Option<String>,
    /// Index of the primitive within the mesh.
    pub primitive_index: usize,
    /// Buffer view index containing the Draco data.
    pub buffer_view: usize,
    /// Attribute mappings from glTF semantic to Draco attribute ID.
    pub attributes: BTreeMap<String, u32>,
}

impl GltfReader {
    /// Open a glTF or GLB file.
    ///
    /// The file type is detected automatically based on the magic bytes.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let data = fs::read(path)?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let resolver = FileResourceResolver::new(base, ExternalFilePolicy::Allow);
        Self::from_bytes_with_resolver(&data, &resolver, &ResourceLimits::default())
    }

    /// Parse from GLB binary data.
    pub fn from_glb(data: &[u8]) -> Result<Self> {
        let container = parse_gltf_container(data)?;
        if container.format != GltfContainerFormat::Glb {
            return Err(GltfError::InvalidGlb("input is not a GLB container".into()));
        }
        Self::from_bytes(data)
    }

    /// Parse from glTF JSON or GLB binary data.
    ///
    /// The payload type is detected automatically from the GLB magic bytes.
    /// For glTF JSON with external buffers, use [`Self::from_bytes_with_base_path`].
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        Self::from_bytes_impl(data, None, &ResourceLimits::default(), false)
    }

    /// Parse from glTF JSON or GLB binary data with an optional base path for external buffers.
    pub fn from_bytes_with_base_path(data: &[u8], base_path: Option<&Path>) -> Result<Self> {
        if let Some(base) = base_path {
            let resolver = FileResourceResolver::new(base, ExternalFilePolicy::Allow);
            Self::from_bytes_with_resolver(data, &resolver, &ResourceLimits::default())
        } else {
            Self::from_bytes_impl(data, None, &ResourceLimits::default(), false)
        }
    }

    /// Parse with a caller-provided external resource resolver and quotas.
    pub fn from_bytes_with_resolver(
        data: &[u8],
        resolver: &dyn ResourceResolver,
        limits: &ResourceLimits,
    ) -> Result<Self> {
        Self::from_bytes_impl(data, Some(resolver), limits, false)
    }

    /// Parse from glTF JSON data with optional base path for external buffers.
    pub fn from_gltf(json_data: &[u8], base_path: Option<&Path>) -> Result<Self> {
        let container = parse_gltf_container(json_data)?;
        if container.format != GltfContainerFormat::Gltf {
            return Err(GltfError::InvalidGltf("input is a GLB container".into()));
        }
        Self::from_bytes_with_base_path(json_data, base_path)
    }

    /// Parse glTF/GLB bytes, decoding geometry even when the asset uses scene
    /// features this crate does not model (skins, animations, morph targets).
    ///
    /// Unlike [`Self::from_bytes`], which rejects such assets, this reader
    /// ignores those features and decodes only geometry. Use it to read meshes
    /// out of skinned or animated assets, including the output of
    /// [`crate::compress_gltf_bytes`] for those assets. Per-primitive decoding
    /// still fails for unsupported attribute layouts.
    pub fn from_bytes_lenient(data: &[u8]) -> Result<Self> {
        Self::from_bytes_impl(data, None, &ResourceLimits::default(), true)
    }

    /// Like [`Self::from_bytes_lenient`], with a base path for external buffers.
    pub fn from_bytes_lenient_with_base_path(
        data: &[u8],
        base_path: Option<&Path>,
    ) -> Result<Self> {
        if let Some(base) = base_path {
            let resolver = FileResourceResolver::new(base, ExternalFilePolicy::Allow);
            Self::from_bytes_lenient_with_resolver(data, &resolver, &ResourceLimits::default())
        } else {
            Self::from_bytes_impl(data, None, &ResourceLimits::default(), true)
        }
    }

    /// Lenient geometry parse with a caller-provided resolver and quotas.
    pub fn from_bytes_lenient_with_resolver(
        data: &[u8],
        resolver: &dyn ResourceResolver,
        limits: &ResourceLimits,
    ) -> Result<Self> {
        Self::from_bytes_impl(data, Some(resolver), limits, true)
    }

    fn from_bytes_impl(
        data: &[u8],
        resolver: Option<&dyn ResourceResolver>,
        limits: &ResourceLimits,
        lenient: bool,
    ) -> Result<Self> {
        let container = parse_gltf_container(data)?;
        let root: GltfRoot = serde_json::from_slice(container.json)?;
        validate_typed_khr_draco_document(&root)?;
        validate_root_metadata(&root)?;
        if !lenient {
            reject_unsupported_features(&root)?;
        }
        let buffers = load_buffers(
            &root,
            container.format == GltfContainerFormat::Glb,
            container.bin,
            resolver,
            limits,
        )?;
        validate_images(&root, &buffers, resolver, limits)?;
        Ok(Self { root, buffers })
    }

    /// Builds a lenient reader from an already-parsed glTF document (`doc`) and
    /// its resolved buffer bytes.
    ///
    /// This lets a caller that already holds a parsed scene and its buffers
    /// (for example a `gltf-rs` document and the bytes it resolved) decode
    /// geometry through this reader without serializing back to glTF/GLB bytes
    /// and re-resolving the buffers. The same lenient policy as
    /// [`Self::from_bytes_lenient`] applies: skins, animations, and morph
    /// targets are ignored (not rejected), and per-primitive decoding still
    /// fails for unsupported attribute layouts.
    ///
    /// `buffers` must already be resolved and indexed by glTF buffer index; no
    /// URI or BIN-chunk resolution is performed here.
    pub fn from_value(doc: &serde_json::Value, buffers: Vec<Vec<u8>>) -> Result<Self> {
        validate_khr_draco_document(doc)?;
        let root: GltfRoot = serde_json::from_value(doc.clone())?;
        validate_root_metadata(&root)?;
        Ok(Self { root, buffers })
    }

    /// Resolved buffer bytes, indexed by glTF buffer index. Used by the
    /// byte-API compressor, which also requires the writer feature.
    #[cfg(feature = "gltf-writer")]
    pub(crate) fn buffers(&self) -> &[Vec<u8>] {
        &self.buffers
    }

    /// Decode a single non-Draco primitive, returning the mesh and the
    /// `(glTF semantic, Draco unique id)` mapping for its attributes.
    ///
    /// Used by the compressor to build the `KHR_draco_mesh_compression`
    /// attributes map with the original glTF semantic names (including
    /// `TANGENT`, `JOINTS_n`, `WEIGHTS_n`, extra `TEXCOORD_n`/`COLOR_n`, and
    /// custom `_*` attributes), which the Draco attribute model alone cannot
    /// preserve. Errors if the primitive is already Draco-compressed.
    ///
    /// This is the geometry-decode callback expected by
    /// [`crate::compress_gltf_value`], so a caller holding a parsed scene can
    /// drive the compressor: build a reader with [`Self::from_value`] and pass
    /// `|mesh, prim| reader.decode_primitive_with_semantics(mesh, prim)`.
    pub fn decode_primitive_with_semantics(
        &self,
        mesh_idx: usize,
        prim_idx: usize,
    ) -> Result<(Mesh, Vec<(String, u32)>)> {
        let gltf_mesh = self.root.meshes.get(mesh_idx).ok_or_else(|| {
            GltfError::InvalidGltf(format!("Mesh index {} out of range", mesh_idx))
        })?;
        let primitive = gltf_mesh.primitives.get(prim_idx).ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "Primitive index {}:{} out of range",
                mesh_idx, prim_idx
            ))
        })?;
        if primitive
            .extensions
            .as_ref()
            .and_then(|ext| ext.khr_draco_mesh_compression.as_ref())
            .is_some()
        {
            return Err(GltfError::Unsupported(
                "primitive is already Draco-compressed".into(),
            ));
        }
        self.decode_standard_primitive(mesh_idx, prim_idx, primitive)
    }

    /// Check if the glTF file uses Draco compression.
    pub fn has_draco_extension(&self) -> bool {
        self.root
            .extensions_used
            .iter()
            .any(|ext| ext == KHR_DRACO_MESH_COMPRESSION)
    }

    /// Return lightweight metadata from the already-parsed document.
    pub fn document_metadata(&self) -> GltfDocumentMetadata {
        let external_resource_uris = self
            .root
            .buffers
            .iter()
            .filter_map(|buffer| buffer.uri.as_deref())
            .chain(
                self.root
                    .images
                    .iter()
                    .filter_map(|image| image.uri.as_deref()),
            )
            .filter(|uri| !uri.starts_with("data:"))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        GltfDocumentMetadata {
            primitive_names: self
                .root
                .meshes
                .iter()
                .flat_map(|mesh| std::iter::repeat_n(mesh.name.clone(), mesh.primitives.len()))
                .collect(),
            nodes: self
                .root
                .nodes
                .iter()
                .map(|node| GltfNodeMetadata {
                    name: node.name.clone(),
                    mesh: node.mesh,
                    translation: node.translation,
                    rotation: node.rotation,
                    scale: node.scale,
                    children: node.children.clone(),
                })
                .collect(),
            scenes: self
                .root
                .scenes
                .iter()
                .map(|scene| GltfSceneMetadata {
                    name: scene.name.clone(),
                    nodes: scene.nodes.clone(),
                })
                .collect(),
            default_scene: self.root.scene,
            uses_draco: self.has_draco_extension(),
            external_resource_uris,
        }
    }

    /// Get information about all Draco-compressed primitives.
    pub fn draco_primitives(&self) -> Vec<DracoPrimitiveInfo> {
        let mut result = Vec::new();

        for (mesh_idx, mesh) in self.root.meshes.iter().enumerate() {
            for (prim_idx, primitive) in mesh.primitives.iter().enumerate() {
                if let Some(ext) = &primitive.extensions {
                    if let Some(draco) = &ext.khr_draco_mesh_compression {
                        result.push(DracoPrimitiveInfo {
                            mesh_index: mesh_idx,
                            mesh_name: mesh.name.clone(),
                            primitive_index: prim_idx,
                            buffer_view: draco.buffer_view as usize,
                            attributes: draco.attributes.clone(),
                        });
                    }
                }
            }
        }

        result
    }

    /// Get the raw Draco-compressed data for a primitive.
    pub fn get_draco_data(&self, info: &DracoPrimitiveInfo) -> Result<&[u8]> {
        let buffer_view = self
            .root
            .buffer_views
            .get(info.buffer_view)
            .ok_or_else(|| {
                GltfError::InvalidGltf(format!("Invalid buffer view index: {}", info.buffer_view))
            })?;

        let buffer = self.buffers.get(buffer_view.buffer).ok_or_else(|| {
            GltfError::InvalidGltf(format!("Invalid buffer index: {}", buffer_view.buffer))
        })?;

        let offset = buffer_view.byte_offset.unwrap_or(0);
        let end = offset
            .checked_add(buffer_view.byte_length)
            .ok_or_else(|| GltfError::InvalidGltf("Buffer view range overflow".into()))?;

        if end > buffer.len() {
            return Err(GltfError::InvalidGltf(
                "Buffer view extends past buffer end".into(),
            ));
        }

        Ok(&buffer[offset..end])
    }

    /// Decode a Draco-compressed primitive as a Mesh.
    pub fn decode_draco_mesh(&self, info: &DracoPrimitiveInfo) -> Result<Mesh> {
        let data = self.get_draco_data(info)?;
        let mut decoder_buffer = DecoderBuffer::new(data);
        let mut mesh = Mesh::new();
        let mut decoder = MeshDecoder::new();

        decoder
            .decode(&mut decoder_buffer, &mut mesh)
            .map_err(GltfError::DracoDecode)?;

        let primitive = self.primitive_for_draco_info(info)?;
        self.validate_draco_primitive_metadata(info, primitive, &mesh)?;
        self.add_draco_side_attributes(&mut mesh, primitive, info)?;

        Ok(mesh)
    }

    /// Decode a Draco-compressed primitive as a PointCloud.
    #[cfg(feature = "point_cloud_decode")]
    pub fn decode_draco_point_cloud(&self, info: &DracoPrimitiveInfo) -> Result<PointCloud> {
        let data = self.get_draco_data(info)?;
        let mut decoder_buffer = DecoderBuffer::new(data);
        let mut point_cloud = PointCloud::new();
        let mut decoder = PointCloudDecoder::new();

        decoder
            .decode(&mut decoder_buffer, &mut point_cloud)
            .map_err(GltfError::DracoDecode)?;

        Ok(point_cloud)
    }

    /// Decode all Draco-compressed primitives as meshes.
    pub fn decode_all_draco_meshes(&self) -> Result<Vec<(DracoPrimitiveInfo, Mesh)>> {
        let primitives = self.draco_primitives();
        let mut result = Vec::with_capacity(primitives.len());

        for info in primitives {
            let mesh = self.decode_draco_mesh(&info)?;
            result.push((info, mesh));
        }

        Ok(result)
    }

    // ========================================================================
    // Non-Draco Mesh Decoding
    // ========================================================================

    /// Decode a non-Draco primitive from accessors/bufferViews.
    ///
    /// Returns the decoded mesh plus the `(glTF semantic, Draco unique id)`
    /// mapping for each attribute, in attribute add order. The unique id equals
    /// the attribute index, which is what the `KHR_draco_mesh_compression`
    /// attributes map references.
    fn decode_standard_primitive(
        &self,
        _mesh_idx: usize,
        _prim_idx: usize,
        primitive: &Primitive,
    ) -> Result<(Mesh, Vec<(String, u32)>)> {
        let mode = primitive.mode.unwrap_or(GLTF_MODE_TRIANGLES);
        let attributes: Vec<(String, usize)> = primitive
            .attributes
            .iter()
            .map(|(semantic, accessor)| (semantic.clone(), *accessor))
            .collect();
        decode_geometry(
            &self.accessor_reader(),
            mode,
            &attributes,
            primitive.indices,
        )
    }

    fn accessor_reader(&self) -> GltfAccessorReader<'_> {
        GltfAccessorReader::new(&self.root, &self.buffers)
    }

    fn decode_primitive_mesh(
        &self,
        mesh_idx: usize,
        gltf_mesh: &GltfMesh,
        prim_idx: usize,
        primitive: &Primitive,
    ) -> Result<Mesh> {
        if let Some(draco) = primitive
            .extensions
            .as_ref()
            .and_then(|ext| ext.khr_draco_mesh_compression.as_ref())
        {
            let info = DracoPrimitiveInfo {
                mesh_index: mesh_idx,
                mesh_name: gltf_mesh.name.clone(),
                primitive_index: prim_idx,
                buffer_view: draco.buffer_view as usize,
                attributes: draco.attributes.clone(),
            };
            self.decode_draco_mesh(&info)
        } else {
            self.decode_standard_primitive(mesh_idx, prim_idx, primitive)
                .map(|(mesh, _)| mesh)
        }
    }

    fn primitive_for_draco_info(&self, info: &DracoPrimitiveInfo) -> Result<&Primitive> {
        let mesh = self.root.meshes.get(info.mesh_index).ok_or_else(|| {
            GltfError::InvalidGltf(format!("Invalid mesh index: {}", info.mesh_index))
        })?;
        let primitive = mesh.primitives.get(info.primitive_index).ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "Invalid primitive index {} for mesh {}",
                info.primitive_index, info.mesh_index
            ))
        })?;
        let draco = primitive
            .extensions
            .as_ref()
            .and_then(|ext| ext.khr_draco_mesh_compression.as_ref())
            .ok_or_else(|| {
                GltfError::InvalidGltf(format!(
                    "Primitive {}:{} does not use {}",
                    info.mesh_index, info.primitive_index, KHR_DRACO_MESH_COMPRESSION
                ))
            })?;
        if draco.buffer_view as usize != info.buffer_view || draco.attributes != info.attributes {
            return Err(GltfError::InvalidGltf(
                "Draco primitive info does not match source primitive".into(),
            ));
        }
        Ok(primitive)
    }

    fn validate_draco_primitive_metadata(
        &self,
        info: &DracoPrimitiveInfo,
        primitive: &Primitive,
        mesh: &Mesh,
    ) -> Result<()> {
        let mode = primitive.mode.unwrap_or(GLTF_MODE_TRIANGLES);
        if mode != GLTF_MODE_TRIANGLES && mode != 5 {
            return Err(GltfError::Unsupported(format!(
                "{} supports only TRIANGLES=4 or TRIANGLE_STRIP=5, got mode {}",
                KHR_DRACO_MESH_COMPRESSION, mode
            )));
        }

        for (semantic, &draco_attribute_id) in &info.attributes {
            let Some(accessor_idx) = primitive.attributes.get(semantic) else {
                return Err(GltfError::InvalidGltf(format!(
                    "{} attribute {} is not present in primitive.attributes",
                    KHR_DRACO_MESH_COMPRESSION, semantic
                )));
            };
            let attribute_spec = supported_semantic_spec(semantic)?;
            let attribute = mesh
                .attribute_by_unique_id(draco_attribute_id)
                .ok_or_else(|| {
                    GltfError::InvalidGltf(format!(
                        "Draco unique attribute id {} for {} is absent",
                        draco_attribute_id, semantic
                    ))
                })?;
            if attribute.attribute_type() != attribute_spec.attribute_type {
                return Err(GltfError::InvalidGltf(format!(
                    "Draco attribute {} has type {:?}, expected {:?}",
                    semantic,
                    attribute.attribute_type(),
                    attribute_spec.attribute_type
                )));
            }
            self.validate_accessor_matches_attribute(*accessor_idx, semantic, attribute)?;
        }

        if let Some(indices_accessor_idx) = primitive.indices {
            let accessor = self
                .root
                .accessors
                .get(indices_accessor_idx)
                .ok_or_else(|| {
                    GltfError::InvalidGltf(format!(
                        "Invalid indices accessor index: {}",
                        indices_accessor_idx
                    ))
                })?;
            if accessor.sparse.is_some() {
                return Err(GltfError::Unsupported(
                    "Sparse index accessors are not supported".into(),
                ));
            }
            if accessor.accessor_type != "SCALAR" {
                return Err(GltfError::InvalidGltf(format!(
                    "Expected SCALAR accessor for Draco indices, got {}",
                    accessor.accessor_type
                )));
            }
            if accessor.normalized {
                return Err(GltfError::InvalidGltf(
                    "Draco indices accessor must not be normalized".into(),
                ));
            }
            if ![
                GLTF_COMPONENT_UNSIGNED_BYTE,
                GLTF_COMPONENT_UNSIGNED_SHORT,
                GLTF_COMPONENT_UNSIGNED_INT,
            ]
            .contains(&accessor.component_type)
            {
                return Err(GltfError::Unsupported(format!(
                    "Unsupported Draco index accessor component type: {}",
                    accessor.component_type
                )));
            }
            let expected_count = if mode == GLTF_MODE_TRIANGLES {
                mesh.num_faces().checked_mul(3)
            } else {
                mesh.num_faces().checked_add(2)
            }
            .ok_or_else(|| GltfError::InvalidGltf("decoded index count overflow".into()))?;
            if accessor.count != expected_count {
                return Err(GltfError::InvalidGltf(format!(
                    "Draco indices accessor count {} does not match decoded index count {}",
                    accessor.count, expected_count
                )));
            }
        }

        Ok(())
    }

    fn validate_accessor_matches_attribute(
        &self,
        accessor_idx: usize,
        semantic: &str,
        attribute: &PointAttribute,
    ) -> Result<()> {
        let accessor = self.root.accessors.get(accessor_idx).ok_or_else(|| {
            GltfError::InvalidGltf(format!("Invalid accessor index: {}", accessor_idx))
        })?;
        validate_semantic_accessor(
            semantic,
            &accessor.accessor_type,
            accessor.component_type,
            accessor.normalized,
        )?;
        if accessor.sparse.is_some() {
            return Err(GltfError::Unsupported(format!(
                "Sparse accessor for {} is not supported",
                semantic
            )));
        }
        let expected_accessor_type = gltf_type_for_num_components(attribute.num_components())?;
        if accessor.accessor_type != expected_accessor_type {
            return Err(GltfError::InvalidGltf(format!(
                "{} accessor type {} does not match decoded attribute type {}",
                semantic, accessor.accessor_type, expected_accessor_type
            )));
        }
        let expected_component_type = component_type_for_data_type(attribute.data_type())?;
        let attribute_spec = supported_semantic_spec(semantic)?;
        if !attribute_spec
            .allowed_component_types
            .contains(&expected_component_type)
        {
            return Err(GltfError::Unsupported(format!(
                "{} decoded component type {} is not supported by draco-io glTF",
                semantic, expected_component_type
            )));
        }
        if accessor.component_type != expected_component_type {
            return Err(GltfError::InvalidGltf(format!(
                "{} accessor componentType {} does not match decoded componentType {}",
                semantic, accessor.component_type, expected_component_type
            )));
        }
        if accessor.normalized != attribute.normalized() {
            return Err(GltfError::InvalidGltf(format!(
                "{} accessor normalized={} does not match decoded normalized={}",
                semantic,
                accessor.normalized,
                attribute.normalized()
            )));
        }
        if accessor.count != attribute.size() {
            return Err(GltfError::InvalidGltf(format!(
                "{} accessor count {} does not match decoded attribute count {}",
                semantic,
                accessor.count,
                attribute.size()
            )));
        }
        Ok(())
    }

    fn add_draco_side_attributes(
        &self,
        mesh: &mut Mesh,
        primitive: &Primitive,
        info: &DracoPrimitiveInfo,
    ) -> Result<()> {
        let accessor_reader = self.accessor_reader();
        let mut attributes: Vec<_> = primitive.attributes.iter().collect();
        attributes.sort_by_key(|(left, _)| *left);

        for (semantic, accessor_idx) in attributes {
            if info.attributes.contains_key(semantic) {
                continue;
            }
            add_named_attribute(mesh, &accessor_reader, semantic, *accessor_idx, None)?;
        }
        Ok(())
    }

    /// Get the number of meshes in the glTF file.
    pub fn num_meshes(&self) -> usize {
        self.root.meshes.len()
    }

    /// Get the number of buffers in the glTF file.
    pub fn num_buffers(&self) -> usize {
        self.buffers.len()
    }

    /// Get the extensions used by this glTF file.
    pub fn extensions_used(&self) -> &[String] {
        &self.root.extensions_used
    }

    /// Get the extensions required by this glTF file.
    pub fn extensions_required(&self) -> &[String] {
        &self.root.extensions_required
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn validate_typed_khr_draco_document(root: &GltfRoot) -> Result<()> {
    let used = root
        .extensions_used
        .iter()
        .any(|extension| extension == KHR_DRACO_MESH_COMPRESSION);
    let required = root
        .extensions_required
        .iter()
        .any(|extension| extension == KHR_DRACO_MESH_COMPRESSION);
    if required && !used {
        return Err(GltfError::InvalidGltf(format!(
            "{KHR_DRACO_MESH_COMPRESSION} is required but is not listed in extensionsUsed"
        )));
    }

    for mesh in &root.meshes {
        for primitive in &mesh.primitives {
            let Some(extension) = primitive
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.khr_draco_mesh_compression.as_ref())
            else {
                continue;
            };
            validate_khr_draco_contract(
                KhrDracoPrimitiveContract {
                    extension: KhrDracoExtensionContract {
                        buffer_view: extension.buffer_view as usize,
                        attributes: &extension.attributes,
                    },
                    primitive_attributes: primitive
                        .attributes
                        .iter()
                        .map(|(semantic, accessor)| (semantic.as_str(), *accessor)),
                    indices: primitive.indices,
                    mode: primitive.mode.unwrap_or(GLTF_MODE_TRIANGLES),
                    extension_used: used,
                    extension_required: required,
                    buffer_view_count: root.buffer_views.len(),
                },
                |accessor| {
                    root.accessors
                        .get(accessor)
                        .map(|accessor| accessor.buffer_view.is_some() || accessor.sparse.is_some())
                },
            )?;
        }
    }
    Ok(())
}

fn validate_root_metadata(root: &GltfRoot) -> Result<()> {
    if root.asset.version != "2.0" {
        return Err(GltfError::Unsupported(format!(
            "Unsupported glTF asset version: {}",
            root.asset.version
        )));
    }
    if let Some(min_version) = &root.asset.min_version {
        if min_version != "2.0" {
            return Err(GltfError::Unsupported(format!(
                "Unsupported glTF minimum version: {}",
                min_version
            )));
        }
    }

    // glTF validity: every required extension must also be listed as used.
    // Whether an unknown required extension is *acceptable* is a scope decision
    // left to reject_unsupported_features (strict readers only); the lenient
    // document-preserving path tolerates them since it preserves, not
    // interprets, the rest of the document.
    for required in &root.extensions_required {
        if !root.extensions_used.iter().any(|used| used == required) {
            return Err(GltfError::InvalidGltf(format!(
                "Required extension {} is not listed in extensionsUsed",
                required
            )));
        }
    }

    let mut has_draco_primitive = false;
    for mesh in &root.meshes {
        for primitive in &mesh.primitives {
            if primitive
                .extensions
                .as_ref()
                .and_then(|ext| ext.khr_draco_mesh_compression.as_ref())
                .is_some()
            {
                has_draco_primitive = true;
            }
        }
    }
    if has_draco_primitive
        && !root
            .extensions_used
            .iter()
            .any(|used| used == KHR_DRACO_MESH_COMPRESSION)
    {
        return Err(GltfError::InvalidGltf(format!(
            "Primitive uses {} but extensionsUsed does not list it",
            KHR_DRACO_MESH_COMPRESSION
        )));
    }

    validate_document_references(root)?;

    Ok(())
}

fn validate_reference(index: usize, count: usize, label: &str) -> Result<()> {
    if index >= count {
        return Err(GltfError::InvalidGltf(format!(
            "{label} {index} is out of range for {count} entries"
        )));
    }
    Ok(())
}

fn validate_document_references(root: &GltfRoot) -> Result<()> {
    if let Some(scene) = root.scene {
        validate_reference(scene, root.scenes.len(), "default scene")?;
    }
    for (scene_index, scene) in root.scenes.iter().enumerate() {
        for &node in &scene.nodes {
            validate_reference(node, root.nodes.len(), &format!("scene {scene_index} node"))?;
        }
    }

    for (node_index, node) in root.nodes.iter().enumerate() {
        if let Some(mesh) = node.mesh {
            validate_reference(mesh, root.meshes.len(), &format!("node {node_index} mesh"))?;
        }
        if let Some(skin) = node.skin {
            validate_reference(skin, root.skins.len(), &format!("node {node_index} skin"))?;
        }
        for &child in &node.children {
            validate_reference(child, root.nodes.len(), &format!("node {node_index} child"))?;
            if child == node_index {
                return Err(GltfError::InvalidGltf(format!(
                    "node {node_index} cannot be its own child"
                )));
            }
        }
    }

    for (mesh_index, mesh) in root.meshes.iter().enumerate() {
        for (primitive_index, primitive) in mesh.primitives.iter().enumerate() {
            for (semantic, &accessor) in &primitive.attributes {
                validate_reference(
                    accessor,
                    root.accessors.len(),
                    &format!("primitive {mesh_index}:{primitive_index} {semantic} accessor"),
                )?;
            }
            if let Some(indices) = primitive.indices {
                validate_reference(
                    indices,
                    root.accessors.len(),
                    &format!("primitive {mesh_index}:{primitive_index} indices accessor"),
                )?;
            }
            let vertex_count = primitive
                .attributes
                .values()
                .next()
                .and_then(|accessor| root.accessors.get(*accessor))
                .map(|accessor| accessor.count);
            for (target_index, target) in primitive.targets.iter().enumerate() {
                for (semantic, &accessor_index) in target {
                    validate_reference(
                        accessor_index,
                        root.accessors.len(),
                        &format!(
                            "primitive {mesh_index}:{primitive_index} morph target {target_index} {semantic} accessor"
                        ),
                    )?;
                    let accessor = &root.accessors[accessor_index];
                    if vertex_count.is_some_and(|count| accessor.count != count) {
                        return Err(GltfError::InvalidGltf(format!(
                            "primitive {mesh_index}:{primitive_index} morph target accessor count {} does not match vertex count {}",
                            accessor.count,
                            vertex_count.unwrap_or_default()
                        )));
                    }
                    if !matches!(semantic.as_str(), "POSITION" | "NORMAL" | "TANGENT")
                        || accessor.accessor_type != "VEC3"
                        || accessor.component_type != GLTF_COMPONENT_FLOAT
                        || accessor.normalized
                    {
                        return Err(GltfError::InvalidGltf(format!(
                            "primitive {mesh_index}:{primitive_index} morph target {semantic} accessor has an invalid contract"
                        )));
                    }
                }
            }
        }
    }

    for (skin_index, skin) in root.skins.iter().enumerate() {
        if skin.joints.is_empty() {
            return Err(GltfError::InvalidGltf(format!(
                "skin {skin_index} has no joints"
            )));
        }
        for &joint in &skin.joints {
            validate_reference(joint, root.nodes.len(), &format!("skin {skin_index} joint"))?;
        }
        if let Some(skeleton) = skin.skeleton {
            validate_reference(
                skeleton,
                root.nodes.len(),
                &format!("skin {skin_index} skeleton"),
            )?;
        }
        if let Some(accessor_index) = skin.inverse_bind_matrices {
            validate_reference(
                accessor_index,
                root.accessors.len(),
                &format!("skin {skin_index} inverseBindMatrices accessor"),
            )?;
            let accessor = &root.accessors[accessor_index];
            if accessor.accessor_type != "MAT4"
                || accessor.component_type != GLTF_COMPONENT_FLOAT
                || accessor.normalized
                || accessor.count < skin.joints.len()
            {
                return Err(GltfError::InvalidGltf(format!(
                    "skin {skin_index} inverseBindMatrices accessor has an invalid contract"
                )));
            }
        }
    }

    for (animation_index, animation) in root.animations.iter().enumerate() {
        if animation.channels.is_empty() || animation.samplers.is_empty() {
            return Err(GltfError::InvalidGltf(format!(
                "animation {animation_index} must contain channels and samplers"
            )));
        }
        for (sampler_index, sampler) in animation.samplers.iter().enumerate() {
            validate_reference(
                sampler.input,
                root.accessors.len(),
                &format!("animation {animation_index} sampler {sampler_index} input accessor"),
            )?;
            validate_reference(
                sampler.output,
                root.accessors.len(),
                &format!("animation {animation_index} sampler {sampler_index} output accessor"),
            )?;
            let input = &root.accessors[sampler.input];
            if input.accessor_type != "SCALAR"
                || input.component_type != GLTF_COMPONENT_FLOAT
                || input.normalized
            {
                return Err(GltfError::InvalidGltf(format!(
                    "animation {animation_index} sampler {sampler_index} input accessor has an invalid contract"
                )));
            }
            if sampler
                .interpolation
                .as_deref()
                .is_some_and(|interpolation| {
                    !matches!(interpolation, "LINEAR" | "STEP" | "CUBICSPLINE")
                })
            {
                return Err(GltfError::InvalidGltf(format!(
                    "animation {animation_index} sampler {sampler_index} has invalid interpolation"
                )));
            }
        }
        for (channel_index, channel) in animation.channels.iter().enumerate() {
            validate_reference(
                channel.sampler,
                animation.samplers.len(),
                &format!("animation {animation_index} channel {channel_index} sampler"),
            )?;
            if channel.target.path.is_empty() {
                return Err(GltfError::InvalidGltf(format!(
                    "animation {animation_index} channel {channel_index} target path is empty"
                )));
            }
            if let Some(node) = channel.target.node {
                validate_reference(
                    node,
                    root.nodes.len(),
                    &format!("animation {animation_index} channel {channel_index} target node"),
                )?;
            }
        }
    }
    Ok(())
}

/// Rejects glTF features outside this crate's geometry-decoding scope.
///
/// Used by the strict readers ([`GltfReader::from_bytes`] and friends). The
/// document-preserving compressor uses a lenient path instead: it never
/// interprets these features, it just carries them through untouched, so it
/// does not reject them here.
fn reject_unsupported_features(root: &GltfRoot) -> Result<()> {
    // The strict reader cannot faithfully load an asset that *requires* an
    // extension it does not implement. (KHR_draco_mesh_compression is the only
    // one this crate honors.) The lenient/compressor path skips this check.
    for required in &root.extensions_required {
        if required != KHR_DRACO_MESH_COMPRESSION {
            return Err(GltfError::Unsupported(format!(
                "Unsupported required extension: {}",
                required
            )));
        }
    }

    for (mesh_idx, mesh) in root.meshes.iter().enumerate() {
        for (prim_idx, primitive) in mesh.primitives.iter().enumerate() {
            if !primitive.targets.is_empty() {
                return Err(GltfError::Unsupported(format!(
                    "Morph targets are not supported on primitive {}:{}",
                    mesh_idx, prim_idx
                )));
            }
        }
    }

    if !root.skins.is_empty() {
        return Err(GltfError::Unsupported("Skins are not supported".into()));
    }
    if root.nodes.iter().any(|node| node.skin.is_some()) {
        return Err(GltfError::Unsupported(
            "Skinned nodes are not supported".into(),
        ));
    }
    if !root.animations.is_empty() {
        return Err(GltfError::Unsupported(
            "Animations are not supported".into(),
        ));
    }

    Ok(())
}

fn load_buffers(
    root: &GltfRoot,
    is_glb: bool,
    glb_bin_chunk: Option<&[u8]>,
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
) -> Result<Vec<Vec<u8>>> {
    let mut references = Vec::new();
    references
        .try_reserve_exact(root.buffers.len())
        .map_err(|_| GltfError::ResourceLimitExceeded("buffer table allocation failed".into()))?;
    for buffer in &root.buffers {
        references.push(GltfBufferReference {
            uri: buffer.uri.as_deref(),
            byte_length: buffer.byte_length,
        });
    }
    let format = if is_glb {
        GltfContainerFormat::Glb
    } else {
        GltfContainerFormat::Gltf
    };
    resolve_gltf_buffers(&references, format, glb_bin_chunk, resolver, limits)
}

fn validate_images(
    root: &GltfRoot,
    buffers: &[Vec<u8>],
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
) -> Result<()> {
    for (index, image) in root.images.iter().enumerate() {
        match (image.uri.as_deref(), image.buffer_view) {
            (Some(_), Some(_)) => {
                return Err(GltfError::InvalidGltf(format!(
                    "Image {index} defines both uri and bufferView"
                )));
            }
            (None, None) => {
                return Err(GltfError::InvalidGltf(format!(
                    "Image {index} defines neither uri nor bufferView"
                )));
            }
            (Some(uri), None) => {
                // Resolve even though the geometry reader does not decode image
                // pixels: missing companion files and byte quotas must fail at
                // import time rather than producing a false-success document.
                let _ = resolve_resource_uri(uri, resolver, limits.max_resource_bytes)?;
            }
            (None, Some(view_index)) => {
                if image.mime_type.is_none() {
                    return Err(GltfError::InvalidGltf(format!(
                        "Buffer-view image {index} has no mimeType"
                    )));
                }
                let view = root.buffer_views.get(view_index).ok_or_else(|| {
                    GltfError::InvalidGltf(format!(
                        "Image {index} references invalid bufferView {view_index}"
                    ))
                })?;
                let buffer = buffers.get(view.buffer).ok_or_else(|| {
                    GltfError::InvalidGltf(format!(
                        "Image {index} bufferView references invalid buffer {}",
                        view.buffer
                    ))
                })?;
                let start = view.byte_offset.unwrap_or(0);
                let end = start.checked_add(view.byte_length).ok_or_else(|| {
                    GltfError::InvalidGltf(format!("Image {index} byte range overflow"))
                })?;
                if end > buffer.len() {
                    return Err(GltfError::InvalidGltf(format!(
                        "Image {index} extends past its buffer"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn accessor_num_components(accessor_type: &str) -> Result<u8> {
    match accessor_type {
        "SCALAR" => Ok(1),
        "VEC2" => Ok(2),
        "VEC3" => Ok(3),
        "VEC4" => Ok(4),
        _ => Err(GltfError::Unsupported(format!(
            "Unsupported accessor type: {}",
            accessor_type
        ))),
    }
}

fn data_type_for_component_type(component_type: u32) -> Result<DataType> {
    match component_type {
        GLTF_COMPONENT_BYTE => Ok(DataType::Int8),
        GLTF_COMPONENT_UNSIGNED_BYTE => Ok(DataType::Uint8),
        GLTF_COMPONENT_SHORT => Ok(DataType::Int16),
        GLTF_COMPONENT_UNSIGNED_SHORT => Ok(DataType::Uint16),
        GLTF_COMPONENT_UNSIGNED_INT => Ok(DataType::Uint32),
        GLTF_COMPONENT_FLOAT => Ok(DataType::Float32),
        _ => Err(GltfError::Unsupported(format!(
            "Unsupported component type: {}",
            component_type
        ))),
    }
}

// Implement the Reader trait for glTF/GLB files. Decodes all primitives
// (Draco-compressed and standard) and returns them as meshes.
impl crate::traits::Reader for GltfReader {
    fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Self> {
        GltfReader::open(path).map_err(|e| std::io::Error::other(e.to_string()))
    }

    fn read_meshes(&mut self) -> std::io::Result<Vec<draco_core::mesh::Mesh>> {
        self.decode_all_meshes()
            .map_err(|e| std::io::Error::other(e.to_string()))
    }
}

impl ReadFromBytes for GltfReader {
    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        GltfReader::from_bytes(bytes).map_err(|e| io::Error::other(e.to_string()))
    }
}

impl GltfReader {
    /// Decode all primitives (both Draco and standard) as meshes.
    pub fn decode_all_meshes(&self) -> Result<Vec<Mesh>> {
        let mut result = Vec::new();

        for (mesh_idx, gltf_mesh) in self.root.meshes.iter().enumerate() {
            for (prim_idx, primitive) in gltf_mesh.primitives.iter().enumerate() {
                let mesh = self.decode_primitive_mesh(mesh_idx, gltf_mesh, prim_idx, primitive)?;
                result.push(mesh);
            }
        }

        Ok(result)
    }

    /// Compute a node's local transform as a row-major 4x4 matrix.
    fn compute_node_transform(node: &GltfNode) -> Option<crate::scene::Transform> {
        if let Some(m) = &node.matrix {
            // glTF stores column-major; convert to row-major
            Some(crate::scene::Transform {
                matrix: [
                    [m[0], m[4], m[8], m[12]],
                    [m[1], m[5], m[9], m[13]],
                    [m[2], m[6], m[10], m[14]],
                    [m[3], m[7], m[11], m[15]],
                ],
            })
        } else if node.translation.is_some() || node.rotation.is_some() || node.scale.is_some() {
            // Compose T * R * S
            let t = node.translation.unwrap_or([0.0, 0.0, 0.0]);
            let r = node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]); // [x, y, z, w]
            let s = node.scale.unwrap_or([1.0, 1.0, 1.0]);

            // Quaternion to rotation matrix (row-major)
            let (qx, qy, qz, qw) = (r[0], r[1], r[2], r[3]);
            let xx = qx * qx;
            let yy = qy * qy;
            let zz = qz * qz;
            let xy = qx * qy;
            let xz = qx * qz;
            let yz = qy * qz;
            let wx = qw * qx;
            let wy = qw * qy;
            let wz = qw * qz;

            // Rotation matrix (row-major)
            let rot = [
                [1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy)],
                [2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx)],
                [2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy)],
            ];

            // Compose T * R * S into 4x4 row-major
            Some(crate::scene::Transform {
                matrix: [
                    [rot[0][0] * s[0], rot[0][1] * s[1], rot[0][2] * s[2], t[0]],
                    [rot[1][0] * s[0], rot[1][1] * s[1], rot[1][2] * s[2], t[1]],
                    [rot[2][0] * s[0], rot[2][1] * s[1], rot[2][2] * s[2], t[2]],
                    [0.0, 0.0, 0.0, 1.0],
                ],
            })
        } else {
            None
        }
    }

    /// Recursively build a SceneNode from a glTF node index.
    fn build_scene_node(
        &self,
        node_idx: usize,
        visited: &mut Vec<bool>,
    ) -> Result<crate::scene::SceneNode> {
        if node_idx >= self.root.nodes.len() {
            return Err(GltfError::InvalidGltf(format!(
                "Invalid node index: {}",
                node_idx
            )));
        }

        // Cycle detection
        if visited[node_idx] {
            return Err(GltfError::InvalidGltf(format!(
                "Cycle detected at node {}",
                node_idx
            )));
        }
        visited[node_idx] = true;

        let gltf_node = &self.root.nodes[node_idx];

        let mut scene_node = crate::scene::SceneNode::new(gltf_node.name.clone());
        scene_node.transform = Self::compute_node_transform(gltf_node);

        // Attach meshes if this node references a mesh
        if let Some(mesh_idx) = gltf_node.mesh {
            if let Some(gltf_mesh) = self.root.meshes.get(mesh_idx) {
                for (prim_idx, primitive) in gltf_mesh.primitives.iter().enumerate() {
                    let mesh =
                        self.decode_primitive_mesh(mesh_idx, gltf_mesh, prim_idx, primitive)?;

                    let mesh_instance_name = if gltf_mesh.primitives.len() > 1 {
                        gltf_mesh
                            .name
                            .as_ref()
                            .map(|n| format!("{}_{}", n, prim_idx))
                    } else {
                        gltf_mesh.name.clone()
                    };

                    scene_node.mesh_instances.push(crate::scene::MeshInstance {
                        name: mesh_instance_name,
                        mesh,
                        transform: None, // Primitive-level transform is identity
                    });
                }
            }
        }

        // Recursively build children
        for &child_idx in &gltf_node.children {
            let child_node = self.build_scene_node(child_idx, visited)?;
            scene_node.children.push(child_node);
        }

        Ok(scene_node)
    }

    fn root_node_indices_without_scenes(&self) -> Vec<usize> {
        let mut is_child = vec![false; self.root.nodes.len()];
        for node in &self.root.nodes {
            for &child_idx in &node.children {
                if child_idx < is_child.len() {
                    is_child[child_idx] = true;
                }
            }
        }

        (0..self.root.nodes.len())
            .filter(|&i| !is_child[i])
            .collect()
    }

    fn build_scene_from_roots(
        &self,
        name: Option<String>,
        root_node_indices: &[usize],
    ) -> Result<crate::scene::Scene> {
        let mut visited = vec![false; self.root.nodes.len()];
        let mut root_nodes = Vec::with_capacity(root_node_indices.len());
        for &node_idx in root_node_indices {
            root_nodes.push(self.build_scene_node(node_idx, &mut visited)?);
        }

        Ok(crate::scene::Scene { name, root_nodes })
    }

    fn read_scene_result(&self) -> Result<crate::scene::Scene> {
        let scene_idx = self.root.scene.or({
            if self.root.scenes.is_empty() {
                None
            } else {
                Some(0)
            }
        });

        if let Some(idx) = scene_idx {
            let gltf_scene =
                self.root.scenes.get(idx).ok_or_else(|| {
                    GltfError::InvalidGltf(format!("Invalid scene index: {}", idx))
                })?;
            self.build_scene_from_roots(gltf_scene.name.clone(), &gltf_scene.nodes)
        } else {
            let roots = self.root_node_indices_without_scenes();
            self.build_scene_from_roots(None, &roots)
        }
    }

    fn read_scenes_result(&self) -> Result<Vec<crate::scene::Scene>> {
        if self.root.scenes.is_empty() {
            let roots = self.root_node_indices_without_scenes();
            return self
                .build_scene_from_roots(None, &roots)
                .map(|scene| vec![scene]);
        }

        self.root
            .scenes
            .iter()
            .map(|scene| self.build_scene_from_roots(scene.name.clone(), &scene.nodes))
            .collect()
    }
}

impl crate::scene::SceneReader for GltfReader {
    fn read_scene(&mut self) -> std::io::Result<crate::scene::Scene> {
        self.read_scene_result()
            .map_err(|e| std::io::Error::other(e.to_string()))
    }

    fn read_scenes(&mut self) -> std::io::Result<Vec<crate::scene::Scene>> {
        self.read_scenes_result()
            .map_err(|e| std::io::Error::other(e.to_string()))
    }
}
// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::GeometryAttributeType;
    #[cfg(feature = "gltf-writer")]
    use draco_core::geometry_attribute::PointAttribute;
    use draco_core::mesh::Mesh;
    use serde_json::Value;
    use tempfile::tempdir;

    fn build_glb(json: &str) -> Vec<u8> {
        let document: serde_json::Value = serde_json::from_str(json).unwrap();
        crate::gltf_container::build_glb_container(&document, &[]).unwrap()
    }

    fn triangle_positions() -> Vec<u8> {
        [
            0.0f32, 0.0, 0.0, //
            1.0, 0.0, 0.0, //
            0.0, 1.0, 0.0,
        ]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect()
    }

    #[cfg(feature = "gltf-writer")]
    fn triangle_mesh() -> Mesh {
        let mut mesh = Mesh::new();
        mesh.set_num_points(3);
        mesh.add_face([
            draco_core::geometry_indices::PointIndex(0),
            draco_core::geometry_indices::PointIndex(1),
            draco_core::geometry_indices::PointIndex(2),
        ]);

        let mut positions = PointAttribute::new();
        positions.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            3,
        );
        positions.buffer_mut().write(0, &triangle_positions());
        mesh.add_attribute(positions);
        mesh
    }

    #[cfg(feature = "gltf-writer")]
    fn writer_gltf_json_value() -> serde_json::Value {
        let mut writer = crate::gltf_writer::GltfWriter::new();
        writer
            .add_draco_mesh(&triangle_mesh(), Some("triangle"), None)
            .unwrap();
        serde_json::from_str(&writer.to_gltf_embedded().unwrap()).unwrap()
    }

    fn read_attribute_bytes(mesh: &Mesh, attribute_type: GeometryAttributeType) -> Vec<u8> {
        mesh.named_attribute(attribute_type)
            .expect("missing attribute")
            .buffer()
            .data()
            .to_vec()
    }

    #[test]
    fn test_minimal_gltf_json() {
        let json = r#"{
            "asset": {"version": "2.0"},
            "meshes": [],
            "buffers": [],
            "bufferViews": [],
            "accessors": []
        }"#;

        let root: GltfRoot = serde_json::from_str(json).unwrap();
        assert!(root.meshes.is_empty());
        assert!(root.buffers.is_empty());
    }

    #[test]
    fn test_read_scenes_returns_all_gltf_scenes() {
        use crate::scene::SceneReader;

        let json = r#"{
            "asset": {"version": "2.0"},
            "scene": 1,
            "scenes": [
                {"name": "Preview", "nodes": [0]},
                {"name": "Full", "nodes": [1, 2]}
            ],
            "nodes": [
                {"name": "PreviewRoot"},
                {"name": "FullRootA"},
                {"name": "FullRootB"}
            ]
        }"#;

        let mut reader = GltfReader::from_gltf(json.as_bytes(), None).unwrap();
        let default_scene = reader.read_scene().unwrap();
        assert_eq!(default_scene.name, Some("Full".to_string()));
        assert_eq!(default_scene.root_nodes.len(), 2);
        assert_eq!(
            default_scene.root_nodes[0].name,
            Some("FullRootA".to_string())
        );

        let scenes = reader.read_scenes().unwrap();
        assert_eq!(scenes.len(), 2);
        assert_eq!(scenes[0].name, Some("Preview".to_string()));
        assert_eq!(scenes[0].root_nodes.len(), 1);
        assert_eq!(
            scenes[0].root_nodes[0].name,
            Some("PreviewRoot".to_string())
        );
        assert_eq!(scenes[1].name, Some("Full".to_string()));
        assert_eq!(scenes[1].root_nodes.len(), 2);
    }

    #[test]
    fn test_gltf_with_draco_extension() {
        let json = r#"{
            "asset": {"version": "2.0"},
            "extensionsUsed": ["KHR_draco_mesh_compression"],
            "extensionsRequired": ["KHR_draco_mesh_compression"],
            "meshes": [{
                "name": "TestMesh",
                "primitives": [{
                    "attributes": {"POSITION": 0},
                    "extensions": {
                        "KHR_draco_mesh_compression": {
                            "bufferView": 0,
                            "attributes": {"POSITION": 0}
                        }
                    }
                }]
            }],
            "buffers": [{"byteLength": 3, "uri": "data:application/octet-stream;base64,AAAA"}],
            "bufferViews": [{"buffer": 0, "byteLength": 3}],
            "accessors": [{"componentType": 5126, "count": 1, "type": "VEC3"}]
        }"#;

        let reader = GltfReader::from_gltf(json.as_bytes(), None).unwrap();
        assert!(reader.has_draco_extension());
        assert_eq!(reader.num_meshes(), 1);

        let primitives = reader.draco_primitives();
        assert_eq!(primitives.len(), 1);
        assert_eq!(primitives[0].mesh_name, Some("TestMesh".to_string()));
        assert_eq!(primitives[0].buffer_view, 0);
    }

    #[test]
    fn typed_khr_parser_rejects_malformed_schema_and_missing_side_fallback() {
        let base = serde_json::json!({
            "asset": {"version": "2.0"},
            "extensionsUsed": [KHR_DRACO_MESH_COMPRESSION],
            "extensionsRequired": [KHR_DRACO_MESH_COMPRESSION],
            "meshes": [{"primitives": [{
                "attributes": {"POSITION": 0},
                "extensions": {KHR_DRACO_MESH_COMPRESSION: {
                    "bufferView": 0,
                    "attributes": {"POSITION": 10}
                }}
            }]}],
            "buffers": [{"byteLength": 4, "uri": "data:;base64,AAAAAA=="}],
            "bufferViews": [{"buffer": 0, "byteLength": 4}],
            "accessors": [{"componentType": 5126, "count": 1, "type": "VEC3"}]
        });

        let mut malformed = base.clone();
        malformed["meshes"][0]["primitives"][0]["extensions"][KHR_DRACO_MESH_COMPRESSION]
            ["unexpected"] = serde_json::json!(true);
        assert!(GltfReader::from_gltf(&serde_json::to_vec(&malformed).unwrap(), None).is_err());

        let mut too_large = base.clone();
        too_large["meshes"][0]["primitives"][0]["extensions"][KHR_DRACO_MESH_COMPRESSION]
            ["attributes"]["POSITION"] = serde_json::Value::from(u64::from(u32::MAX) + 1);
        assert!(GltfReader::from_gltf(&serde_json::to_vec(&too_large).unwrap(), None).is_err());

        let mut empty = base.clone();
        empty["meshes"][0]["primitives"][0]["extensions"][KHR_DRACO_MESH_COMPRESSION]
            ["attributes"] = serde_json::json!({});
        assert!(GltfReader::from_gltf(&serde_json::to_vec(&empty).unwrap(), None).is_err());

        let mut side_attribute = base;
        side_attribute["meshes"][0]["primitives"][0]["attributes"]["NORMAL"] =
            serde_json::Value::from(1);
        side_attribute["accessors"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "componentType": 5126,
                "count": 1,
                "type": "VEC3"
            }));
        assert!(
            GltfReader::from_gltf(&serde_json::to_vec(&side_attribute).unwrap(), None).is_err()
        );
    }

    #[test]
    fn test_rejects_unknown_required_extension() {
        let json = r#"{
            "asset": {"version": "2.0"},
            "extensionsUsed": ["EXT_required"],
            "extensionsRequired": ["EXT_required"]
        }"#;

        assert!(matches!(
            GltfReader::from_gltf(json.as_bytes(), None),
            Err(GltfError::Unsupported(_))
        ));
    }

    #[test]
    fn test_rejects_draco_primitive_missing_extensions_used() {
        let json = r#"{
            "asset": {"version": "2.0"},
            "meshes": [{
                "primitives": [{
                    "attributes": {"POSITION": 0},
                    "extensions": {
                        "KHR_draco_mesh_compression": {
                            "bufferView": 0,
                            "attributes": {"POSITION": 0}
                        }
                    }
                }]
            }],
            "buffers": [{"byteLength": 3, "uri": "data:application/octet-stream;base64,AAAA"}],
            "bufferViews": [{"buffer": 0, "byteLength": 3}],
            "accessors": []
        }"#;

        assert!(matches!(
            GltfReader::from_gltf(json.as_bytes(), None),
            Err(GltfError::InvalidGltf(_))
        ));
    }

    #[test]
    fn test_glb_open_loads_relative_external_buffer() {
        let dir = tempdir().unwrap();
        let bin_path = dir.path().join("mesh.bin");
        std::fs::write(&bin_path, triangle_positions()).unwrap();

        let json = r#"{
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": 36, "uri": "mesh.bin"}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}],
            "accessors": [{
                "bufferView": 0,
                "componentType": 5126,
                "count": 3,
                "type": "VEC3"
            }],
            "meshes": [{
                "primitives": [{
                    "attributes": {"POSITION": 0},
                    "mode": 4
                }]
            }]
        }"#;
        let glb = build_glb(json);
        let glb_path = dir.path().join("external.glb");
        std::fs::write(&glb_path, &glb).unwrap();

        let reader = GltfReader::open(&glb_path).unwrap();
        let meshes = reader.decode_all_meshes().unwrap();

        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].num_points(), 3);
        assert_eq!(meshes[0].num_faces(), 1);
        assert_eq!(
            read_attribute_bytes(&meshes[0], GeometryAttributeType::Position),
            triangle_positions()
        );
    }

    #[test]
    fn test_from_glb_rejects_external_buffer_without_base_path() {
        let json = r#"{
            "asset": {"version": "2.0"},
            "buffers": [{"byteLength": 36, "uri": "mesh.bin"}],
            "bufferViews": [{"buffer": 0, "byteOffset": 0, "byteLength": 36}],
            "accessors": [],
            "meshes": []
        }"#;

        let err = match GltfReader::from_glb(&build_glb(json)) {
            Ok(_) => panic!("external buffer unexpectedly loaded without a base path"),
            Err(err) => err,
        };
        assert!(matches!(err, GltfError::ExternalResourceDenied(_)));
    }

    #[test]
    fn external_images_are_resolved_and_reported_without_reparsing() {
        let json = br#"{
            "asset": {"version": "2.0"},
            "images": [{"uri": "textures%2Falbedo.png"}]
        }"#;
        assert!(matches!(
            GltfReader::from_gltf(json, None),
            Err(GltfError::ExternalResourceDenied(uri)) if uri == "textures%2Falbedo.png"
        ));

        let resolver = |uri: &str| -> Result<Vec<u8>> {
            if uri == "textures%2Falbedo.png" {
                Ok(vec![1, 2, 3])
            } else {
                Err(GltfError::ExternalResourceDenied(uri.to_owned()))
            }
        };
        let reader =
            GltfReader::from_bytes_with_resolver(json, &resolver, &ResourceLimits::default())
                .unwrap();
        assert_eq!(
            reader.document_metadata().external_resource_uris,
            ["textures%2Falbedo.png"]
        );

        let limits = ResourceLimits {
            max_resource_bytes: Some(2),
            ..ResourceLimits::default()
        };
        assert!(matches!(
            GltfReader::from_bytes_with_resolver(json, &resolver, &limits),
            Err(GltfError::ResourceLimitExceeded(_))
        ));
    }

    #[test]
    fn test_texcoord_unsigned_short_normalized_vec2() {
        let mut bytes = triangle_positions();
        let texcoords = [0u16, 0, 65535, 0, 0, 65535];
        bytes.extend(texcoords.into_iter().flat_map(u16::to_le_bytes));
        let data_uri = format!(
            "data:application/octet-stream;base64,{}",
            base64_for_test(&bytes)
        );
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}, "uri": "{}"}}],
                "bufferViews": [
                    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
                    {{"buffer": 0, "byteOffset": 36, "byteLength": 12}}
                ],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                    {{
                        "bufferView": 1,
                        "componentType": 5123,
                        "normalized": true,
                        "count": 3,
                        "type": "VEC2"
                    }}
                ],
                "meshes": [{{
                    "primitives": [{{
                        "attributes": {{"POSITION": 0, "TEXCOORD_0": 1}},
                        "mode": 4
                    }}]
                }}]
            }}"#,
            bytes.len(),
            data_uri
        );

        let mesh = GltfReader::from_gltf(json.as_bytes(), None)
            .unwrap()
            .decode_all_meshes()
            .unwrap()
            .remove(0);
        let texcoord = mesh
            .named_attribute(GeometryAttributeType::TexCoord)
            .expect("missing texcoord");

        assert_eq!(texcoord.data_type(), DataType::Uint16);
        assert!(texcoord.normalized());
        assert_eq!(texcoord.num_components(), 2);
        assert_eq!(texcoord.buffer().data(), &bytes[36..48]);
    }

    #[test]
    fn test_color_unsigned_byte_normalized_vec3() {
        let mut bytes = triangle_positions();
        let colors = [255u8, 0, 0, 0, 255, 0, 0, 0, 255];
        bytes.extend(colors);
        let data_uri = format!(
            "data:application/octet-stream;base64,{}",
            base64_for_test(&bytes)
        );
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}, "uri": "{}"}}],
                "bufferViews": [
                    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
                    {{"buffer": 0, "byteOffset": 36, "byteLength": 9}}
                ],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                    {{
                        "bufferView": 1,
                        "componentType": 5121,
                        "normalized": true,
                        "count": 3,
                        "type": "VEC3"
                    }}
                ],
                "meshes": [{{
                    "primitives": [{{
                        "attributes": {{"POSITION": 0, "COLOR_0": 1}},
                        "mode": 4
                    }}]
                }}]
            }}"#,
            bytes.len(),
            data_uri
        );

        let mesh = GltfReader::from_gltf(json.as_bytes(), None)
            .unwrap()
            .decode_all_meshes()
            .unwrap()
            .remove(0);
        let color = mesh
            .named_attribute(GeometryAttributeType::Color)
            .expect("missing color");

        assert_eq!(color.data_type(), DataType::Uint8);
        assert!(color.normalized());
        assert_eq!(color.num_components(), 3);
        assert_eq!(color.buffer().data(), &bytes[36..45]);
    }

    #[test]
    fn test_points_primitive_decodes_without_faces() {
        let indices = [2u16, 0];
        let mut bytes = triangle_positions();
        bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
        let data_uri = format!(
            "data:application/octet-stream;base64,{}",
            base64_for_test(&bytes)
        );
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}, "uri": "{}"}}],
                "bufferViews": [
                    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
                    {{"buffer": 0, "byteOffset": 36, "byteLength": 4}}
                ],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                    {{"bufferView": 1, "componentType": 5123, "count": 2, "type": "SCALAR"}}
                ],
                "meshes": [{{
                    "primitives": [{{
                        "attributes": {{"POSITION": 0}},
                        "indices": 1,
                        "mode": 0
                    }}]
                }}]
            }}"#,
            bytes.len(),
            data_uri
        );

        let mesh = GltfReader::from_gltf(json.as_bytes(), None)
            .unwrap()
            .decode_all_meshes()
            .unwrap()
            .remove(0);

        assert_eq!(mesh.num_points(), 2);
        assert_eq!(mesh.num_faces(), 0);
        let positions = read_attribute_bytes(&mesh, GeometryAttributeType::Position);
        assert_eq!(&positions[0..12], &triangle_positions()[24..36]);
        assert_eq!(&positions[12..24], &triangle_positions()[0..12]);
    }

    #[cfg(feature = "gltf-writer")]
    #[test]
    fn test_writer_glb_roundtrips_through_reader() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roundtrip.glb");

        let mut writer = crate::gltf_writer::GltfWriter::new();
        writer
            .add_draco_mesh(&triangle_mesh(), Some("triangle"), None)
            .unwrap();
        writer.write_glb(&path).unwrap();

        let reader = GltfReader::open(&path).unwrap();
        let primitives = reader.draco_primitives();
        assert_eq!(primitives.len(), 1);
        assert_eq!(primitives[0].attributes.get("POSITION"), Some(&0));

        let decoded = reader.decode_all_meshes().unwrap().remove(0);
        let position = decoded
            .named_attribute(GeometryAttributeType::Position)
            .expect("missing position");
        assert_eq!(position.data_type(), DataType::Float32);
        assert_eq!(position.num_components(), 3);
        assert_eq!(decoded.num_faces(), 1);
    }

    #[cfg(feature = "gltf-writer")]
    #[test]
    fn test_draco_decode_rejects_extension_attribute_not_in_primitive_attributes() {
        let mut value = writer_gltf_json_value();
        value["meshes"][0]["primitives"][0]["extensions"]["KHR_draco_mesh_compression"]
            ["attributes"]["TEXCOORD_0"] = serde_json::json!(0);
        let json = serde_json::to_string(&value).unwrap();

        let err = match GltfReader::from_gltf(json.as_bytes(), None) {
            Err(error) => error,
            Ok(_) => panic!("malformed extension unexpectedly parsed"),
        };
        assert!(matches!(err, GltfError::InvalidGltf(_)));
    }

    #[cfg(feature = "gltf-writer")]
    #[test]
    fn test_draco_decode_rejects_accessor_metadata_mismatch_and_sparse() {
        let mut count_mismatch = writer_gltf_json_value();
        let pos_accessor = count_mismatch["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
            .as_u64()
            .unwrap() as usize;
        count_mismatch["accessors"][pos_accessor]["count"] = serde_json::json!(99);
        let json = serde_json::to_string(&count_mismatch).unwrap();
        let err = GltfReader::from_gltf(json.as_bytes(), None)
            .unwrap()
            .decode_all_meshes()
            .unwrap_err();
        assert!(matches!(err, GltfError::InvalidGltf(_)));

        let mut sparse = writer_gltf_json_value();
        let pos_accessor = sparse["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
            .as_u64()
            .unwrap() as usize;
        sparse["accessors"][pos_accessor]["sparse"] = serde_json::json!({
            "count": 1,
            "indices": {"bufferView": 0, "componentType": 5123},
            "values": {"bufferView": 0}
        });
        let json = serde_json::to_string(&sparse).unwrap();
        let err = GltfReader::from_gltf(json.as_bytes(), None)
            .unwrap()
            .decode_all_meshes()
            .unwrap_err();
        assert!(matches!(err, GltfError::Unsupported(_)));
    }

    #[test]
    fn test_standard_triangle_rejects_out_of_bounds_indices() {
        let indices = [0u16, 1, 3];
        let mut bytes = triangle_positions();
        bytes.extend(indices.into_iter().flat_map(u16::to_le_bytes));
        let data_uri = format!(
            "data:application/octet-stream;base64,{}",
            base64_for_test(&bytes)
        );
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}, "uri": "{}"}}],
                "bufferViews": [
                    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
                    {{"buffer": 0, "byteOffset": 36, "byteLength": 6}}
                ],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}},
                    {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}}
                ],
                "meshes": [{{
                    "primitives": [{{
                        "attributes": {{"POSITION": 0}},
                        "indices": 1,
                        "mode": 4
                    }}]
                }}]
            }}"#,
            bytes.len(),
            data_uri
        );

        let err = GltfReader::from_gltf(json.as_bytes(), None)
            .unwrap()
            .decode_all_meshes()
            .unwrap_err();
        assert!(matches!(err, GltfError::InvalidGltf(_)));
    }

    #[test]
    fn semantic_and_index_normalized_contracts_are_strict() {
        fn document_with_attribute(
            semantic: &str,
            accessor_type: &str,
            component_type: u32,
            normalized: bool,
        ) -> Vec<u8> {
            let components = match accessor_type {
                "VEC3" => 3,
                "VEC4" => 4,
                _ => 1,
            };
            let component_size = if component_type == GLTF_COMPONENT_FLOAT {
                4
            } else {
                1
            };
            let mut bytes = triangle_positions();
            let extra_offset = bytes.len();
            bytes.resize(extra_offset + 3 * components * component_size, 0);
            let document = serde_json::json!({
                "asset": {"version": "2.0"},
                "buffers": [{
                    "byteLength": bytes.len(),
                    "uri": format!("data:application/octet-stream;base64,{}", base64_for_test(&bytes))
                }],
                "bufferViews": [
                    {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                    {"buffer": 0, "byteOffset": extra_offset, "byteLength": bytes.len() - extra_offset}
                ],
                "accessors": [
                    {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
                    {"bufferView": 1, "componentType": component_type, "normalized": normalized,
                     "count": 3, "type": accessor_type}
                ],
                "meshes": [{"primitives": [{
                    "attributes": {"POSITION": 0, (semantic): 1}, "mode": 4
                }]}]
            });
            serde_json::to_vec(&document).unwrap()
        }

        for document in [
            document_with_attribute("TANGENT", "VEC3", 5126, false),
            document_with_attribute("JOINTS_0", "VEC4", 5126, false),
            document_with_attribute("WEIGHTS_0", "VEC4", 5121, false),
        ] {
            let error = GltfReader::from_bytes_lenient(&document)
                .unwrap()
                .decode_all_meshes()
                .unwrap_err();
            assert!(matches!(
                error,
                GltfError::InvalidGltf(_) | GltfError::Unsupported(_)
            ));
        }

        let mut bytes = triangle_positions();
        let indices_offset = bytes.len();
        bytes.extend([0u16, 1, 2].into_iter().flat_map(u16::to_le_bytes));
        let document = serde_json::json!({
            "asset": {"version": "2.0"},
            "buffers": [{
                "byteLength": bytes.len(),
                "uri": format!("data:application/octet-stream;base64,{}", base64_for_test(&bytes))
            }],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": indices_offset, "byteLength": 6}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 1, "componentType": 5123, "normalized": true,
                 "count": 3, "type": "SCALAR"}
            ],
            "meshes": [{"primitives": [{
                "attributes": {"POSITION": 0}, "indices": 1, "mode": 4
            }]}]
        });
        let error = GltfReader::from_bytes_lenient(&serde_json::to_vec(&document).unwrap())
            .unwrap()
            .decode_all_meshes()
            .unwrap_err();
        assert!(matches!(error, GltfError::InvalidGltf(_)));
    }

    #[test]
    fn lenient_reader_validates_scene_animation_and_skin_references() {
        let valid = serde_json::json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0, "skin": 0, "children": [1]}, {}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "mode": 4}]}],
            "accessors": [
                {"componentType": 5126, "count": 3, "type": "VEC3"},
                {"componentType": 5126, "count": 1, "type": "MAT4"},
                {"componentType": 5126, "count": 1, "type": "SCALAR"},
                {"componentType": 5126, "count": 1, "type": "VEC3"}
            ],
            "skins": [{"inverseBindMatrices": 1, "skeleton": 0, "joints": [0]}],
            "animations": [{
                "samplers": [{"input": 2, "output": 3}],
                "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]
            }]
        });
        let valid_bytes = serde_json::to_vec(&valid).unwrap();
        GltfReader::from_bytes_lenient(&valid_bytes).unwrap();
        assert!(matches!(
            GltfReader::from_bytes(&valid_bytes),
            Err(GltfError::Unsupported(_))
        ));

        for path in [
            "/scene",
            "/scenes/0/nodes/0",
            "/nodes/0/mesh",
            "/nodes/0/children/0",
            "/nodes/0/skin",
            "/skins/0/joints/0",
            "/skins/0/inverseBindMatrices",
            "/animations/0/samplers/0/input",
            "/animations/0/samplers/0/output",
            "/animations/0/channels/0/sampler",
            "/animations/0/channels/0/target/node",
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(path).unwrap() = Value::from(99);
            let invalid_bytes = serde_json::to_vec(&invalid).unwrap();
            let error = match GltfReader::from_bytes_lenient(&invalid_bytes) {
                Ok(_) => panic!("invalid reference at {path} unexpectedly parsed"),
                Err(error) => error,
            };
            assert!(
                matches!(error, GltfError::InvalidGltf(_)),
                "path {path}: {error}"
            );
        }
    }

    #[cfg(feature = "legacy-bitstream-decode")]
    #[test]
    fn test_draco_legacy_bitstream_data_uri() {
        let draco_bytes =
            include_bytes!("../../../testdata/legacy_draco/cube_att.mesh_seq.1.1.0.drc");
        let data_uri = format!(
            "data:application/octet-stream;base64,{}",
            base64_for_test(draco_bytes)
        );
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "extensionsUsed": ["KHR_draco_mesh_compression"],
                "buffers": [{{"byteLength": {}, "uri": "{}"}}],
                "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": {}}}],
                "accessors": [
                    {{"componentType": 5126, "count": 24, "type": "VEC3"}}
                ],
                "meshes": [{{
                    "primitives": [{{
                        "attributes": {{"POSITION": 0}},
                        "mode": 4,
                        "extensions": {{
                            "KHR_draco_mesh_compression": {{
                                "bufferView": 0,
                                "attributes": {{"POSITION": 0}}
                            }}
                        }}
                    }}]
                }}]
            }}"#,
            draco_bytes.len(),
            data_uri,
            draco_bytes.len()
        );

        let mesh = GltfReader::from_gltf(json.as_bytes(), None)
            .unwrap()
            .decode_all_draco_meshes()
            .unwrap()
            .remove(0)
            .1;

        assert_eq!(mesh.num_faces(), 12);
        assert_eq!(mesh.num_points(), 24);
        assert_eq!(
            mesh.named_attribute(GeometryAttributeType::Position)
                .expect("missing position")
                .size(),
            24
        );
    }

    fn base64_for_test(bytes: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();

        for chunk in bytes.chunks(3) {
            let b0 = chunk[0];
            let b1 = *chunk.get(1).unwrap_or(&0);
            let b2 = *chunk.get(2).unwrap_or(&0);
            let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;

            out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
            if chunk.len() > 1 {
                out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() > 2 {
                out.push(TABLE[(n & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }

        out
    }
}
