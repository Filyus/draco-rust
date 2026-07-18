//! Compact WebAssembly profile for reading and optionally writing glTF geometry.

use std::collections::BTreeMap;

#[cfg(feature = "draco-encode")]
use draco_gltf::GeometryEncoding;
#[cfg(feature = "write")]
use draco_gltf::OutputFormat;
use draco_gltf::{
    parse, parse_with_options, ComponentType, ExtensionRegistry, Import, PackedAttribute,
    PackedIndices, PrimitiveIndex, PrimitiveMode, ResourceLimits, ResourceResolver,
    ValidationProfile,
};
use js_sys::{Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

/// Stateful compact handle backed by one lossless document and resource store.
#[wasm_bindgen]
pub struct CompactDocument {
    import: Import,
}

/// Materialized primitive geometry with contiguous byte buffers.
#[wasm_bindgen]
pub struct PackedGeometry {
    mode: PrimitiveMode,
    attributes: Vec<PackedAttribute>,
    indices: Option<PackedIndices>,
}

/// Options for a compact geometry write.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub struct GeometryWriteOptions {
    inner: draco_gltf::GeometryWriteOptions,
}

/// JSON glTF output plus its companion resources.
#[cfg(feature = "write")]
#[wasm_bindgen]
pub struct GltfBundle {
    inner: draco_gltf::GltfOutput,
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

fn component_type(value: u32) -> Result<ComponentType, JsValue> {
    ComponentType::from_gltf(value as u64)
        .ok_or_else(|| JsValue::from_str("unsupported glTF component type"))
}

fn primitive_mode(value: u32) -> Result<PrimitiveMode, JsValue> {
    PrimitiveMode::from_gltf(value)
        .ok_or_else(|| JsValue::from_str("primitive mode must be in 0..=6"))
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
impl CompactDocument {
    /// Opens JSON glTF or GLB v2/v3 with embedded or data-URI resources.
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8], validation_profile: &str) -> Result<CompactDocument, JsValue> {
        parse(data, profile(validation_profile)?)
            .map(|import| Self { import })
            .map_err(wasm_error)
    }

    /// Opens a document with an explicit URI-to-`Uint8Array` resource map.
    #[wasm_bindgen(js_name = withResources)]
    pub fn with_resources(
        data: &[u8],
        resources: JsValue,
        validation_profile: &str,
    ) -> Result<CompactDocument, JsValue> {
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

    /// Reads an ordinary or Draco-compressed primitive into packed buffers.
    #[wasm_bindgen(js_name = readPrimitive)]
    pub fn read_primitive(&self, mesh: usize, primitive: usize) -> Result<PackedGeometry, JsValue> {
        self.import
            .read_primitive(PrimitiveIndex::new(draco_gltf::MeshIndex(mesh), primitive))
            .map(PackedGeometry::from_inner)
            .map_err(wasm_error)
    }

    /// Returns the number of meshes in the document.
    #[wasm_bindgen(js_name = meshCount)]
    pub fn mesh_count(&self) -> usize {
        self.import.document.meshes().len()
    }

    /// Returns the number of primitives in one mesh.
    #[wasm_bindgen(js_name = primitiveCount)]
    pub fn primitive_count(&self, mesh: usize) -> Result<usize, JsValue> {
        self.import
            .document
            .mesh(draco_gltf::MeshIndex(mesh))
            .map(|mesh| {
                mesh.value()
                    .get("primitives")
                    .and_then(|value| value.as_array())
                    .map_or(0, |values| values.len())
            })
            .ok_or_else(|| JsValue::from_str("mesh index is out of range"))
    }

    /// Replaces one primitive with packed geometry.
    #[cfg(feature = "write")]
    #[wasm_bindgen(js_name = writePrimitive)]
    pub fn write_primitive(
        &mut self,
        mesh: usize,
        primitive: usize,
        geometry: &PackedGeometry,
        options: &GeometryWriteOptions,
    ) -> Result<(), JsValue> {
        let geometry = geometry.to_inner(ValidationProfile::Gltf21Draft)?;
        self.import
            .write_primitive(
                PrimitiveIndex::new(draco_gltf::MeshIndex(mesh), primitive),
                &geometry,
                options.inner,
            )
            .map(|_| ())
            .map_err(wasm_error)
    }

    /// Appends packed geometry to one mesh and returns its primitive index.
    #[cfg(feature = "write")]
    #[wasm_bindgen(js_name = pushPrimitive)]
    pub fn push_primitive(
        &mut self,
        mesh: usize,
        geometry: &PackedGeometry,
        options: &GeometryWriteOptions,
    ) -> Result<usize, JsValue> {
        let geometry = geometry.to_inner(ValidationProfile::Gltf21Draft)?;
        self.import
            .push_primitive(draco_gltf::MeshIndex(mesh), &geometry, options.inner)
            .map(|index| index.primitive)
            .map_err(wasm_error)
    }

    /// Creates a minimal scene from one packed primitive.
    #[cfg(feature = "write")]
    #[wasm_bindgen(js_name = fromGeometry)]
    pub fn from_geometry(
        geometry: &PackedGeometry,
        validation_profile: &str,
        options: &GeometryWriteOptions,
    ) -> Result<CompactDocument, JsValue> {
        let profile = profile(validation_profile)?;
        let geometry = geometry.to_inner(profile)?;
        Import::from_geometry(&geometry, profile, options.inner)
            .map(|import| Self { import })
            .map_err(wasm_error)
    }

    /// Serializes a JSON glTF bundle with companion resources.
    #[cfg(feature = "write")]
    #[wasm_bindgen(js_name = gltfBundle)]
    pub fn gltf_bundle(&self) -> Result<GltfBundle, JsValue> {
        self.import
            .to_gltf_output()
            .map(|inner| GltfBundle { inner })
            .map_err(wasm_error)
    }

    /// Serializes a GLB version 2 or 3 container.
    #[cfg(feature = "write")]
    pub fn glb(&self, version: u32) -> Result<Vec<u8>, JsValue> {
        let output = match version {
            2 => OutputFormat::GlbV2,
            3 => OutputFormat::GlbV3,
            _ => return Err(JsValue::from_str("GLB version must be 2 or 3")),
        };
        self.import.to_bytes(output).map_err(wasm_error)
    }
}

#[wasm_bindgen]
impl PackedGeometry {
    /// Creates an initially empty packed primitive with the selected topology.
    #[wasm_bindgen(constructor)]
    pub fn new(mode: u32) -> Result<PackedGeometry, JsValue> {
        Ok(Self {
            mode: primitive_mode(mode)?,
            attributes: Vec::new(),
            indices: None,
        })
    }

