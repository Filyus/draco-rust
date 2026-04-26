//! glTF/GLB Reader WASM module.
//!
//! Provides glTF 2.0 file parsing functionality for web applications.
//! Supports both .gltf (JSON) and .glb (binary) formats with Draco compression.
//!
//! Uses nanoserde for minimal WASM binary size (no serde_json monomorphization).

use nanoserde::{DeJson, SerJson};
use std::collections::HashMap;
use wasm_bindgen::prelude::*;

/// Mesh data structure for JavaScript interop.
#[derive(SerJson, Clone, Default)]
pub struct MeshData {
    /// Mesh name
    pub name: Option<String>,
    /// Vertex positions as flat array [x0, y0, z0, x1, y1, z1, ...]
    pub positions: Vec<f32>,
    /// Face indices as flat array (triangles)
    pub indices: Vec<u32>,
    /// Vertex normals (if present)
    pub normals: Vec<f32>,
    /// Texture coordinates (if present)
    pub uvs: Vec<f32>,
    /// Vertex colors (if present)
    pub colors: Vec<f32>,
}

/// Node in the scene graph.
#[derive(SerJson, Clone, Default)]
pub struct SceneNode {
    pub name: Option<String>,
    #[nserde(rename = "meshIndex")]
    pub mesh_index: Option<usize>,
    pub translation: Option<Vec<f32>>,
    pub rotation: Option<Vec<f32>>,
    pub scale: Option<Vec<f32>>,
    pub children: Vec<usize>,
}

/// Scene data.
#[derive(SerJson, Clone, Default)]
pub struct SceneData {
    pub name: Option<String>,
    pub nodes: Vec<usize>,
}

/// Parse result containing meshes and scene graph.
#[derive(SerJson, Default)]
pub struct ParseResult {
    pub success: bool,
    pub meshes: Vec<MeshData>,
    pub scenes: Vec<SceneData>,
    pub nodes: Vec<SceneNode>,
    #[nserde(rename = "defaultScene")]
    pub default_scene: Option<usize>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    /// Whether the file uses Draco compression
    #[nserde(rename = "usesDraco")]
    pub uses_draco: bool,
}

/// Initialize panic hook for better error messages in browser console.
#[wasm_bindgen(start)]
pub fn init() {
    // Panic hook removed for smaller binary size
}

/// Get the version of this WASM module.
#[wasm_bindgen]
pub fn version() -> String {
    "0.1.0".to_string()
}

/// Get the module name.
#[wasm_bindgen]
pub fn module_name() -> String {
    "glTF Reader".to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["gltf".to_string(), "glb".to_string()]
}

/// Helper to convert a ParseResult to JsValue via JSON.
fn to_js_value(result: &ParseResult) -> JsValue {
    let json = SerJson::serialize_json(result);
    js_sys::JSON::parse(&json).unwrap_or(JsValue::NULL)
}

/// Parse glTF JSON content.
#[wasm_bindgen]
pub fn parse_gltf(json_content: &str) -> JsValue {
    let result = parse_gltf_json(json_content, None);
    to_js_value(&result)
}

/// Parse GLB binary content.
#[wasm_bindgen]
pub fn parse_glb(data: &[u8]) -> JsValue {
    let result = parse_glb_internal(data);
    to_js_value(&result)
}

/// Parse glTF with external binary buffer.
#[wasm_bindgen]
pub fn parse_gltf_with_buffer(json_content: &str, buffer: &[u8]) -> JsValue {
    let result = parse_gltf_json(json_content, Some(buffer));
    to_js_value(&result)
}

// GLB magic and header
const GLB_MAGIC: u32 = 0x46546C67; // "glTF"
const GLB_CHUNK_JSON: u32 = 0x4E4F534A; // "JSON"
const GLB_CHUNK_BIN: u32 = 0x004E4942; // "BIN\0"

