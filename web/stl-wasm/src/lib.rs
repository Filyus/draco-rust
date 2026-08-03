//! STL reader and writer WASM module.
//!
//! The two halves are independent: build with `--features read` or
//! `--features write` (both are on by default) to control which is exported.
//! The JavaScript shapes match the OBJ and PLY modules, so the shell's intake
//! and export routes treat every flat mesh format the same way.

use wasm_bindgen::prelude::*;

#[cfg(feature = "write")]
use js_sys::Uint8Array;
use js_sys::{Array, Float32Array, Object, Reflect, Uint32Array};
#[cfg(feature = "write")]
use wasm_bindgen::JsCast;

// ===========================================================================
// JavaScript bridge
//
// Values cross the wasm boundary as plain JavaScript objects built and read by
// hand instead of serde structures: geometry goes over as typed arrays, which
// cross in bulk and give JavaScript an array it owns outright. The cost of
// dropping serde is that nothing here validates untrusted input for us, so
// every field read from JavaScript is guarded individually.
// ===========================================================================

/// Set a property on a JavaScript object.
fn set_js(obj: &Object, key: &str, value: &JsValue) {
    let _ = Reflect::set(obj, &JsValue::from_str(key), value);
}

fn set_bool(obj: &Object, key: &str, value: bool) {
    set_js(obj, key, &JsValue::from_bool(value));
}

/// Set an optional string, to `undefined` when absent, so the key stays present
/// with the same shape the serde output used to have.
fn set_opt_string(obj: &Object, key: &str, value: &Option<String>) {
    match value {
        Some(text) => set_js(obj, key, &JsValue::from_str(text)),
        None => set_js(obj, key, &JsValue::UNDEFINED),
    }
}

#[cfg(feature = "read")]
fn set_string_array(obj: &Object, key: &str, values: &[String]) {
    let array = Array::new();
    for value in values {
        array.push(&JsValue::from_str(value));
    }
    set_js(obj, key, &array.into());
}

#[cfg(feature = "read")]
fn f32_array_to_js(values: &[f32]) -> JsValue {
    Float32Array::from(values).into()
}

#[cfg(feature = "read")]
fn u32_array_to_js(values: &[u32]) -> JsValue {
    Uint32Array::from(values).into()
}

#[cfg(feature = "write")]
fn bytes_to_js(values: &[u8]) -> JsValue {
    Uint8Array::from(values).into()
}

/// Read a field that must be present on an object. An absent key reads back as
/// `undefined` from JavaScript, which is an error to be reported, not a value.
#[cfg(feature = "write")]
fn get_field(obj: &JsValue, key: &str) -> Option<JsValue> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) if value.is_undefined() || value.is_null() => None,
        Ok(value) => Some(value),
        Err(_) => None,
    }
}

/// Read a string field, tolerating an absent one.
#[cfg(feature = "write")]
fn opt_string_from_js(obj: &JsValue, key: &str) -> Option<String> {
    if !obj.is_object() {
        return None;
    }
    match Reflect::get(obj, &JsValue::from_str(key)) {
        Ok(value) if value.is_string() => value.as_string(),
        _ => None,
    }
}

/// A float array from JavaScript: a `Float32Array` copies across in bulk, a
/// plain array is walked element by element. Every element must be a number.
#[cfg(feature = "write")]
fn f32_array_from_js(value: &JsValue, field: &str) -> Result<Vec<f32>, String> {
    if let Some(typed) = value.dyn_ref::<Float32Array>() {
        return Ok(typed.to_vec());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be a Float32Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        match array.get(index).as_f64() {
            Some(value) => out.push(value as f32),
            None => return Err(format!("{field} must contain only numbers")),
        }
    }
    Ok(out)
}

#[cfg(feature = "write")]
fn u32_array_from_js(value: &JsValue, field: &str) -> Result<Vec<u32>, String> {
    if let Some(typed) = value.dyn_ref::<Uint32Array>() {
        return Ok(typed.to_vec());
    }
    let array = value
        .dyn_ref::<Array>()
        .ok_or_else(|| format!("{field} must be a Uint32Array or a plain array"))?;
    let mut out = Vec::with_capacity(array.length() as usize);
    for index in 0..array.length() {
        match array.get(index).as_f64() {
            Some(value) => out.push(value as u32),
            None => return Err(format!("{field} must contain only numbers")),
        }
    }
    Ok(out)
}

