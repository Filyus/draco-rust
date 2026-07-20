//! FBX reader and writer WASM module.
//!
//! Provides FBX binary parsing (FBX 7.x) and generation (FBX 7.5) for web
//! applications. The reader and writer are independent: build with
//! `--features read` or `--features write` (both are on by default) to control
//! which half of the API is exported.

use serde::{Deserialize, Serialize};
use std::io::{Cursor, Seek, SeekFrom};
use wasm_bindgen::prelude::*;

/// FBX file magic: "Kaydara FBX Binary  \0". Shared by reader and writer.
const FBX_MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";

/// Initialize panic hook for better error messages in browser console.
#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Get the version of this WASM module.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["fbx".to_string()]
}

// ===========================================================================
// Reader
// ===========================================================================

#[cfg(feature = "read")]
use std::io::Read;

/// Mesh data produced by the FBX reader, for JavaScript interop.
#[cfg(feature = "read")]
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
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize)]
pub struct ParseResult {
    pub success: bool,
    pub meshes: Vec<MeshData>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    /// FBX version
    pub version: Option<u32>,
}

/// Parse FBX binary file content.
#[cfg(feature = "read")]
#[wasm_bindgen]
pub fn parse_fbx(data: &[u8]) -> JsValue {
    let result = parse_fbx_internal(data);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// FBX property value.
#[cfg(feature = "read")]
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
#[cfg(feature = "read")]
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct FbxNode {
    name: String,
    properties: Vec<FbxProperty>,
    children: Vec<FbxNode>,
}

#[cfg(feature = "read")]
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

#[cfg(feature = "read")]
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

#[cfg(feature = "read")]
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

#[cfg(feature = "read")]
fn read_array_header<R: Read>(reader: &mut R) -> Result<(usize, u32, usize), String> {
    let mut buf = [0u8; 12];
    reader.read_exact(&mut buf).map_err(|e| e.to_string())?;

    let count = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    let encoding = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let payload_len = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;

    Ok((count, encoding, payload_len))
}

#[cfg(feature = "read")]
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

#[cfg(feature = "read")]
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
                tri_indices.push(polygon[i]);
                tri_indices.push(polygon[i + 1]);
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

// ===========================================================================
// Writer
// ===========================================================================

#[cfg(feature = "write")]
use std::io::Write;

/// Input mesh data consumed by the FBX writer, from JavaScript.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Clone)]
pub struct MeshInput {
    /// Mesh name
    pub name: Option<String>,
    /// Vertex positions as flat array [x0, y0, z0, x1, y1, z1, ...]
    pub positions: Vec<f32>,
    /// Face indices as flat array (triangles)
    pub indices: Vec<u32>,
    /// Vertex normals (optional)
    pub normals: Option<Vec<f32>>,
    /// Texture coordinates (optional)
    pub uvs: Option<Vec<f32>>,
}

/// Export options.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Default)]
pub struct ExportOptions {
    /// FBX version (default: 7500 for FBX 7.5)
    pub version: Option<u32>,
}

/// Export result.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize)]
pub struct ExportResult {
    pub success: bool,
    pub binary_data: Option<Vec<u8>>,
    pub error: Option<String>,
}

/// Create FBX binary content from mesh data.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_fbx(meshes_js: JsValue, options_js: JsValue) -> JsValue {
    let meshes: Vec<MeshInput> = match serde_wasm_bindgen::from_value(meshes_js) {
        Ok(m) => m,
        Err(e) => {
            let result = ExportResult {
                success: false,
                binary_data: None,
                error: Some(format!("Invalid mesh data: {}", e)),
            };
            return serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL);
        }
    };

    let options: ExportOptions = serde_wasm_bindgen::from_value(options_js).unwrap_or_default();
    let result = create_fbx_internal(&meshes, &options);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Size of null node record for 64-bit FBX
#[cfg(feature = "write")]
const NULL_RECORD_SIZE: usize = 25;