fn parse_glb_internal(data: &[u8]) -> ParseResult {
    if data.len() < 12 {
        return ParseResult {
            success: false,
            error: Some("Invalid GLB: file too small".to_string()),
            ..Default::default()
        };
    }

    // Parse header
    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if magic != GLB_MAGIC {
        return ParseResult {
            success: false,
            error: Some("Invalid GLB: wrong magic number".to_string()),
            ..Default::default()
        };
    }

    let _version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let _length = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);

    // Parse chunks
    let mut offset = 12usize;
    let mut json_data: Option<String> = None;
    let mut bin_data: Option<&[u8]> = None;

    while offset + 8 <= data.len() {
        let chunk_length = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        let chunk_type = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]);

        offset += 8;

        if offset + chunk_length > data.len() {
            break;
        }

        match chunk_type {
            GLB_CHUNK_JSON => {
                if let Ok(s) = std::str::from_utf8(&data[offset..offset + chunk_length]) {
                    json_data = Some(s.to_string());
                }
            }
            GLB_CHUNK_BIN => {
                bin_data = Some(&data[offset..offset + chunk_length]);
            }
            _ => {} // Unknown chunk type
        }

        offset += chunk_length;
    }

    match json_data {
        Some(json) => parse_gltf_json(&json, bin_data),
        None => ParseResult {
            success: false,
            error: Some("Invalid GLB: no JSON chunk found".to_string()),
            ..Default::default()
        },
    }
}

// glTF data structures using nanoserde
#[derive(DeJson, Default)]
struct GltfRoot {
    #[nserde(default)]
    accessors: Vec<Accessor>,
    #[nserde(default, rename = "bufferViews")]
    buffer_views: Vec<BufferView>,
    #[nserde(default)]
    #[allow(dead_code)]
    buffers: Vec<Buffer>,
    #[nserde(default)]
    meshes: Vec<GltfMesh>,
    #[nserde(default)]
    nodes: Vec<GltfNode>,
    #[nserde(default)]
    scenes: Vec<GltfScene>,
    #[nserde(default)]
    scene: Option<usize>,
    #[nserde(default, rename = "extensionsUsed")]
    extensions_used: Vec<String>,
}

#[derive(DeJson, Default)]
struct Accessor {
    #[nserde(default, rename = "bufferView")]
    buffer_view: Option<usize>,
    #[nserde(default, rename = "byteOffset")]
    byte_offset: Option<usize>,
    #[nserde(default, rename = "componentType")]
    component_type: u32,
    #[nserde(default)]
    count: usize,
    #[nserde(default, rename = "type")]
    #[allow(dead_code)]
    accessor_type: String,
}

#[derive(DeJson, Default)]
struct BufferView {
    #[nserde(default)]
    #[allow(dead_code)]
    buffer: usize,
    #[nserde(default, rename = "byteOffset")]
    byte_offset: Option<usize>,
    #[nserde(default, rename = "byteLength")]
    byte_length: usize,
    #[nserde(default, rename = "byteStride")]
    #[allow(dead_code)]
    byte_stride: Option<usize>,
}

#[derive(DeJson, Default)]
struct Buffer {
    #[nserde(default, rename = "byteLength")]
    #[allow(dead_code)]
    byte_length: usize,
    #[nserde(default)]
    #[allow(dead_code)]
    uri: Option<String>,
}

#[derive(DeJson, Default)]
struct GltfMesh {
    #[nserde(default)]
    name: Option<String>,
    #[nserde(default)]
    primitives: Vec<Primitive>,
}

#[derive(DeJson, Default)]
struct Primitive {
    #[nserde(default)]
    attributes: HashMap<String, usize>,
    #[nserde(default)]
    indices: Option<usize>,
    #[nserde(default)]
    extensions: Option<PrimitiveExtensions>,
}

#[derive(DeJson, Default)]
struct PrimitiveExtensions {
    #[nserde(default, rename = "KHR_draco_mesh_compression")]
    khr_draco: Option<DracoExtension>,
}

#[derive(DeJson, Default)]
struct DracoExtension {
    #[nserde(default, rename = "bufferView")]
    buffer_view: usize,
    #[nserde(default)]
    #[allow(dead_code)]
    attributes: HashMap<String, usize>,
}

