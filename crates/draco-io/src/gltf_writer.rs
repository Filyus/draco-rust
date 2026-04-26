/// Quantization settings for each attribute type.
#[derive(Clone, Copy, Debug)]
pub struct QuantizationBits {
    pub position: i32,
    pub normal: i32,
    pub color: i32,
    pub texcoord: i32,
    pub generic: i32,
}

impl Default for QuantizationBits {
    fn default() -> Self {
        Self {
            position: 14,
            normal: 10,
            color: 8,
            texcoord: 12,
            generic: 8,
        }
    }
}
// glTF/GLB writer with Draco mesh compression support.
//
// This module provides support for writing glTF 2.0 files with the
// `KHR_draco_mesh_compression` extension. Multiple output formats are supported:
//
// - **GLB** - Binary container (single .glb file)
// - **glTF + .bin** - JSON + separate binary file
// - **glTF (embedded)** - Single JSON file with base64-encoded data URIs
//
// # Example - GLB
//
// ```ignore
// use draco_io::gltf_writer::GltfWriter;
// use draco_core::mesh::Mesh;
//
// let mesh: Mesh = /* ... */;
// let mut writer = GltfWriter::new();
// writer.add_draco_mesh(&mesh, Some("MyMesh"), None)?;  // Uses default quantization
// writer.write_glb("output.glb")?;
// ```
//
// # Example - Pure Text glTF (Embedded)
//
// ```ignore
// writer.write_gltf_embedded("output.gltf")?;
// // Creates a single text file with base64-embedded binary data
// ```
//
// # Example - Writing a Scene Graph
//
// ```ignore
// use draco_io::gltf_writer::GltfWriter;
// use draco_io::traits::{Scene, SceneNode, SceneObject};
//
// let mut root = SceneNode::new(Some("Root".to_string()));
// root.parts.push(SceneObject { name: Some("Mesh".to_string()), mesh, transform: None });
// let scene = Scene { name: Some("Scene".to_string()), parts: Vec::new(), root_nodes: vec![root] };
//
// let mut writer = GltfWriter::new();
// writer.add_scene(&scene, None)?;
// writer.write_glb("scene.glb")?;
// ```

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;

use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::MeshEncoder;
use serde::Serialize;
use thiserror::Error;

use crate::traits::Writer;

/// Errors that can occur when writing glTF files.
#[derive(Error, Debug)]
pub enum GltfWriteError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON serialize error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Draco encode error: {0}")]
    DracoEncode(String),

    #[error("Invalid mesh: {0}")]
    InvalidMesh(String),
}

pub type Result<T> = std::result::Result<T, GltfWriteError>;

// ============================================================================
// glTF JSON Schema for Writing
// ============================================================================

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GltfRoot {
    asset: Asset,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    accessors: Vec<AccessorOut>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    buffer_views: Vec<BufferViewOut>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    buffers: Vec<BufferOut>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    meshes: Vec<MeshOut>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    nodes: Vec<NodeOut>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scene: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    scenes: Vec<SceneOut>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    extensions_used: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    extensions_required: Vec<String>,
}

