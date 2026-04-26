//! FBX Reader WASM module.
//!
//! Provides FBX binary file parsing functionality for web applications.
//! Supports FBX 7.x binary format.

use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Seek, SeekFrom};
use wasm_bindgen::prelude::*;

/// Mesh data structure for JavaScript interop.
#[derive(Serialize, Deserialize, Clone)]
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
}

/// Parse result containing meshes and any warnings/errors.
#[derive(Serialize, Deserialize)]
pub struct ParseResult {
    pub success: bool,
    pub meshes: Vec<MeshData>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    /// FBX version
    pub version: Option<u32>,
}

/// Initialize panic hook for better error messages in browser console.
#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Get the version of this WASM module.
#[wasm_bindgen]
pub fn version() -> String {
    "0.1.0".to_string()
}

/// Get the module name.
#[wasm_bindgen]
pub fn module_name() -> String {
    "FBX Reader".to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["fbx".to_string()]
}

/// Parse FBX binary file content.
#[wasm_bindgen]
pub fn parse_fbx(data: &[u8]) -> JsValue {
    let result = parse_fbx_internal(data);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// FBX file magic: "Kaydara FBX Binary  \0"
const FBX_MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";

/// FBX property value.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum FbxProperty {
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    String(String),
    Raw(Vec<u8>),
    BoolArray(Vec<bool>),
    I32Array(Vec<i32>),
    I64Array(Vec<i64>),
    F32Array(Vec<f32>),
    F64Array(Vec<f64>),
}

/// FBX node.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct FbxNode {
    name: String,
    properties: Vec<FbxProperty>,
    children: Vec<FbxNode>,
}

fn parse_fbx_internal(data: &[u8]) -> ParseResult {
    if data.len() < 27 {
        return ParseResult {
            success: false,
            meshes: vec![],
            error: Some("File too small to be a valid FBX".to_string()),
            warnings: vec![],
            version: None,
        };
    }

    // Check magic
    if &data[0..21] != FBX_MAGIC {
        return ParseResult {
            success: false,
            meshes: vec![],
            error: Some("Not a valid FBX binary file".to_string()),
            warnings: vec![],
            version: None,
        };
    }

    // Read version
    let version = u32::from_le_bytes([data[23], data[24], data[25], data[26]]);
    let is_64bit = version >= 7500;

    let mut cursor = Cursor::new(data);
    cursor.set_position(27);

    let mut warnings: Vec<String> = Vec::new();
    let mut root_nodes: Vec<FbxNode> = Vec::new();

    // Parse top-level nodes
    loop {
        match parse_node(&mut cursor, is_64bit) {
            Ok(Some(node)) => {
                root_nodes.push(node);
            }
            Ok(None) => break,
            Err(e) => {
                warnings.push(format!("Parse error: {}", e));
                break;
            }
        }
    }

    // Find Objects node and extract meshes
    let mut meshes: Vec<MeshData> = Vec::new();

    for node in &root_nodes {
        if node.name == "Objects" {
            for child in &node.children {
                if child.name == "Geometry" {
                    if let Some(mesh) = extract_mesh_from_geometry(child) {
                        meshes.push(mesh);
                    }
                }
            }
        }
    }

    ParseResult {
        success: true,
        meshes,
        error: None,
        warnings,
        version: Some(version),
    }
}

