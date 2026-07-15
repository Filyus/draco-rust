//! glTF/GLB writer with Draco mesh compression support.
//!
//! This module provides support for writing glTF 2.0 files with the
//! `KHR_draco_mesh_compression` extension. Multiple output formats are supported:
//!
//! - **GLB** - Binary container (single .glb file)
//! - **glTF + .bin** - JSON + separate binary file
//! - **glTF (embedded)** - Single JSON file with base64-encoded data URIs
//!
//! # Example - GLB
//!
//! ```no_run
//! use draco_io::gltf_writer::GltfWriter;
//!
//! let mesh = draco_core::mesh::Mesh::new();
//! let mut writer = GltfWriter::new();
//! writer.add_draco_mesh(&mesh, Some("MyMesh"), None)?;  // Uses default quantization
//! writer.write_glb("output.glb")?;
//! # Ok::<(), draco_io::GltfWriteError>(())
//! ```
//!
//! # Example - Pure Text glTF (Embedded)
//!
//! ```no_run
//! # let writer = draco_io::gltf_writer::GltfWriter::new();
//! writer.write_gltf_embedded("output.gltf")?;
//! // Creates a single text file with base64-embedded binary data
//! # Ok::<(), draco_io::GltfWriteError>(())
//! ```
//!
//! # Example - Writing a Scene Graph
//!
//! ```no_run
//! use draco_io::gltf_writer::GltfWriter;
//! use draco_io::{MeshInstance, Scene, SceneNode};
//! # let mesh = draco_core::mesh::Mesh::new();
//!
//! let mut root = SceneNode::new(Some("Root".to_string()));
//! root.mesh_instances.push(MeshInstance {
//!     name: Some("Mesh".to_string()),
//!     mesh,
//!     transform: None,
//! });
//! let scene = Scene { name: Some("Scene".to_string()), root_nodes: vec![root] };
//!
//! let mut writer = GltfWriter::new();
//! writer.add_scene(&scene, None)?;
//! writer.write_glb("scene.glb")?;
//! # Ok::<(), draco_io::GltfWriteError>(())
//! ```

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use draco_core::draco_types::DataType;
use draco_core::encoder_buffer::EncoderBuffer;
use draco_core::encoder_options::EncoderOptions;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::mesh::Mesh;
use draco_core::mesh_encoder::{EncodedAttributeInfo, EncodedMeshInfo, MeshEncoder};
use serde::Serialize;
use thiserror::Error;

use crate::gltf_compress::{EncodingMethod, GltfCompressionOptions};
use crate::gltf_container::{serialize_gltf_document, GltfContainerFormat, OutputFormat};
use crate::traits::{WriteToBytes, Writer};