#[cfg(feature = "write")]
fn create_fbx_internal(meshes: &[MeshInput], options: &ExportOptions) -> ExportResult {
    let version = options.version.unwrap_or(7500);
    let mut buffer: Vec<u8> = Vec::new();
    let mut cursor = Cursor::new(&mut buffer);

    // Write header
    if cursor.write_all(FBX_MAGIC).is_err() {
        return ExportResult {
            success: false,
            binary_data: None,
            error: Some("Failed to write FBX magic".to_string()),
        };
    }

    // Two padding bytes
    let _ = cursor.write_all(&[0x1A, 0x00]);

    // Version
    let _ = cursor.write_all(&version.to_le_bytes());

    // Generate unique IDs
    let mut next_id: i64 = 1000000;

    // Collect mesh data for Objects and Connections
    let mut geometry_ids: Vec<i64> = Vec::new();
    let mut model_ids: Vec<i64> = Vec::new();

    for _ in meshes {
        geometry_ids.push(next_id);
        next_id += 1;
        model_ids.push(next_id);
        next_id += 1;
    }

    // Write FBXHeaderExtension
    write_header_extension(&mut cursor, version);

    // Write GlobalSettings
    write_global_settings(&mut cursor);

    // Write Documents
    write_documents(&mut cursor);

    // Write References (empty)
    write_node(&mut cursor, "References", &[], &[]);

    // Write Definitions
    write_definitions(&mut cursor, meshes.len());

    // Write Objects
    let mut objects_children: Vec<Vec<u8>> = Vec::new();

    for (i, mesh) in meshes.iter().enumerate() {
        // Geometry node
        let mut geom_buf: Vec<u8> = Vec::new();
        write_geometry(&mut Cursor::new(&mut geom_buf), mesh, geometry_ids[i]);
        objects_children.push(geom_buf);

        // Model node
        let mut model_buf: Vec<u8> = Vec::new();
        write_model(&mut Cursor::new(&mut model_buf), mesh, model_ids[i]);
        objects_children.push(model_buf);
    }

    write_node_with_children(&mut cursor, "Objects", &[], &objects_children);

    // Write Connections
    let mut connections_children: Vec<Vec<u8>> = Vec::new();
    for i in 0..meshes.len() {
        // Connect model to root
        let mut conn_buf: Vec<u8> = Vec::new();
        write_connection(&mut Cursor::new(&mut conn_buf), model_ids[i], 0);
        connections_children.push(conn_buf);

        // Connect geometry to model
        let mut conn_buf2: Vec<u8> = Vec::new();
        write_connection(
            &mut Cursor::new(&mut conn_buf2),
            geometry_ids[i],
            model_ids[i],
        );
        connections_children.push(conn_buf2);
    }
    write_node_with_children(&mut cursor, "Connections", &[], &connections_children);

    // Write null record to end
    let null_record = vec![0u8; NULL_RECORD_SIZE];
    let _ = cursor.write_all(&null_record);

    // Footer
    write_footer(&mut cursor, version);

    ExportResult {
        success: true,
        binary_data: Some(buffer),
        error: None,
    }
}

#[cfg(feature = "write")]
fn write_node<W: Write + Seek>(
    writer: &mut W,
    name: &str,
    properties: &[FbxProp],
    _children: &[Vec<u8>],
) {
    write_node_with_children(writer, name, properties, &[]);
}

#[cfg(feature = "write")]
fn write_node_with_children<W: Write + Seek>(
    writer: &mut W,
    name: &str,
    properties: &[FbxProp],
    children: &[Vec<u8>],
) {
    let start_pos = writer.stream_position().unwrap();

    // Reserve the three 64-bit header fields. The name length is written
    // immediately afterwards and is itself the final byte of the 25-byte
    // FBX 7.5 node header.
    let _ = writer.write_all(&[0u8; 24]);

    // Write name
    let _ = writer.write_all(&[name.len() as u8]);
    let _ = writer.write_all(name.as_bytes());

    // Write properties
    let props_start = writer.stream_position().unwrap();
    for prop in properties {
        write_property(writer, prop);
    }
    let props_end = writer.stream_position().unwrap();
    let props_len = props_end - props_start;

    // Write children
    for child in children {
        let child_start = writer.stream_position().unwrap();
        let mut relocated = child.clone();
        rebase_node_offsets(&mut relocated, child_start);
        let _ = writer.write_all(&relocated);
    }

    // Write null record if we have children
    if !children.is_empty() {
        let _ = writer.write_all(&[0u8; NULL_RECORD_SIZE]);
    }

    let end_pos = writer.stream_position().unwrap();

    // Go back and fill in header
    let _ = writer.seek(SeekFrom::Start(start_pos));
    let _ = writer.write_all(&end_pos.to_le_bytes()); // end offset
    let _ = writer.write_all(&(properties.len() as u64).to_le_bytes()); // num properties
    let _ = writer.write_all(&props_len.to_le_bytes()); // properties list len
    let _ = writer.write_all(&[name.len() as u8]); // name len

    // Seek back to end
    let _ = writer.seek(SeekFrom::Start(end_pos));
}