fn parse_node<R: Read + Seek>(reader: &mut R, is_64bit: bool) -> Result<Option<FbxNode>, String> {
    let (end_offset, num_properties, _properties_len, name_len) = if is_64bit {
        let mut buf = [0u8; 25];
        reader.read_exact(&mut buf).map_err(|e| e.to_string())?;

        let end_offset = u64::from_le_bytes([
            buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
        ]);
        let num_properties = u64::from_le_bytes([
            buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15],
        ]);
        let properties_len = u64::from_le_bytes([
            buf[16], buf[17], buf[18], buf[19], buf[20], buf[21], buf[22], buf[23],
        ]);
        let name_len = buf[24];

        (
            end_offset,
            num_properties as usize,
            properties_len as usize,
            name_len as usize,
        )
    } else {
        let mut buf = [0u8; 13];
        reader.read_exact(&mut buf).map_err(|e| e.to_string())?;

        let end_offset = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as u64;
        let num_properties = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        let properties_len = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;
        let name_len = buf[12] as usize;

        (end_offset, num_properties, properties_len, name_len)
    };

    // Null node (end marker)
    if end_offset == 0 {
        return Ok(None);
    }

    // Read name
    let mut name_buf = vec![0u8; name_len];
    reader
        .read_exact(&mut name_buf)
        .map_err(|e| e.to_string())?;
    let name = String::from_utf8_lossy(&name_buf).to_string();

    // Parse properties
    let mut properties = Vec::with_capacity(num_properties);
    for _ in 0..num_properties {
        if let Ok(prop) = parse_property(reader) {
            properties.push(prop);
        }
    }

    // Parse children
    let mut children = Vec::new();
    let current_pos = reader.stream_position().map_err(|e| e.to_string())?;

    if current_pos < end_offset {
        loop {
            let child_pos = reader.stream_position().map_err(|e| e.to_string())?;
            if child_pos >= end_offset {
                break;
            }

            match parse_node(reader, is_64bit) {
                Ok(Some(child)) => children.push(child),
                Ok(None) => break,
                Err(_) => break,
            }
        }
    }

    // Seek to end of node
    reader
        .seek(SeekFrom::Start(end_offset))
        .map_err(|e| e.to_string())?;

    Ok(Some(FbxNode {
        name,
        properties,
        children,
    }))
}

fn parse_property<R: Read>(reader: &mut R) -> Result<FbxProperty, String> {
    let mut type_code = [0u8; 1];
    reader
        .read_exact(&mut type_code)
        .map_err(|e| e.to_string())?;

    match type_code[0] {
        b'C' => {
            let mut buf = [0u8; 1];
            reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
            Ok(FbxProperty::Bool(buf[0] != 0))
        }
        b'Y' => {
            let mut buf = [0u8; 2];
            reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
            Ok(FbxProperty::I16(i16::from_le_bytes(buf)))
        }
        b'I' => {
            let mut buf = [0u8; 4];
            reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
            Ok(FbxProperty::I32(i32::from_le_bytes(buf)))
        }
        b'L' => {
            let mut buf = [0u8; 8];
            reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
            Ok(FbxProperty::I64(i64::from_le_bytes(buf)))
        }
        b'F' => {
            let mut buf = [0u8; 4];
            reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
            Ok(FbxProperty::F32(f32::from_le_bytes(buf)))
        }
        b'D' => {
            let mut buf = [0u8; 8];
            reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
            Ok(FbxProperty::F64(f64::from_le_bytes(buf)))
        }
        b'S' | b'R' => {
            let mut len_buf = [0u8; 4];
            reader.read_exact(&mut len_buf).map_err(|e| e.to_string())?;
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut data = vec![0u8; len];
            reader.read_exact(&mut data).map_err(|e| e.to_string())?;
            if type_code[0] == b'S' {
                Ok(FbxProperty::String(
                    String::from_utf8_lossy(&data).to_string(),
                ))
            } else {
                Ok(FbxProperty::Raw(data))
            }
        }
        b'b' | b'c' => {
            let (count, encoding, payload_len) = read_array_header(reader)?;
            let data = read_array_data(reader, encoding, payload_len, count)?;
            let bools: Vec<bool> = data.iter().map(|&b| b != 0).collect();
            Ok(FbxProperty::BoolArray(bools))
        }
        b'i' => {
            let (count, encoding, payload_len) = read_array_header(reader)?;
            let data = read_array_data(reader, encoding, payload_len, count * 4)?;
            let mut ints = Vec::with_capacity(count);
            for chunk in data.chunks(4) {
                if chunk.len() == 4 {
                    ints.push(i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                }
            }
            Ok(FbxProperty::I32Array(ints))
        }
        b'l' => {
            let (count, encoding, payload_len) = read_array_header(reader)?;
            let data = read_array_data(reader, encoding, payload_len, count * 8)?;
            let mut longs = Vec::with_capacity(count);
            for chunk in data.chunks(8) {
                if chunk.len() == 8 {
                    longs.push(i64::from_le_bytes([
                        chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6],
                        chunk[7],
                    ]));
                }
            }
            Ok(FbxProperty::I64Array(longs))
        }
        b'f' => {
            let (count, encoding, payload_len) = read_array_header(reader)?;
            let data = read_array_data(reader, encoding, payload_len, count * 4)?;
            let mut floats = Vec::with_capacity(count);
            for chunk in data.chunks(4) {
                if chunk.len() == 4 {
                    floats.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
                }
            }
            Ok(FbxProperty::F32Array(floats))
        }
        b'd' => {
            let (count, encoding, payload_len) = read_array_header(reader)?;
            let data = read_array_data(reader, encoding, payload_len, count * 8)?;
            let mut doubles = Vec::with_capacity(count);
            for chunk in data.chunks(8) {
                if chunk.len() == 8 {
                    doubles.push(f64::from_le_bytes([
                        chunk[0], chunk[1], chunk[2], chunk[3], chunk[4], chunk[5], chunk[6],
                        chunk[7],
                    ]));
                }
            }
            Ok(FbxProperty::F64Array(doubles))
        }
        _ => Err(format!("Unknown property type: {}", type_code[0] as char)),
    }
}