#[derive(Debug, Serialize)]
struct Asset {
    version: String,
    generator: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccessorOut {
    buffer_view: Option<usize>,
    byte_offset: Option<usize>,
    component_type: u32,
    count: usize,
    #[serde(rename = "type")]
    accessor_type: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    min: Vec<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    max: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BufferViewOut {
    buffer: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_offset: Option<usize>,
    byte_length: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BufferOut {
    byte_length: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    uri: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct MeshOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    primitives: Vec<PrimitiveOut>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrimitiveOut {
    attributes: HashMap<String, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    indices: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extensions: Option<PrimitiveExtensionsOut>,
}

#[derive(Debug, Clone, Serialize)]
struct PrimitiveExtensionsOut {
    #[serde(rename = "KHR_draco_mesh_compression")]
    khr_draco_mesh_compression: DracoExtensionOut,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DracoExtensionOut {
    buffer_view: usize,
    attributes: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    mesh: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<usize>,
    /// 4x4 transformation matrix (column-major).
    #[serde(skip_serializing_if = "Option::is_none")]
    matrix: Option<[f32; 16]>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SceneOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    nodes: Vec<usize>,
}

// ============================================================================
// GLB Constants
// ============================================================================

const GLB_MAGIC: u32 = 0x46546C67; // "glTF"
const GLB_VERSION: u32 = 2;
const GLB_CHUNK_JSON: u32 = 0x4E4F534A; // "JSON"
const GLB_CHUNK_BIN: u32 = 0x004E4942; // "BIN\0"

// ============================================================================
// GltfWriter
// ============================================================================

/// A writer for creating glTF/GLB files with Draco-compressed meshes.
pub struct GltfWriter {
    accessors: Vec<AccessorOut>,
    buffer_views: Vec<BufferViewOut>,
    meshes: Vec<MeshOut>,
    nodes: Vec<NodeOut>,
    scenes: Vec<SceneOut>,
    default_scene: Option<usize>,
    binary_data: Vec<u8>,
    has_draco: bool,
}

impl Default for GltfWriter {
    fn default() -> Self {
        Self::new()
    }
}

fn encode_draco_mesh_bytes(mesh: &Mesh, quantization: &QuantizationBits) -> Result<Vec<u8>> {
    if mesh.num_faces() == 0 {
        return Err(GltfWriteError::InvalidMesh("Mesh has no faces".into()));
    }

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());

    let mut options = EncoderOptions::new();

    // Set quantization for each attribute type, clamped to 1..=31.
    // This matches the behavior used by the glTF writer.
    for i in 0..mesh.num_attributes() {
        let att = mesh.attribute(i);
        if att.data_type() == draco_core::draco_types::DataType::Float32 {
            let bits = match att.attribute_type() {
                GeometryAttributeType::Position => quantization.position,
                GeometryAttributeType::Normal => quantization.normal,
                GeometryAttributeType::Color => quantization.color,
                GeometryAttributeType::TexCoord => quantization.texcoord,
                GeometryAttributeType::Generic => quantization.generic,
                GeometryAttributeType::Invalid => 8,
            };
            let bits = bits.clamp(1, 31);
            options.set_attribute_int(i, "quantization_bits", bits);
        }
    }

    let mut enc_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut enc_buffer)
        .map_err(|e| GltfWriteError::DracoEncode(format!("{:?}", e)))?;

    Ok(enc_buffer.data().to_vec())
}

/// Encode a mesh to a Draco bitstream using the same settings as `GltfWriter`.
///
/// This is useful for tools/tests that need the raw `.drc` bytes without
/// wrapping them into a glTF/GLB container.
pub fn encode_draco_mesh(
    mesh: &Mesh,
    quantization: impl Into<Option<QuantizationBits>>,
) -> Result<Vec<u8>> {
    let quantization = quantization.into().unwrap_or_default();
    encode_draco_mesh_bytes(mesh, &quantization)
}

impl GltfWriter {
    /// Create a new glTF writer.
    pub fn new() -> Self {
        Self {
            accessors: Vec::new(),
            buffer_views: Vec::new(),
            meshes: Vec::new(),
            nodes: Vec::new(),
            scenes: Vec::new(),
            default_scene: None,
            binary_data: Vec::new(),
            has_draco: false,
        }
    }

    /// Add a full scene graph (nodes + hierarchy + transforms) to the output.
    ///
    /// Geometry is written using Draco compression (KHR_draco_mesh_compression).
    /// Non-Draco writing (raw accessors) is not currently supported by this writer.
    pub fn add_scene(
        &mut self,
        scene: &crate::traits::Scene,
        quantization: impl Into<Option<QuantizationBits>>,
    ) -> Result<usize> {
        let quantization = quantization.into().unwrap_or_default();

        // Build nodes recursively and record root node indices.
        let mut root_node_indices = Vec::with_capacity(scene.root_nodes.len());
        for root in &scene.root_nodes {
            let node_idx = self
                .push_scene_node(root, &quantization)
                .map_err(|e| match e {
                    GltfWriteError::Io(_)
                    | GltfWriteError::Json(_)
                    | GltfWriteError::DracoEncode(_)
                    | GltfWriteError::InvalidMesh(_) => e,
                })?;
            root_node_indices.push(node_idx);
        }

        let scene_idx = self.scenes.len();
        self.scenes.push(SceneOut {
            name: scene.name.clone(),
            nodes: root_node_indices,
        });

        if self.default_scene.is_none() {
            self.default_scene = Some(scene_idx);
        }

        Ok(scene_idx)
    }

    fn transform_to_gltf_matrix(transform: &crate::traits::Transform) -> [f32; 16] {
        // Input is row-major; glTF expects column-major.
        let m = &transform.matrix;
        [
            m[0][0], m[1][0], m[2][0], m[3][0], m[0][1], m[1][1], m[2][1], m[3][1], m[0][2],
            m[1][2], m[2][2], m[3][2], m[0][3], m[1][3], m[2][3], m[3][3],
        ]
    }

    fn push_scene_node(
        &mut self,
        node: &crate::traits::SceneNode,
        quantization: &QuantizationBits,
    ) -> Result<usize> {
        // glTF nodes can reference at most one mesh; if multiple parts exist,
        // we create child nodes for each part.

        // First, create this node (without children for now).
        let node_idx = self.nodes.len();
        self.nodes.push(NodeOut {
            mesh: None,
            name: node.name.clone(),
            children: Vec::new(),
            matrix: node.transform.as_ref().map(Self::transform_to_gltf_matrix),
        });

        // Attach parts.
        if node.parts.len() == 1 {
            let part = &node.parts[0];
            let mesh_idx =
                self.encode_draco_mesh_internal(&part.mesh, part.name.as_deref(), quantization)?;
            self.nodes[node_idx].mesh = Some(mesh_idx);
        } else if !node.parts.is_empty() {
            for (i, part) in node.parts.iter().enumerate() {
                let part_mesh_idx = self.encode_draco_mesh_internal(
                    &part.mesh,
                    part.name.as_deref(),
                    quantization,
                )?;
                let child_idx = self.nodes.len();
                self.nodes.push(NodeOut {
                    mesh: Some(part_mesh_idx),
                    name: part
                        .name
                        .clone()
                        .or_else(|| node.name.as_ref().map(|n| format!("{}_part{}", n, i))),
                    children: Vec::new(),
                    matrix: part.transform.as_ref().map(Self::transform_to_gltf_matrix),
                });
                self.nodes[node_idx].children.push(child_idx);
            }
        }

        // Recurse into children.
        for child in &node.children {
            let child_idx = self.push_scene_node(child, quantization)?;
            self.nodes[node_idx].children.push(child_idx);
        }

        Ok(node_idx)
    }

    fn encode_draco_mesh_internal(
        &mut self,
        mesh: &Mesh,
        name: Option<&str>,
        quantization: &QuantizationBits,
    ) -> Result<usize> {
        let draco_data = encode_draco_mesh_bytes(mesh, quantization)?;
        let draco_buffer_view_idx = self.append_buffer_view(&draco_data);
        let primitive = self.build_draco_primitive(mesh, draco_buffer_view_idx);

        let mesh_idx = self.meshes.len();
        self.meshes.push(MeshOut {
            name: name.map(String::from),
            primitives: vec![primitive],
        });

        self.has_draco = true;
        Ok(mesh_idx)
    }

    fn append_buffer_view(&mut self, data: &[u8]) -> usize {
        while !self.binary_data.len().is_multiple_of(4) {
            self.binary_data.push(0);
        }
        let aligned_offset = self.binary_data.len();

        self.binary_data.extend_from_slice(data);
        let buffer_view_idx = self.buffer_views.len();
        self.buffer_views.push(BufferViewOut {
            buffer: 0,
            byte_offset: Some(aligned_offset),
            byte_length: data.len(),
        });

        buffer_view_idx
    }

    fn build_draco_primitive(&mut self, mesh: &Mesh, draco_buffer_view_idx: usize) -> PrimitiveOut {
        let (attributes, draco_attributes) = self.add_mesh_attribute_accessors(mesh);
        let indices_accessor_idx = self.add_indices_accessor(mesh.num_faces() * 3);

        PrimitiveOut {
            attributes,
            indices: Some(indices_accessor_idx),
            mode: Some(4), // TRIANGLES
            extensions: Some(PrimitiveExtensionsOut {
                khr_draco_mesh_compression: DracoExtensionOut {
                    buffer_view: draco_buffer_view_idx,
                    attributes: draco_attributes,
                },
            }),
        }
    }

    fn add_mesh_attribute_accessors(
        &mut self,
        mesh: &Mesh,
    ) -> (HashMap<String, usize>, HashMap<String, usize>) {
        let mut attributes = HashMap::new();
        let mut draco_attributes: HashMap<String, usize> = HashMap::new();

        for i in 0..mesh.num_attributes() {
            let att = mesh.attribute(i);
            let Some((semantic, accessor_type)) =
                gltf_attribute_info(att.attribute_type(), att.num_components(), i as usize)
            else {
                continue;
            };

            let accessor_idx = self.add_attribute_accessor(att, accessor_type);
            attributes.insert(semantic.clone(), accessor_idx);
            draco_attributes.insert(semantic, i as usize);
        }

        (attributes, draco_attributes)
    }

    fn add_attribute_accessor(&mut self, att: &PointAttribute, accessor_type: &str) -> usize {
        let accessor_idx = self.accessors.len();
        self.accessors.push(AccessorOut {
            buffer_view: None,
            byte_offset: None,
            component_type: component_type_for_data_type(att.data_type()),
            count: att.size(),
            accessor_type: accessor_type.to_string(),
            min: Vec::new(),
            max: Vec::new(),
        });
        accessor_idx
    }

    fn add_indices_accessor(&mut self, count: usize) -> usize {
        let accessor_idx = self.accessors.len();
        self.accessors.push(AccessorOut {
            buffer_view: None,
            byte_offset: None,
            component_type: 5125, // UNSIGNED_INT
            count,
            accessor_type: "SCALAR".to_string(),
            min: Vec::new(),
            max: Vec::new(),
        });
        accessor_idx
    }

    /// Add a mesh with Draco compression.
    ///
    /// # Arguments
    /// * `mesh` - The mesh to encode
    /// * `name` - Optional name for the mesh
    /// * `quantization` - Optional quantization settings. Pass `None` for defaults.
    ///
    /// # Returns
    /// The index of the added mesh.
    ///
    /// # Examples
    /// ```ignore
    /// // Using defaults (recommended for most cases)
    /// writer.add_draco_mesh(&mesh, Some("MyMesh"), None)?;
    ///
    /// // Custom quantization
    /// writer.add_draco_mesh(&mesh, Some("HighQuality"), QuantizationBits { position: 16, ..Default::default() })?;
    /// ```
    pub fn add_draco_mesh(
        &mut self,
        mesh: &Mesh,
        name: Option<&str>,
        quantization: impl Into<Option<QuantizationBits>>,
    ) -> Result<usize> {
        let quantization = quantization.into().unwrap_or_default();
        let mesh_idx = self.encode_draco_mesh_internal(mesh, name, &quantization)?;

        // Add a root node for this mesh.
        let node_idx = self.nodes.len();
        self.nodes.push(NodeOut {
            mesh: Some(mesh_idx),
            name: name.map(String::from),
            children: Vec::new(),
            matrix: None,
        });

        // Default behavior: if caller isn't explicitly constructing scenes,
        // keep a single default scene that references every node.
        if self.scenes.is_empty() {
            self.default_scene = Some(0);
            self.scenes.push(SceneOut {
                name: None,
                nodes: Vec::new(),
            });
        }
        if let Some(0) = self.default_scene {
            self.scenes[0].nodes.push(node_idx);
        }

        Ok(mesh_idx)
    }

    /// Write as GLB (binary glTF) file.
    pub fn write_glb<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let glb_data = self.to_glb()?;
        fs::write(path, glb_data)?;
        Ok(())
    }

    /// Write as separate glTF JSON and .bin files.
    pub fn write_gltf<P: AsRef<Path>>(&self, json_path: P, bin_path: P) -> Result<()> {
        let json_path = json_path.as_ref();
        let bin_path = bin_path.as_ref();

        // Write binary buffer
        fs::write(bin_path, &self.binary_data)?;

        // Get relative path for URI
        let bin_uri = bin_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "buffer.bin".to_string());

        // Build glTF JSON
        let root = self.build_gltf_root(Some(&bin_uri));
        let json = serde_json::to_string_pretty(&root)?;
        fs::write(json_path, json)?;

        Ok(())
    }

    /// Write as a single glTF JSON file with embedded base64 data URI.
    ///
    /// This creates a pure text file with no external dependencies.
    /// The binary data is embedded directly in the JSON using base64 encoding.
    ///
    /// # Example
    /// ```ignore
    /// writer.write_gltf_embedded("model.gltf")?;
    /// ```
    pub fn write_gltf_embedded<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let data_uri = Self::encode_data_uri(&self.binary_data);
        let root = self.build_gltf_root(Some(&data_uri));
        let json = serde_json::to_string_pretty(&root)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Convert to glTF JSON string with embedded base64 data.
    pub fn to_gltf_embedded(&self) -> Result<String> {
        let data_uri = Self::encode_data_uri(&self.binary_data);
        let root = self.build_gltf_root(Some(&data_uri));
        let json = serde_json::to_string_pretty(&root)?;
        Ok(json)
    }

    /// Convert to GLB bytes.
    pub fn to_glb(&self) -> Result<Vec<u8>> {
        let root = self.build_gltf_root(None);
        let json = serde_json::to_string(&root)?;
        Ok(build_glb(json.as_bytes(), &self.binary_data))
    }

    fn encode_data_uri(data: &[u8]) -> String {
        const ENCODE_TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

        let mut output = String::from("data:application/octet-stream;base64,");

        for chunk in data.chunks(3) {
            let b1 = chunk[0];
            let b2 = chunk.get(1).copied().unwrap_or(0);
            let b3 = chunk.get(2).copied().unwrap_or(0);

            let n = ((b1 as u32) << 16) | ((b2 as u32) << 8) | (b3 as u32);

            output.push(ENCODE_TABLE[((n >> 18) & 0x3F) as usize] as char);
            output.push(ENCODE_TABLE[((n >> 12) & 0x3F) as usize] as char);

            if chunk.len() > 1 {
                output.push(ENCODE_TABLE[((n >> 6) & 0x3F) as usize] as char);
            } else {
                output.push('=');
            }

            if chunk.len() > 2 {
                output.push(ENCODE_TABLE[(n & 0x3F) as usize] as char);
            } else {
                output.push('=');
            }
        }

        output
    }

    fn build_gltf_root(&self, bin_uri: Option<&str>) -> GltfRoot {
        let mut extensions_used = Vec::new();
        let mut extensions_required = Vec::new();

        if self.has_draco {
            extensions_used.push("KHR_draco_mesh_compression".to_string());
            extensions_required.push("KHR_draco_mesh_compression".to_string());
        }

        let buffers = if self.binary_data.is_empty() {
            Vec::new()
        } else {
            vec![BufferOut {
                byte_length: self.binary_data.len(),
                uri: bin_uri.map(String::from),
            }]
        };

        let scene = if self.scenes.is_empty() {
            if self.nodes.is_empty() {
                None
            } else {
                Some(0)
            }
        } else {
            self.default_scene
        };

        let scenes = if self.scenes.is_empty() {
            if self.nodes.is_empty() {
                Vec::new()
            } else {
                vec![SceneOut {
                    name: None,
                    nodes: (0..self.nodes.len()).collect(),
                }]
            }
        } else {
            self.scenes.clone()
        };

        GltfRoot {
            asset: Asset {
                version: "2.0".to_string(),
                generator: Some("draco-io-rs".to_string()),
            },
            accessors: self.accessors.clone(),
            buffer_views: self.buffer_views.clone(),
            buffers,
            meshes: self.meshes.clone(),
            nodes: self.nodes.clone(),
            scene,
            scenes,
            extensions_used,
            extensions_required,
        }
    }
}

fn component_type_for_data_type(dt: draco_core::draco_types::DataType) -> u32 {
    use draco_core::draco_types::DataType;
    match dt {
        DataType::Int8 => 5120,
        DataType::Uint8 => 5121,
        DataType::Int16 => 5122,
        DataType::Uint16 => 5123,
        DataType::Uint32 => 5125,
        DataType::Float32 => 5126,
        _ => 5126, // Default to float
    }
}

fn gltf_attribute_info(
    attribute_type: GeometryAttributeType,
    num_components: u8,
    attribute_index: usize,
) -> Option<(String, &'static str)> {
    match attribute_type {
        GeometryAttributeType::Position => Some(("POSITION".to_string(), "VEC3")),
        GeometryAttributeType::Normal => Some(("NORMAL".to_string(), "VEC3")),
        GeometryAttributeType::Color => Some(("COLOR_0".to_string(), "VEC4")),
        GeometryAttributeType::TexCoord => Some(("TEXCOORD_0".to_string(), "VEC2")),
        GeometryAttributeType::Generic => Some((
            format!("_GENERIC_{}", attribute_index),
            gltf_type_for_num_components(num_components),
        )),
        GeometryAttributeType::Invalid => None,
    }
}

fn gltf_type_for_num_components(num_components: u8) -> &'static str {
    match num_components {
        1 => "SCALAR",
        2 => "VEC2",
        3 => "VEC3",
        4 => "VEC4",
        _ => "SCALAR",
    }
}