#[derive(DeJson, Default)]
struct GltfNode {
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
struct GltfScene {
    #[nserde(default)]
    name: Option<String>,
    #[nserde(default)]
    nodes: Vec<usize>,
}

fn parse_gltf_json(json_content: &str, bin_buffer: Option<&[u8]>) -> ParseResult {
    let root: GltfRoot = match DeJson::deserialize_json(json_content) {
        Ok(r) => r,
        Err(e) => {
            return ParseResult {
                success: false,
                error: Some(format!("Failed to parse glTF JSON: {:?}", e)),
                ..Default::default()
            };
        }
    };

    let uses_draco = root
        .extensions_used
        .iter()
        .any(|e| e == "KHR_draco_mesh_compression");
    let mut warnings: Vec<String> = Vec::new();
    let mut meshes: Vec<MeshData> = Vec::new();

    // Parse meshes
    for gltf_mesh in &root.meshes {
        for primitive in &gltf_mesh.primitives {
            let mut mesh = MeshData {
                name: gltf_mesh.name.clone(),
                ..Default::default()
            };

            // Check for Draco extension
            if let Some(ref extensions) = primitive.extensions {
                if let Some(ref draco) = extensions.khr_draco {
                    if let Some(buffer_data) = bin_buffer {
                        if let Some(bv) = root.buffer_views.get(draco.buffer_view) {
                            let offset = bv.byte_offset.unwrap_or(0);
                            let end = offset + bv.byte_length;
                            if end <= buffer_data.len() {
                                let draco_data = &buffer_data[offset..end];
                                match decode_draco_mesh(draco_data) {
                                    Ok(decoded) => {
                                        mesh = decoded;
                                        mesh.name = gltf_mesh.name.clone();
                                        meshes.push(mesh);
                                        continue;
                                    }
                                    Err(e) => {
                                        warnings
                                            .push(format!("Failed to decode Draco mesh: {}", e));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Standard accessor-based geometry
            if let Some(bin) = bin_buffer {
                // Positions
                if let Some(&pos_idx) = primitive.attributes.get("POSITION") {
                    mesh.positions =
                        read_accessor_vec3(&root.accessors, &root.buffer_views, bin, pos_idx);
                }

                // Normals
                if let Some(&norm_idx) = primitive.attributes.get("NORMAL") {
                    mesh.normals =
                        read_accessor_vec3(&root.accessors, &root.buffer_views, bin, norm_idx);
                }

                // UVs
                if let Some(&uv_idx) = primitive.attributes.get("TEXCOORD_0") {
                    mesh.uvs = read_accessor_vec2(&root.accessors, &root.buffer_views, bin, uv_idx);
                }

                // Indices
                if let Some(indices_idx) = primitive.indices {
                    mesh.indices = read_accessor_indices(
                        &root.accessors,
                        &root.buffer_views,
                        bin,
                        indices_idx,
                    );
                }
            }

            meshes.push(mesh);
        }
    }

    // Parse nodes
    let nodes: Vec<SceneNode> = root
        .nodes
        .iter()
        .map(|n| SceneNode {
            name: n.name.clone(),
            mesh_index: n.mesh,
            translation: n.translation.clone(),
            rotation: n.rotation.clone(),
            scale: n.scale.clone(),
            children: n.children.clone(),
        })
        .collect();

    // Parse scenes
    let scenes: Vec<SceneData> = root
        .scenes
        .iter()
        .map(|s| SceneData {
            name: s.name.clone(),
            nodes: s.nodes.clone(),
        })
        .collect();

    ParseResult {
        success: true,
        meshes,
        scenes,
        nodes,
        default_scene: root.scene,
        error: None,
        warnings,
        uses_draco,
    }
}

fn read_accessor_vec3(
    accessors: &[Accessor],
    buffer_views: &[BufferView],
    buffer: &[u8],
    accessor_idx: usize,
) -> Vec<f32> {
    let accessor = match accessors.get(accessor_idx) {
        Some(a) => a,
        None => return vec![],
    };

    let bv_idx = match accessor.buffer_view {
        Some(idx) => idx,
        None => return vec![],
    };

    let bv = match buffer_views.get(bv_idx) {
        Some(bv) => bv,
        None => return vec![],
    };

    let byte_offset = bv.byte_offset.unwrap_or(0) + accessor.byte_offset.unwrap_or(0);
    let mut result = Vec::with_capacity(accessor.count * 3);

    for i in 0..accessor.count {
        let offset = byte_offset + i * 12; // 3 * 4 bytes for float32
        if offset + 12 <= buffer.len() {
            let x = f32::from_le_bytes([
                buffer[offset],
                buffer[offset + 1],
                buffer[offset + 2],
                buffer[offset + 3],
            ]);
            let y = f32::from_le_bytes([
                buffer[offset + 4],
                buffer[offset + 5],
                buffer[offset + 6],
                buffer[offset + 7],
            ]);
            let z = f32::from_le_bytes([
                buffer[offset + 8],
                buffer[offset + 9],
                buffer[offset + 10],
                buffer[offset + 11],
            ]);
            result.push(x);
            result.push(y);
            result.push(z);
        }
    }

    result
}

fn read_accessor_vec2(
    accessors: &[Accessor],
    buffer_views: &[BufferView],
    buffer: &[u8],
    accessor_idx: usize,
) -> Vec<f32> {
    let accessor = match accessors.get(accessor_idx) {
        Some(a) => a,
        None => return vec![],
    };

    let bv_idx = match accessor.buffer_view {
        Some(idx) => idx,
        None => return vec![],
    };

    let bv = match buffer_views.get(bv_idx) {
        Some(bv) => bv,
        None => return vec![],
    };

    let byte_offset = bv.byte_offset.unwrap_or(0) + accessor.byte_offset.unwrap_or(0);
    let mut result = Vec::with_capacity(accessor.count * 2);

    for i in 0..accessor.count {
        let offset = byte_offset + i * 8; // 2 * 4 bytes for float32
        if offset + 8 <= buffer.len() {
            let u = f32::from_le_bytes([
                buffer[offset],
                buffer[offset + 1],
                buffer[offset + 2],
                buffer[offset + 3],
            ]);
            let v = f32::from_le_bytes([
                buffer[offset + 4],
                buffer[offset + 5],
                buffer[offset + 6],
                buffer[offset + 7],
            ]);
            result.push(u);
            result.push(v);
        }
    }

    result
}

fn read_accessor_indices(
    accessors: &[Accessor],
    buffer_views: &[BufferView],
    buffer: &[u8],
    accessor_idx: usize,
) -> Vec<u32> {
    let accessor = match accessors.get(accessor_idx) {
        Some(a) => a,
        None => return vec![],
    };

    let bv_idx = match accessor.buffer_view {
        Some(idx) => idx,
        None => return vec![],
    };

    let bv = match buffer_views.get(bv_idx) {
        Some(bv) => bv,
        None => return vec![],
    };

    let byte_offset = bv.byte_offset.unwrap_or(0) + accessor.byte_offset.unwrap_or(0);
    let mut result = Vec::with_capacity(accessor.count);

    // Component types: 5121 = UNSIGNED_BYTE, 5123 = UNSIGNED_SHORT, 5125 = UNSIGNED_INT
    let elem_size = match accessor.component_type {
        5121 => 1,
        5123 => 2,
        5125 => 4,
        _ => return vec![],
    };

    for i in 0..accessor.count {
        let offset = byte_offset + i * elem_size;
        if offset + elem_size <= buffer.len() {
            let idx = match accessor.component_type {
                5121 => buffer[offset] as u32,
                5123 => u16::from_le_bytes([buffer[offset], buffer[offset + 1]]) as u32,
                5125 => u32::from_le_bytes([
                    buffer[offset],
                    buffer[offset + 1],
                    buffer[offset + 2],
                    buffer[offset + 3],
                ]),
                _ => 0,
            };
            result.push(idx);
        }
    }

    result
}

fn decode_draco_mesh(data: &[u8]) -> Result<MeshData, String> {
    use draco_core::decoder_buffer::DecoderBuffer;
    use draco_core::geometry_attribute::GeometryAttributeType;
    use draco_core::geometry_indices::{FaceIndex, PointIndex};
    use draco_core::mesh::Mesh;
    use draco_core::mesh_decoder::MeshDecoder;

    let mut decoder_buffer = DecoderBuffer::new(data);
    let mut mesh = Mesh::new();
    let mut decoder = MeshDecoder::new();
    decoder
        .decode(&mut decoder_buffer, &mut mesh)
        .map_err(|e| format!("{:?}", e))?;

    let mut result = MeshData::default();

    // Extract positions
    let pos_att_id = mesh.named_attribute_id(GeometryAttributeType::Position);
    if pos_att_id != -1 {
        let pos_attr = mesh.attribute(pos_att_id);
        let num_points = mesh.num_points();
        result.positions.reserve(num_points * 3);
        for i in 0..num_points {
            let val_index = pos_attr.mapped_index(PointIndex(i as u32));
            if val_index.0 != u32::MAX {
                let byte_stride = pos_attr.byte_stride() as usize;
                let byte_offset = val_index.0 as usize * byte_stride;
                let mut bytes = [0u8; 12];
                pos_attr.buffer().read(byte_offset, &mut bytes);
                result
                    .positions
                    .push(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                result
                    .positions
                    .push(f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]));
                result.positions.push(f32::from_le_bytes([
                    bytes[8], bytes[9], bytes[10], bytes[11],
                ]));
            }
        }
    }

    // Extract normals
    let norm_att_id = mesh.named_attribute_id(GeometryAttributeType::Normal);
    if norm_att_id != -1 {
        let norm_attr = mesh.attribute(norm_att_id);
        let num_points = mesh.num_points();
        result.normals.reserve(num_points * 3);
        for i in 0..num_points {
            let val_index = norm_attr.mapped_index(PointIndex(i as u32));
            if val_index.0 != u32::MAX {
                let byte_stride = norm_attr.byte_stride() as usize;
                let byte_offset = val_index.0 as usize * byte_stride;
                let mut bytes = [0u8; 12];
                norm_attr.buffer().read(byte_offset, &mut bytes);
                result
                    .normals
                    .push(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                result
                    .normals
                    .push(f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]));
                result.normals.push(f32::from_le_bytes([
                    bytes[8], bytes[9], bytes[10], bytes[11],
                ]));
            }
        }
    }

    // Extract UVs
    let uv_att_id = mesh.named_attribute_id(GeometryAttributeType::TexCoord);
    if uv_att_id != -1 {
        let uv_attr = mesh.attribute(uv_att_id);
        let num_points = mesh.num_points();
        result.uvs.reserve(num_points * 2);
        for i in 0..num_points {
            let val_index = uv_attr.mapped_index(PointIndex(i as u32));
            if val_index.0 != u32::MAX {
                let byte_stride = uv_attr.byte_stride() as usize;
                let byte_offset = val_index.0 as usize * byte_stride;
                let mut bytes = [0u8; 8];
                uv_attr.buffer().read(byte_offset, &mut bytes);
                result
                    .uvs
                    .push(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]));
                result
                    .uvs
                    .push(f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]));
            }
        }
    }

    // Extract faces
    let num_faces = mesh.num_faces();
    result.indices.reserve(num_faces * 3);
    for face_idx in 0..num_faces {
        let face = mesh.face(FaceIndex(face_idx as u32));
        result.indices.push(face[0].0);
        result.indices.push(face[1].0);
        result.indices.push(face[2].0);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_gltf() {
        let gltf = r#"{
            "asset": { "version": "2.0" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": [{ "name": "TestNode" }],
            "meshes": []
        }"#;

        let result = parse_gltf_json(gltf, None);
        assert!(result.success);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].name, Some("TestNode".to_string()));
    }
}
