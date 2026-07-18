//! Compact glTF/GLB reader for the browser.
//!
//! This crate is a thin `wasm-bindgen` adapter over the serde-free compact
//! reader in [`draco_io::gltf_compact`], which shares strict GLB container
//! validation with the rest of `draco-io` and parses the JSON document with
//! `nanoserde`. Keeping the serde-backed document model out of the binary is
//! what holds the WASM module under its size budget.

#![allow(clippy::question_mark)]

use draco_io::{gltf_compact, parse_glb_json_and_bin};
use nanoserde::SerJson;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// Mesh data structure for JavaScript interop.
#[derive(SerJson, Clone, Default)]
pub struct MeshData {
    /// Mesh name.
    pub name: Option<String>,
    /// Vertex positions as `[x0, y0, z0, ...]`.
    pub positions: Vec<f32>,
    /// Triangle indices as `[i0, i1, i2, ...]`.
    pub indices: Vec<u32>,
    /// Vertex normals, when present.
    pub normals: Vec<f32>,
    /// First texture-coordinate set, when present.
    pub uvs: Vec<f32>,
    /// First color set, when present.
    pub colors: Vec<f32>,
}

/// Node in the scene graph.
#[derive(SerJson, Clone, Default)]
pub struct SceneNode {
    pub name: Option<String>,
    #[nserde(rename = "meshIndex")]
    pub mesh_index: Option<usize>,
    pub translation: Option<Vec<f32>>,
    pub rotation: Option<Vec<f32>>,
    pub scale: Option<Vec<f32>>,
    pub children: Vec<usize>,
}

/// Scene data.
#[derive(SerJson, Clone, Default)]
pub struct SceneData {
    pub name: Option<String>,
    pub nodes: Vec<usize>,
}

/// Parse result containing decoded geometry and scene metadata.
#[derive(SerJson, Default)]
pub struct ParseResult {
    pub success: bool,
    pub meshes: Vec<MeshData>,
    pub scenes: Vec<SceneData>,
    pub nodes: Vec<SceneNode>,
    #[nserde(rename = "defaultScene")]
    pub default_scene: Option<usize>,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    #[nserde(rename = "usesDraco")]
    pub uses_draco: bool,
}

impl ParseResult {
    fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(message.into()),
            ..Self::default()
        }
    }
}

fn copy_uint8_array(array: &js_sys::Uint8Array) -> Result<Vec<u8>, String> {
    let len = usize::try_from(array.length())
        .map_err(|_| "Companion resource is too large".to_string())?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| "Companion resource is too large".to_string())?;
    bytes.resize(len, 0);
    array.copy_to(&mut bytes);
    Ok(bytes)
}

/// Initialize the WASM module.
#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Get the crate version used to build this module.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Get the module name.
#[wasm_bindgen]
pub fn module_name() -> String {
    "glTF Reader".to_string()
}

/// Get supported file extensions.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["gltf".to_string(), "glb".to_string()]
}

fn to_js_value(result: &ParseResult) -> JsValue {
    let json = SerJson::serialize_json(result);
    js_sys::JSON::parse(&json).unwrap_or(JsValue::NULL)
}

/// Parse an embedded/self-contained glTF JSON document.
#[wasm_bindgen]
pub fn parse_gltf(json_content: &str) -> JsValue {
    to_js_value(&parse_document_to_result(json_content, None, &[]))
}

/// Parse a GLB document.
#[wasm_bindgen]
pub fn parse_glb(data: &[u8]) -> JsValue {
    let result = match parse_glb_json_and_bin(data) {
        Ok((json, bin)) => match std::str::from_utf8(json) {
            Ok(json) => parse_document_to_result(json, bin, &[]),
            Err(_) => ParseResult::error("Invalid GLB JSON"),
        },
        Err(_) => ParseResult::error("Invalid GLB"),
    };
    to_js_value(&result)
}

/// Parse glTF/GLB bytes with a map of companion URI to exact resource bytes.
///
/// JavaScript should pass an object such as
/// `{ "model.bin": Uint8Array, "albedo.png": Uint8Array }`. Missing external
/// buffers are reported as controlled errors; the resolver never reads files.
#[wasm_bindgen]
pub fn parse_gltf_with_resources(data: &[u8], resources_js: JsValue) -> JsValue {
    let resources = match parse_companion_resources(resources_js) {
        Ok(resources) => resources,
        Err(error) => {
            return to_js_value(&ParseResult::error(error));
        }
    };
    let result = match parse_glb_json_and_bin(data) {
        Ok((json, bin)) => match std::str::from_utf8(json) {
            Ok(json) => parse_document_to_result(json, bin, &resources),
            Err(_) => ParseResult::error("Invalid glTF JSON"),
        },
        Err(_) => match std::str::from_utf8(data) {
            Ok(json) => parse_document_to_result(json, None, &resources),
            Err(_) => ParseResult::error("Invalid glTF JSON"),
        },
    };
    to_js_value(&result)
}