/// Rebase every node end offset in a temporary node buffer before that buffer
/// is copied into its final position in the FBX stream.
#[cfg(feature = "write")]
fn rebase_node_offsets(data: &mut [u8], base_offset: u64) {
    fn visit(data: &mut [u8], node_start: usize, base_offset: u64) -> Option<usize> {
        if node_start.checked_add(25)? > data.len() {
            return None;
        }

        let local_end = u64::from_le_bytes(data[node_start..node_start + 8].try_into().ok()?);
        let local_end = usize::try_from(local_end).ok()?;
        if local_end <= node_start || local_end > data.len() {
            return None;
        }

        let property_len =
            u64::from_le_bytes(data[node_start + 16..node_start + 24].try_into().ok()?);
        let property_len = usize::try_from(property_len).ok()?;
        let name_len = data[node_start + 24] as usize;
        let mut child_start = node_start
            .checked_add(25)?
            .checked_add(name_len)?
            .checked_add(property_len)?;

        data[node_start..node_start + 8]
            .copy_from_slice(&(base_offset + local_end as u64).to_le_bytes());

        while child_start.checked_add(25)? <= local_end {
            if data[child_start..child_start + 25]
                .iter()
                .all(|byte| *byte == 0)
            {
                break;
            }
            child_start = visit(data, child_start, base_offset)?;
        }

        Some(local_end)
    }

    let _ = visit(data, 0, base_offset);
}

#[cfg(feature = "write")]
#[derive(Clone)]
#[allow(dead_code)]
enum FbxProp {
    I32(i32),
    I64(i64),
    F64(f64),
    String(String),
    F64Array(Vec<f64>),
    I32Array(Vec<i32>),
}

#[cfg(feature = "write")]
fn write_property<W: Write>(writer: &mut W, prop: &FbxProp) {
    match prop {
        FbxProp::I32(v) => {
            let _ = writer.write_all(b"I");
            let _ = writer.write_all(&v.to_le_bytes());
        }
        FbxProp::I64(v) => {
            let _ = writer.write_all(b"L");
            let _ = writer.write_all(&v.to_le_bytes());
        }
        FbxProp::F64(v) => {
            let _ = writer.write_all(b"D");
            let _ = writer.write_all(&v.to_le_bytes());
        }
        FbxProp::String(s) => {
            let _ = writer.write_all(b"S");
            let _ = writer.write_all(&(s.len() as u32).to_le_bytes());
            let _ = writer.write_all(s.as_bytes());
        }
        FbxProp::F64Array(arr) => {
            let _ = writer.write_all(b"d");
            let _ = writer.write_all(&(arr.len() as u32).to_le_bytes());
            let _ = writer.write_all(&0u32.to_le_bytes()); // encoding (0 = uncompressed)
            let _ = writer.write_all(&((arr.len() * 8) as u32).to_le_bytes()); // byte length
            for v in arr {
                let _ = writer.write_all(&v.to_le_bytes());
            }
        }
        FbxProp::I32Array(arr) => {
            let _ = writer.write_all(b"i");
            let _ = writer.write_all(&(arr.len() as u32).to_le_bytes());
            let _ = writer.write_all(&0u32.to_le_bytes()); // encoding
            let _ = writer.write_all(&((arr.len() * 4) as u32).to_le_bytes());
            for v in arr {
                let _ = writer.write_all(&v.to_le_bytes());
            }
        }
    }
}

