//! OBJ reader and writer WASM module.
//!
//! Provides OBJ parsing and generation for web applications. The reader and
//! writer are independent: build with `--features read` or `--features write`
//! (both are on by default) to control which half of the API is exported.

use wasm_bindgen::prelude::*;

use js_sys::{Array, Object};

// The conversion layer is `wasm-bridge`, shared with the other four modules:
// geometry crosses as typed arrays, and everything read back from JavaScript is
// validated there rather than trusted. See that crate for why it is one copy.
#[cfg(feature = "read")]
use wasm_bridge::{f32_array_to_js, set_js, set_string_array, u32_array_to_js};
#[cfg(feature = "write")]
use wasm_bridge::{
    opt_bool_from_js, opt_string_from_js, opt_u32_from_js, optional_f32_array, required_f32_array,
    required_u32_array,
};
use wasm_bridge::{set_bool, set_opt_string};

#[cfg(feature = "read")]
use std::collections::HashMap;

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
    pub material: Option<String>,
}

/// Parse result containing meshes and any warnings/errors.
#[cfg(feature = "read")]
pub struct ParseResult {
    pub success: bool,
    pub meshes: Vec<MeshData>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
}

/// Parse OBJ file content from bytes.
///
/// Bytes rather than a string: a file arrives as bytes, and handing them over
/// as one avoids decoding the whole of it into UTF-16 on the JavaScript side
/// only to encode it back on the way in. The string entry point this replaced
/// is gone rather than kept beside it, so there is one way in and no second
/// signature to keep in step.
#[cfg(feature = "read")]
#[wasm_bindgen]
pub fn parse_obj_bytes(data: &[u8]) -> JsValue {
    match std::str::from_utf8(data) {
        Ok(content) => parse_result_to_js(&parse_obj_internal(content)),
        Err(e) => {
            let result = ParseResult {
                success: false,
                meshes: vec![],
                error: Some(format!("Invalid UTF-8 content: {}", e)),
                warnings: vec![],
            };
            parse_result_to_js(&result)
        }
    }
}

