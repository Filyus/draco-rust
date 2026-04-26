//! PLY Writer WASM module.
//!
//! Provides PLY file generation functionality for web applications.

use draco_core::draco_types::DataType;
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
use draco_core::geometry_indices::{FaceIndex, PointIndex};
use draco_core::mesh::Mesh;
use draco_io::{PlyFormat, PlyWriter, Writer};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// Input mesh data from JavaScript.
#[derive(Serialize, Deserialize, Clone)]
pub struct MeshInput {
    /// Vertex positions as flat array [x0, y0, z0, x1, y1, z1, ...]
    pub positions: Vec<f32>,
    /// Face indices as flat array (triangles)
    pub indices: Vec<u32>,
    /// Vertex normals (optional)
    pub normals: Option<Vec<f32>>,
    /// Vertex colors as [r, g, b, a, ...] 0-255 (optional)
    pub colors: Option<Vec<u8>>,
}

/// Export options.
#[derive(Serialize, Deserialize, Default)]
pub struct ExportOptions {
    /// Include normals in output
    pub include_normals: Option<bool>,
    /// Include colors in output
    pub include_colors: Option<bool>,
    /// Decimal precision for coordinates
    pub precision: Option<u32>,
    /// Output format: "ascii" or "binary_little_endian"
    pub format: Option<String>,
}

/// Export result.
#[derive(Serialize, Deserialize)]
pub struct ExportResult {
    pub success: bool,
    pub data: Option<String>,
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
    "PLY Writer".to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["ply".to_string()]
}

/// Create PLY content from mesh data.
#[wasm_bindgen]
pub fn create_ply(mesh_js: JsValue, options_js: JsValue) -> JsValue {
    let mesh: MeshInput = match serde_wasm_bindgen::from_value(mesh_js) {
        Ok(m) => m,
        Err(e) => {
            let result = ExportResult {
                success: false,
                data: None,
                binary_data: None,
                error: Some(format!("Invalid mesh data: {}", e)),
            };
            return serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL);
        }
    };

    let options: ExportOptions = serde_wasm_bindgen::from_value(options_js).unwrap_or_default();
    let result = create_ply_internal(&mesh, &options);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

fn create_ply_internal(mesh: &MeshInput, options: &ExportOptions) -> ExportResult {
    let format = options.format.as_deref().unwrap_or("ascii");
    let ply_format = match format {
        "ascii" => PlyFormat::Ascii,
        "binary_little_endian" => PlyFormat::BinaryLittleEndian,
        "binary_big_endian" => PlyFormat::BinaryBigEndian,
        other => {
            return ExportResult {
                success: false,
                data: None,
                binary_data: None,
                error: Some(format!("Unsupported PLY format: {}", other)),
            };
        }
    };

    match create_ply_with_core(mesh, options, ply_format) {
        Ok(bytes) => {
            if ply_format == PlyFormat::Ascii {
                match String::from_utf8(bytes) {
                    Ok(data) => ExportResult {
                        success: true,
                        data: Some(data),
                        binary_data: None,
                        error: None,
                    },
                    Err(error) => ExportResult {
                        success: false,
                        data: None,
                        binary_data: None,
                        error: Some(error.to_string()),
                    },
                }
            } else {
                ExportResult {
                    success: true,
                    data: None,
                    binary_data: Some(bytes),
                    error: None,
                }
            }
        }
        Err(error) => ExportResult {
            success: false,
            data: None,
            binary_data: None,
            error: Some(error),
        },
    }
}

fn create_ply_with_core(
    input: &MeshInput,
    options: &ExportOptions,
    format: PlyFormat,
) -> Result<Vec<u8>, String> {
    let mesh = mesh_input_to_core_mesh(input, options)?;
    let mut writer = PlyWriter::new().with_format(format);
    Writer::add_mesh(&mut writer, &mesh, None).map_err(|error| error.to_string())?;
    writer.write_to_vec().map_err(|error| error.to_string())
}

fn mesh_input_to_core_mesh(input: &MeshInput, options: &ExportOptions) -> Result<Mesh, String> {
    if input.positions.len() % 3 != 0 {
        return Err("positions length must be divisible by 3".to_string());
    }
    if input.indices.len() % 3 != 0 {
        return Err("indices length must be divisible by 3".to_string());
    }

    let vertex_count = input.positions.len() / 3;
    let mut mesh = Mesh::new();
    mesh.set_num_points(vertex_count);

    let mut pos_att = PointAttribute::new();
    pos_att.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        vertex_count,
    );
    for (i, chunk) in input.positions.chunks_exact(3).enumerate() {
        let bytes: Vec<u8> = chunk.iter().flat_map(|value| value.to_le_bytes()).collect();
        pos_att.buffer_mut().write(i * 12, &bytes);
    }
    mesh.add_attribute(pos_att);

    if options.include_normals.unwrap_or(true) {
        if let Some(normals) = &input.normals {
            if normals.len() >= vertex_count * 3 {
                let mut normal_att = PointAttribute::new();
                normal_att.init(
                    GeometryAttributeType::Normal,
                    3,
                    DataType::Float32,
                    false,
                    vertex_count,
                );
                for (i, chunk) in normals.chunks_exact(3).take(vertex_count).enumerate() {
                    let bytes: Vec<u8> =
                        chunk.iter().flat_map(|value| value.to_le_bytes()).collect();
                    normal_att.buffer_mut().write(i * 12, &bytes);
                }
                mesh.add_attribute(normal_att);
            }
        }
    }

    if options.include_colors.unwrap_or(true) {
        if let Some(colors) = &input.colors {
            if colors.len() >= vertex_count * 4 {
                let mut color_att = PointAttribute::new();
                color_att.init(
                    GeometryAttributeType::Color,
                    4,
                    DataType::Uint8,
                    true,
                    vertex_count,
                );
                color_att.buffer_mut().write(0, &colors[..vertex_count * 4]);
                mesh.add_attribute(color_att);
            }
        }
    }

    mesh.set_num_faces(input.indices.len() / 3);
    for (i, chunk) in input.indices.chunks_exact(3).enumerate() {
        mesh.set_face(
            FaceIndex(i as u32),
            [
                PointIndex(chunk[0]),
                PointIndex(chunk[1]),
                PointIndex(chunk[2]),
            ],
        );
    }
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_simple_ply() {
        let mesh = MeshInput {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0],
            indices: vec![0, 1, 2],
            normals: None,
            colors: None,
        };

        let result = create_ply_internal(&mesh, &ExportOptions::default());
        assert!(result.success);
        assert!(result.data.is_some());
        let data = result.data.unwrap();
        assert!(data.contains("ply"));
        assert!(data.contains("element vertex 3"));
        assert!(data.contains("element face 1"));
    }
}
