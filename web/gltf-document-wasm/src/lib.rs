//! Full glTF document API for browser consumers.

use std::collections::BTreeMap;

use draco_gltf::{
    parse, parse_with_options, CompressionOptions, Document, ExtensionRegistry, Import, MeshIndex,
    OutputFormat, ResourceLimits, ResourceResolver, ValidationProfile,
};
use js_sys::{Object, Reflect, Uint8Array};
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

/// Stateful document API backed directly by `draco_gltf::Import`.
#[wasm_bindgen]
pub struct GltfDocument {
    import: Import,
    resolver: BrowserResourceResolver,
}

#[derive(Clone, Default)]
struct BrowserResourceResolver(BTreeMap<String, Vec<u8>>);

impl ResourceResolver for BrowserResourceResolver {
    fn resolve(&self, uri: &str) -> Result<Vec<u8>, draco_gltf::GltfError> {
        self.0
            .get(uri)
            .cloned()
            .ok_or_else(|| draco_gltf::GltfError::ExternalResourceDenied(uri.into()))
    }
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

fn browser_resources(value: JsValue) -> Result<BrowserResourceResolver, JsValue> {
    if value.is_null() || value.is_undefined() {
        return Ok(BrowserResourceResolver::default());
    }
    let object = Object::from(value);
    let mut resources = BTreeMap::new();
    for key in Object::keys(&object).iter() {
        let name = key
            .as_string()
            .ok_or_else(|| JsValue::from_str("resource key is not a string"))?;
        let value = Reflect::get(&object, &key)
            .map_err(|_| JsValue::from_str("could not read resource value"))?;
        if !value.is_instance_of::<Uint8Array>() {
            return Err(JsValue::from_str(
                "resource values must be Uint8Array instances",
            ));
        }
        let bytes = Uint8Array::new(&value).to_vec();
        resources.insert(name, bytes);
    }
    Ok(BrowserResourceResolver(resources))
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

/// Parses a JSON glTF or GLB document with the lossless document model.
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
    /// Parses a JSON glTF or GLB document with the full-scene model.
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8], validation_profile: &str) -> Result<GltfDocument, JsValue> {
        let profile = profile(validation_profile)?;
        parse(data, profile)
            .map(|import| Self {
                import,
                resolver: BrowserResourceResolver::default(),
            })
            .map_err(wasm_error)
    }

    /// Parses a document with an explicit URI-to-byte resource map. Values in
    /// `resources` must be `Uint8Array`; no browser fetches are performed.
    #[wasm_bindgen(js_name = withResources)]
    pub fn with_resources(
        data: &[u8],
        resources: JsValue,
        validation_profile: &str,
    ) -> Result<GltfDocument, JsValue> {
        let profile = profile(validation_profile)?;
        let resolver = browser_resources(resources)?;
        parse_with_options(
            data,
            None,
            Some(&resolver),
            &ResourceLimits::default(),
            profile,
            &ExtensionRegistry::default(),
        )
        .map(|import| Self { import, resolver })
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

    /// Strictly validates the document with the selected glTF profile.
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

    /// Compresses one ordinary primitive with the document-preserving transform.
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
                    ..CompressionOptions::default()
                },
            )
            .map(|report| report.encoded_bytes)
            .map_err(wasm_error)
    }

    /// Materializes every Draco primitive atomically into ordinary geometry.
    pub fn decompress(&mut self) -> Result<(), JsValue> {
        self.import.decompress_in_place().map_err(wasm_error)
    }

    /// Explicitly loads one entry from the glTF 2.1 `files` array. The returned
    /// document shares the supplied immutable resource map and never fetches.
    pub fn load_asset(
        &self,
        file: usize,
        validation_profile: &str,
        max_depth: usize,
    ) -> Result<GltfDocument, JsValue> {
        self.import
            .load_asset_with_depth(
                draco_gltf::FileIndex(file),
                &self.resolver,
                &ResourceLimits::default(),
                profile(validation_profile)?,
                &ExtensionRegistry::default(),
                max_depth,
            )
            .map(|import| Self {
                import,
                resolver: self.resolver.clone(),
            })
            .map_err(wasm_error)
    }

    /// Returns a JSON summary of the current full document state.
    pub fn summary(&self) -> JsValue {
        to_js(result(self.import.document.clone()))
    }
}