#[cfg(feature = "write")]
fn write_header_extension<W: Write + Seek>(writer: &mut W, version: u32) {
    let mut children: Vec<Vec<u8>> = Vec::new();

    // FBXHeaderVersion
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "FBXHeaderVersion",
        &[FbxProp::I32(1003)],
        &[],
    );
    children.push(buf);

    // FBXVersion
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "FBXVersion",
        &[FbxProp::I32(version as i32)],
        &[],
    );
    children.push(buf);

    // Creator
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "Creator",
        &[FbxProp::String("draco-io WASM".to_string())],
        &[],
    );
    children.push(buf);

    write_node_with_children(writer, "FBXHeaderExtension", &[], &children);
}

#[cfg(feature = "write")]
fn write_global_settings<W: Write + Seek>(writer: &mut W) {
    let mut children = Vec::new();

    let mut version = Vec::new();
    write_node(
        &mut Cursor::new(&mut version),
        "Version",
        &[FbxProp::I32(1000)],
        &[],
    );
    children.push(version);

    let mut properties = Vec::new();
    let mut property_children = Vec::new();
    for (name, value) in [
        ("UpAxis", 1),
        ("UpAxisSign", 1),
        ("FrontAxis", 2),
        ("FrontAxisSign", 1),
        ("CoordAxis", 0),
        ("CoordAxisSign", 1),
    ] {
        let mut property = Vec::new();
        write_node(
            &mut Cursor::new(&mut property),
            "P",
            &[
                FbxProp::String(name.to_string()),
                FbxProp::String("int".to_string()),
                FbxProp::String("Integer".to_string()),
                FbxProp::String(String::new()),
                FbxProp::I32(value),
            ],
            &[],
        );
        property_children.push(property);
    }
    for name in ["UnitScaleFactor", "OriginalUnitScaleFactor"] {
        let mut property = Vec::new();
        write_node(
            &mut Cursor::new(&mut property),
            "P",
            &[
                FbxProp::String(name.to_string()),
                FbxProp::String("double".to_string()),
                FbxProp::String("Number".to_string()),
                FbxProp::String(String::new()),
                FbxProp::F64(1.0),
            ],
            &[],
        );
        property_children.push(property);
    }
    write_node_with_children(
        &mut Cursor::new(&mut properties),
        "Properties70",
        &[],
        &property_children,
    );
    children.push(properties);

    write_node_with_children(writer, "GlobalSettings", &[], &children);
}

#[cfg(feature = "write")]
fn write_documents<W: Write + Seek>(writer: &mut W) {
    let mut children: Vec<Vec<u8>> = Vec::new();

    let mut buf = Vec::new();
    write_node(&mut Cursor::new(&mut buf), "Count", &[FbxProp::I64(1)], &[]);
    children.push(buf);

    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "Document",
        &[
            FbxProp::I64(1),
            FbxProp::String("Scene".to_string()),
            FbxProp::String("Scene".to_string()),
        ],
        &[],
    );
    children.push(buf);

    write_node_with_children(writer, "Documents", &[], &children);
}

#[cfg(feature = "write")]
fn write_definitions<W: Write + Seek>(writer: &mut W, mesh_count: usize) {
    let mut children: Vec<Vec<u8>> = Vec::new();

    // Version
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "Version",
        &[FbxProp::I64(100)],
        &[],
    );
    children.push(buf);

    // Count
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "Count",
        &[FbxProp::I64((mesh_count * 2) as i64)],
        &[],
    );
    children.push(buf);

    // Geometry definition
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "ObjectType",
        &[FbxProp::String("Geometry".to_string())],
        &[],
    );
    children.push(buf);

    // Model definition
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "ObjectType",
        &[FbxProp::String("Model".to_string())],
        &[],
    );
    children.push(buf);

    write_node_with_children(writer, "Definitions", &[], &children);
}

