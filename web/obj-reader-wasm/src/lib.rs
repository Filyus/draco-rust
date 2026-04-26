//! OBJ Reader WASM module.
//!
//! Provides OBJ file parsing functionality for web applications.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// Mesh data structure for JavaScript interop.
#[derive(Serialize, Deserialize)]
pub struct MeshData {
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
    "OBJ Reader".to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["obj".to_string()]
}

/// Parse OBJ file content from a string.
#[wasm_bindgen]
pub fn parse_obj(content: &str) -> JsValue {
    let result = parse_obj_internal(content);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Parse OBJ file content from bytes.
#[wasm_bindgen]
pub fn parse_obj_bytes(data: &[u8]) -> JsValue {
    match std::str::from_utf8(data) {
        Ok(content) => parse_obj(content),
        Err(e) => {
            let result = ParseResult {
                success: false,
                meshes: vec![],
                error: Some(format!("Invalid UTF-8 content: {}", e)),
                warnings: vec![],
            };
            serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
        }
    }
}

fn parse_obj_internal(content: &str) -> ParseResult {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut texcoords: Vec<[f32; 2]> = Vec::new();
    let mut faces: Vec<Vec<(usize, Option<usize>, Option<usize>)>> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "v" => {
                if parts.len() >= 4 {
                    if let (Ok(x), Ok(y), Ok(z)) = (
                        parts[1].parse::<f32>(),
                        parts[2].parse::<f32>(),
                        parts[3].parse::<f32>(),
                    ) {
                        positions.push([x, y, z]);
                    } else {
                        warnings.push(format!("Line {}: Invalid vertex coordinates", line_num + 1));
                    }
                }
            }
            "vn" => {
                if parts.len() >= 4 {
                    if let (Ok(x), Ok(y), Ok(z)) = (
                        parts[1].parse::<f32>(),
                        parts[2].parse::<f32>(),
                        parts[3].parse::<f32>(),
                    ) {
                        normals.push([x, y, z]);
                    }
                }
            }
            "vt" => {
                if parts.len() >= 3 {
                    if let (Ok(u), Ok(v)) = (parts[1].parse::<f32>(), parts[2].parse::<f32>()) {
                        texcoords.push([u, v]);
                    }
                }
            }
            "f" => {
                let mut face_verts: Vec<(usize, Option<usize>, Option<usize>)> = Vec::new();
                for i in 1..parts.len() {
                    let indices: Vec<&str> = parts[i].split('/').collect();
                    let vi: usize = indices[0].parse::<usize>().unwrap_or(1) - 1;
                    let ti: Option<usize> = indices
                        .get(1)
                        .and_then(|s| {
                            if s.is_empty() {
                                None
                            } else {
                                s.parse::<usize>().ok()
                            }
                        })
                        .map(|i| i - 1);
                    let ni: Option<usize> = indices
                        .get(2)
                        .and_then(|s| s.parse::<usize>().ok())
                        .map(|i| i - 1);
                    face_verts.push((vi, ti, ni));
                }
                if face_verts.len() >= 3 {
                    faces.push(face_verts);
                }
            }
            _ => {} // Ignore other directives
        }
    }

    // Convert to indexed mesh (triangulate if needed)
    let mut out_positions: Vec<f32> = Vec::new();
    let mut out_normals: Vec<f32> = Vec::new();
    let mut out_uvs: Vec<f32> = Vec::new();
    let mut out_indices: Vec<u32> = Vec::new();

    // Simple approach: expand all vertices (no deduplication for simplicity)
    let mut vertex_count: u32 = 0;
    for face in &faces {
        // Triangulate polygon (fan triangulation)
        for i in 1..face.len() - 1 {
            let triangle = [&face[0], &face[i], &face[i + 1]];
            for &(vi, ti, ni) in &triangle {
                if *vi < positions.len() {
                    out_positions.extend_from_slice(&positions[*vi]);
                } else {
                    out_positions.extend_from_slice(&[0.0, 0.0, 0.0]);
                    warnings.push(format!("Invalid vertex index: {}", vi + 1));
                }

                if let Some(ni) = ni {
                    if *ni < normals.len() {
                        out_normals.extend_from_slice(&normals[*ni]);
                    } else {
                        out_normals.extend_from_slice(&[0.0, 0.0, 0.0]);
                    }
                }

                if let Some(ti) = ti {
                    if *ti < texcoords.len() {
                        out_uvs.extend_from_slice(&texcoords[*ti]);
                    } else {
                        out_uvs.extend_from_slice(&[0.0, 0.0]);
                    }
                }

                out_indices.push(vertex_count);
                vertex_count += 1;
            }
        }
    }

    let mesh = MeshData {
        positions: out_positions,
        indices: out_indices,
        normals: out_normals,
        uvs: out_uvs,
    };

    ParseResult {
        success: true,
        meshes: vec![mesh],
        error: None,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_obj() {
        let obj = r#"
# Simple cube
v 0 0 0
v 1 0 0
v 1 1 0
v 0 1 0
f 1 2 3
f 1 3 4
        "#;

        let result = parse_obj_internal(obj);
        assert!(result.success);
        assert_eq!(result.meshes.len(), 1);
        assert_eq!(result.meshes[0].positions.len(), 18); // 6 vertices * 3 components
        assert_eq!(result.meshes[0].indices.len(), 6); // 2 triangles * 3 indices
    }
}
