//! Compact geometry-oriented glTF JSON inspection for browser consumers.

use draco_gltf::{CompactDocument, ValidationProfile};
use nanoserde::SerJson;
use wasm_bindgen::prelude::*;

#[derive(SerJson, Default)]
struct CompactResult {
    success: bool,
    #[nserde(rename = "meshCount")]
    mesh_count: usize,
    #[nserde(rename = "primitiveCount")]
    primitive_count: usize,
    #[nserde(rename = "primitiveCounts")]
    primitive_counts: Vec<usize>,
    error: Option<String>,
}

fn to_js(result: CompactResult) -> JsValue {
    js_sys::JSON::parse(&SerJson::serialize_json(&result)).unwrap_or(JsValue::NULL)
}

#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Parses JSON glTF through the compact facade over the shared `Document`.
#[wasm_bindgen]
pub fn inspect_compact_gltf(data: &[u8], validation_profile: &str) -> JsValue {
    let profile = match validation_profile {
        "2.0" => ValidationProfile::Gltf20,
        "2.1" => ValidationProfile::Gltf21Draft,
        _ => {
            return to_js(CompactResult {
                error: Some("profile must be \"2.0\" or \"2.1\"".into()),
                ..CompactResult::default()
            })
        }
    };
    match CompactDocument::parse(data, profile) {
        Ok(document) => {
            let primitive_counts = document
                .mesh_primitive_ranges()
                .map(|range| range.primitives)
                .collect::<Vec<_>>();
            let primitive_count = primitive_counts.iter().sum();
            to_js(CompactResult {
                success: true,
                mesh_count: primitive_counts.len(),
                primitive_count,
                primitive_counts,
                error: None,
            })
        }
        Err(error) => to_js(CompactResult {
            error: Some(error.to_string()),
            ..CompactResult::default()
        }),
    }
}