#[cfg(feature = "read")]
fn parse_result_to_js(result: &ParseResult) -> JsValue {
    let obj = Object::new();
    set_bool(&obj, "success", result.success);
    let meshes = Array::new();
    for mesh in &result.meshes {
        let mesh_obj = Object::new();
        set_js(&mesh_obj, "positions", &f32_array_to_js(&mesh.positions));
        set_js(&mesh_obj, "indices", &u32_array_to_js(&mesh.indices));
        set_js(&mesh_obj, "normals", &f32_array_to_js(&mesh.normals));
        set_js(&mesh_obj, "uvs", &f32_array_to_js(&mesh.uvs));
        match &mesh.material {
            Some(material) => set_js(&mesh_obj, "material", &JsValue::from_str(material)),
            None => set_js(&mesh_obj, "material", &JsValue::UNDEFINED),
        }
        meshes.push(&mesh_obj.into());
    }
    set_js(&obj, "meshes", &meshes.into());
    set_opt_string(&obj, "error", &result.error);
    set_string_array(&obj, "warnings", &result.warnings);
    obj.into()
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

        // Tokenized with a forward-only iterator rather than collected into a
        // Vec: on a 51MB file that is one heap allocation per line (and,
        // inside "f", one more per face-vertex token below) purely to hold
        // tokens that are only ever read in sequence. `tokens` starts past the
        // keyword in every arm, matching what `parts[1..]` used to mean.
        let mut tokens = line.split_whitespace();
        let Some(keyword) = tokens.next() else {
            continue;
        };

        match keyword {
            "v" => {
                if let (Some(x), Some(y), Some(z)) = (tokens.next(), tokens.next(), tokens.next()) {
                    if let (Ok(x), Ok(y), Ok(z)) =
                        (x.parse::<f32>(), y.parse::<f32>(), z.parse::<f32>())
                    {
                        positions.push([x, y, z]);
                    } else {
                        warnings.push(format!("Line {}: Invalid vertex coordinates", line_num + 1));
                    }
                }
            }
            "vn" => {
                if let (Some(x), Some(y), Some(z)) = (tokens.next(), tokens.next(), tokens.next()) {
                    if let (Ok(x), Ok(y), Ok(z)) =
                        (x.parse::<f32>(), y.parse::<f32>(), z.parse::<f32>())
                    {
                        normals.push([x, y, z]);
                    }
                }
            }
            "vt" => {
                if let (Some(u), Some(v)) = (tokens.next(), tokens.next()) {
                    if let (Ok(u), Ok(v)) = (u.parse::<f32>(), v.parse::<f32>()) {
                        texcoords.push([u, v]);
                    }
                }
            }
            "usemtl" => {
                let rest: Vec<&str> = tokens.collect();
                current_material = (!rest.is_empty()).then(|| rest.join(" "));
            }
            "f" => {
                let mut face_verts: Vec<ObjVertexRef> = Vec::new();
                for part in tokens {
                    // `part` comes from `split_whitespace`, which never yields
                    // an empty slice, so `part.split('/')` always has a first
                    // element -- the same guarantee the old `indices[0]`
                    // relied on.
                    let mut corner = part.split('/');
                    let vi: usize = corner.next().unwrap().parse::<usize>().unwrap_or(1) - 1;
                    let ti: Option<usize> = corner
                        .next()
                        .and_then(|s| {
                            if s.is_empty() {
                                None
                            } else {
                                s.parse::<usize>().ok()
                            }
                        })
                        .map(|i| i - 1);
                    let ni: Option<usize> = corner
                        .next()
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
    // Convert to indexed mesh (triangulate if needed). Deduplicate corners in
    // face-traversal order, matching the crate/C++ OBJ loader behavior: two
    // corners sharing (position, normal, uv) indices are one vertex.
    let mut out_positions: Vec<f32> = Vec::new();
    let mut out_normals: Vec<f32> = Vec::new();
    let mut out_uvs: Vec<f32> = Vec::new();
    let mut out_indices: Vec<u32> = Vec::new();
    let mut vertex_map: HashMap<ObjVertexRef, u32> = HashMap::new();

    for face in faces {
        // Triangulate polygon (fan triangulation)
        for i in 1..face.len() - 1 {
            let triangle = [&face[0], &face[i], &face[i + 1]];
            for &(vi, ti, ni) in &triangle {
                let vertex_ref = (*vi, *ti, *ni);
                if let Some(&point_id) = vertex_map.get(&vertex_ref) {
                    out_indices.push(point_id);
                    continue;
                }

                let point_id = (out_positions.len() / 3) as u32;
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

                vertex_map.insert(vertex_ref, point_id);
                out_indices.push(point_id);
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
#[derive(Default)]
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
pub struct ExportResult {
    pub success: bool,
    pub data: Option<String>,
    pub error: Option<String>,
}

/// Create OBJ content from mesh data.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_obj(mesh_js: JsValue, options_js: JsValue) -> JsValue {
    let mesh = match mesh_input_from_js(&mesh_js) {
        Ok(mesh) => mesh,
        Err(error) => {
            return export_result_to_js(&ExportResult {
                success: false,
                data: None,
                error: Some(error),
            });
        }
    };
    let options = export_options_from_js(&options_js);
    export_result_to_js(&create_obj_internal(&mesh, &options))
}

/// Create OBJ content from multiple meshes.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_obj_multi(meshes_js: JsValue, options_js: JsValue) -> JsValue {
    let meshes = match mesh_input_array_from_js(&meshes_js) {
        Ok(meshes) => meshes,
        Err(error) => {
            return export_result_to_js(&ExportResult {
                success: false,
                data: None,
                error: Some(error),
            });
        }
    };
    let options = export_options_from_js(&options_js);
    export_result_to_js(&create_obj_multi_internal(&meshes, &options))
}

#[cfg(feature = "write")]
fn export_result_to_js(result: &ExportResult) -> JsValue {
    let obj = Object::new();
    set_bool(&obj, "success", result.success);
    set_opt_string(&obj, "data", &result.data);
    set_opt_string(&obj, "error", &result.error);
    obj.into()
}

#[cfg(feature = "write")]
fn mesh_input_from_js(value: &JsValue) -> Result<MeshInput, String> {
    Ok(MeshInput {
        positions: required_f32_array(value, "positions")?,
        indices: required_u32_array(value, "indices")?,
        normals: optional_f32_array(value, "normals")?,
        uvs: optional_f32_array(value, "uvs")?,
        name: opt_string_from_js(value, "name"),
    })
}

#[cfg(feature = "write")]
fn mesh_input_array_from_js(value: &JsValue) -> Result<Vec<MeshInput>, String> {
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| "meshes must be an array".to_string())?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        out.push(mesh_input_from_js(&array.get(index))?);
    }
    Ok(out)
}

#[cfg(feature = "write")]
fn export_options_from_js(value: &JsValue) -> ExportOptions {
    ExportOptions {
        include_normals: opt_bool_from_js(value, "include_normals"),
        include_uvs: opt_bool_from_js(value, "include_uvs"),
        precision: opt_u32_from_js(value, "precision"),
    }
}

#[cfg(feature = "write")]
fn create_obj_internal(mesh: &MeshInput, options: &ExportOptions) -> ExportResult {
    create_obj_multi_internal(std::slice::from_ref(mesh), options)
}

/// Roughly how much text a mesh list will produce, so the buffer is taken in
/// one allocation rather than grown into.
///
/// A `String` that doubles copies everything it already holds each time, and
/// this output reaches tens of megabytes. Over-estimating costs one oversized
/// allocation; under-estimating costs only the growth that would have happened
/// anyway, so these figures are deliberately generous rather than tight.
#[cfg(feature = "write")]
fn estimated_bytes(meshes: &[MeshInput], precision: usize) -> usize {
    // A sign, a few integer digits, the point and a separator.
    let number = precision + 8;
    meshes
        .iter()
        .map(|mesh| {
            let vertices = mesh.positions.len() / 3;
            let normals = mesh.normals.as_ref().map_or(0, |values| values.len() / 3);
            let uvs = mesh.uvs.as_ref().map_or(0, |values| values.len() / 2);
            let faces = mesh.indices.len() / 3;
            vertices * (3 * number + 3)
                + normals * (3 * number + 4)
                + uvs * (2 * number + 4)
                // Nine indices at up to ten digits each, with their separators.
                + faces * 40
        })
        .sum::<usize>()
        + 64
}

#[cfg(feature = "write")]
fn create_obj_multi_internal(meshes: &[MeshInput], options: &ExportOptions) -> ExportResult {
    let precision = options.precision.unwrap_or(6) as usize;
    let include_normals = options.include_normals.unwrap_or(true);
    let include_uvs = options.include_uvs.unwrap_or(true);

    // Formatted straight into the output rather than through `format!`, which
    // would build and drop a fresh String for every line. On a 263k-vertex mesh
    // that is over a million allocations, and it was the whole cost of this
    // writer. Writing into a String cannot fail, so the results are discarded
    // rather than propagated.
    use std::fmt::Write as _;

    let mut output = String::with_capacity(estimated_bytes(meshes, precision));
    output.push_str("# OBJ file generated by draco-io WASM\n");
    let _ = writeln!(output, "# Meshes: {}\n", meshes.len());

    let mut vertex_offset: u32 = 0;
    let mut normal_offset: u32 = 0;
    let mut uv_offset: u32 = 0;

    for (mesh_idx, mesh) in meshes.iter().enumerate() {
        // Write object/group name
        match mesh.name.as_deref() {
            Some(name) => {
                let _ = writeln!(output, "o {name}");
            }
            None => {
                let _ = writeln!(output, "o mesh_{mesh_idx}");
            }
        }

        // Write vertices
        let vertex_count = mesh.positions.len() / 3;
        for position in mesh.positions.as_chunks::<3>().0.iter().take(vertex_count) {
            let _ = writeln!(
                output,
                "v {:.*} {:.*} {:.*}",
                precision, position[0], precision, position[1], precision, position[2]
            );
        }

        // Write normals
        let mut has_normals = false;
        if include_normals {
            if let Some(ref normals) = mesh.normals {
                if !normals.is_empty() {
                    has_normals = true;
                    for normal in normals.as_chunks::<3>().0 {
                        let _ = writeln!(
                            output,
                            "vn {:.*} {:.*} {:.*}",
                            precision, normal[0], precision, normal[1], precision, normal[2]
                        );
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
                    for uv in uvs.as_chunks::<2>().0 {
                        let _ =
                            writeln!(output, "vt {:.*} {:.*}", precision, uv[0], precision, uv[1]);
                    }
                }
            }
        }

        // Write faces
        for face in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = *face;
            let (i0, i1, i2) = (
                a + vertex_offset + 1,
                b + vertex_offset + 1,
                c + vertex_offset + 1,
            );

            if has_normals && has_uvs {
                // f v/vt/vn
                let (n0, n1, n2) = (
                    a + normal_offset + 1,
                    b + normal_offset + 1,
                    c + normal_offset + 1,
                );
                let (t0, t1, t2) = (a + uv_offset + 1, b + uv_offset + 1, c + uv_offset + 1);
                let _ = writeln!(output, "f {i0}/{t0}/{n0} {i1}/{t1}/{n1} {i2}/{t2}/{n2}");
            } else if has_normals {
                // f v//vn
                let (n0, n1, n2) = (
                    a + normal_offset + 1,
                    b + normal_offset + 1,
                    c + normal_offset + 1,
                );
                let _ = writeln!(output, "f {i0}//{n0} {i1}//{n1} {i2}//{n2}");
            } else if has_uvs {
                // f v/vt
                let (t0, t1, t2) = (a + uv_offset + 1, b + uv_offset + 1, c + uv_offset + 1);
                let _ = writeln!(output, "f {i0}/{t0} {i1}/{t1} {i2}/{t2}");
            } else {
                // f v
                let _ = writeln!(output, "f {i0} {i1} {i2}");
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
        assert_eq!(result.meshes[0].positions.len(), 12); // 4 unique vertices * 3
        assert_eq!(result.meshes[0].indices.len(), 6); // 2 triangles * 3 indices
        assert!(result.meshes[0].indices.iter().all(|&i| i < 4));
    }

    #[test]
    fn test_dedups_shared_vertices() {
        let result = parse_obj_internal(include_str!("../../../testdata/test_cube_shared.obj"));
        assert!(result.success);
        assert_eq!(result.meshes.len(), 1);
        assert_eq!(result.meshes[0].positions.len(), 24); // 8 vertices * 3
        assert_eq!(result.meshes[0].indices.len(), 36); // 12 triangles * 3
        assert!(result.meshes[0].indices.iter().all(|&i| i < 8));
    }

    #[test]
    fn test_dedups_by_full_corner_not_position() {
        // The same position with different normals must stay separate vertices.
        let obj =
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 0 1\nvn 0 0 -1\nf 1//1 2//1 3//1\nf 1//2 3//2 2//2\n";
        let result = parse_obj_internal(obj);

        assert!(result.success);
        assert_eq!(result.meshes[0].positions.len(), 18); // 6 corner-vertices * 3
        assert_eq!(result.meshes[0].normals.len(), 18);
        assert_eq!(result.meshes[0].indices.len(), 6);
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
        assert!(result.meshes[0].positions.len() < 170 * 3 * 3); // deduplicated
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
