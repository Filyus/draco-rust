//! OBJ reader and writer WASM module.
//!
//! Provides OBJ parsing and generation for web applications. The reader and
//! writer are independent: build with `--features read` or `--features write`
//! (both are on by default) to control which half of the API is exported.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

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
    vec!["obj".to_string()]
}

// ===========================================================================
// Reader
// ===========================================================================

/// Mesh data produced by the OBJ reader, for JavaScript interop.
#[cfg(feature = "read")]
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
    /// Name selected by the most recent OBJ `usemtl` directive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
}

/// Parse result containing meshes and any warnings/errors.
#[cfg(feature = "read")]
#[derive(Serialize, Deserialize)]
pub struct ParseResult {
    pub success: bool,
    pub meshes: Vec<MeshData>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
}

/// Parse OBJ file content from a string.
#[cfg(feature = "read")]
#[wasm_bindgen]
pub fn parse_obj(content: &str) -> JsValue {
    let result = parse_obj_internal(content);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Parse OBJ file content from bytes.
#[cfg(feature = "read")]
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

#[cfg(feature = "read")]
type ObjVertexRef = (usize, Option<usize>, Option<usize>);

#[cfg(feature = "read")]
struct ObjFace {
    vertices: Vec<ObjVertexRef>,
    material: Option<String>,
}

#[cfg(feature = "read")]
fn parse_obj_internal(content: &str) -> ParseResult {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut texcoords: Vec<[f32; 2]> = Vec::new();
    let mut faces: Vec<ObjFace> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut current_material = None;

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
            "v" if parts.len() >= 4 => {
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
            "vn" if parts.len() >= 4 => {
                if let (Ok(x), Ok(y), Ok(z)) = (
                    parts[1].parse::<f32>(),
                    parts[2].parse::<f32>(),
                    parts[3].parse::<f32>(),
                ) {
                    normals.push([x, y, z]);
                }
            }
            "vt" if parts.len() >= 3 => {
                if let (Ok(u), Ok(v)) = (parts[1].parse::<f32>(), parts[2].parse::<f32>()) {
                    texcoords.push([u, v]);
                }
            }
            "usemtl" => {
                current_material = (!parts[1..].is_empty()).then(|| parts[1..].join(" "));
            }
            "f" => {
                let mut face_verts: Vec<ObjVertexRef> = Vec::new();
                for part in parts.iter().skip(1) {
                    let indices: Vec<&str> = part.split('/').collect();
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
                    faces.push(ObjFace {
                        vertices: face_verts,
                        material: current_material.clone(),
                    });
                }
            }
            _ => {} // Ignore other directives
        }
    }

    let mut face_groups: Vec<(Option<String>, Vec<Vec<ObjVertexRef>>)> = Vec::new();
    for face in faces {
        if let Some((_, group)) = face_groups
            .iter_mut()
            .find(|(material, _)| *material == face.material)
        {
            group.push(face.vertices);
        } else {
            face_groups.push((face.material, vec![face.vertices]));
        }
    }

    let mut meshes = Vec::with_capacity(face_groups.len().max(1));
    for (material, faces) in face_groups {
        meshes.push(build_mesh(
            &faces,
            &positions,
            &normals,
            &texcoords,
            &mut warnings,
            material,
        ));
    }
    if meshes.is_empty() {
        meshes.push(MeshData {
            positions: Vec::new(),
            indices: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            material: None,
        });
    }

    ParseResult {
        success: true,
        meshes,
        error: None,
        warnings,
    }
}

