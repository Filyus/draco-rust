//! Native glTF document inspection for browser consumers.

use draco_gltf::{
    parse_native, CompressionOptions, Document, MeshIndex, NativeImport, OutputFormat,
    ValidationProfile,
};
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

/// Stateful document API backed directly by `draco_gltf::NativeImport`.
#[wasm_bindgen]
pub struct GltfDocument {
    import: NativeImport,
}

fn wasm_error(error: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

fn profile(name: &str) -> Result<ValidationProfile, JsValue> {
    match name {
        "2.0" => Ok(ValidationProfile::Gltf20),
        "2.1" => Ok(ValidationProfile::Gltf21Draft),
        _ => Err(JsValue::from_str("profile must be \"2.0\" or \"2.1\"")),
    }
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

#[wasm_bindgen]
impl GltfDocument {
    /// Parses a JSON glTF or GLB document with the native full-scene model.
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8], validation_profile: &str) -> Result<GltfDocument, JsValue> {
        let profile = profile(validation_profile)?;
        parse_native(data, profile)
            .map(|import| Self { import })
            .map_err(wasm_error)
    }

    /// Returns the lossless JSON document. Untouched JSON retains its source bytes.
    pub fn json(&self) -> Result<Vec<u8>, JsValue> {
        self.import.document.to_json_bytes().map_err(wasm_error)
    }

    /// Returns the selected GLB container version, consolidating all buffers.
    pub fn glb(&self, version: u32) -> Result<Vec<u8>, JsValue> {
        let output = match version {
            2 => OutputFormat::GlbV2,
            3 => OutputFormat::GlbV3,
            _ => return Err(JsValue::from_str("GLB version must be 2 or 3")),
        };
        self.import.to_bytes(output).map_err(wasm_error)
    }

    /// Strictly validates the native document with the selected glTF profile.
    pub fn validate(&self, validation_profile: &str) -> Result<(), JsValue> {
        self.import
            .document
            .validate(profile(validation_profile)?)
            .map_err(wasm_error)
    }

    /// Returns one root object as JSON. `kind` is a glTF root array name.
    pub fn object_json(&self, kind: &str, index: usize) -> Result<Vec<u8>, JsValue> {
        let supported = [
            "accessors",
            "animations",
            "buffers",
            "bufferViews",
            "cameras",
            "files",
            "images",
            "materials",
            "meshes",
            "nodes",
            "samplers",
            "scenes",
            "shapes",
            "skins",
            "textures",
        ];
        if !supported.contains(&kind) {
            return Err(JsValue::from_str("unsupported glTF root array"));
        }
        self.import
            .document
            .as_value()
            .get(kind)
            .and_then(|value| value.as_array())
            .and_then(|values| values.get(index))
            .map(|value| value.to_vec())
            .ok_or_else(|| JsValue::from_str("glTF object index is out of range"))
    }

    /// Compresses one ordinary primitive with the native document-preserving transform.
    pub fn compress_primitive(
        &mut self,
        mesh: usize,
        primitive: usize,
        encoding_speed: u8,
        decoding_speed: u8,
    ) -> Result<usize, JsValue> {
        self.import
            .compress_primitive(
                MeshIndex(mesh),
                primitive,
                CompressionOptions {
                    encoding_speed,
                    decoding_speed,
                },
            )
            .map(|report| report.encoded_bytes)
            .map_err(wasm_error)
    }

    /// Materializes every Draco primitive atomically into ordinary geometry.
    pub fn decompress(&mut self) -> Result<(), JsValue> {
        self.import.decompress_in_place().map_err(wasm_error)
    }

    /// Returns a JSON summary of the current full document state.
    pub fn summary(&self) -> JsValue {
        to_js(result(self.import.document.clone()))
    }
}
