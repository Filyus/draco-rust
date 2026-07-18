//! Native glTF document transformation entry points for browser consumers.

use draco_gltf::{Document, ValidationProfile};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Validates and canonicalizes a JSON glTF document with the native model.
#[wasm_bindgen]
pub fn canonicalize_gltf(data: &[u8]) -> Result<Vec<u8>, JsValue> {
    let document = Document::from_json_bytes(data)
        .and_then(|document| {
            document.validate(ValidationProfile::Gltf21Draft)?;
            Ok(document)
        })
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    document
        .to_json_bytes()
        .map_err(|error| JsValue::from_str(&error.to_string()))
}