#[cfg(feature = "read")]
fn build_mesh(
    faces: &[Vec<ObjVertexRef>],
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    texcoords: &[[f32; 2]],
    warnings: &mut Vec<String>,
    material: Option<String>,
) -> MeshData {
    // Convert to indexed mesh (triangulate if needed).
    let mut out_positions: Vec<f32> = Vec::new();
    let mut out_normals: Vec<f32> = Vec::new();
    let mut out_uvs: Vec<f32> = Vec::new();
    let mut out_indices: Vec<u32> = Vec::new();

    // Simple approach: expand all vertices (no deduplication for simplicity)
    let mut vertex_count: u32 = 0;
    for face in faces {
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

    MeshData {
        positions: out_positions,
        indices: out_indices,
        normals: out_normals,
        uvs: out_uvs,
        material,
    }
}

// ===========================================================================
// Writer
// ===========================================================================

/// Input mesh data consumed by the OBJ writer, from JavaScript.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize)]
pub struct MeshInput {
    /// Vertex positions as flat array [x0, y0, z0, x1, y1, z1, ...]
    pub positions: Vec<f32>,
    /// Face indices as flat array (triangles)
    pub indices: Vec<u32>,
    /// Vertex normals (optional)
    pub normals: Option<Vec<f32>>,
    /// Texture coordinates (optional)
    pub uvs: Option<Vec<f32>>,
    /// Mesh name (optional)
    pub name: Option<String>,
}

/// Export options.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Default)]
pub struct ExportOptions {
    /// Include normals in output
    pub include_normals: Option<bool>,
    /// Include UVs in output
    pub include_uvs: Option<bool>,
    /// Decimal precision for coordinates
    pub precision: Option<u32>,
}

/// Export result.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize)]
pub struct ExportResult {
    pub success: bool,
    pub data: Option<String>,
    pub error: Option<String>,
}

/// Create OBJ content from mesh data.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_obj(mesh_js: JsValue, options_js: JsValue) -> JsValue {
    let mesh: MeshInput = match serde_wasm_bindgen::from_value(mesh_js) {
        Ok(m) => m,
        Err(e) => {
            let result = ExportResult {
                success: false,
                data: None,
                error: Some(format!("Invalid mesh data: {}", e)),
            };
            return serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL);
        }
    };

    let options: ExportOptions = serde_wasm_bindgen::from_value(options_js).unwrap_or_default();
    let result = create_obj_internal(&mesh, &options);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

/// Create OBJ content from multiple meshes.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_obj_multi(meshes_js: JsValue, options_js: JsValue) -> JsValue {
    let meshes: Vec<MeshInput> = match serde_wasm_bindgen::from_value(meshes_js) {
        Ok(m) => m,
        Err(e) => {
            let result = ExportResult {
                success: false,
                data: None,
                error: Some(format!("Invalid mesh data: {}", e)),
            };
            return serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL);
        }
    };

    let options: ExportOptions = serde_wasm_bindgen::from_value(options_js).unwrap_or_default();
    let result = create_obj_multi_internal(&meshes, &options);
    serde_wasm_bindgen::to_value(&result).unwrap_or(JsValue::NULL)
}

#[cfg(feature = "write")]
fn create_obj_internal(mesh: &MeshInput, options: &ExportOptions) -> ExportResult {
    create_obj_multi_internal(std::slice::from_ref(mesh), options)
}

