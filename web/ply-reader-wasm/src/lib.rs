//! PLY Reader WASM module.
//!
//! Provides PLY file parsing functionality for web applications.
//! Supports ASCII and binary PLY parsing via `parse_ply_bytes`.

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::GeometryAttributeType;
use draco_core::geometry_indices::FaceIndex;
use draco_core::mesh::Mesh;
use draco_io::ply_reader::PlyReader;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

/// Mesh data structure for JavaScript interop.
#[derive(Serialize, Deserialize)]
pub struct MeshData {
    /// Vertex positions as flat array [x0, y0, z0, x1, y1, z1, ...]
    pub positions: Vec<f32>,
    /// Face indices as flat array (triangles)
    pub indices: Vec<u32>,
    /// Vertex normals (if present)
    pub normals: Vec<f32>,
    /// Vertex colors as flat array [r0, g0, b0, a0, ...] (0-255)
    pub colors: Vec<u8>,
}

/// Parse result containing meshes and any warnings/errors.
#[derive(Serialize, Deserialize)]
pub struct ParseResult {
    pub success: bool,
    pub meshes: Vec<MeshData>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    /// PLY header information
    pub header: Option<PlyHeader>,
}

/// PLY header information.
#[derive(Serialize, Deserialize, Clone)]
pub struct PlyHeader {
    pub format: String,
    pub vertex_count: usize,
    pub face_count: usize,
    pub properties: Vec<String>,
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
    "PLY Reader".to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["ply".to_string()]
}

/// Helper to convert a ParseResult to JsValue via JSON.
fn to_js_value(result: &ParseResult) -> JsValue {
    let json = serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string());
    js_sys::JSON::parse(&json).unwrap_or(JsValue::NULL)
}

/// Parse PLY file content from a string (ASCII PLY).
#[wasm_bindgen]
pub fn parse_ply(content: &str) -> JsValue {
    let result =
        parse_ply_with_core(content.as_bytes()).unwrap_or_else(|_| parse_ply_internal(content));
    to_js_value(&result)
}

/// Parse PLY file content from bytes.
#[wasm_bindgen]
pub fn parse_ply_bytes(data: &[u8]) -> JsValue {
    // Catch any panics
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        parse_ply_with_core(data).unwrap_or_else(|error| match std::str::from_utf8(data) {
            Ok(content) if is_ascii_ply(data) => parse_ply_internal(content),
            _ => ParseResult {
                success: false,
                meshes: vec![],
                error: Some(error),
                warnings: vec![],
                header: parse_header_info(data),
            },
        })
    }));

    let result = match result {
        Ok(r) => r,
        Err(e) => {
            let msg = if let Some(s) = e.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = e.downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic during PLY parsing".to_string()
            };
            ParseResult {
                success: false,
                meshes: vec![],
                error: Some(format!("Internal error: {}", msg)),
                warnings: vec![],
                header: None,
            }
        }
    };

    to_js_value(&result)
}

fn parse_ply_with_core(data: &[u8]) -> Result<ParseResult, String> {
    let mesh = PlyReader::read_from_bytes(data).map_err(|error| error.to_string())?;
    let mesh_data = mesh_to_js_data(&mesh);
    Ok(ParseResult {
        success: true,
        meshes: vec![mesh_data],
        error: None,
        warnings: vec![],
        header: parse_header_info(data),
    })
}

fn parse_header_info(data: &[u8]) -> Option<PlyHeader> {
    let header_end = data
        .windows(b"end_header".len())
        .position(|window| window == b"end_header")?;
    let text = std::str::from_utf8(&data[..header_end]).ok()?;
    let mut format = String::new();
    let mut vertex_count = 0usize;
    let mut face_count = 0usize;
    let mut properties = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.as_slice() {
            ["format", value, ..] => format = (*value).to_string(),
            ["element", "vertex", count, ..] => vertex_count = count.parse().unwrap_or(0),
            ["element", "face", count, ..] => face_count = count.parse().unwrap_or(0),
            ["property", "list", _, _, name, ..] => properties.push((*name).to_string()),
            ["property", _, name, ..] => properties.push((*name).to_string()),
            _ => {}
        }
    }
    Some(PlyHeader {
        format,
        vertex_count,
        face_count,
        properties,
    })
}

fn is_ascii_ply(data: &[u8]) -> bool {
    data.starts_with(b"ply")
        && parse_header_info(data)
            .map(|header| header.format == "ascii")
            .unwrap_or(false)
}