fn read_array_header<R: Read>(reader: &mut R) -> Result<(usize, u32, usize), String> {
    let mut buf = [0u8; 12];
    reader.read_exact(&mut buf).map_err(|e| e.to_string())?;

    let count = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    let encoding = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let payload_len = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;

    Ok((count, encoding, payload_len))
}

fn read_array_data<R: Read>(
    reader: &mut R,
    encoding: u32,
    payload_len: usize,
    expected_uncompressed_len: usize,
) -> Result<Vec<u8>, String> {
    match encoding {
        0 => {
            let mut data = vec![0u8; expected_uncompressed_len];
            reader.read_exact(&mut data).map_err(|e| e.to_string())?;
            Ok(data)
        }
        1 => {
            let mut compressed = vec![0u8; payload_len];
            reader
                .read_exact(&mut compressed)
                .map_err(|e| e.to_string())?;
            let data = miniz_oxide::inflate::decompress_to_vec_zlib(&compressed)
                .map_err(|e| format!("Compressed FBX array decompression failed: {:?}", e))?;
            if data.len() != expected_uncompressed_len {
                return Err(format!(
                    "Compressed FBX array length mismatch: expected {} bytes, decoded {} bytes",
                    expected_uncompressed_len,
                    data.len()
                ));
            }
            Ok(data)
        }
        _ => Err(format!("Unknown FBX array encoding: {}", encoding)),
    }
}