fn build_glb(json_bytes: &[u8], bin_bytes: &[u8]) -> Vec<u8> {
    let json_padding = padding_len(json_bytes.len());
    let bin_padding = padding_len(bin_bytes.len());
    let padded_json_len = json_bytes.len() + json_padding;
    let padded_bin_len = bin_bytes.len() + bin_padding;
    let total_len = 12 + 8 + padded_json_len + 8 + padded_bin_len;

    let mut output = Vec::with_capacity(total_len);
    output.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    output.extend_from_slice(&GLB_VERSION.to_le_bytes());
    output.extend_from_slice(&(total_len as u32).to_le_bytes());

    append_glb_chunk(&mut output, GLB_CHUNK_JSON, json_bytes, b' ');
    append_glb_chunk(&mut output, GLB_CHUNK_BIN, bin_bytes, 0);

    output
}

fn append_glb_chunk(output: &mut Vec<u8>, chunk_type: u32, data: &[u8], padding_byte: u8) {
    let padding = padding_len(data.len());
    let padded_len = data.len() + padding;

    output.extend_from_slice(&(padded_len as u32).to_le_bytes());
    output.extend_from_slice(&chunk_type.to_le_bytes());
    output.extend_from_slice(data);
    output.extend(std::iter::repeat_n(padding_byte, padding));
}

