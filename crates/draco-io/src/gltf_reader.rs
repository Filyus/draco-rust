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
//! ```ignore
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
//!     println!("Node: {:?}, parts: {}", node.name, node.parts.len());
//! }
//! ```

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use draco_core::decoder_buffer::DecoderBuffer;
use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::mesh::Mesh;
use draco_core::mesh_decoder::MeshDecoder;
#[cfg(feature = "point_cloud_decode")]
use draco_core::point_cloud::PointCloud;
#[cfg(feature = "point_cloud_decode")]
use draco_core::point_cloud_decoder::PointCloudDecoder;
use serde::Deserialize;
use thiserror::Error;

/// Errors that can occur when reading glTF files.
#[derive(Error, Debug)]
pub enum GltfError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid GLB: {0}")]
    InvalidGlb(String),

    #[error("Invalid glTF: {0}")]
    InvalidGltf(String),

    #[error("Draco decode error: {0}")]
    DracoDecode(String),

    #[error("Unsupported feature: {0}")]
    Unsupported(String),
}

pub type Result<T> = std::result::Result<T, GltfError>;

// ============================================================================
// glTF JSON Schema (full scene graph support)
// ============================================================================

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GltfRoot {
    #[serde(default)]
    accessors: Vec<Accessor>,
    #[serde(default)]
    buffer_views: Vec<BufferView>,
    #[serde(default)]
    buffers: Vec<Buffer>,
    #[serde(default)]
    meshes: Vec<GltfMesh>,
    #[serde(default)]
    nodes: Vec<GltfNode>,
    #[serde(default)]
    scenes: Vec<GltfScene>,
    /// Default scene index (if present).
    scene: Option<usize>,
    #[serde(default)]
    extensions_used: Vec<String>,
    #[serde(default)]
    extensions_required: Vec<String>,
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
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Accessor {
    buffer_view: Option<usize>,
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
    uri: Option<String>,
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
    attributes: HashMap<String, usize>,
    indices: Option<usize>,
    mode: Option<u32>,
    material: Option<usize>,
    extensions: Option<PrimitiveExtensions>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrimitiveExtensions {
    #[serde(rename = "KHR_draco_mesh_compression")]
    khr_draco_mesh_compression: Option<DracoExtension>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DracoExtension {
    buffer_view: usize,
    #[serde(default)]
    attributes: HashMap<String, usize>,
}

// ============================================================================
// GLB Binary Format Constants
// ============================================================================

const GLB_MAGIC: u32 = 0x46546C67; // "glTF" in little-endian
const GLB_VERSION: u32 = 2;
const GLB_CHUNK_JSON: u32 = 0x4E4F534A; // "JSON"
const GLB_CHUNK_BIN: u32 = 0x004E4942; // "BIN\0"
const GLTF_MODE_POINTS: u32 = 0;
const GLTF_MODE_TRIANGLES: u32 = 4;
const GLTF_COMPONENT_UNSIGNED_BYTE: u32 = 5121;
const GLTF_COMPONENT_UNSIGNED_SHORT: u32 = 5123;
const GLTF_COMPONENT_UNSIGNED_INT: u32 = 5125;
const GLTF_COMPONENT_FLOAT: u32 = 5126;

// ============================================================================
// GltfReader
// ============================================================================

/// A reader for glTF/GLB files with Draco mesh decompression support.
pub struct GltfReader {
    root: GltfRoot,
    buffers: Vec<Vec<u8>>,
}

struct DecodedAccessor {
    count: usize,
    num_components: u8,
    data_type: DataType,
    normalized: bool,
    bytes: Vec<u8>,
}

impl DecodedAccessor {
    fn gather(&self, indices: &[u32]) -> Result<Self> {
        let stride = self.num_components as usize * self.data_type.byte_length();
        let mut bytes = Vec::with_capacity(indices.len() * stride);

        for &index in indices {
            let index = index as usize;
            if index >= self.count {
                return Err(GltfError::InvalidGltf(format!(
                    "Accessor index {} out of bounds for {} values",
                    index, self.count
                )));
            }
            let offset = index * stride;
            bytes.extend_from_slice(&self.bytes[offset..offset + stride]);
        }

        Ok(Self {
            count: indices.len(),
            num_components: self.num_components,
            data_type: self.data_type,
            normalized: self.normalized,
            bytes,
        })
    }
}

struct GltfAccessorReader<'a> {
    accessors: &'a [Accessor],
    buffer_views: &'a [BufferView],
    buffers: &'a [Vec<u8>],
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
        let row_size = num_components as usize * component_size;
        let layout = self.accessor_layout(accessor, row_size, "Accessor")?;

        let mut bytes = Vec::with_capacity(accessor.count * row_size);
        for i in 0..accessor.count {
            let offset = layout
                .start
                .checked_add(i * layout.stride)
                .ok_or_else(|| GltfError::InvalidGltf("Accessor range overflow".into()))?;
            if offset + row_size > layout.view_end {
                return Err(GltfError::InvalidGltf(format!(
                    "{} accessor out of bounds",
                    accessor.accessor_type
                )));
            }
            bytes.extend_from_slice(&layout.buffer[offset..offset + row_size]);
        }

        Ok(DecodedAccessor {
            count: accessor.count,
            num_components,
            data_type,
            normalized: accessor.normalized,
            bytes,
        })
    }

    fn read_indices(&self, accessor_idx: usize) -> Result<Vec<u32>> {
        let accessor = self.accessor(accessor_idx)?;

        if accessor.accessor_type != "SCALAR" {
            return Err(GltfError::InvalidGltf(format!(
                "Expected SCALAR accessor for indices, got {}",
                accessor.accessor_type
            )));
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
        let layout = self.accessor_layout(accessor, component_size, "Index accessor")?;
        let mut result = Vec::with_capacity(accessor.count);

        match accessor.component_type {
            GLTF_COMPONENT_UNSIGNED_BYTE => {
                for i in 0..accessor.count {
                    let offset = layout.start + i * layout.stride;
                    if offset + component_size > layout.view_end {
                        return Err(GltfError::InvalidGltf(
                            "Index accessor out of bounds".into(),
                        ));
                    }
                    result.push(layout.buffer[offset] as u32);
                }
            }
            GLTF_COMPONENT_UNSIGNED_SHORT => {
                for i in 0..accessor.count {
                    let offset = layout.start + i * layout.stride;
                    if offset + component_size > layout.view_end {
                        return Err(GltfError::InvalidGltf(
                            "Index accessor out of bounds".into(),
                        ));
                    }
                    let val =
                        u16::from_le_bytes([layout.buffer[offset], layout.buffer[offset + 1]]);
                    result.push(val as u32);
                }
            }
            GLTF_COMPONENT_UNSIGNED_INT => {
                for i in 0..accessor.count {
                    let offset = layout.start + i * layout.stride;
                    if offset + component_size > layout.view_end {
                        return Err(GltfError::InvalidGltf(
                            "Index accessor out of bounds".into(),
                        ));
                    }
                    let val = u32::from_le_bytes([
                        layout.buffer[offset],
                        layout.buffer[offset + 1],
                        layout.buffer[offset + 2],
                        layout.buffer[offset + 3],
                    ]);
                    result.push(val);
                }
            }
            _ => unreachable!(),
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
        label: &str,
    ) -> Result<AccessorLayout<'a>> {
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
        let start = view_offset + accessor_offset;
        let stride = buffer_view.byte_stride.unwrap_or(element_size);

        if stride < element_size {
            return Err(GltfError::InvalidGltf(format!(
                "{} byteStride {} is smaller than element size {}",
                label, stride, element_size
            )));
        }

        let view_end = view_offset
            .checked_add(buffer_view.byte_length)
            .ok_or_else(|| GltfError::InvalidGltf("Buffer view range overflow".into()))?;
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

struct GlbChunks<'a> {
    json: &'a [u8],
    bin: Option<&'a [u8]>,
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
    pub attributes: HashMap<String, usize>,
}

impl GltfReader {
    /// Open a glTF or GLB file.
    ///
    /// The file type is detected automatically based on the magic bytes.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let data = fs::read(path)?;

        if data.len() >= 4 && read_u32_le(&data[0..4]) == GLB_MAGIC {
            Self::from_glb_with_base_path(&data, path.parent())
        } else {
            let base_path = path.parent();
            Self::from_gltf(&data, base_path)
        }
    }

    /// Parse from GLB binary data.
    pub fn from_glb(data: &[u8]) -> Result<Self> {
        Self::from_glb_with_base_path(data, None)
    }

    fn from_glb_with_base_path(data: &[u8], base_path: Option<&Path>) -> Result<Self> {
        let chunks = parse_glb_chunks(data)?;
        let root: GltfRoot = serde_json::from_slice(chunks.json)?;
        let buffers = load_buffers(&root, true, chunks.bin, base_path)?;

        Ok(Self { root, buffers })
    }

    /// Parse from glTF JSON data with optional base path for external buffers.
    pub fn from_gltf(json_data: &[u8], base_path: Option<&Path>) -> Result<Self> {
        let root: GltfRoot = serde_json::from_slice(json_data)?;
        let buffers = load_buffers(&root, false, None, base_path)?;

        Ok(Self { root, buffers })
    }

    /// Check if the glTF file uses Draco compression.
    pub fn has_draco_extension(&self) -> bool {
        self.root
            .extensions_used
            .iter()
            .any(|ext| ext == "KHR_draco_mesh_compression")
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
                            buffer_view: draco.buffer_view,
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
        let end = offset + buffer_view.byte_length;

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
            .map_err(|e| GltfError::DracoDecode(format!("{:?}", e)))?;

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
            .map_err(|e| GltfError::DracoDecode(format!("{:?}", e)))?;

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
    fn decode_standard_primitive(
        &self,
        mesh_idx: usize,
        prim_idx: usize,
        primitive: &Primitive,
    ) -> Result<Mesh> {
        use draco_core::geometry_indices::PointIndex;

        let mode = primitive.mode.unwrap_or(GLTF_MODE_TRIANGLES);
        if mode != GLTF_MODE_TRIANGLES && mode != GLTF_MODE_POINTS {
            return Err(GltfError::Unsupported(format!(
                "Primitive mode {} not supported (only POINTS=0 and TRIANGLES=4)",
                mode
            )));
        }

        // Get POSITION accessor (required for mesh)
        let pos_accessor_idx = primitive.attributes.get("POSITION").ok_or_else(|| {
            GltfError::InvalidGltf(format!(
                "Primitive {}:{} has no POSITION attribute",
                mesh_idx, prim_idx
            ))
        })?;

        let accessor_reader = self.accessor_reader();
        let positions = accessor_reader.read_attribute(
            *pos_accessor_idx,
            &["VEC3"],
            &[GLTF_COMPONENT_FLOAT],
        )?;

        let mut mesh = Mesh::new();
        let point_indices = if mode == GLTF_MODE_POINTS {
            primitive
                .indices
                .map(|indices_accessor_idx| accessor_reader.read_indices(indices_accessor_idx))
                .transpose()?
        } else {
            None
        };
        let positions = if let Some(indices) = &point_indices {
            positions.gather(indices)?
        } else {
            positions
        };
        mesh.set_num_points(positions.count);

        Self::add_decoded_attribute(&mut mesh, GeometryAttributeType::Position, positions)?;

        if mode == GLTF_MODE_TRIANGLES {
            if let Some(indices_accessor_idx) = primitive.indices {
                let indices = accessor_reader.read_indices(indices_accessor_idx)?;
                if indices.len() % 3 != 0 {
                    return Err(GltfError::InvalidGltf(
                        "Index count not divisible by 3 for triangles".into(),
                    ));
                }
                let num_faces = indices.len() / 3;
                for i in 0..num_faces {
                    let face = [
                        PointIndex(indices[i * 3]),
                        PointIndex(indices[i * 3 + 1]),
                        PointIndex(indices[i * 3 + 2]),
                    ];
                    mesh.add_face(face);
                }
            } else {
                // Non-indexed: generate sequential triangle faces
                if mesh.num_points() % 3 != 0 {
                    return Err(GltfError::InvalidGltf(
                        "Non-indexed primitive point count not divisible by 3".into(),
                    ));
                }
                for i in 0..(mesh.num_points() / 3) {
                    let base = (i * 3) as u32;
                    mesh.add_face([PointIndex(base), PointIndex(base + 1), PointIndex(base + 2)]);
                }
            }
        }

        // Optionally read NORMAL
        if let Some(&normal_idx) = primitive.attributes.get("NORMAL") {
            let normals =
                accessor_reader.read_attribute(normal_idx, &["VEC3"], &[GLTF_COMPONENT_FLOAT])?;
            let normals = if let Some(indices) = &point_indices {
                normals.gather(indices)?
            } else {
                normals
            };
            Self::add_decoded_attribute(&mut mesh, GeometryAttributeType::Normal, normals)?;
        }

        // Optionally read TEXCOORD_0
        if let Some(&tex_idx) = primitive.attributes.get("TEXCOORD_0") {
            let texcoords = accessor_reader.read_attribute(
                tex_idx,
                &["VEC2"],
                &[
                    GLTF_COMPONENT_FLOAT,
                    GLTF_COMPONENT_UNSIGNED_BYTE,
                    GLTF_COMPONENT_UNSIGNED_SHORT,
                ],
            )?;
            let texcoords = if let Some(indices) = &point_indices {
                texcoords.gather(indices)?
            } else {
                texcoords
            };
            Self::add_decoded_attribute(&mut mesh, GeometryAttributeType::TexCoord, texcoords)?;
        }

        // Optionally read COLOR_0.
        if let Some(&color_idx) = primitive.attributes.get("COLOR_0") {
            let colors = accessor_reader.read_attribute(
                color_idx,
                &["VEC3", "VEC4"],
                &[
                    GLTF_COMPONENT_FLOAT,
                    GLTF_COMPONENT_UNSIGNED_BYTE,
                    GLTF_COMPONENT_UNSIGNED_SHORT,
                ],
            )?;
            let colors = if let Some(indices) = &point_indices {
                colors.gather(indices)?
            } else {
                colors
            };
            Self::add_decoded_attribute(&mut mesh, GeometryAttributeType::Color, colors)?;
        }

        // Match C++ Draco behavior: deduplicate point IDs in face-traversal order.
        // This ensures binary compatibility when encoding.
        // Note: Draco-compressed meshes don't need this as they're already in the correct format.
        mesh.deduplicate_point_ids();

        Ok(mesh)
    }

    fn add_decoded_attribute(
        mesh: &mut Mesh,
        attribute_type: GeometryAttributeType,
        decoded: DecodedAccessor,
    ) -> Result<()> {
        if decoded.count != mesh.num_points() {
            return Err(GltfError::InvalidGltf(format!(
                "Attribute {:?} has {} values but primitive has {} points",
                attribute_type,
                decoded.count,
                mesh.num_points()
            )));
        }

        let mut attribute = PointAttribute::new();
        attribute.init(
            attribute_type,
            decoded.num_components,
            decoded.data_type,
            decoded.normalized,
            decoded.count,
        );
        attribute.buffer_mut().write(0, &decoded.bytes);
        mesh.add_attribute(attribute);
        Ok(())
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
                buffer_view: draco.buffer_view,
                attributes: draco.attributes.clone(),
            };
            self.decode_draco_mesh(&info)
        } else {
            self.decode_standard_primitive(mesh_idx, prim_idx, primitive)
        }
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

fn read_u32_le(data: &[u8]) -> u32 {
    u32::from_le_bytes([data[0], data[1], data[2], data[3]])
}

fn parse_glb_chunks(data: &[u8]) -> Result<GlbChunks<'_>> {
    if data.len() < 12 {
        return Err(GltfError::InvalidGlb(
            "File too small for GLB header".into(),
        ));
    }

    let magic = read_u32_le(&data[0..4]);
    let version = read_u32_le(&data[4..8]);
    let length = read_u32_le(&data[8..12]) as usize;

    if magic != GLB_MAGIC {
        return Err(GltfError::InvalidGlb("Invalid GLB magic".into()));
    }
    if version != GLB_VERSION {
        return Err(GltfError::InvalidGlb(format!(
            "Unsupported GLB version: {}",
            version
        )));
    }
    if length > data.len() {
        return Err(GltfError::InvalidGlb("File truncated".into()));
    }

    let mut offset = 12;
    let mut json_chunk: Option<&[u8]> = None;
    let mut bin_chunk: Option<&[u8]> = None;

    while offset + 8 <= length {
        let chunk_length = read_u32_le(&data[offset..offset + 4]) as usize;
        let chunk_type = read_u32_le(&data[offset + 4..offset + 8]);
        offset += 8;

        if offset + chunk_length > length {
            return Err(GltfError::InvalidGlb("Chunk extends past file end".into()));
        }

        let chunk_data = &data[offset..offset + chunk_length];
        offset += chunk_length;

        match chunk_type {
            GLB_CHUNK_JSON => json_chunk = Some(chunk_data),
            GLB_CHUNK_BIN => bin_chunk = Some(chunk_data),
            _ => {}
        }
    }

    Ok(GlbChunks {
        json: json_chunk.ok_or_else(|| GltfError::InvalidGlb("No JSON chunk".into()))?,
        bin: bin_chunk,
    })
}

fn load_buffers(
    root: &GltfRoot,
    is_glb: bool,
    glb_bin_chunk: Option<&[u8]>,
    base_path: Option<&Path>,
) -> Result<Vec<Vec<u8>>> {
    let mut buffers = Vec::with_capacity(root.buffers.len());
    for (i, buffer) in root.buffers.iter().enumerate() {
        buffers.push(load_buffer(i, buffer, is_glb, glb_bin_chunk, base_path)?);
    }
    Ok(buffers)
}

fn load_buffer(
    index: usize,
    buffer: &Buffer,
    is_glb: bool,
    glb_bin_chunk: Option<&[u8]>,
    base_path: Option<&Path>,
) -> Result<Vec<u8>> {
    if let Some(uri) = &buffer.uri {
        if uri.starts_with("data:") {
            return decode_data_uri(uri);
        }
        if let Some(base) = base_path {
            return Ok(fs::read(base.join(uri))?);
        }
        if is_glb {
            return Err(GltfError::Unsupported(
                "External buffer URIs require opening GLB from a filesystem path".into(),
            ));
        }
        return Ok(fs::read(Path::new(uri))?);
    }

    if is_glb {
        if index == 0 {
            return glb_bin_chunk.map(|bin| bin.to_vec()).ok_or_else(|| {
                GltfError::InvalidGlb("Buffer 0 has no URI but no BIN chunk present".into())
            });
        }
        return Err(GltfError::InvalidGlb(format!(
            "Buffer {} has no URI and is not buffer 0",
            index
        )));
    }

    Err(GltfError::InvalidGltf(
        "Buffer without URI in non-GLB file".into(),
    ))
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
        GLTF_COMPONENT_UNSIGNED_BYTE => Ok(DataType::Uint8),
        GLTF_COMPONENT_UNSIGNED_SHORT => Ok(DataType::Uint16),
        GLTF_COMPONENT_UNSIGNED_INT => Ok(DataType::Uint32),
        GLTF_COMPONENT_FLOAT => Ok(DataType::Float32),
        _ => Err(GltfError::Unsupported(format!(
            "Unsupported component type: {}",
            component_type
        ))),
    }
}

fn decode_data_uri(uri: &str) -> Result<Vec<u8>> {
    // Format: data:[<mediatype>][;base64],<data>
    let comma_pos = uri
        .find(',')
        .ok_or_else(|| GltfError::InvalidGltf("Invalid data URI: no comma".into()))?;

    let header = &uri[5..comma_pos]; // Skip "data:"
    let data = &uri[comma_pos + 1..];

    if header.contains(";base64") {
        decode_base64(data)
    } else {
        // URL-encoded data
        Ok(percent_decode(data))
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
    fn compute_node_transform(node: &GltfNode) -> Option<crate::traits::Transform> {
        if let Some(m) = &node.matrix {
            // glTF stores column-major; convert to row-major
            Some(crate::traits::Transform {
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
            Some(crate::traits::Transform {
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
    ) -> Result<crate::traits::SceneNode> {
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

        let mut scene_node = crate::traits::SceneNode::new(gltf_node.name.clone());
        scene_node.transform = Self::compute_node_transform(gltf_node);

        // Attach meshes if this node references a mesh
        if let Some(mesh_idx) = gltf_node.mesh {
            if let Some(gltf_mesh) = self.root.meshes.get(mesh_idx) {
                for (prim_idx, primitive) in gltf_mesh.primitives.iter().enumerate() {
                    let mesh =
                        self.decode_primitive_mesh(mesh_idx, gltf_mesh, prim_idx, primitive)?;

                    let part_name = if gltf_mesh.primitives.len() > 1 {
                        gltf_mesh
                            .name
                            .as_ref()
                            .map(|n| format!("{}_{}", n, prim_idx))
                    } else {
                        gltf_mesh.name.clone()
                    };

                    scene_node.parts.push(crate::traits::SceneObject {
                        name: part_name,
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
}

impl crate::traits::SceneReader for GltfReader {
    fn read_scene(&mut self) -> std::io::Result<crate::traits::Scene> {
        let map_err = |e: GltfError| std::io::Error::other(e.to_string());

        // Select scene: prefer default, else first, else empty
        let scene_idx = self.root.scene.or({
            if self.root.scenes.is_empty() {
                None
            } else {
                Some(0)
            }
        });

        let (scene_name, root_node_indices) = if let Some(idx) = scene_idx {
            let gltf_scene = self.root.scenes.get(idx).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Invalid scene index: {}", idx),
                )
            })?;
            (gltf_scene.name.clone(), gltf_scene.nodes.clone())
        } else {
            // No scenes defined: treat all root-level nodes as roots
            // (nodes not referenced as children by any other node)
            let mut is_child = vec![false; self.root.nodes.len()];
            for node in &self.root.nodes {
                for &child_idx in &node.children {
                    if child_idx < is_child.len() {
                        is_child[child_idx] = true;
                    }
                }
            }
            let roots: Vec<usize> = (0..self.root.nodes.len())
                .filter(|&i| !is_child[i])
                .collect();
            (None, roots)
        };

        // Build node hierarchy
        let mut visited = vec![false; self.root.nodes.len()];
        let mut root_nodes = Vec::with_capacity(root_node_indices.len());
        for &node_idx in &root_node_indices {
            let scene_node = self
                .build_scene_node(node_idx, &mut visited)
                .map_err(map_err)?;
            root_nodes.push(scene_node);
        }

        // Collect all parts (flattened) for backward compatibility
        fn collect_parts(
            node: &crate::traits::SceneNode,
            out: &mut Vec<crate::traits::SceneObject>,
        ) {
            out.extend(node.parts.clone());
            for child in &node.children {
                collect_parts(child, out);
            }
        }

        let mut parts = Vec::new();
        for node in &root_nodes {
            collect_parts(node, &mut parts);
        }

        Ok(crate::traits::Scene {
            name: scene_name,
            parts,
            root_nodes,
        })
    }
}
fn decode_base64(input: &str) -> Result<Vec<u8>> {
    // Simple base64 decoder (no external dependency)
    const DECODE_TABLE: [i8; 128] = [
        -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1,
        -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, -1, 62, -1, -1,
        -1, 63, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, -1, -1, -1, -1, -1, -1, -1, 0, 1, 2, 3, 4,
        5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, -1, -1, -1,
        -1, -1, -1, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45,
        46, 47, 48, 49, 50, 51, -1, -1, -1, -1, -1,
    ];

    let input: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'\n' && b != b'\r')
        .collect();
    let mut output = Vec::with_capacity(input.len() * 3 / 4);

    let chunks = input.chunks(4);
    for chunk in chunks {
        if chunk.is_empty() {
            break;
        }

        let mut buf = [0u8; 4];
        let mut valid_count = 0;

        for (i, &byte) in chunk.iter().enumerate() {
            if byte == b'=' {
                buf[i] = 0;
            } else if byte < 128 {
                let val = DECODE_TABLE[byte as usize];
                if val < 0 {
                    return Err(GltfError::InvalidGltf("Invalid base64 character".into()));
                }
                buf[i] = val as u8;
                valid_count = i + 1;
            } else {
                return Err(GltfError::InvalidGltf("Invalid base64 character".into()));
            }
        }

        // Fill remaining slots with 0 if chunk is incomplete
        for item in buf.iter_mut().skip(chunk.len()) {
            *item = 0;
        }

        let n = ((buf[0] as u32) << 18)
            | ((buf[1] as u32) << 12)
            | ((buf[2] as u32) << 6)
            | (buf[3] as u32);

        output.push((n >> 16) as u8);
        if valid_count > 2 {
            output.push((n >> 8) as u8);
        }
        if valid_count > 3 {
            output.push(n as u8);
        }
    }

    Ok(output)
}

fn percent_decode(input: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_digit(bytes[i + 1]), hex_digit(bytes[i + 2])) {
                output.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        output.push(bytes[i]);
        i += 1;
    }

    output
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
    use draco_core::mesh::Mesh;
    use tempfile::tempdir;

    fn build_glb(json: &str) -> Vec<u8> {
        let mut json_bytes = json.as_bytes().to_vec();
        while !json_bytes.len().is_multiple_of(4) {
            json_bytes.push(b' ');
        }

        let total_len = 12 + 8 + json_bytes.len();
        let mut glb = Vec::with_capacity(total_len);
        glb.extend_from_slice(&GLB_MAGIC.to_le_bytes());
        glb.extend_from_slice(&GLB_VERSION.to_le_bytes());
        glb.extend_from_slice(&(total_len as u32).to_le_bytes());
        glb.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
        glb.extend_from_slice(&GLB_CHUNK_JSON.to_le_bytes());
        glb.extend_from_slice(&json_bytes);
        glb
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

    fn read_attribute_bytes(mesh: &Mesh, attribute_type: GeometryAttributeType) -> Vec<u8> {
        mesh.named_attribute(attribute_type)
            .expect("missing attribute")
            .buffer()
            .data()
            .to_vec()
    }

    #[test]
    fn test_base64_decode() {
        assert_eq!(decode_base64("SGVsbG8=").unwrap(), b"Hello");
        assert_eq!(decode_base64("SGVsbG8gV29ybGQ=").unwrap(), b"Hello World");
        assert_eq!(decode_base64("YQ==").unwrap(), b"a");
        assert_eq!(decode_base64("YWI=").unwrap(), b"ab");
        assert_eq!(decode_base64("YWJj").unwrap(), b"abc");
    }

    #[test]
    fn test_percent_decode() {
        assert_eq!(percent_decode("Hello%20World"), b"Hello World");
        assert_eq!(percent_decode("%2F"), b"/");
        assert_eq!(percent_decode("test"), b"test");
    }

    #[test]
    fn test_glb_magic() {
        // "glTF" in ASCII = 0x67, 0x6C, 0x54, 0x46
        // In little-endian u32: 0x46546C67
        let magic_bytes = b"glTF";
        let magic = u32::from_le_bytes(*magic_bytes);
        assert_eq!(magic, GLB_MAGIC);
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
    fn test_gltf_with_draco_extension() {
        let json = r#"{
            "asset": {"version": "2.0"},
            "extensionsUsed": ["KHR_draco_mesh_compression"],
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
            "buffers": [{"byteLength": 100, "uri": "data:application/octet-stream;base64,AAAA"}],
            "bufferViews": [{"buffer": 0, "byteLength": 3}],
            "accessors": []
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
        assert!(matches!(err, GltfError::Unsupported(_)));
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

    #[test]
    fn test_writer_glb_roundtrips_through_reader() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("roundtrip.glb");

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

        let mut writer = crate::gltf_writer::GltfWriter::new();
        writer
            .add_draco_mesh(&mesh, Some("triangle"), None)
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