/// Errors that can occur when writing glTF files.
#[derive(Error, Debug)]
pub enum GltfWriteError {
    /// Filesystem or stream I/O failed.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    /// glTF JSON serialization failed.
    #[error("JSON serialize error: {0}")]
    Json(#[from] serde_json::Error),

    /// Draco encoding failed.
    #[error("Draco encode error: {0}")]
    DracoEncode(#[source] draco_core::DracoError),

    /// The encoder violated its documented postcondition.
    #[error("Draco encoder invariant failed: {0}")]
    EncoderInvariant(String),

    /// Mesh data cannot be represented as supported glTF Draco geometry.
    #[error("Invalid mesh: {0}")]
    InvalidMesh(String),

    /// The mesh or scene uses a feature outside this writer's supported scope.
    #[error("Unsupported feature: {0}")]
    Unsupported(String),

    /// Compression options are outside their supported range.
    #[error("Invalid compression options: {0}")]
    InvalidOptions(String),

    /// A checked size computation or allocation failed.
    #[error("Resource limit exceeded: {0}")]
    ResourceLimit(String),

    /// Generated JSON was unexpectedly not UTF-8.
    #[error("UTF-8 conversion error: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

/// Result type used by glTF writers.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    generator: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccessorOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    buffer_view: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    byte_offset: Option<usize>,
    component_type: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    normalized: Option<bool>,
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

pub(crate) fn encode_draco_mesh_with_info(
    mesh: &Mesh,
    compression: &GltfCompressionOptions,
) -> Result<(Vec<u8>, EncodedMeshInfo)> {
    compression
        .validate()
        .map_err(|error| GltfWriteError::InvalidOptions(error.to_string()))?;
    validate_mesh_for_gltf_draco(mesh)?;

    if mesh.num_faces() == 0 {
        return Err(GltfWriteError::InvalidMesh("Mesh has no faces".into()));
    }

    let mut encoder = MeshEncoder::new();
    encoder.set_mesh(mesh.clone());

    let mut options = EncoderOptions::new();
    options.set_global_int("encoding_speed", compression.encoding_speed as i32);
    options.set_global_int("decoding_speed", compression.decoding_speed as i32);
    match compression.encoding_method {
        EncodingMethod::Auto => {}
        EncodingMethod::Sequential => options.set_encoding_method(0),
        EncodingMethod::Edgebreaker => options.set_encoding_method(1),
    }

    // `None` deliberately leaves a floating-point attribute unquantized.
    for i in 0..mesh.num_attributes() {
        let att = mesh.attribute(i);
        if att.data_type() == draco_core::draco_types::DataType::Float32 {
            let bits = match att.attribute_type() {
                GeometryAttributeType::Position => compression.quantization.position,
                GeometryAttributeType::Normal => compression.quantization.normal,
                GeometryAttributeType::Color => compression.quantization.color,
                GeometryAttributeType::TexCoord => compression.quantization.texcoord,
                GeometryAttributeType::Generic | GeometryAttributeType::Invalid => {
                    compression.quantization.generic
                }
            };
            if let Some(bits) = bits {
                options.set_attribute_int(i, "quantization_bits", bits as i32);
            }
        }
    }

    let mut enc_buffer = EncoderBuffer::new();
    encoder
        .encode(&options, &mut enc_buffer)
        .map_err(GltfWriteError::DracoEncode)?;
    let encoded_info = encoder.encoded_mesh_info().cloned().ok_or_else(|| {
        GltfWriteError::EncoderInvariant("encoder did not return mesh info".into())
    })?;

    let encoded = enc_buffer.data();
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(encoded.len())
        .map_err(|_| GltfWriteError::ResourceLimit("Draco output allocation failed".into()))?;
    bytes.extend_from_slice(encoded);
    Ok((bytes, encoded_info))
}

/// Encode a mesh to a Draco bitstream using the same settings as `GltfWriter`.
///
/// This is useful for tools/tests that need the raw `.drc` bytes without
/// wrapping them into a glTF/GLB container.
pub fn encode_draco_mesh(
    mesh: &Mesh,
    options: impl Into<Option<GltfCompressionOptions>>,
) -> Result<Vec<u8>> {
    let options = options.into().unwrap_or_default();
    encode_draco_mesh_with_info(mesh, &options).map(|(bytes, _)| bytes)
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
        scene: &crate::scene::Scene,
        options: impl Into<Option<GltfCompressionOptions>>,
    ) -> Result<usize> {
        let options = options.into().unwrap_or_default();

        // Build nodes recursively and record root node indices.
        let mut root_node_indices = Vec::new();
        root_node_indices
            .try_reserve_exact(scene.root_nodes.len())
            .map_err(|_| {
                GltfWriteError::ResourceLimit("scene root table allocation failed".into())
            })?;
        for root in &scene.root_nodes {
            let node_idx = self.push_scene_node(root, &options)?;
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

    fn transform_to_gltf_matrix(transform: &crate::scene::Transform) -> [f32; 16] {
        // Input is row-major; glTF expects column-major.
        let m = &transform.matrix;
        [
            m[0][0], m[1][0], m[2][0], m[3][0], m[0][1], m[1][1], m[2][1], m[3][1], m[0][2],
            m[1][2], m[2][2], m[3][2], m[0][3], m[1][3], m[2][3], m[3][3],
        ]
    }

    fn push_scene_node(
        &mut self,
        node: &crate::scene::SceneNode,
        options: &GltfCompressionOptions,
    ) -> Result<usize> {
        // glTF nodes can reference at most one mesh; if multiple mesh instances
        // exist, we create child nodes for each instance.

        // First, create this node (without children for now).
        let node_idx = self.nodes.len();
        self.nodes.push(NodeOut {
            mesh: None,
            name: node.name.clone(),
            children: Vec::new(),
            matrix: node.transform.as_ref().map(Self::transform_to_gltf_matrix),
        });

        // Attach mesh instances.
        if node.mesh_instances.len() == 1 && node.mesh_instances[0].transform.is_none() {
            let mesh_instance = &node.mesh_instances[0];
            let mesh_idx = self.encode_draco_mesh_internal(
                &mesh_instance.mesh,
                mesh_instance.name.as_deref(),
                options,
            )?;
            self.nodes[node_idx].mesh = Some(mesh_idx);
        } else if !node.mesh_instances.is_empty() {
            for (i, mesh_instance) in node.mesh_instances.iter().enumerate() {
                let mesh_idx = self.encode_draco_mesh_internal(
                    &mesh_instance.mesh,
                    mesh_instance.name.as_deref(),
                    options,
                )?;
                let child_idx = self.nodes.len();
                self.nodes.push(NodeOut {
                    mesh: Some(mesh_idx),
                    name: mesh_instance.name.clone().or_else(|| {
                        node.name
                            .as_ref()
                            .map(|n| format!("{}_mesh_instance{}", n, i))
                    }),
                    children: Vec::new(),
                    matrix: mesh_instance
                        .transform
                        .as_ref()
                        .map(Self::transform_to_gltf_matrix),
                });
                self.nodes[node_idx].children.push(child_idx);
            }
        }

        // Recurse into children.
        for child in &node.children {
            let child_idx = self.push_scene_node(child, options)?;
            self.nodes[node_idx].children.push(child_idx);
        }

        Ok(node_idx)
    }

    fn encode_draco_mesh_internal(
        &mut self,
        mesh: &Mesh,
        name: Option<&str>,
        options: &GltfCompressionOptions,
    ) -> Result<usize> {
        let (draco_data, encoded_info) = encode_draco_mesh_with_info(mesh, options)?;
        let draco_buffer_view_idx = self.append_buffer_view(&draco_data)?;
        let primitive = self.build_draco_primitive(&encoded_info, draco_buffer_view_idx)?;

        let mesh_idx = self.meshes.len();
        self.meshes.push(MeshOut {
            name: name.map(String::from),
            primitives: vec![primitive],
        });

        self.has_draco = true;
        Ok(mesh_idx)
    }

    fn append_buffer_view(&mut self, data: &[u8]) -> Result<usize> {
        let padding = (4 - self.binary_data.len() % 4) % 4;
        let additional = padding
            .checked_add(data.len())
            .ok_or_else(|| GltfWriteError::ResourceLimit("binary buffer size overflow".into()))?;
        let aligned_len =
            self.binary_data.len().checked_add(padding).ok_or_else(|| {
                GltfWriteError::ResourceLimit("binary buffer size overflow".into())
            })?;
        self.binary_data
            .try_reserve_exact(additional)
            .map_err(|_| GltfWriteError::ResourceLimit("binary buffer allocation failed".into()))?;
        self.binary_data.resize(aligned_len, 0);
        let aligned_offset = self.binary_data.len();

        self.binary_data.extend_from_slice(data);
        let buffer_view_idx = self.buffer_views.len();
        self.buffer_views.push(BufferViewOut {
            buffer: 0,
            byte_offset: Some(aligned_offset),
            byte_length: data.len(),
        });

        Ok(buffer_view_idx)
    }

    fn build_draco_primitive(
        &mut self,
        encoded_info: &EncodedMeshInfo,
        draco_buffer_view_idx: usize,
    ) -> Result<PrimitiveOut> {
        let (attributes, draco_attributes) = self.add_mesh_attribute_accessors(encoded_info)?;
        let index_count = encoded_info
            .num_encoded_faces
            .checked_mul(3)
            .ok_or_else(|| GltfWriteError::ResourceLimit("index count overflow".into()))?;
        let indices_accessor_idx = self.add_indices_accessor(index_count);

        Ok(PrimitiveOut {
            attributes,
            indices: Some(indices_accessor_idx),
            mode: Some(4), // TRIANGLES
            extensions: Some(PrimitiveExtensionsOut {
                khr_draco_mesh_compression: DracoExtensionOut {
                    buffer_view: draco_buffer_view_idx,
                    attributes: draco_attributes,
                },
            }),
        })
    }

    fn add_mesh_attribute_accessors(
        &mut self,
        encoded_info: &EncodedMeshInfo,
    ) -> Result<(HashMap<String, usize>, HashMap<String, usize>)> {
        let mut attributes = HashMap::new();
        let mut draco_attributes: HashMap<String, usize> = HashMap::new();
        let mut counters = GltfSemanticCounters::default();

        for att in &encoded_info.attributes {
            let (semantic, accessor_type) =
                gltf_attribute_info(att.attribute_type, att.num_components, &mut counters)?;

            let accessor_idx =
                self.add_attribute_accessor(att, accessor_type, encoded_info.num_encoded_points)?;
            attributes.insert(semantic.clone(), accessor_idx);
            draco_attributes.insert(semantic, att.unique_id as usize);
        }

        Ok((attributes, draco_attributes))
    }

    fn add_attribute_accessor(
        &mut self,
        att: &EncodedAttributeInfo,
        accessor_type: &str,
        count: usize,
    ) -> Result<usize> {
        let accessor_idx = self.accessors.len();
        let (min, max) = if att.attribute_type == GeometryAttributeType::Position {
            (
                att.position_min.clone().ok_or_else(|| {
                    GltfWriteError::InvalidMesh("POSITION accessor is missing min bounds".into())
                })?,
                att.position_max.clone().ok_or_else(|| {
                    GltfWriteError::InvalidMesh("POSITION accessor is missing max bounds".into())
                })?,
            )
        } else {
            (Vec::new(), Vec::new())
        };
        self.accessors.push(AccessorOut {
            buffer_view: None,
            byte_offset: None,
            component_type: component_type_for_data_type(att.data_type)?,
            normalized: att.normalized.then_some(true),
            count,
            accessor_type: accessor_type.to_string(),
            min,
            max,
        });
        Ok(accessor_idx)
    }

    fn add_indices_accessor(&mut self, count: usize) -> usize {
        let accessor_idx = self.accessors.len();
        self.accessors.push(AccessorOut {
            buffer_view: None,
            byte_offset: None,
            component_type: 5125, // UNSIGNED_INT
            normalized: None,
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
    /// * `options` - Optional compression settings. Pass `None` for defaults.
    ///
    /// # Returns
    /// The index of the added mesh.
    ///
    /// # Examples
    /// ```no_run
    /// use draco_io::{GltfCompressionOptions, GltfWriter, QuantizationOptions};
    /// # let mesh = draco_core::mesh::Mesh::new();
    ///
    /// let mut writer = GltfWriter::new();
    ///
    /// // Using defaults (recommended for most cases)
    /// writer.add_draco_mesh(&mesh, Some("MyMesh"), None)?;
    ///
    /// // Custom quantization
    /// writer.add_draco_mesh(&mesh, Some("HighQuality"), GltfCompressionOptions {
    ///     quantization: QuantizationOptions { position: Some(16), ..Default::default() },
    ///     ..Default::default()
    /// })?;
    /// # Ok::<(), draco_io::GltfWriteError>(())
    /// ```
    pub fn add_draco_mesh(
        &mut self,
        mesh: &Mesh,
        name: Option<&str>,
        options: impl Into<Option<GltfCompressionOptions>>,
    ) -> Result<usize> {
        let options = options.into().unwrap_or_default();
        let mesh_idx = self.encode_draco_mesh_internal(mesh, name, &options)?;

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
    /// ```no_run
    /// # let writer = draco_io::gltf_writer::GltfWriter::new();
    /// writer.write_gltf_embedded("model.gltf")?;
    /// # Ok::<(), draco_io::GltfWriteError>(())
    /// ```
    pub fn write_gltf_embedded<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        fs::write(path, self.to_gltf_embedded()?)?;
        Ok(())
    }

    /// Convert to glTF JSON string with embedded base64 data.
    pub fn to_gltf_embedded(&self) -> Result<String> {
        let root = serde_json::to_value(self.build_gltf_root(None))?;
        let bytes = serialize_gltf_document(
            &root,
            &self.binary_data,
            GltfContainerFormat::Gltf,
            OutputFormat::GltfEmbeddedBuffers,
        )
        .map_err(|error| GltfWriteError::InvalidMesh(error.to_string()))?;
        Ok(String::from_utf8(bytes)?)
    }

    /// Convert to GLB bytes.
    pub fn to_glb(&self) -> Result<Vec<u8>> {
        let root = serde_json::to_value(self.build_gltf_root(None))?;
        serialize_gltf_document(
            &root,
            &self.binary_data,
            GltfContainerFormat::Glb,
            OutputFormat::Glb,
        )
        .map_err(|error| GltfWriteError::InvalidMesh(error.to_string()))
    }

    /// Write the default GLB output into a byte vector.
    pub fn write_to_vec(&self) -> Result<Vec<u8>> {
        self.to_glb()
    }

    /// Write the default GLB output into a byte sink.
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        writer.write_all(&self.write_to_vec()?)?;
        Ok(())
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

fn validate_mesh_for_gltf_draco(mesh: &Mesh) -> Result<()> {
    let mut position_count = 0usize;
    if mesh.num_faces() == 0 {
        return Err(GltfWriteError::InvalidMesh("Mesh has no faces".into()));
    }

    for face_id in 0..mesh.num_faces() {
        let face_id_u32 = u32::try_from(face_id)
            .map_err(|_| GltfWriteError::InvalidMesh("mesh has more than u32::MAX faces".into()))?;
        let face = mesh.face(draco_core::geometry_indices::FaceIndex(face_id_u32));
        for point in face {
            if point.0 as usize >= mesh.num_points() {
                return Err(GltfWriteError::InvalidMesh(format!(
                    "Face {} references point {} but mesh has {} points",
                    face_id,
                    point.0,
                    mesh.num_points()
                )));
            }
        }
    }

    for i in 0..mesh.num_attributes() {
        let att = mesh.attribute(i);
        validate_attribute_for_gltf(att)?;
        if att.attribute_type() == GeometryAttributeType::Position {
            position_count += 1;
        }
    }

    if position_count == 0 {
        return Err(GltfWriteError::InvalidMesh(
            "glTF Draco mesh requires a POSITION attribute".into(),
        ));
    }
    if position_count > 1 {
        return Err(GltfWriteError::Unsupported(
            "glTF supports only one POSITION attribute".into(),
        ));
    }

    Ok(())
}

fn validate_attribute_for_gltf(att: &PointAttribute) -> Result<()> {
    if att.size() == 0 {
        return Err(GltfWriteError::InvalidMesh(format!(
            "Attribute {:?} has no values",
            att.attribute_type()
        )));
    }

    match att.attribute_type() {
        GeometryAttributeType::Position => {
            if att.num_components() != 3 || att.data_type() != DataType::Float32 {
                return Err(GltfWriteError::Unsupported(
                    "POSITION must be VEC3 FLOAT".into(),
                ));
            }
            if att.normalized() {
                return Err(GltfWriteError::Unsupported(
                    "POSITION must not be normalized".into(),
                ));
            }
        }
        GeometryAttributeType::Normal => {
            if att.num_components() != 3 || att.data_type() != DataType::Float32 {
                return Err(GltfWriteError::Unsupported(
                    "NORMAL must be VEC3 FLOAT".into(),
                ));
            }
            if att.normalized() {
                return Err(GltfWriteError::Unsupported(
                    "NORMAL must not be normalized".into(),
                ));
            }
        }
        GeometryAttributeType::Color => {
            if !(att.num_components() == 3 || att.num_components() == 4) {
                return Err(GltfWriteError::Unsupported(
                    "COLOR attributes must be VEC3 or VEC4".into(),
                ));
            }
            validate_normalized_integer_or_float(att, "COLOR")?;
        }
        GeometryAttributeType::TexCoord => {
            if att.num_components() != 2 {
                return Err(GltfWriteError::Unsupported(
                    "TEXCOORD attributes must be VEC2".into(),
                ));
            }
            validate_normalized_integer_or_float(att, "TEXCOORD")?;
        }
        GeometryAttributeType::Generic => {
            if !(1..=4).contains(&att.num_components()) {
                return Err(GltfWriteError::Unsupported(
                    "Generic attributes must have 1..=4 components".into(),
                ));
            }
            component_type_for_data_type(att.data_type())?;
            if att.data_type() == DataType::Uint32 {
                return Err(GltfWriteError::Unsupported(
                    "UNSIGNED_INT is only valid for glTF indices, not vertex attributes".into(),
                ));
            }
        }
        GeometryAttributeType::Invalid => {
            return Err(GltfWriteError::Unsupported(
                "Invalid Draco attribute type cannot be written to glTF".into(),
            ));
        }
    }

    Ok(())
}

fn validate_normalized_integer_or_float(att: &PointAttribute, semantic: &str) -> Result<()> {
    match att.data_type() {
        DataType::Float32 => Ok(()),
        DataType::Uint8 | DataType::Uint16 => {
            if att.normalized() {
                Ok(())
            } else {
                Err(GltfWriteError::Unsupported(format!(
                    "{} integer attributes must be normalized",
                    semantic
                )))
            }
        }
        other => Err(GltfWriteError::Unsupported(format!(
            "{} does not support component data type {:?}",
            semantic, other
        ))),
    }
}

fn component_type_for_data_type(dt: DataType) -> Result<u32> {
    match dt {
        DataType::Int8 => Ok(5120),
        DataType::Uint8 => Ok(5121),
        DataType::Int16 => Ok(5122),
        DataType::Uint16 => Ok(5123),
        DataType::Uint32 => Ok(5125),
        DataType::Float32 => Ok(5126),
        _ => Err(GltfWriteError::Unsupported(format!(
            "Unsupported glTF component data type: {:?}",
            dt
        ))),
    }
}

#[derive(Debug, Default)]
struct GltfSemanticCounters {
    color: usize,
    texcoord: usize,
    generic: usize,
}

fn gltf_attribute_info(
    attribute_type: GeometryAttributeType,
    num_components: u8,
    counters: &mut GltfSemanticCounters,
) -> Result<(String, &'static str)> {
    match attribute_type {
        GeometryAttributeType::Position => Ok(("POSITION".to_string(), "VEC3")),
        GeometryAttributeType::Normal => Ok(("NORMAL".to_string(), "VEC3")),
        GeometryAttributeType::Color => {
            let semantic = format!("COLOR_{}", counters.color);
            counters.color += 1;
            Ok((semantic, gltf_type_for_num_components(num_components)?))
        }
        GeometryAttributeType::TexCoord => {
            let semantic = format!("TEXCOORD_{}", counters.texcoord);
            counters.texcoord += 1;
            Ok((semantic, "VEC2"))
        }
        GeometryAttributeType::Generic => {
            let semantic = format!("_GENERIC_{}", counters.generic);
            counters.generic += 1;
            Ok((semantic, gltf_type_for_num_components(num_components)?))
        }
        GeometryAttributeType::Invalid => Err(GltfWriteError::Unsupported(
            "Invalid Draco attribute type cannot be written to glTF".into(),
        )),
    }
}

fn gltf_type_for_num_components(num_components: u8) -> Result<&'static str> {
    match num_components {
        1 => Ok("SCALAR"),
        2 => Ok("VEC2"),
        3 => Ok("VEC3"),
        4 => Ok("VEC4"),
        _ => Err(GltfWriteError::Unsupported(format!(
            "Unsupported glTF accessor component count: {}",
            num_components
        ))),
    }
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

impl WriteToBytes for GltfWriter {
    fn write_to_vec(&self) -> io::Result<Vec<u8>> {
        GltfWriter::write_to_vec(self).map_err(|e| io::Error::other(e.to_string()))
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        GltfWriter::write_to(self, writer).map_err(|e| io::Error::other(e.to_string()))
    }
}

impl crate::scene::SceneWriter for GltfWriter {
    fn add_scene(&mut self, scene: &crate::scene::Scene) -> io::Result<()> {
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
    use draco_core::draco_types::DataType;
    use draco_core::geometry_attribute::PointAttribute;
    use draco_core::geometry_indices::{AttributeValueIndex, FaceIndex, PointIndex};

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

    fn add_attribute(
        mesh: &mut Mesh,
        attribute_type: GeometryAttributeType,
        components: u8,
        data_type: DataType,
        normalized: bool,
        bytes: Vec<u8>,
    ) {
        let mut attribute = PointAttribute::new();
        attribute.init(
            attribute_type,
            components,
            data_type,
            normalized,
            bytes.len() / (components as usize * data_type.byte_length()),
        );
        attribute.buffer_mut().write(0, &bytes);
        mesh.add_attribute(attribute);
    }

    fn repeated_zero_attribute_bytes(data_type: DataType, components: u8, count: usize) -> Vec<u8> {
        vec![0; data_type.byte_length() * components as usize * count]
    }

    fn write_f32s(attribute: &mut PointAttribute, values: &[f32]) {
        for (i, value) in values.iter().enumerate() {
            attribute
                .buffer_mut()
                .write(i * DataType::Float32.byte_length(), &value.to_le_bytes());
        }
    }

    fn create_test_uv_seam_mesh() -> Mesh {
        let mut mesh = Mesh::new();
        mesh.set_num_points(6);

        let mut positions = PointAttribute::new();
        positions.init(
            GeometryAttributeType::Position,
            3,
            DataType::Float32,
            false,
            4,
        );
        write_f32s(
            &mut positions,
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0],
        );
        positions.set_explicit_mapping(6);
        for (point, entry) in [0, 1, 2, 1, 3, 2].iter().copied().enumerate() {
            positions.set_point_map_entry(PointIndex(point as u32), AttributeValueIndex(entry));
        }
        mesh.add_attribute(positions);

        add_attribute(
            &mut mesh,
            GeometryAttributeType::TexCoord,
            2,
            DataType::Float32,
            false,
            [
                0.0f32, 0.0, 1.0, 0.0, 0.0, 1.0, 0.2, 0.0, 1.0, 1.0, 0.2, 1.0,
            ]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect(),
        );

        mesh.add_face([PointIndex(0), PointIndex(1), PointIndex(2)]);
        mesh.add_face([PointIndex(3), PointIndex(4), PointIndex(5)]);
        mesh
    }

    #[cfg(feature = "gltf-reader")]
    fn make_translation_transform(x: f32, y: f32, z: f32) -> crate::scene::Transform {
        crate::scene::Transform {
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
                GltfCompressionOptions {
                    quantization: crate::QuantizationOptions {
                        position: Some(10),
                        normal: Some(10),
                        color: Some(8),
                        texcoord: Some(8),
                        generic: Some(8),
                    },
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(idx, 0);

        let glb = writer.to_glb().unwrap();

        // Check GLB header
        assert_eq!(&glb[0..4], b"glTF");
        assert!(glb.len() > 12);
    }

    #[cfg(feature = "gltf-reader")]
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

    #[test]
    fn compression_method_and_ranges_are_honored() {
        let mesh = create_test_triangle();
        let sequential = GltfCompressionOptions {
            encoding_method: EncodingMethod::Sequential,
            encoding_speed: 0,
            decoding_speed: 10,
            ..Default::default()
        };
        let encoded = encode_draco_mesh(&mesh, sequential).unwrap();
        assert_eq!(encoded[8], 0, "sequential method must reach the encoder");

        let edgebreaker = GltfCompressionOptions {
            encoding_method: EncodingMethod::Edgebreaker,
            ..Default::default()
        };
        let encoded = encode_draco_mesh(&mesh, edgebreaker).unwrap();
        assert_eq!(encoded[8], 1, "EdgeBreaker method must reach the encoder");

        let invalid = GltfCompressionOptions {
            encoding_speed: 11,
            ..Default::default()
        };
        assert!(matches!(
            encode_draco_mesh(&mesh, invalid),
            Err(GltfWriteError::InvalidOptions(_))
        ));
    }

    #[cfg(feature = "gltf-reader")]
    #[test]
    fn test_scene_graph_roundtrip() {
        use crate::gltf_reader::GltfReader;
        use crate::scene::{MeshInstance, Scene, SceneNode, SceneReader};

        let mesh = create_test_triangle();

        // Build a small hierarchy: Root -> Child
        let mut root = SceneNode::new(Some("Root".to_string()));
        root.transform = Some(make_translation_transform(1.0, 2.0, 3.0));

        let mut child = SceneNode::new(Some("Child".to_string()));
        child.transform = Some(make_translation_transform(4.0, 5.0, 6.0));
        child.mesh_instances.push(MeshInstance {
            name: Some("Triangle".to_string()),
            mesh: mesh.clone(),
            transform: None,
        });
        root.children.push(child);

        let scene = Scene {
            name: Some("TestScene".to_string()),
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
        assert_eq!(out_scene.root_nodes[0].children[0].mesh_instances.len(), 1);

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

    #[cfg(feature = "gltf-reader")]
    #[test]
    fn test_flat_scene_mesh_instances_roundtrip() {
        use crate::gltf_reader::GltfReader;
        use crate::scene::{MeshInstance, Scene, SceneReader};

        let mesh = create_test_triangle();
        let scene = Scene::from_mesh_instances(
            Some("FlatScene".to_string()),
            vec![
                MeshInstance {
                    name: Some("FlatA".to_string()),
                    mesh: mesh.clone(),
                    transform: Some(make_translation_transform(1.0, 2.0, 3.0)),
                },
                MeshInstance {
                    name: Some("FlatB".to_string()),
                    mesh,
                    transform: Some(make_translation_transform(4.0, 5.0, 6.0)),
                },
            ],
        );

        let mut writer = GltfWriter::new();
        writer.add_scene(&scene, None).unwrap();

        let glb = writer.to_glb().unwrap();
        let mut reader = GltfReader::from_glb(&glb).unwrap();
        let out_scene = reader.read_scene().unwrap();

        assert_eq!(out_scene.name, Some("FlatScene".to_string()));
        assert_eq!(out_scene.root_nodes.len(), 1);
        assert_eq!(out_scene.root_nodes[0].name, Some("FlatScene".to_string()));
        assert_eq!(out_scene.root_nodes[0].children.len(), 2);
        assert_eq!(
            out_scene.root_nodes[0].children[0].mesh_instances[0].name,
            Some("FlatA".to_string())
        );
        assert_eq!(
            out_scene.root_nodes[0].children[1].mesh_instances[0].name,
            Some("FlatB".to_string())
        );

        let first_m = out_scene.root_nodes[0].children[0]
            .transform
            .as_ref()
            .unwrap()
            .matrix;
        assert_eq!(first_m[0][3], 1.0);
        assert_eq!(first_m[1][3], 2.0);
        assert_eq!(first_m[2][3], 3.0);

        let second_m = out_scene.root_nodes[0].children[1]
            .transform
            .as_ref()
            .unwrap()
            .matrix;
        assert_eq!(second_m[0][3], 4.0);
        assert_eq!(second_m[1][3], 5.0);
        assert_eq!(second_m[2][3], 6.0);
    }

    #[cfg(feature = "gltf-reader")]
    #[test]
    fn test_scene_writer_trait_exports_flat_scene_mesh_instances() {
        use crate::gltf_reader::GltfReader;
        use crate::scene::{MeshInstance, Scene, SceneReader, SceneWriter};

        let scene = Scene::from_mesh_instances(
            Some("TraitScene".to_string()),
            vec![MeshInstance {
                name: Some("TraitMeshInstance".to_string()),
                mesh: create_test_triangle(),
                transform: None,
            }],
        );

        let mut writer = GltfWriter::new();
        SceneWriter::add_scene(&mut writer, &scene).unwrap();

        let glb = writer.to_glb().unwrap();
        let mut reader = GltfReader::from_glb(&glb).unwrap();
        let out_scene = reader.read_scene().unwrap();

        assert_eq!(out_scene.name, Some("TraitScene".to_string()));
        assert_eq!(out_scene.root_nodes.len(), 1);
        assert_eq!(out_scene.root_nodes[0].mesh_instances.len(), 1);
        assert_eq!(
            out_scene.root_nodes[0].mesh_instances[0].name,
            Some("TraitMeshInstance".to_string())
        );
    }

    #[cfg(feature = "gltf-reader")]
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
        let encoded = crate::encode_data_uri("application/octet-stream", data).unwrap();
        assert!(encoded.starts_with("data:application/octet-stream;base64,"));
        assert!(encoded.contains("SGVsbG8="));

        let data = b"Hello World";
        let encoded = crate::encode_data_uri("application/octet-stream", data).unwrap();
        assert!(encoded.contains("SGVsbG8gV29ybGQ="));
    }

    #[test]
    fn test_writer_emits_position_bounds_and_normalized_metadata() {
        let mut mesh = create_test_triangle();
        add_attribute(
            &mut mesh,
            GeometryAttributeType::Color,
            4,
            DataType::Uint8,
            true,
            vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255],
        );
        add_attribute(
            &mut mesh,
            GeometryAttributeType::TexCoord,
            2,
            DataType::Uint16,
            true,
            [0u16, 0, 65535, 0, 0, 65535]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect(),
        );
        mesh.attribute_mut(0).set_unique_id(10);
        mesh.attribute_mut(1).set_unique_id(20);
        mesh.attribute_mut(2).set_unique_id(30);

        let mut writer = GltfWriter::new();
        writer
            .add_draco_mesh(&mesh, Some("Triangle"), None)
            .unwrap();
        let json_text = writer.to_gltf_embedded().unwrap();
        let json: serde_json::Value =
            serde_json::from_str(&json_text).expect("writer JSON should parse");

        let primitive = &json["meshes"][0]["primitives"][0];
        let position_accessor = primitive["attributes"]["POSITION"].as_u64().unwrap() as usize;
        let color_accessor = primitive["attributes"]["COLOR_0"].as_u64().unwrap() as usize;
        let texcoord_accessor = primitive["attributes"]["TEXCOORD_0"].as_u64().unwrap() as usize;
        let draco_attributes = &primitive["extensions"]["KHR_draco_mesh_compression"]["attributes"];

        assert_eq!(json["accessors"][position_accessor]["count"], 3);
        assert!(json["accessors"][position_accessor]
            .get("bufferView")
            .is_none());
        assert!(json["accessors"][position_accessor]
            .get("byteOffset")
            .is_none());
        assert!(json["asset"]["generator"].is_string());
        assert_eq!(json["accessors"][color_accessor]["count"], 3);
        assert_eq!(json["accessors"][texcoord_accessor]["count"], 3);
        assert_eq!(draco_attributes["POSITION"], 10);
        assert_eq!(draco_attributes["COLOR_0"], 20);
        assert_eq!(draco_attributes["TEXCOORD_0"], 30);
        assert_eq!(
            json["accessors"][position_accessor]["min"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            json["accessors"][position_accessor]["max"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(json["accessors"][color_accessor]["normalized"], true);
        assert_eq!(json["accessors"][texcoord_accessor]["normalized"], true);

        #[cfg(feature = "gltf-reader")]
        {
            let reader = crate::GltfReader::from_gltf(json_text.as_bytes(), None).unwrap();
            let info = reader.draco_primitives().remove(0);
            let decoded = reader.decode_draco_mesh(&info).unwrap();
            assert!(decoded.attribute_by_unique_id(10).is_some());
            assert!(decoded.attribute_by_unique_id(20).is_some());
            assert!(decoded.attribute_by_unique_id(30).is_some());

            let mut invalid_json = json.clone();
            invalid_json["meshes"][0]["primitives"][0]["extensions"]
                ["KHR_draco_mesh_compression"]["attributes"]["POSITION"] =
                serde_json::Value::from(99);
            let invalid = serde_json::to_vec(&invalid_json).unwrap();
            let reader = crate::GltfReader::from_gltf(&invalid, None).unwrap();
            let info = reader.draco_primitives().remove(0);
            assert!(reader.decode_draco_mesh(&info).is_err());
        }
    }

    #[test]
    fn test_writer_uses_encoded_point_count_for_split_connectivity_accessors() {
        let mesh = create_test_uv_seam_mesh();
        let mut writer = GltfWriter::new();
        writer.add_draco_mesh(&mesh, Some("Seam"), None).unwrap();
        let json: serde_json::Value = serde_json::from_str(&writer.to_gltf_embedded().unwrap())
            .expect("writer JSON should parse");

        let primitive = &json["meshes"][0]["primitives"][0];
        let position_accessor = primitive["attributes"]["POSITION"].as_u64().unwrap() as usize;
        let texcoord_accessor = primitive["attributes"]["TEXCOORD_0"].as_u64().unwrap() as usize;

        assert_eq!(json["accessors"][position_accessor]["count"], 6);
        assert_eq!(json["accessors"][texcoord_accessor]["count"], 6);
    }

    #[test]
    fn test_writer_rejects_missing_position() {
        let mut mesh = Mesh::new();
        mesh.set_num_points(3);
        mesh.set_num_faces(1);
        mesh.set_face(FaceIndex(0), [PointIndex(0), PointIndex(1), PointIndex(2)]);
        add_attribute(
            &mut mesh,
            GeometryAttributeType::Normal,
            3,
            DataType::Float32,
            false,
            repeated_zero_attribute_bytes(DataType::Float32, 3, 3),
        );

        let err = GltfWriter::new()
            .add_draco_mesh(&mesh, Some("invalid"), None)
            .unwrap_err();
        assert!(matches!(err, GltfWriteError::InvalidMesh(_)));
    }

    #[test]
    fn test_writer_rejects_unsupported_attribute_data_types() {
        for data_type in [DataType::Float64, DataType::Int64, DataType::Uint32] {
            let mut mesh = create_test_triangle();
            add_attribute(
                &mut mesh,
                GeometryAttributeType::Generic,
                1,
                data_type,
                false,
                repeated_zero_attribute_bytes(data_type, 1, 3),
            );

            let err = GltfWriter::new()
                .add_draco_mesh(&mesh, Some("invalid"), None)
                .unwrap_err();
            assert!(matches!(err, GltfWriteError::Unsupported(_)));
        }
    }

    #[test]
    fn test_writer_rejects_attributes_it_would_previously_skip() {
        let mut mesh = create_test_triangle();
        add_attribute(
            &mut mesh,
            GeometryAttributeType::Color,
            2,
            DataType::Uint8,
            true,
            vec![255, 0, 0, 255, 0, 0],
        );

        let err = GltfWriter::new()
            .add_draco_mesh(&mesh, Some("invalid"), None)
            .unwrap_err();
        assert!(matches!(err, GltfWriteError::Unsupported(_)));
    }

    #[test]
    fn test_empty_glb_omits_bin_chunk() {
        let glb = GltfWriter::new().to_glb().unwrap();
        assert_eq!(&glb[0..4], b"glTF");
        assert_eq!(read_u32_le_for_test(&glb[12..16]) as usize + 20, glb.len());
        assert!(!glb.windows(4).any(|window| window == b"BIN\0"));
    }

    fn read_u32_le_for_test(data: &[u8]) -> u32 {
        u32::from_le_bytes([data[0], data[1], data[2], data[3]])
    }
}