fn padding_len(len: usize) -> usize {
    (4 - (len % 4)) % 4
}

// ============================================================================
// Trait Implementations
// ============================================================================

impl Writer for GltfWriter {
    fn new() -> Self {
        GltfWriter::new()
    }

    fn add_mesh(&mut self, mesh: &Mesh, name: Option<&str>) -> io::Result<()> {
        // Use default quantization
        self.add_draco_mesh(mesh, name, None)
            .map(|_| ())
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn write<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        // Default to GLB format for Writer trait
        self.write_glb(path)
            .map_err(|e| io::Error::other(e.to_string()))
    }

    fn vertex_count(&self) -> usize {
        // Count vertices from all accessors
        self.accessors.iter().map(|a| a.count).sum()
    }

    fn face_count(&self) -> usize {
        // Count faces from meshes
        self.meshes
            .iter()
            .flat_map(|m| &m.primitives)
            .filter_map(|p| p.indices)
            .map(|idx| self.accessors.get(idx).map(|a| a.count / 3).unwrap_or(0))
            .sum()
    }
}

impl crate::traits::SceneWriter for GltfWriter {
    fn add_scene(&mut self, scene: &crate::traits::Scene) -> io::Result<()> {
        // Use default quantization for the trait method.
        self.add_scene(scene, None)
            .map(|_| ())
            .map_err(|e| io::Error::other(e.to_string()))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use draco_core::geometry_attribute::PointAttribute;
    use draco_core::geometry_indices::{FaceIndex, PointIndex};

    fn create_test_triangle() -> Mesh {
        let mut mesh = Mesh::new();
        let mut pos_att = PointAttribute::new();

        pos_att.init(
            GeometryAttributeType::Position,
            3,
            draco_core::draco_types::DataType::Float32,
            false,
            3,
        );

        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0];

        let buffer = pos_att.buffer_mut();
        for i in 0..3 {
            let bytes = [
                positions[i * 3].to_le_bytes(),
                positions[i * 3 + 1].to_le_bytes(),
                positions[i * 3 + 2].to_le_bytes(),
            ]
            .concat();
            buffer.write(i * 12, &bytes);
        }

        mesh.add_attribute(pos_att);
        mesh.set_num_faces(1);
        mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);