fn mesh_to_js_data(mesh: &Mesh) -> MeshData {
    let positions = read_attribute_as_f32(mesh, GeometryAttributeType::Position, 3);
    let normals = read_attribute_as_f32(mesh, GeometryAttributeType::Normal, 3);
    let colors = read_color_attribute(mesh);
    let mut indices = Vec::with_capacity(mesh.num_faces() * 3);
    for i in 0..mesh.num_faces() {
        let face = mesh.face(FaceIndex(i as u32));
        indices.extend([face[0].0, face[1].0, face[2].0]);
    }
    MeshData {
        positions,
        indices,
        normals,
        colors,
    }
}

fn read_attribute_as_f32(
    mesh: &Mesh,
    attribute_type: GeometryAttributeType,
    components: usize,
) -> Vec<f32> {
    let att_id = mesh.named_attribute_id(attribute_type);
    if att_id < 0 {
        return Vec::new();
    }
    let att = mesh.attribute(att_id);
    let stride = att.byte_stride() as usize;
    let count = mesh.num_points();
    let mut out = Vec::with_capacity(count * components);
    for point in 0..count {
        let base = point * stride;
        for component in 0..components.min(att.num_components() as usize) {
            let offset = base + component * att.data_type().byte_length();
            let data = att.buffer().data();
            let value = match att.data_type() {
                DataType::Float32 => {
                    f32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
                }
                DataType::Float64 => {
                    f64::from_le_bytes(data[offset..offset + 8].try_into().unwrap()) as f32
                }
                DataType::Int32 => {
                    i32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as f32
                }
                DataType::Uint32 => {
                    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap()) as f32
                }
                DataType::Int16 => {
                    i16::from_le_bytes(data[offset..offset + 2].try_into().unwrap()) as f32
                }
                DataType::Uint16 => {
                    u16::from_le_bytes(data[offset..offset + 2].try_into().unwrap()) as f32
                }
                DataType::Int8 => data[offset] as i8 as f32,
                DataType::Uint8 => data[offset] as f32,
                _ => 0.0,
            };
            out.push(value);
        }
    }
    out
}

