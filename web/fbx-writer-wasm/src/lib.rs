//! FBX Writer WASM module.
//!
//! Provides FBX binary file generation functionality for web applications.
//! Outputs FBX 7.5 format (64-bit headers).

use serde::{Deserialize, Serialize};
use std::io::{Cursor, Seek, SeekFrom, Write};
use wasm_bindgen::prelude::*;

/// Input mesh data from JavaScript.
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
#[derive(Serialize, Deserialize, Default)]
pub struct ExportOptions {
    /// FBX version (default: 7500 for FBX 7.5)
    pub version: Option<u32>,
}

/// Export result.
#[derive(Serialize, Deserialize)]
pub struct ExportResult {
    pub success: bool,
    pub binary_data: Option<Vec<u8>>,
    pub error: Option<String>,
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
    "FBX Writer".to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["fbx".to_string()]
}

/// Create FBX binary content from mesh data.
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

/// FBX file magic
const FBX_MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \0";

/// Size of null node record for 64-bit FBX
const NULL_RECORD_SIZE: usize = 25;

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

    drop(cursor);

    ExportResult {
        success: true,
        binary_data: Some(buffer),
        error: None,
    }
}

fn write_node<W: Write + Seek>(
    writer: &mut W,
    name: &str,
    properties: &[FbxProp],
    _children: &[Vec<u8>],
) {
    write_node_with_children(writer, name, properties, &[]);
}

fn write_node_with_children<W: Write + Seek>(
    writer: &mut W,
    name: &str,
    properties: &[FbxProp],
    children: &[Vec<u8>],
) {
    let start_pos = writer.stream_position().unwrap();

    // Reserve space for header (25 bytes for 64-bit)
    let _ = writer.write_all(&[0u8; 25]);

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
        let _ = writer.write_all(child);
    }

    // Write null record if we have children
    if !children.is_empty() {
        let _ = writer.write_all(&[0u8; NULL_RECORD_SIZE]);
    }

    let end_pos = writer.stream_position().unwrap();

    // Go back and fill in header
    let _ = writer.seek(SeekFrom::Start(start_pos));
    let _ = writer.write_all(&(end_pos as u64).to_le_bytes()); // end offset
    let _ = writer.write_all(&(properties.len() as u64).to_le_bytes()); // num properties
    let _ = writer.write_all(&(props_len as u64).to_le_bytes()); // properties list len
    let _ = writer.write_all(&[name.len() as u8]); // name len

    // Seek back to end
    let _ = writer.seek(SeekFrom::Start(end_pos));
}

#[derive(Clone)]
#[allow(dead_code)]
enum FbxProp {
    I64(i64),
    F64(f64),
    String(String),
    F64Array(Vec<f64>),
    I32Array(Vec<i32>),
}

fn write_property<W: Write>(writer: &mut W, prop: &FbxProp) {
    match prop {
        FbxProp::I64(v) => {
            let _ = writer.write_all(&[b'L']);
            let _ = writer.write_all(&v.to_le_bytes());
        }
        FbxProp::F64(v) => {
            let _ = writer.write_all(&[b'D']);
            let _ = writer.write_all(&v.to_le_bytes());
        }
        FbxProp::String(s) => {
            let _ = writer.write_all(&[b'S']);
            let _ = writer.write_all(&(s.len() as u32).to_le_bytes());
            let _ = writer.write_all(s.as_bytes());
        }
        FbxProp::F64Array(arr) => {
            let _ = writer.write_all(&[b'd']);
            let _ = writer.write_all(&(arr.len() as u32).to_le_bytes());
            let _ = writer.write_all(&0u32.to_le_bytes()); // encoding (0 = uncompressed)
            let _ = writer.write_all(&((arr.len() * 8) as u32).to_le_bytes()); // byte length
            for v in arr {
                let _ = writer.write_all(&v.to_le_bytes());
            }
        }
        FbxProp::I32Array(arr) => {
            let _ = writer.write_all(&[b'i']);
            let _ = writer.write_all(&(arr.len() as u32).to_le_bytes());
            let _ = writer.write_all(&0u32.to_le_bytes()); // encoding
            let _ = writer.write_all(&((arr.len() * 4) as u32).to_le_bytes());
            for v in arr {
                let _ = writer.write_all(&v.to_le_bytes());
            }
        }
    }
}

fn write_header_extension<W: Write + Seek>(writer: &mut W, version: u32) {
    let mut children: Vec<Vec<u8>> = Vec::new();

    // FBXHeaderVersion
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "FBXHeaderVersion",
        &[FbxProp::I64(1003)],
        &[],
    );
    children.push(buf);

    // FBXVersion
    let mut buf = Vec::new();
    write_node(
        &mut Cursor::new(&mut buf),
        "FBXVersion",
        &[FbxProp::I64(version as i64)],
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

fn write_global_settings<W: Write + Seek>(writer: &mut W) {
    write_node(writer, "GlobalSettings", &[], &[]);
}

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

fn write_geometry<W: Write + Seek>(writer: &mut W, mesh: &MeshInput, id: i64) {
    let name = mesh.name.as_deref().unwrap_or("Mesh");
    let full_name = format!("Geometry::{}", name);

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

    // Normals (if present)
    if let Some(ref normals) = mesh.normals {
        if !normals.is_empty() {
            let norm_doubles: Vec<f64> = normals.iter().map(|&v| v as f64).collect();

            let mut layer_children: Vec<Vec<u8>> = Vec::new();

            let mut buf = Vec::new();
            write_node(
                &mut Cursor::new(&mut buf),
                "Version",
                &[FbxProp::I64(101)],
                &[],
            );
            layer_children.push(buf);

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
                &[FbxProp::I64(0)],
                &layer_children,
            );
            children.push(buf);
        }
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

fn write_model<W: Write + Seek>(writer: &mut W, mesh: &MeshInput, id: i64) {
    let name = mesh.name.as_deref().unwrap_or("Model");
    let full_name = format!("Model::{}", name);

    write_node(
        writer,
        "Model",
        &[
            FbxProp::I64(id),
            FbxProp::String(full_name),
            FbxProp::String("Mesh".to_string()),
        ],
        &[],
    );
}

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

#[cfg(test)]
mod tests {
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
    }
}