/// Run the compact reader and adapt its output into the JS-facing result.
fn parse_document_to_result(
    json: &str,
    bin: Option<&[u8]>,
    resources: &[(String, Vec<u8>)],
) -> ParseResult {
    match gltf_compact::parse_compact_document(json, bin, resources) {
        Ok(document) => ParseResult {
            success: true,
            meshes: document
                .meshes
                .into_iter()
                .map(|mesh| MeshData {
                    name: mesh.name,
                    positions: mesh.positions,
                    indices: mesh.indices,
                    normals: mesh.normals,
                    uvs: mesh.uvs,
                    colors: mesh.colors,
                })
                .collect(),
            scenes: document
                .scenes
                .into_iter()
                .map(|scene| SceneData {
                    name: scene.name,
                    nodes: scene.nodes,
                })
                .collect(),
            nodes: document
                .nodes
                .into_iter()
                .map(|node| SceneNode {
                    name: node.name,
                    mesh_index: node.mesh,
                    translation: node.translation,
                    rotation: node.rotation,
                    scale: node.scale,
                    children: node.children,
                })
                .collect(),
            default_scene: document.default_scene,
            error: None,
            warnings: Vec::new(),
            uses_draco: document.uses_draco,
        },
        Err(error) => ParseResult::error(error.to_string()),
    }
}

fn parse_companion_resources(resources_js: JsValue) -> Result<Vec<(String, Vec<u8>)>, String> {
    if !resources_js.is_object() || resources_js.is_null() {
        return Err("expected an object whose values are Uint8Array instances".to_string());
    }
    let object: js_sys::Object = resources_js.unchecked_into();
    let entries = js_sys::Object::entries(&object);
    let mut resources = Vec::new();
    resources
        .try_reserve(entries.length() as usize)
        .map_err(|_| "Companion resource map is too large".to_string())?;
    for index in 0..entries.length() {
        let pair = js_sys::Array::from(&entries.get(index));
        if pair.length() != 2 {
            return Err("Invalid companion resource map".to_string());
        }
        let uri = pair
            .get(0)
            .as_string()
            .ok_or("Companion resource URI is not a string")?;
        let value = pair.get(1);
        if !value.is_instance_of::<js_sys::Uint8Array>() {
            return Err("Invalid companion resource map".to_string());
        }
        let bytes = copy_uint8_array(&js_sys::Uint8Array::new(&value))?;
        if resources.iter().any(|(candidate, _)| candidate == &uri) {
            return Err("Duplicate companion resource URI".to_string());
        }
        resources.push((uri, bytes));
    }
    Ok(resources)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle_resource() -> Vec<u8> {
        [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect()
    }

    fn external_triangle_json() -> Vec<u8> {
        br#"{
          "asset":{"version":"2.0"},
          "buffers":[{"uri":"triangle.bin","byteLength":36}],
          "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}],
          "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],
          "meshes":[{"name":"Triangle","primitives":[{"attributes":{"POSITION":0}}]}],
          "nodes":[{"mesh":0}],
          "scenes":[{"nodes":[0]}],
          "scene":0
        }"#
        .to_vec()
    }

    #[test]
    fn zero_geometry_is_not_reported_as_success() {
        let input = br#"{"asset":{"version":"2.0"},"meshes":[]}"#;
        let result = parse_document_to_result(std::str::from_utf8(input).unwrap(), None, &[]);
        assert!(!result.success);
        assert!(result.meshes.is_empty());
    }

    #[test]
    fn compact_reader_decodes_external_resource() {
        let resources = vec![("triangle.bin".to_string(), triangle_resource())];
        let input = external_triangle_json();
        let json = std::str::from_utf8(&input).unwrap();
        let result = parse_document_to_result(json, None, &resources);
        assert!(result.success);
        assert_eq!(result.meshes.len(), 1);
        assert_eq!(result.meshes[0].positions.len(), 9);
        assert_eq!(result.meshes[0].indices, vec![0, 1, 2]);
    }

    #[test]
    fn compact_reader_rejects_malformed_accessor() {
        let json = r#"{
            "asset":{"version":"2.0"},
            "buffers":[{"byteLength":4}],
            "bufferViews":[{"buffer":0,"byteLength":4}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":1,"type":"VEC3"}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}]
        }"#;
        assert!(!parse_document_to_result(json, Some(&[0; 4]), &[]).success);
    }
}