fn read_color_attribute(mesh: &Mesh) -> Vec<u8> {
    let att_id = mesh.named_attribute_id(GeometryAttributeType::Color);
    if att_id < 0 {
        return Vec::new();
    }
    let att = mesh.attribute(att_id);
    if att.data_type() == DataType::Uint8 {
        return att.buffer().data().to_vec();
    }
    Vec::new()
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PlyProperty {
    name: String,
    data_type: String,
    is_list: bool,
    list_count_type: Option<String>,
    list_elem_type: Option<String>,
}

fn parse_ply_internal(content: &str) -> ParseResult {
    let mut lines = content.lines().peekable();
    let mut warnings: Vec<String> = Vec::new();

    // Parse header
    let first_line = lines.next().unwrap_or("").trim();
    if first_line != "ply" {
        return ParseResult {
            success: false,
            meshes: vec![],
            error: Some("Invalid PLY file: missing 'ply' header".to_string()),
            warnings: vec![],
            header: None,
        };
    }

    let mut format = String::new();
    let mut vertex_count = 0usize;
    let mut face_count = 0usize;
    let mut vertex_properties: Vec<PlyProperty> = Vec::new();
    let mut face_properties: Vec<PlyProperty> = Vec::new();
    let mut current_element = String::new();
    let mut property_names: Vec<String> = Vec::new();

    // Parse header lines
    loop {
        let line = match lines.next() {
            Some(l) => l.trim(),
            None => break,
        };

        if line == "end_header" {
            break;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "format" => {
                if parts.len() >= 2 {
                    format = parts[1].to_string();
                }
            }
            "element" => {
                if parts.len() >= 3 {
                    current_element = parts[1].to_string();
                    let count: usize = parts[2].parse().unwrap_or(0);
                    if current_element == "vertex" {
                        vertex_count = count;
                    } else if current_element == "face" {
                        face_count = count;
                    }
                }
            }
            "property" => {
                if parts.len() >= 3 {
                    let prop = if parts[1] == "list" && parts.len() >= 5 {
                        PlyProperty {
                            name: parts[4].to_string(),
                            data_type: parts[3].to_string(),
                            is_list: true,
                            list_count_type: Some(parts[2].to_string()),
                            list_elem_type: Some(parts[3].to_string()),
                        }
                    } else {
                        PlyProperty {
                            name: parts[2].to_string(),
                            data_type: parts[1].to_string(),
                            is_list: false,
                            list_count_type: None,
                            list_elem_type: None,
                        }
                    };

                    property_names.push(prop.name.clone());

                    if current_element == "vertex" {
                        vertex_properties.push(prop);
                    } else if current_element == "face" {
                        face_properties.push(prop);
                    }
                }
            }
            _ => {}
        }
    }

    if format != "ascii" {
        return ParseResult {
            success: false,
            meshes: vec![],
            error: Some(format!("Binary PLY format '{}' not supported via string parsing. Use parse_ply_bytes for binary files.", format)),
            warnings: vec![],
            header: Some(PlyHeader {
                format,
                vertex_count,
                face_count,
                properties: property_names,
            }),
        };
    }

    // Find property indices
    let x_idx = vertex_properties.iter().position(|p| p.name == "x");
    let y_idx = vertex_properties.iter().position(|p| p.name == "y");
    let z_idx = vertex_properties.iter().position(|p| p.name == "z");
    let nx_idx = vertex_properties.iter().position(|p| p.name == "nx");
    let ny_idx = vertex_properties.iter().position(|p| p.name == "ny");
    let nz_idx = vertex_properties.iter().position(|p| p.name == "nz");
    let r_idx = vertex_properties
        .iter()
        .position(|p| p.name == "red" || p.name == "r");
    let g_idx = vertex_properties
        .iter()
        .position(|p| p.name == "green" || p.name == "g");
    let b_idx = vertex_properties
        .iter()
        .position(|p| p.name == "blue" || p.name == "b");
    let a_idx = vertex_properties
        .iter()
        .position(|p| p.name == "alpha" || p.name == "a");

    let has_positions = x_idx.is_some() && y_idx.is_some() && z_idx.is_some();
    let has_normals = nx_idx.is_some() && ny_idx.is_some() && nz_idx.is_some();
    let has_colors = r_idx.is_some() && g_idx.is_some() && b_idx.is_some();

    if !has_positions {
        return ParseResult {
            success: false,
            meshes: vec![],
            error: Some("PLY file missing position properties (x, y, z)".to_string()),
            warnings: vec![],
            header: Some(PlyHeader {
                format,
                vertex_count,
                face_count,
                properties: property_names,
            }),
        };
    }

    let mut positions: Vec<f32> = Vec::with_capacity(vertex_count * 3);
    let mut normals: Vec<f32> = Vec::new();
    let mut colors: Vec<u8> = Vec::new();

    if has_normals {
        normals.reserve(vertex_count * 3);
    }
    if has_colors {
        colors.reserve(vertex_count * 4);
    }

    // Parse vertices
    for i in 0..vertex_count {
        let line = match lines.next() {
            Some(l) => l.trim(),
            None => {
                warnings.push(format!("Unexpected end of file at vertex {}", i));
                break;
            }
        };

        let values: Vec<f32> = line
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();

        if let (Some(xi), Some(yi), Some(zi)) = (x_idx, y_idx, z_idx) {
            positions.push(*values.get(xi).unwrap_or(&0.0));
            positions.push(*values.get(yi).unwrap_or(&0.0));
            positions.push(*values.get(zi).unwrap_or(&0.0));
        }

        if has_normals {
            if let (Some(nxi), Some(nyi), Some(nzi)) = (nx_idx, ny_idx, nz_idx) {
                normals.push(*values.get(nxi).unwrap_or(&0.0));
                normals.push(*values.get(nyi).unwrap_or(&0.0));
                normals.push(*values.get(nzi).unwrap_or(&0.0));
            }
        }

        if has_colors {
            if let (Some(ri), Some(gi), Some(bi)) = (r_idx, g_idx, b_idx) {
                colors.push((*values.get(ri).unwrap_or(&255.0)) as u8);
                colors.push((*values.get(gi).unwrap_or(&255.0)) as u8);
                colors.push((*values.get(bi).unwrap_or(&255.0)) as u8);
                if let Some(ai) = a_idx {
                    colors.push((*values.get(ai).unwrap_or(&255.0)) as u8);
                } else {
                    colors.push(255);
                }
            }
        }
    }

    // Parse faces
    let mut indices: Vec<u32> = Vec::with_capacity(face_count * 3);

    for i in 0..face_count {
        let line = match lines.next() {
            Some(l) => l.trim(),
            None => {
                warnings.push(format!("Unexpected end of file at face {}", i));
                break;
            }
        };

        let values: Vec<u32> = line
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();

        if values.is_empty() {
            continue;
        }

        let count = values[0] as usize;
        if values.len() < count + 1 {
            warnings.push(format!("Face {} has incomplete indices", i));
            continue;
        }

        // Triangulate (fan triangulation for polygons)
        for j in 1..count - 1 {
            indices.push(values[1]);
            indices.push(values[j + 1]);
            indices.push(values[j + 2]);
        }
    }

    let mesh = MeshData {
        positions,
        indices,
        normals,
        colors,
    };

    ParseResult {
        success: true,
        meshes: vec![mesh],
        error: None,
        warnings,
        header: Some(PlyHeader {
            format,
            vertex_count,
            face_count,
            properties: property_names,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_ply_quad(format: &str) -> Vec<u8> {
        let mut ply = format!(
            "ply\nformat {format} 1.0\nelement vertex 4\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n"
        )
        .into_bytes();

        for vertex in [
            [0.0f32, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ] {
            for component in vertex {
                if format == "binary_big_endian" {
                    ply.extend_from_slice(&component.to_be_bytes());
                } else {
                    ply.extend_from_slice(&component.to_le_bytes());
                }
            }
        }

        ply.push(4);
        for index in [0i32, 1, 2, 3] {
            if format == "binary_big_endian" {
                ply.extend_from_slice(&index.to_be_bytes());
            } else {
                ply.extend_from_slice(&index.to_le_bytes());
            }
        }

        ply
    }

    fn assert_binary_quad_result(result: ParseResult, format: &str) {
        assert!(
            result.success,
            "binary PLY parse should succeed: {:?}",
            result.error
        );
        assert_eq!(result.meshes.len(), 1);

        let mesh = &result.meshes[0];
        assert_eq!(
            mesh.positions,
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0]
        );
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);

        let header = result.header.expect("header should be parsed");
        assert_eq!(header.format, format);
        assert_eq!(header.vertex_count, 4);
        assert_eq!(header.face_count, 1);
    }

    #[test]
    fn test_parse_simple_ply() {
        let ply = r#"ply
format ascii 1.0
element vertex 3
property float x
property float y
property float z
element face 1
property list uchar int vertex_indices
end_header
0 0 0
1 0 0
0.5 1 0
3 0 1 2
"#;

        let result = parse_ply_internal(ply);
        assert!(result.success);
        assert_eq!(result.meshes.len(), 1);
        assert_eq!(result.meshes[0].positions.len(), 9); // 3 vertices * 3 components
        assert_eq!(result.meshes[0].indices.len(), 3); // 1 triangle
    }

    #[test]
    fn test_parse_bunny_ply() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/bun_zipper.ply");
        let content = std::fs::read_to_string(&path).expect("Failed to read bunny PLY");
        println!("Content length: {} bytes", content.len());

        let result = parse_ply_internal(&content);
        println!("Result success: {}", result.success);
        if let Some(ref err) = result.error {
            println!("Error: {}", err);
        }
        for w in &result.warnings {
            println!("Warning: {}", w);
        }
        if let Some(ref header) = result.header {
            println!(
                "Header: {} vertices, {} faces",
                header.vertex_count, header.face_count
            );
        }
        if !result.meshes.is_empty() {
            let mesh = &result.meshes[0];
            println!(
                "Mesh: {} positions, {} indices",
                mesh.positions.len(),
                mesh.indices.len()
            );
        }

        assert!(result.success, "Parsing should succeed");
        assert_eq!(result.meshes.len(), 1, "Should have 1 mesh");
        assert_eq!(
            result.meshes[0].positions.len(),
            35947 * 3,
            "Should have 35947 vertices"
        );
        assert_eq!(
            result.meshes[0].indices.len(),
            69451 * 3,
            "Should have 69451 faces (triangulated)"
        );
    }

    #[test]
    fn test_parse_binary_little_endian_ply_bytes() {
        let data = binary_ply_quad("binary_little_endian");
        let result = parse_ply_with_core(&data).expect("little-endian binary PLY should parse");
        assert_binary_quad_result(result, "binary_little_endian");
    }

    #[test]
    fn test_parse_binary_big_endian_ply_bytes() {
        let data = binary_ply_quad("binary_big_endian");
        let result = parse_ply_with_core(&data).expect("big-endian binary PLY should parse");
        assert_binary_quad_result(result, "binary_big_endian");
    }
}