#[cfg(feature = "write")]
fn write_geometry<W: Write + Seek>(writer: &mut W, mesh: &MeshInput, id: i64) {
    let name = mesh.name.as_deref().unwrap_or("Mesh");
    let full_name = format!("{}\0\x01Geometry", name);

    let mut children: Vec<Vec<u8>> = Vec::new();

    // Vertices
    let vertices: Vec<f64> = mesh.positions.iter().map(|&v| v as f64).collect();
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "Vertices",
        &[FbxProp::F64Array(vertices)],
        &[],
    );
    children.push(buf);

    // PolygonVertexIndex (convert triangles to FBX format with negative end markers)
    let mut polygon_indices: Vec<i32> = Vec::new();
    for chunk in mesh.indices.chunks(3) {
        if chunk.len() == 3 {
            polygon_indices.push(chunk[0] as i32);
            polygon_indices.push(chunk[1] as i32);
            polygon_indices.push(!(chunk[2] as i32)); // Bitwise NOT marks end of polygon
        }
    }
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "PolygonVertexIndex",
        &[FbxProp::I32Array(polygon_indices)],
        &[],
    );
    children.push(buf);

    let has_normals = mesh
        .normals
        .as_ref()
        .is_some_and(|values| !values.is_empty());
    let has_uvs = mesh.uvs.as_ref().is_some_and(|values| !values.is_empty());

    // Normals (if present)
    if let Some(ref normals) = mesh.normals {
        if !normals.is_empty() {
            let norm_doubles: Vec<f64> = normals.iter().map(|&v| v as f64).collect();

            let mut layer_children: Vec<Vec<u8>> = Vec::new();

            let mut buf = Vec::new();
            write_node(
                &mut Cursor::new(&mut buf),
                "Version",
                &[FbxProp::I32(101)],
                &[],
            );
            layer_children.push(buf);

            for (node_name, value) in [
                ("Name", ""),
                ("MappingInformationType", "ByVertice"),
                ("ReferenceInformationType", "Direct"),
            ] {
                let mut buf = Vec::new();
                write_node(
                    &mut Cursor::new(&mut buf),
                    node_name,
                    &[FbxProp::String(value.to_string())],
                    &[],
                );
                layer_children.push(buf);
            }

            let mut buf = Vec::new();
            write_node(
                &mut Cursor::new(&mut buf),
                "Normals",
                &[FbxProp::F64Array(norm_doubles)],
                &[],
            );
            layer_children.push(buf);

            let mut buf = Vec::new();
            write_node_with_children(
                &mut Cursor::new(&mut buf),
                "LayerElementNormal",
                &[FbxProp::I32(0)],
                &layer_children,
            );
            children.push(buf);
        }
    }

    if let Some(ref uvs) = mesh.uvs {
        if !uvs.is_empty() {
            let mut layer_children = Vec::new();
            for (node_name, property) in [
                ("Version", FbxProp::I32(101)),
                ("Name", FbxProp::String("UVMap".to_string())),
                (
                    "MappingInformationType",
                    FbxProp::String("ByVertice".to_string()),
                ),
                (
                    "ReferenceInformationType",
                    FbxProp::String("Direct".to_string()),
                ),
            ] {
                let mut buf = Vec::new();
                write_node(&mut Cursor::new(&mut buf), node_name, &[property], &[]);
                layer_children.push(buf);
            }

            let mut buf = Vec::new();
            write_node(
                &mut Cursor::new(&mut buf),
                "UV",
                &[FbxProp::F64Array(
                    uvs.iter().map(|&value| value as f64).collect(),
                )],
                &[],
            );
            layer_children.push(buf);

            let mut buf = Vec::new();
            write_node_with_children(
                &mut Cursor::new(&mut buf),
                "LayerElementUV",
                &[FbxProp::I32(0)],
                &layer_children,
            );
            children.push(buf);
        }
    }

    if has_normals || has_uvs {
        let mut layer_children = Vec::new();
        let mut version = Vec::new();
        write_node(
            &mut Cursor::new(&mut version),
            "Version",
            &[FbxProp::I32(100)],
            &[],
        );
        layer_children.push(version);

        for layer_type in [
            has_normals.then_some("LayerElementNormal"),
            has_uvs.then_some("LayerElementUV"),
        ]
        .into_iter()
        .flatten()
        {
            let mut element_children = Vec::new();
            for (node_name, property) in [
                ("Type", FbxProp::String(layer_type.to_string())),
                ("TypedIndex", FbxProp::I32(0)),
            ] {
                let mut buf = Vec::new();
                write_node(&mut Cursor::new(&mut buf), node_name, &[property], &[]);
                element_children.push(buf);
            }

            let mut element = Vec::new();
            write_node_with_children(
                &mut Cursor::new(&mut element),
                "LayerElement",
                &[],
                &element_children,
            );
            layer_children.push(element);
        }

        let mut layer = Vec::new();
        write_node_with_children(
            &mut Cursor::new(&mut layer),
            "Layer",
            &[FbxProp::I32(0)],
            &layer_children,
        );
        children.push(layer);
    }

    write_node_with_children(
        writer,
        "Geometry",
        &[
            FbxProp::I64(id),
            FbxProp::String(full_name),
            FbxProp::String("Mesh".to_string()),
        ],
        &children,
    );
}

