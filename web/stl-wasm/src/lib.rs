//! STL reader and writer WASM module.
//!
//! The two halves are independent: build with `--features read` or
//! `--features write` (both are on by default) to control which is exported.
//! The JavaScript shapes match the OBJ and PLY modules, so the shell's intake
//! and export routes treat every flat mesh format the same way.

use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

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
#[derive(Serialize, Deserialize)]
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
#[derive(Serialize, Deserialize)]
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
    let result = parse_stl_internal(data);
    let json = serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string());
    js_sys::JSON::parse(&json).unwrap_or(JsValue::NULL)
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
#[derive(Serialize, Deserialize, Clone)]
pub struct MeshInput {
    /// Vertex positions as a flat array `[x0, y0, z0, x1, ...]`.
    pub positions: Vec<f32>,
    /// Triangle indices as a flat array.
    pub indices: Vec<u32>,
}

/// Export options.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize, Default)]
pub struct ExportOptions {
    /// `binary` (the default) or `ascii`.
    pub format: Option<String>,
    /// The name written into the ASCII `solid` line.
    pub name: Option<String>,
}

/// Export result.
#[cfg(feature = "write")]
#[derive(Serialize, Deserialize)]
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
    let mesh: MeshInput = match serde_wasm_bindgen::from_value(mesh_js) {
        Ok(mesh) => mesh,
        Err(error) => {
            return to_js(&ExportResult {
                success: false,
                data: None,
                binary_data: None,
                error: Some(format!("Invalid mesh data: {error}")),
            });
        }
    };
    let options: ExportOptions = serde_wasm_bindgen::from_value(options_js).unwrap_or_default();
    to_js(&create_stl_internal(&mesh, &options))
}

#[cfg(feature = "write")]
fn to_js(result: &ExportResult) -> JsValue {
    serde_wasm_bindgen::to_value(result).unwrap_or(JsValue::NULL)
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
    for (index, chunk) in input.positions.chunks_exact(3).enumerate() {
        let bytes: Vec<u8> = chunk.iter().flat_map(|value| value.to_le_bytes()).collect();
        positions.buffer_mut().write(index * 12, &bytes);
    }
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