/// Install the panic hook, so a panic reads as a message rather than a trap.
#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// The version of this WASM module.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// File extensions this module reads and writes.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["stl".to_string()]
}

#[cfg(any(feature = "read", feature = "write"))]
use draco_core::draco_types::DataType;
#[cfg(any(feature = "read", feature = "write"))]
use draco_core::geometry_attribute::{GeometryAttributeType, PointAttribute};
#[cfg(any(feature = "read", feature = "write"))]
use draco_core::geometry_indices::{FaceIndex, PointIndex};
#[cfg(any(feature = "read", feature = "write"))]
use draco_core::mesh::Mesh;

// ===========================================================================
// Reader
// ===========================================================================

/// Mesh data produced by the reader, for JavaScript interop.
#[cfg(feature = "read")]
pub struct MeshData {
    /// Vertex positions as a flat array `[x0, y0, z0, x1, ...]`.
    pub positions: Vec<f32>,
    /// Triangle indices as a flat array.
    pub indices: Vec<u32>,
    /// Vertex normals, empty when the file stated none.
    pub normals: Vec<f32>,
}

/// What the reader hands back: the mesh, or why there is none.
#[cfg(feature = "read")]
pub struct ParseResult {
    /// Whether a mesh was produced.
    pub success: bool,
    /// The meshes read; STL holds exactly one solid, so at most one.
    pub meshes: Vec<MeshData>,
    /// Why the parse failed, when it did.
    pub error: Option<String>,
    /// Non-fatal remarks about the file.
    pub warnings: Vec<String>,
    /// Which container the file turned out to be: `binary` or `ascii`.
    pub container: Option<String>,
}

/// Parse an STL file from bytes. Handles both the binary and ASCII containers.
#[cfg(feature = "read")]
#[wasm_bindgen]
pub fn parse_stl_bytes(data: &[u8]) -> JsValue {
    parse_result_to_js(&parse_stl_internal(data))
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
        meshes.push(&mesh_obj.into());
    }
    set_js(&obj, "meshes", &meshes.into());
    set_opt_string(&obj, "error", &result.error);
    set_string_array(&obj, "warnings", &result.warnings);
    set_opt_string(&obj, "container", &result.container);
    obj.into()
}

#[cfg(feature = "read")]
fn parse_stl_internal(data: &[u8]) -> ParseResult {
    // Named from the same rule the reader applies, so the panel reports the
    // container that was actually read rather than the one the bytes look like.
    let container = if data.len() >= 84
        && data.len() == 84 + u32::from_le_bytes(data[80..84].try_into().unwrap()) as usize * 50
    {
        "binary"
    } else if data.starts_with(b"solid") {
        "ascii"
    } else {
        "binary"
    };

    match draco_io::stl_reader::StlReader::read_from_bytes(data) {
        Ok(mesh) => {
            let mut warnings = Vec::new();
            if mesh.num_faces() == 0 {
                warnings.push("STL contains no triangles".to_string());
            }
            ParseResult {
                success: true,
                meshes: vec![mesh_to_js_data(&mesh)],
                error: None,
                warnings,
                container: Some(container.to_string()),
            }
        }
        Err(error) => ParseResult {
            success: false,
            meshes: vec![],
            error: Some(error.to_string()),
            warnings: vec![],
            container: Some(container.to_string()),
        },
    }
}

#[cfg(feature = "read")]
fn mesh_to_js_data(mesh: &Mesh) -> MeshData {
    let mut indices = Vec::with_capacity(mesh.num_faces() * 3);
    for index in 0..mesh.num_faces() {
        let face = mesh.face(FaceIndex(index as u32));
        indices.extend([face[0].0, face[1].0, face[2].0]);
    }
    MeshData {
        positions: read_attribute_as_f32(mesh, GeometryAttributeType::Position),
        indices,
        normals: read_attribute_as_f32(mesh, GeometryAttributeType::Normal),
    }
}

