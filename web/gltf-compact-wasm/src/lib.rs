//! Compact WebAssembly runtime for reading glTF primitive geometry.
//!
//! The runtime deliberately exposes only geometry-oriented operations. It uses
//! the shared full `Document` internally, accepts JSON glTF and GLB v2/v3, and
//! returns packed byte buffers rather than JavaScript arrays of numeric values.

use std::collections::BTreeMap;

use draco_gltf::{
    parse, parse_with_options, ExtensionRegistry, Import, MeshIndex, PackedPrimitive,
    ResourceLimits, ResourceResolver, ValidationProfile,
};
use js_sys::{Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

/// Stateful compact geometry reader backed by the shared glTF document model.
#[wasm_bindgen]
pub struct CompactGeometry {
    import: Import,
}

/// One decoded primitive whose attribute data remains in packed byte buffers.
#[wasm_bindgen]
pub struct PackedGeometry {
    primitive: PackedPrimitive,
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
        resources.insert(name, Uint8Array::new(&value).to_vec());
    }
    Ok(BrowserResourceResolver(resources))
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

#[wasm_bindgen]
impl CompactGeometry {
    /// Opens a JSON glTF or GLB v2/v3 document with embedded or data-URI resources.
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8], validation_profile: &str) -> Result<CompactGeometry, JsValue> {
        parse(data, profile(validation_profile)?)
            .map(|import| Self { import })
            .map_err(wasm_error)
    }

    /// Opens a document with explicit URI-to-byte resources. Values must be `Uint8Array`.
    /// The reader never fetches network resources itself.
    #[wasm_bindgen(js_name = withResources)]
    pub fn with_resources(
        data: &[u8],
        resources: JsValue,
        validation_profile: &str,
    ) -> Result<CompactGeometry, JsValue> {
        let resolver = browser_resources(resources)?;
        parse_with_options(
            data,
            None,
            Some(&resolver),
            &ResourceLimits::default(),
            profile(validation_profile)?,
            &ExtensionRegistry::default(),
        )
        .map(|import| Self { import })
        .map_err(wasm_error)
    }

    /// Decodes an accessor or `KHR_draco_mesh_compression` primitive into packed buffers.
    #[wasm_bindgen(js_name = decodePrimitive)]
    pub fn decode_primitive(
        &self,
        mesh: usize,
        primitive: usize,
    ) -> Result<PackedGeometry, JsValue> {
        self.import
            .decode_packed_primitive(MeshIndex(mesh), primitive)
            .map(|primitive| PackedGeometry { primitive })
            .map_err(wasm_error)
    }

    pub fn mesh_count(&self) -> usize {
        self.import.document.meshes().len()
    }

    pub fn primitive_count(&self, mesh: usize) -> Result<usize, JsValue> {
        self.import
            .document
            .mesh(MeshIndex(mesh))
            .map(|mesh| {
                mesh.value()
                    .get("primitives")
                    .and_then(|value| value.as_array())
                    .map_or(0, |values| values.len())
            })
            .ok_or_else(|| JsValue::from_str("mesh index is out of range"))
    }
}

#[wasm_bindgen]
impl PackedGeometry {
    pub fn mode(&self) -> u32 {
        self.primitive.mode
    }

    #[wasm_bindgen(js_name = attributeCount)]
    pub fn attribute_count(&self) -> usize {
        self.primitive.attributes.len()
    }

    #[wasm_bindgen(js_name = attributeSemantic)]
    pub fn attribute_semantic(&self, index: usize) -> Result<String, JsValue> {
        self.attribute(index)
            .map(|attribute| attribute.semantic.clone())
    }

    #[wasm_bindgen(js_name = attributeComponents)]
    pub fn attribute_components(&self, index: usize) -> Result<u8, JsValue> {
        self.attribute(index).map(|attribute| attribute.components)
    }

    #[wasm_bindgen(js_name = attributeComponentType)]
    pub fn attribute_component_type(&self, index: usize) -> Result<u32, JsValue> {
        self.attribute(index)
            .map(|attribute| attribute.component_type)
    }

    #[wasm_bindgen(js_name = attributeNormalized)]
    pub fn attribute_normalized(&self, index: usize) -> Result<bool, JsValue> {
        self.attribute(index).map(|attribute| attribute.normalized)
    }

    #[wasm_bindgen(js_name = attributeBytes)]
    pub fn attribute_bytes(&self, index: usize) -> Result<Vec<u8>, JsValue> {
        self.attribute(index)
            .map(|attribute| attribute.bytes.clone())
    }

    #[wasm_bindgen(js_name = hasIndices)]
    pub fn has_indices(&self) -> bool {
        self.primitive.indices.is_some()
    }

    #[wasm_bindgen(js_name = indexComponentType)]
    pub fn index_component_type(&self) -> Result<u32, JsValue> {
        self.indices().map(|indices| indices.component_type)
    }

    #[wasm_bindgen(js_name = indexBytes)]
    pub fn index_bytes(&self) -> Result<Vec<u8>, JsValue> {
        self.indices().map(|indices| indices.bytes.clone())
    }

    fn attribute(&self, index: usize) -> Result<&draco_gltf::PackedAttribute, JsValue> {
        self.primitive
            .attributes
            .get(index)
            .ok_or_else(|| JsValue::from_str("attribute index is out of range"))
    }

    fn indices(&self) -> Result<&draco_gltf::PackedAttribute, JsValue> {
        self.primitive
            .indices
            .as_ref()
            .ok_or_else(|| JsValue::from_str("primitive has no indices"))
    }
}