#[cfg(feature = "write")]
fn write_model<W: Write + Seek>(writer: &mut W, mesh: &MeshInput, id: i64) {
    let name = mesh.name.as_deref().unwrap_or("Model");
    let full_name = format!("{}\0\x01Model", name);

    let mut children = Vec::new();
    for (node_name, property) in [
        ("Version", FbxProp::I32(232)),
        ("Shading", FbxProp::I32(1)),
        ("Culling", FbxProp::String("CullingOff".to_string())),
    ] {
        let mut child = Vec::new();
        write_node(&mut Cursor::new(&mut child), node_name, &[property], &[]);
        children.push(child);
    }

    let mut properties = Vec::new();
    write_node(&mut Cursor::new(&mut properties), "Properties70", &[], &[]);
    children.insert(1, properties);

    write_node_with_children(
        writer,
        "Model",
        &[
            FbxProp::I64(id),
            FbxProp::String(full_name),
            FbxProp::String("Mesh".to_string()),
        ],
        &children,
    );
}

#[cfg(feature = "write")]
fn write_connection<W: Write + Seek>(writer: &mut W, child_id: i64, parent_id: i64) {
    write_node(
        writer,
        "C",
        &[
            FbxProp::String("OO".to_string()),
            FbxProp::I64(child_id),
            FbxProp::I64(parent_id),
        ],
        &[],
    );
}

#[cfg(feature = "write")]
fn write_footer<W: Write>(writer: &mut W, version: u32) {
    // Footer padding and signature
    let footer_id = [
        0xF8, 0x5A, 0x8C, 0x6A, 0xDE, 0xF5, 0xD9, 0x7E, 0xEC, 0xE9, 0x0C, 0xE3, 0x75, 0x8F, 0x29,
        0x0B,
    ];

    let _ = writer.write_all(&[0u8; 4]); // padding
    let _ = writer.write_all(&footer_id);
    let _ = writer.write_all(&[0u8; 4]); // padding
    let _ = writer.write_all(&version.to_le_bytes());
    let _ = writer.write_all(&[0u8; 120]); // padding to 128 bytes
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(all(test, feature = "read"))]
mod reader_tests {
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

#[cfg(all(test, feature = "write"))]
mod writer_tests {
    use super::*;

    #[test]
    fn test_create_simple_fbx() {
        let mesh = MeshInput {
            name: Some("Triangle".to_string()),
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0],
            indices: vec![0, 1, 2],
            normals: None,
            uvs: None,
        };

        let result = create_fbx_internal(&[mesh], &ExportOptions::default());
        assert!(result.success);
        assert!(result.binary_data.is_some());

        let data = result.binary_data.unwrap();
        assert!(data.len() > 27);
        assert_eq!(&data[0..21], FBX_MAGIC);

        let parsed = parse_fbx_internal(&data);
        assert!(
            parsed.success,
            "generated FBX should parse: {:?}",
            parsed.error
        );
        assert_eq!(parsed.meshes.len(), 1);
    }
}