    /// Adds one tightly packed vertex attribute.
    #[wasm_bindgen(js_name = addAttribute)]
    pub fn add_attribute(
        &mut self,
        semantic: &str,
        count: usize,
        components: u8,
        component_type_code: u32,
        normalized: bool,
        bytes: &[u8],
    ) -> Result<(), JsValue> {
        if self
            .attributes
            .iter()
            .any(|attribute| attribute.semantic() == semantic)
        {
            return Err(JsValue::from_str("attribute semantic already exists"));
        }
        let attribute = PackedAttribute::new(
            semantic,
            count,
            components,
            component_type(component_type_code)?,
            normalized,
            bytes.to_vec(),
        )
        .map_err(wasm_error)?;
        self.attributes.push(attribute);
        Ok(())
    }

    /// Replaces the tightly packed scalar index stream.
    #[wasm_bindgen(js_name = setIndices)]
    pub fn set_indices(
        &mut self,
        count: usize,
        component_type_code: u32,
        bytes: &[u8],
    ) -> Result<(), JsValue> {
        self.indices = Some(
            PackedIndices::new(count, component_type(component_type_code)?, bytes.to_vec())
                .map_err(wasm_error)?,
        );
        Ok(())
    }

    /// Validates this value for glTF 2.0 or the pinned 2.1 profile.
    pub fn validate(&self, validation_profile: &str) -> Result<(), JsValue> {
        self.to_inner(profile(validation_profile)?).map(|_| ())
    }

    /// Returns the glTF primitive mode code.
    pub fn mode(&self) -> u32 {
        self.mode.to_gltf()
    }

    #[wasm_bindgen(js_name = attributeCount)]
    pub fn attribute_count(&self) -> usize {
        self.attributes.len()
    }

    #[wasm_bindgen(js_name = attributeSemantic)]
    pub fn attribute_semantic(&self, index: usize) -> Result<String, JsValue> {
        self.attribute(index)
            .map(|attribute| attribute.semantic().to_owned())
    }

    #[wasm_bindgen(js_name = attributeElementCount)]
    pub fn attribute_element_count(&self, index: usize) -> Result<usize, JsValue> {
        self.attribute(index).map(PackedAttribute::count)
    }

    #[wasm_bindgen(js_name = attributeComponents)]
    pub fn attribute_components(&self, index: usize) -> Result<u8, JsValue> {
        self.attribute(index).map(PackedAttribute::components)
    }