/// Read a float3 attribute out of the mesh, or nothing when it has none.
///
/// The STL reader writes float32 triples and nothing else, so this does not
/// carry the type ladder the PLY module needs.
#[cfg(feature = "read")]
fn read_attribute_as_f32(mesh: &Mesh, attribute_type: GeometryAttributeType) -> Vec<f32> {
    let attribute_id = mesh.named_attribute_id(attribute_type);
    if attribute_id < 0 {
        return Vec::new();
    }
    let attribute = mesh.attribute(attribute_id);
    if attribute.data_type() != DataType::Float32 || attribute.num_components() != 3 {
        return Vec::new();
    }
    let stride = attribute.byte_stride() as usize;
    let mut values = Vec::with_capacity(mesh.num_points() * 3);
    for point in 0..mesh.num_points() {
        let mut bytes = [0u8; 12];
        attribute.buffer().read(point * stride, &mut bytes);
        for component in 0..3 {
            let start = component * 4;
            values.push(f32::from_le_bytes(
                bytes[start..start + 4].try_into().unwrap(),
            ));
        }
    }
    values
}

// ===========================================================================
// Writer
// ===========================================================================

/// Input mesh data consumed by the writer, from JavaScript.
#[cfg(feature = "write")]
pub struct MeshInput {
    /// Vertex positions as a flat array `[x0, y0, z0, x1, ...]`.
    pub positions: Vec<f32>,
    /// Triangle indices as a flat array.
    pub indices: Vec<u32>,
}

/// Export options.
#[cfg(feature = "write")]
pub struct ExportOptions {
    /// `binary` (the default) or `ascii`.
    pub format: Option<String>,
    /// The name written into the ASCII `solid` line.
    pub name: Option<String>,
}

/// Export result.
#[cfg(feature = "write")]
pub struct ExportResult {
    /// Whether the file was written.
    pub success: bool,
    /// ASCII output, when that container was asked for.
    pub data: Option<String>,
    /// Binary output, when that container was asked for.
    pub binary_data: Option<Vec<u8>>,
    /// Why the export failed, when it did.
    pub error: Option<String>,
}

/// Create an STL file from mesh data.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub fn create_stl(mesh_js: JsValue, options_js: JsValue) -> JsValue {
    let mesh = match mesh_input_from_js(&mesh_js) {
        Ok(mesh) => mesh,
        Err(error) => {
            return export_result_to_js(&ExportResult {
                success: false,
                data: None,
                binary_data: None,
                error: Some(error),
            });
        }
    };
    let options = export_options_from_js(&options_js);
    export_result_to_js(&create_stl_internal(&mesh, &options))
}

#[cfg(feature = "write")]
fn export_result_to_js(result: &ExportResult) -> JsValue {
    let obj = Object::new();
    set_bool(&obj, "success", result.success);
    set_opt_string(&obj, "data", &result.data);
    match &result.binary_data {
        Some(bytes) => set_js(&obj, "binary_data", &bytes_to_js(bytes)),
        None => set_js(&obj, "binary_data", &JsValue::UNDEFINED),
    }
    set_opt_string(&obj, "error", &result.error);
    obj.into()
}

#[cfg(feature = "write")]
fn mesh_input_from_js(value: &JsValue) -> Result<MeshInput, String> {
    let positions = get_field(value, "positions")
        .ok_or_else(|| "mesh must be an object with positions and indices".to_string())?;
    let indices = get_field(value, "indices")
        .ok_or_else(|| "mesh must be an object with positions and indices".to_string())?;
    Ok(MeshInput {
        positions: f32_array_from_js(&positions, "positions")?,
        indices: u32_array_from_js(&indices, "indices")?,
    })
}

#[cfg(feature = "write")]
fn export_options_from_js(value: &JsValue) -> ExportOptions {
    ExportOptions {
        format: opt_string_from_js(value, "format"),
        name: opt_string_from_js(value, "name"),
    }
}

#[cfg(feature = "write")]
fn create_stl_internal(input: &MeshInput, options: &ExportOptions) -> ExportResult {
    use draco_io::stl_writer::{StlFormat, StlWriter};
    use draco_io::{WriteToBytes, Writer};

    let ascii = match options.format.as_deref().unwrap_or("binary") {
        "binary" => false,
        "ascii" => true,
        other => {
            return ExportResult {
                success: false,
                data: None,
                binary_data: None,
                error: Some(format!("Unsupported STL format: {other}")),
            };
        }
    };

    let mesh = match mesh_input_to_core_mesh(input) {
        Ok(mesh) => mesh,
        Err(error) => {
            return ExportResult {
                success: false,
                data: None,
                binary_data: None,
                error: Some(error),
            }
        }
    };

    let mut writer = StlWriter::new().with_format(if ascii {
        StlFormat::Ascii
    } else {
        StlFormat::Binary
    });
    if let Err(error) = Writer::add_mesh(&mut writer, &mesh, options.name.as_deref()) {
        return ExportResult {
            success: false,
            data: None,
            binary_data: None,
            error: Some(error.to_string()),
        };
    }

    match writer.write_to_vec() {
        Ok(bytes) if ascii => match String::from_utf8(bytes) {
            Ok(text) => ExportResult {
                success: true,
                data: Some(text),
                binary_data: None,
                error: None,
            },
            Err(error) => ExportResult {
                success: false,
                data: None,
                binary_data: None,
                error: Some(error.to_string()),
            },
        },
        Ok(bytes) => ExportResult {
            success: true,
            data: None,
            binary_data: Some(bytes),
            error: None,
        },
        Err(error) => ExportResult {
            success: false,
            data: None,
            binary_data: None,
            error: Some(error.to_string()),
        },
    }
}