        mesh
    }

    #[cfg(feature = "decoder")]
    fn make_translation_transform(x: f32, y: f32, z: f32) -> crate::traits::Transform {
        crate::traits::Transform {
            matrix: [
                [1.0, 0.0, 0.0, x],
                [0.0, 1.0, 0.0, y],
                [0.0, 0.0, 1.0, z],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    #[test]
    fn test_create_glb() {
        let mesh = create_test_triangle();
        let mut writer = GltfWriter::new();

        // Use custom quantization (still works with explicit values)
        let idx = writer
            .add_draco_mesh(
                &mesh,
                Some("Triangle"),
                QuantizationBits {
                    position: 10,
                    normal: 10,
                    color: 8,
                    texcoord: 8,
                    generic: 8,
                },
            )
            .unwrap();
        assert_eq!(idx, 0);

        let glb = writer.to_glb().unwrap();

        // Check GLB header
        assert_eq!(&glb[0..4], b"glTF");
        assert!(glb.len() > 12);
    }

    #[cfg(feature = "decoder")]
    #[test]
    fn test_roundtrip() {
        use crate::gltf_reader::GltfReader;

        let mesh = create_test_triangle();
        let mut writer = GltfWriter::new();
        // Use default quantization with None
        writer
            .add_draco_mesh(&mesh, Some("Triangle"), None)
            .unwrap();

        let glb = writer.to_glb().unwrap();

        // Read back
        let reader = GltfReader::from_glb(&glb).unwrap();
        assert!(reader.has_draco_extension());
        assert_eq!(reader.num_meshes(), 1);

        let primitives = reader.draco_primitives();
        assert_eq!(primitives.len(), 1);

        let decoded = reader.decode_draco_mesh(&primitives[0]).unwrap();
        assert_eq!(decoded.num_faces(), 1);
        assert_eq!(decoded.num_points(), 3);
    }

    #[test]
    fn test_gltf_writer_uses_default_mesh_encoding_method_selection() {
        let encoded = encode_draco_mesh(&create_test_triangle(), None).unwrap();

        assert!(encoded.len() > 8, "encoded Draco buffer is too small");
        assert_eq!(
            encoded[8], 1,
            "default mesh encoding method should match C++ ExpertEncoder selection"
        );
    }

    #[cfg(feature = "decoder")]
    #[test]
    fn test_scene_graph_roundtrip() {
        use crate::gltf_reader::GltfReader;
        use crate::traits::{Scene, SceneNode, SceneObject, SceneReader};

        let mesh = create_test_triangle();

        // Build a small hierarchy: Root -> Child
        let mut root = SceneNode::new(Some("Root".to_string()));
        root.transform = Some(make_translation_transform(1.0, 2.0, 3.0));

        let mut child = SceneNode::new(Some("Child".to_string()));
        child.transform = Some(make_translation_transform(4.0, 5.0, 6.0));
        child.parts.push(SceneObject {
            name: Some("Triangle".to_string()),
            mesh: mesh.clone(),
            transform: None,
        });
        root.children.push(child);

        let scene = Scene {
            name: Some("TestScene".to_string()),
            parts: Vec::new(),
            root_nodes: vec![root],
        };

        let mut writer = GltfWriter::new();
        writer.add_scene(&scene, None).unwrap();

        let glb = writer.to_glb().unwrap();
        let mut reader = GltfReader::from_glb(&glb).unwrap();

        let out_scene = reader.read_scene().unwrap();
        assert_eq!(out_scene.name, Some("TestScene".to_string()));
        assert_eq!(out_scene.root_nodes.len(), 1);
        assert_eq!(out_scene.root_nodes[0].name, Some("Root".to_string()));
        assert_eq!(out_scene.root_nodes[0].children.len(), 1);
        assert_eq!(
            out_scene.root_nodes[0].children[0].name,
            Some("Child".to_string())
        );
        assert_eq!(out_scene.root_nodes[0].children[0].parts.len(), 1);

        // Verify transforms survived matrix column/row conversion.
        let root_m = out_scene.root_nodes[0].transform.as_ref().unwrap().matrix;
        assert_eq!(root_m[0][3], 1.0);
        assert_eq!(root_m[1][3], 2.0);
        assert_eq!(root_m[2][3], 3.0);

        let child_m = out_scene.root_nodes[0].children[0]
            .transform
            .as_ref()
            .unwrap()
            .matrix;
        assert_eq!(child_m[0][3], 4.0);
        assert_eq!(child_m[1][3], 5.0);
        assert_eq!(child_m[2][3], 6.0);
    }

    #[cfg(feature = "decoder")]
    #[test]
    fn test_embedded_gltf() {
        use crate::gltf_reader::GltfReader;

        let mesh = create_test_triangle();
        let mut writer = GltfWriter::new();
        // Use default quantization with None
        writer
            .add_draco_mesh(&mesh, Some("Triangle"), None)
            .unwrap();

        // Generate embedded glTF JSON
        let json = writer.to_gltf_embedded().unwrap();

        // Verify it contains data URI
        assert!(json.contains("data:application/octet-stream;base64,"));
        assert!(json.contains("KHR_draco_mesh_compression"));

        // Read back
        let reader = GltfReader::from_gltf(json.as_bytes(), None).unwrap();
        assert!(reader.has_draco_extension());
        assert_eq!(reader.num_meshes(), 1);

        let primitives = reader.draco_primitives();
        assert_eq!(primitives.len(), 1);

        let decoded = reader.decode_draco_mesh(&primitives[0]).unwrap();
        assert_eq!(decoded.num_faces(), 1);
        assert_eq!(decoded.num_points(), 3);
    }

    #[test]
    fn test_base64_encoding() {
        // Test base64 encoding
        let data = b"Hello";
        let encoded = GltfWriter::encode_data_uri(data);
        assert!(encoded.starts_with("data:application/octet-stream;base64,"));
        assert!(encoded.contains("SGVsbG8="));

        let data = b"Hello World";
        let encoded = GltfWriter::encode_data_uri(data);
        assert!(encoded.contains("SGVsbG8gV29ybGQ="));
    }
}