fn extract_mesh_from_geometry(node: &FbxNode) -> Option<MeshData> {
    let name = node.properties.iter().find_map(|p| {
        if let FbxProperty::String(s) = p {
            Some(s.clone())
        } else {
            None
        }
    });

    let mut vertices: Vec<f64> = Vec::new();
    let mut indices: Vec<i32> = Vec::new();
    let mut normals: Vec<f64> = Vec::new();

    for child in &node.children {
        match child.name.as_str() {
            "Vertices" => {
                if let Some(FbxProperty::F64Array(arr)) = child.properties.first() {
                    vertices = arr.clone();
                }
            }
            "PolygonVertexIndex" => {
                if let Some(FbxProperty::I32Array(arr)) = child.properties.first() {
                    indices = arr.clone();
                }
            }
            "LayerElementNormal" => {
                for sub in &child.children {
                    if sub.name == "Normals" {
                        if let Some(FbxProperty::F64Array(arr)) = sub.properties.first() {
                            normals = arr.clone();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if vertices.is_empty() {
        return None;
    }

    // Convert vertices to f32
    let positions: Vec<f32> = vertices.iter().map(|&v| v as f32).collect();

    // Convert and triangulate indices
    let mut tri_indices: Vec<u32> = Vec::new();
    let mut polygon: Vec<u32> = Vec::new();

    for &idx in &indices {
        if idx < 0 {
            // End of polygon (index is bitwise complement)
            polygon.push((!idx) as u32);

            // Triangulate polygon (fan triangulation)
            for i in 1..polygon.len() - 1 {
                tri_indices.push(polygon[0]);
                tri_indices.push(polygon[i] as u32);
                tri_indices.push(polygon[i + 1] as u32);
            }
            polygon.clear();
        } else {
            polygon.push(idx as u32);
        }
    }

    let norm_f32: Vec<f32> = normals.iter().map(|&v| v as f32).collect();

    Some(MeshData {
        name,
        positions,
        indices: tri_indices,
        normals: norm_f32,
        uvs: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniz_oxide::deflate::compress_to_vec_zlib;

    #[test]
    fn test_invalid_file() {
        let result = parse_fbx_internal(&[0, 1, 2, 3]);
        assert!(!result.success);
    }

    #[test]
    fn test_read_array_data_decompresses_compressed_arrays() {
        let raw = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let compressed = compress_to_vec_zlib(&raw, 6);
        let mut cursor = Cursor::new(compressed.clone());
        let decoded = read_array_data(&mut cursor, 1, compressed.len(), raw.len()).unwrap();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn test_read_array_data_rejects_decompressed_length_mismatch() {
        let raw = vec![1u8, 2, 3, 4];
        let compressed = compress_to_vec_zlib(&raw, 6);
        let mut cursor = Cursor::new(compressed.clone());
        let error = read_array_data(&mut cursor, 1, compressed.len(), raw.len() + 1).unwrap_err();
        assert!(error.contains("length mismatch"));
    }

    #[test]
    fn test_parse_fbx_with_compressed_geometry_arrays() {
        let mut data = Vec::new();
        data.extend_from_slice(FBX_MAGIC);
        data.extend_from_slice(&[0x1A, 0x00]);
        data.extend_from_slice(&7300u32.to_le_bytes());

        let vertices = prop_f64_array_compressed(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
        let indices = prop_i32_array_compressed(&[0, 1, !2]);

        let geometry = TestNode {
            name: "Geometry",
            props: vec![
                prop_i64(1),
                prop_string("Geometry::Triangle"),
                prop_string("Mesh"),
            ],
            children: vec![
                TestNode {
                    name: "Vertices",
                    props: vec![vertices],
                    children: vec![],
                },
                TestNode {
                    name: "PolygonVertexIndex",
                    props: vec![indices],
                    children: vec![],
                },
            ],
        };
        let objects = TestNode {
            name: "Objects",
            props: vec![],
            children: vec![geometry],
        };

        let objects_bytes = encode_node(&objects, data.len() as u64);
        data.extend_from_slice(&objects_bytes);
        data.extend_from_slice(&[0u8; 13]);

        let result = parse_fbx_internal(&data);
        assert!(result.success, "{:?}", result.error);
        assert_eq!(result.meshes.len(), 1);
        assert_eq!(result.meshes[0].positions.len(), 9);
        assert_eq!(result.meshes[0].indices, vec![0, 1, 2]);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }

    struct TestNode {
        name: &'static str,
        props: Vec<Vec<u8>>,
        children: Vec<TestNode>,
    }

    fn encode_node(node: &TestNode, start_abs: u64) -> Vec<u8> {
        let mut out = vec![0u8; 13];
        out.extend_from_slice(node.name.as_bytes());

        let props_len: usize = node.props.iter().map(Vec::len).sum();
        for prop in &node.props {
            out.extend_from_slice(prop);
        }

        for child in &node.children {
            let child_start = start_abs + out.len() as u64;
            let child_bytes = encode_node(child, child_start);
            out.extend_from_slice(&child_bytes);
        }
        if !node.children.is_empty() {
            out.extend_from_slice(&[0u8; 13]);
        }

        let end_abs = start_abs + out.len() as u64;
        out[0..4].copy_from_slice(&(end_abs as u32).to_le_bytes());
        out[4..8].copy_from_slice(&(node.props.len() as u32).to_le_bytes());
        out[8..12].copy_from_slice(&(props_len as u32).to_le_bytes());
        out[12] = node.name.len() as u8;
        out
    }

    fn prop_i64(value: i64) -> Vec<u8> {
        let mut out = vec![b'L'];
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    fn prop_string(value: &str) -> Vec<u8> {
        let mut out = vec![b'S'];
        out.extend_from_slice(&(value.len() as u32).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
        out
    }

    fn prop_f64_array_compressed(values: &[f64]) -> Vec<u8> {
        let mut raw = Vec::with_capacity(values.len() * 8);
        for value in values {
            raw.extend_from_slice(&value.to_le_bytes());
        }
        prop_array_compressed(b'd', values.len(), &raw)
    }

    fn prop_i32_array_compressed(values: &[i32]) -> Vec<u8> {
        let mut raw = Vec::with_capacity(values.len() * 4);
        for value in values {
            raw.extend_from_slice(&value.to_le_bytes());
        }
        prop_array_compressed(b'i', values.len(), &raw)
    }

    fn prop_array_compressed(type_code: u8, count: usize, raw: &[u8]) -> Vec<u8> {
        let compressed = compress_to_vec_zlib(raw, 6);
        let mut out = vec![type_code];
        out.extend_from_slice(&(count as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        out.extend_from_slice(&compressed);
        out
    }
}