#[cfg(feature = "write")]
fn mesh_input_to_core_mesh(input: &MeshInput) -> Result<Mesh, String> {
    if !input.positions.len().is_multiple_of(3) {
        return Err("positions length must be divisible by 3".to_string());
    }
    if !input.indices.len().is_multiple_of(3) {
        return Err("indices length must be divisible by 3".to_string());
    }

    let vertex_count = input.positions.len() / 3;
    let mut mesh = Mesh::new();
    mesh.set_num_points(vertex_count);

    let mut positions = PointAttribute::new();
    positions.init(
        GeometryAttributeType::Position,
        3,
        DataType::Float32,
        false,
        vertex_count,
    );
    positions.buffer_mut().write_f32s_le(0, &input.positions);
    mesh.add_attribute(positions);

    mesh.set_num_faces(input.indices.len() / 3);
    for (index, chunk) in input.indices.chunks_exact(3).enumerate() {
        mesh.set_face(
            FaceIndex(index as u32),
            [
                PointIndex(chunk[0]),
                PointIndex(chunk[1]),
                PointIndex(chunk[2]),
            ],
        );
    }
    Ok(mesh)
}

#[cfg(all(test, feature = "read", feature = "write"))]
mod tests {
    use super::*;

    fn quad() -> MeshInput {
        MeshInput {
            positions: vec![
                0.0, 0.0, 0.0, //
                1.0, 0.0, 0.0, //
                1.0, 1.0, 0.0, //
                0.0, 1.0, 0.0,
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
        }
    }

    /// What the shell actually does with these two entry points: writes a file
    /// and opens it again. STL unshares every vertex, so a quad comes back as
    /// six points rather than four -- the geometry survives, the indexing does
    /// not, and that is the format rather than a loss in the binding.
    #[test]
    fn test_roundtrip_through_the_js_entry_points() {
        for format in ["binary", "ascii"] {
            let exported = create_stl_internal(
                &quad(),
                &ExportOptions {
                    format: Some(format.to_string()),
                    name: Some("Quad".to_string()),
                },
            );
            assert!(exported.success, "{format}: {:?}", exported.error);
            let bytes = exported
                .binary_data
                .unwrap_or_else(|| exported.data.unwrap().into_bytes());

            let parsed = parse_stl_internal(&bytes);
            assert!(parsed.success, "{format}: {:?}", parsed.error);
            assert_eq!(parsed.container.as_deref(), Some(format));
            let mesh = &parsed.meshes[0];
            assert_eq!(mesh.indices, vec![0, 1, 2, 3, 4, 5], "{format}");
            assert_eq!(mesh.positions.len(), 18, "{format}");
            assert_eq!(&mesh.positions[..3], &[0.0, 0.0, 0.0], "{format}");
            // Both triangles face +Z, and the normal reaches every corner.
            assert_eq!(mesh.normals.len(), 18, "{format}");
            assert!(mesh.normals.chunks_exact(3).all(|n| n == [0.0, 0.0, 1.0]));
        }
    }

    /// A truncated download is the ordinary bad file, and the count field is
    /// what makes it detectable. It has to come back as an error string rather
    /// than a trap: a panic across the wasm boundary takes the page with it.
    #[test]
    fn test_parse_reports_a_truncated_file_rather_than_panicking() {
        let mut bytes = vec![0u8; 84];
        bytes[80..84].copy_from_slice(&50_000u32.to_le_bytes());
        let result = parse_stl_internal(&bytes);
        assert!(!result.success);
        assert!(result.error.is_some());
    }
}