#[cfg(feature = "write")]
fn create_obj_multi_internal(meshes: &[MeshInput], options: &ExportOptions) -> ExportResult {
    let precision = options.precision.unwrap_or(6) as usize;
    let include_normals = options.include_normals.unwrap_or(true);
    let include_uvs = options.include_uvs.unwrap_or(true);

    let mut output = String::new();
    output.push_str("# OBJ file generated by draco-io WASM\n");
    output.push_str(&format!("# Meshes: {}\n\n", meshes.len()));

    let mut vertex_offset: u32 = 0;
    let mut normal_offset: u32 = 0;
    let mut uv_offset: u32 = 0;

    for (mesh_idx, mesh) in meshes.iter().enumerate() {
        // Write object/group name
        let default_name = format!("mesh_{}", mesh_idx);
        let name = mesh.name.as_deref().unwrap_or(&default_name);
        output.push_str(&format!("o {}\n", name));

        // Write vertices
        let vertex_count = mesh.positions.len() / 3;
        for i in 0..vertex_count {
            let x = mesh.positions[i * 3];
            let y = mesh.positions[i * 3 + 1];
            let z = mesh.positions[i * 3 + 2];
            output.push_str(&format!(
                "v {:.*} {:.*} {:.*}\n",
                precision, x, precision, y, precision, z
            ));
        }

        // Write normals
        let mut has_normals = false;
        if include_normals {
            if let Some(ref normals) = mesh.normals {
                if !normals.is_empty() {
                    has_normals = true;
                    let normal_count = normals.len() / 3;
                    for i in 0..normal_count {
                        let nx = normals[i * 3];
                        let ny = normals[i * 3 + 1];
                        let nz = normals[i * 3 + 2];
                        output.push_str(&format!(
                            "vn {:.*} {:.*} {:.*}\n",
                            precision, nx, precision, ny, precision, nz
                        ));
                    }
                }
            }
        }

        // Write UVs
        let mut has_uvs = false;
        if include_uvs {
            if let Some(ref uvs) = mesh.uvs {
                if !uvs.is_empty() {
                    has_uvs = true;
                    let uv_count = uvs.len() / 2;
                    for i in 0..uv_count {
                        let u = uvs[i * 2];
                        let v = uvs[i * 2 + 1];
                        output.push_str(&format!("vt {:.*} {:.*}\n", precision, u, precision, v));
                    }
                }
            }
        }

        // Write faces
        let face_count = mesh.indices.len() / 3;
        for i in 0..face_count {
            let i0 = mesh.indices[i * 3] + vertex_offset + 1;
            let i1 = mesh.indices[i * 3 + 1] + vertex_offset + 1;
            let i2 = mesh.indices[i * 3 + 2] + vertex_offset + 1;

            if has_normals && has_uvs {
                // f v/vt/vn
                let n0 = mesh.indices[i * 3] + normal_offset + 1;
                let n1 = mesh.indices[i * 3 + 1] + normal_offset + 1;
                let n2 = mesh.indices[i * 3 + 2] + normal_offset + 1;
                let t0 = mesh.indices[i * 3] + uv_offset + 1;
                let t1 = mesh.indices[i * 3 + 1] + uv_offset + 1;
                let t2 = mesh.indices[i * 3 + 2] + uv_offset + 1;
                output.push_str(&format!(
                    "f {}/{}/{} {}/{}/{} {}/{}/{}\n",
                    i0, t0, n0, i1, t1, n1, i2, t2, n2
                ));
            } else if has_normals {
                // f v//vn
                let n0 = mesh.indices[i * 3] + normal_offset + 1;
                let n1 = mesh.indices[i * 3 + 1] + normal_offset + 1;
                let n2 = mesh.indices[i * 3 + 2] + normal_offset + 1;
                output.push_str(&format!("f {}//{} {}//{} {}//{}\n", i0, n0, i1, n1, i2, n2));
            } else if has_uvs {
                // f v/vt
                let t0 = mesh.indices[i * 3] + uv_offset + 1;
                let t1 = mesh.indices[i * 3 + 1] + uv_offset + 1;
                let t2 = mesh.indices[i * 3 + 2] + uv_offset + 1;
                output.push_str(&format!("f {}/{} {}/{} {}/{}\n", i0, t0, i1, t1, i2, t2));
            } else {
                // f v
                output.push_str(&format!("f {} {} {}\n", i0, i1, i2));
            }
        }

        // Update offsets for next mesh
        vertex_offset += vertex_count as u32;
        if has_normals {
            if let Some(ref normals) = mesh.normals {
                normal_offset += (normals.len() / 3) as u32;
            }
        }
        if has_uvs {
            if let Some(ref uvs) = mesh.uvs {
                uv_offset += (uvs.len() / 2) as u32;
            }
        }

        output.push('\n');
    }

    ExportResult {
        success: true,
        data: Some(output),
        error: None,
    }
}

#[cfg(feature = "write")]
impl Clone for MeshInput {
    fn clone(&self) -> Self {
        Self {
            positions: self.positions.clone(),
            indices: self.indices.clone(),
            normals: self.normals.clone(),
            uvs: self.uvs.clone(),
            name: self.name.clone(),
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(all(test, feature = "read"))]
mod reader_tests {
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

    #[test]
    fn test_parse_sequential_normal_fixture() {
        let result = parse_obj_internal(include_str!("../../../testdata/test_nm_seq_100.obj"));

        assert!(result.success);
        assert_eq!(result.meshes.len(), 1);
        assert!(!result.meshes[0].positions.is_empty());
        assert_eq!(
            result.meshes[0].positions.len(),
            result.meshes[0].normals.len()
        );
        assert_eq!(result.meshes[0].indices.len(), 170 * 3);
    }

    #[test]
    fn test_splits_meshes_on_usemtl() {
        let result = parse_obj_internal(
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\nusemtl red\nf 1 2 3\nusemtl blue\nf 2 4 3\n",
        );

        assert!(result.success);
        assert_eq!(result.meshes.len(), 2);
        assert_eq!(result.meshes[0].material.as_deref(), Some("red"));
        assert_eq!(result.meshes[1].material.as_deref(), Some("blue"));
    }
}

#[cfg(all(test, feature = "write"))]
mod writer_tests {
    use super::*;

    #[test]
    fn test_create_simple_obj() {
        let mesh = MeshInput {
            positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0, 0.0],
            indices: vec![0, 1, 2],
            normals: None,
            uvs: None,
            name: Some("triangle".to_string()),
        };

        let result = create_obj_internal(&mesh, &ExportOptions::default());
        assert!(result.success);
        assert!(result.data.is_some());
        let data = result.data.unwrap();
        assert!(data.contains("v 0."));
        assert!(data.contains("f 1 2 3"));
    }
}