    #[wasm_bindgen(js_name = attributeComponentType)]
    pub fn attribute_component_type(&self, index: usize) -> Result<u32, JsValue> {
        self.attribute(index)
            .map(|attribute| attribute.component_type().to_gltf())
    }

    #[wasm_bindgen(js_name = attributeNormalized)]
    pub fn attribute_normalized(&self, index: usize) -> Result<bool, JsValue> {
        self.attribute(index).map(PackedAttribute::normalized)
    }

    #[wasm_bindgen(js_name = attributeBytes)]
    pub fn attribute_bytes(&self, index: usize) -> Result<Vec<u8>, JsValue> {
        self.attribute(index)
            .map(|attribute| attribute.bytes().to_vec())
    }

    #[wasm_bindgen(js_name = hasIndices)]
    pub fn has_indices(&self) -> bool {
        self.indices.is_some()
    }

    #[wasm_bindgen(js_name = indexCount)]
    pub fn index_count(&self) -> Result<usize, JsValue> {
        self.indices().map(PackedIndices::count)
    }

    #[wasm_bindgen(js_name = indexComponentType)]
    pub fn index_component_type(&self) -> Result<u32, JsValue> {
        self.indices()
            .map(|indices| indices.component_type().to_gltf())
    }

    #[wasm_bindgen(js_name = indexBytes)]
    pub fn index_bytes(&self) -> Result<Vec<u8>, JsValue> {
        self.indices().map(|indices| indices.bytes().to_vec())
    }
}

impl PackedGeometry {
    fn from_inner(inner: draco_gltf::PackedGeometry) -> Self {
        Self {
            mode: inner.mode(),
            attributes: inner.attributes().to_vec(),
            indices: inner.indices().cloned(),
        }
    }

    fn to_inner(&self, profile: ValidationProfile) -> Result<draco_gltf::PackedGeometry, JsValue> {
        let geometry = draco_gltf::PackedGeometry::new(
            self.mode,
            self.attributes.clone(),
            self.indices.clone(),
        )
        .map_err(wasm_error)?;
        geometry.validate(profile).map_err(wasm_error)?;
        Ok(geometry)
    }

    fn attribute(&self, index: usize) -> Result<&PackedAttribute, JsValue> {
        self.attributes
            .get(index)
            .ok_or_else(|| JsValue::from_str("attribute index is out of range"))
    }

    fn indices(&self) -> Result<&PackedIndices, JsValue> {
        self.indices
            .as_ref()
            .ok_or_else(|| JsValue::from_str("primitive has no indices"))
    }
}

#[cfg(feature = "write")]
#[wasm_bindgen]
impl GeometryWriteOptions {
    /// Creates raw-accessor write options.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            inner: draco_gltf::GeometryWriteOptions::default(),
        }
    }

    /// Selects Draco-only or Draco-with-fallback storage.
    #[cfg(feature = "draco-encode")]
    #[wasm_bindgen(js_name = useDraco)]
    pub fn use_draco(&mut self, encoding_speed: u8, decoding_speed: u8, fallback: bool) {
        self.inner.encoding = GeometryEncoding::Draco(draco_gltf::CompressionOptions {
            encoding_speed,
            decoding_speed,
            mode: if fallback {
                draco_gltf::CompressionMode::Fallback
            } else {
                draco_gltf::CompressionMode::DracoOnly
            },
            ..draco_gltf::CompressionOptions::default()
        });
    }
}

#[cfg(feature = "write")]
impl Default for GeometryWriteOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "write")]
#[wasm_bindgen]
impl GltfBundle {
    /// Returns the JSON `.gltf` bytes.
    pub fn json(&self) -> Vec<u8> {
        self.inner.json.clone()
    }

    /// Returns the number of companion resources.
    #[wasm_bindgen(js_name = resourceCount)]
    pub fn resource_count(&self) -> usize {
        self.inner.resources.len()
    }

    /// Returns one companion resource URI.
    #[wasm_bindgen(js_name = resourceUri)]
    pub fn resource_uri(&self, index: usize) -> Result<String, JsValue> {
        self.inner
            .resources
            .get(index)
            .map(|resource| resource.uri.clone())
            .ok_or_else(|| JsValue::from_str("resource index is out of range"))
    }

    /// Returns one companion resource payload.
    #[wasm_bindgen(js_name = resourceBytes)]
    pub fn resource_bytes(&self, index: usize) -> Result<Vec<u8>, JsValue> {
        self.inner
            .resources
            .get(index)
            .map(|resource| resource.bytes.clone())
            .ok_or_else(|| JsValue::from_str("resource index is out of range"))
    }
}
