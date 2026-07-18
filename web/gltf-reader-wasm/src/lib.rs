//! Native glTF document inspection for browser consumers.

use draco_gltf::{Document, ValidationProfile};
use nanoserde::SerJson;
use wasm_bindgen::prelude::*;

#[derive(SerJson, Default)]
pub struct ParseResult {
    pub success: bool,
    #[nserde(rename = "meshCount")]
    pub mesh_count: usize,
    #[nserde(rename = "primitiveCount")]
    pub primitive_count: usize,
    #[nserde(rename = "sceneCount")]
    pub scene_count: usize,
    #[nserde(rename = "usesDraco")]
    pub uses_draco: bool,
    pub error: Option<String>,
}

fn result(document: Document) -> ParseResult {
    let primitive_count = document
        .meshes()
        .into_iter()
        .map(|mesh| {
            mesh.value()
                .get("primitives")
                .and_then(|v| v.as_array())
                .map_or(0, |v| v.len())
        })
        .sum();
    let uses_draco = document.meshes().into_iter().any(|mesh| {
        mesh.value()
            .get("primitives")
            .and_then(|v| v.as_array())
            .is_some_and(|primitives| {
                primitives.iter().any(|primitive| {
                    primitive
                        .get("extensions")
                        .and_then(|v| v.get("KHR_draco_mesh_compression"))
                        .is_some()
                })
            })
    });
    ParseResult {
        success: true,
        mesh_count: document.meshes().len(),
        primitive_count,
        scene_count: document.scenes().len(),
        uses_draco,
        error: None,
    }
}

fn to_js(result: ParseResult) -> JsValue {
    js_sys::JSON::parse(&SerJson::serialize_json(&result)).unwrap_or(JsValue::NULL)
}

#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["gltf".into(), "glb".into()]
}

/// Parses a JSON glTF or GLB document with the native document model.
#[wasm_bindgen]
pub fn inspect_gltf(data: &[u8]) -> JsValue {
    let json = if data.len() >= 4 && &data[..4] == b"glTF" {
        match draco_io::parse_gltf_container(data) {
            Ok(container) => container.json,
            Err(error) => {
                return to_js(ParseResult {
                    error: Some(error.to_string()),
                    ..ParseResult::default()
                })
            }
        }
    } else {
        data
    };
    match Document::from_json_bytes(json).and_then(|document| {
        document.validate(ValidationProfile::Gltf21Draft)?;
        Ok(document)
    }) {
        Ok(document) => to_js(result(document)),
        Err(error) => to_js(ParseResult {
            error: Some(error.to_string()),
            ..ParseResult::default()
        }),
    }
}
