//! WebAssembly glTF scene and geometry API with optional writing.

use std::collections::BTreeMap;

use draco_gltf::{
    parse, parse_with_options, Document, ExtensionRegistry, Import, JsonValue, OutputFormat,
    ResourceLimits, ResourceResolver, ValidationProfile,
};
#[cfg(feature = "read")]
use draco_gltf::{
    AccessorData, ComponentType, DocumentAccessorSource, PackedAttribute, PackedIndices,
    PrimitiveIndex, PrimitiveMode,
};
#[cfg(feature = "draco-encode")]
use draco_gltf::{CompressionOptions, GeometryEncoding, MeshIndex};
use js_sys::{Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

/// Stateful glTF asset backed by one lossless document and resource store.
#[wasm_bindgen]
pub struct GltfAsset {
    import: Import,
    resolver: BrowserResourceResolver,
}

/// Materialized primitive geometry with contiguous byte buffers.
#[cfg(feature = "read")]
#[wasm_bindgen]
pub struct PackedGeometry {
    mode: PrimitiveMode,
    attributes: Vec<PackedAttribute>,
    indices: Option<PackedIndices>,
}

/// One materialized glTF accessor with tightly packed owned bytes.
#[cfg(feature = "read")]
#[wasm_bindgen]
pub struct PackedAccessor {
    inner: AccessorData,
}

/// Options for writing packed geometry.
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

#[cfg(feature = "read")]
fn component_type(value: u32) -> Result<ComponentType, JsValue> {
    ComponentType::from_gltf(value as u64)
        .ok_or_else(|| JsValue::from_str("unsupported glTF component type"))
}

#[cfg(feature = "read")]
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

#[derive(Default)]
struct AssetSummary {
    success: bool,
    mesh_count: usize,
    primitive_count: usize,
    scene_count: usize,
    uses_draco: bool,
    error: Option<String>,
}

fn asset_summary(document: Document) -> AssetSummary {
    let primitive_count = document
        .meshes()
        .into_iter()
        .map(|mesh| {
            mesh.value()
                .get("primitives")
                .and_then(|value| value.as_array())
                .map_or(0, |values| values.len())
        })
        .sum();
    let uses_draco = document.meshes().into_iter().any(|mesh| {
        mesh.value()
            .get("primitives")
            .and_then(|value| value.as_array())
            .is_some_and(|primitives| {
                primitives.iter().any(|primitive| {
                    primitive
                        .get("extensions")
                        .and_then(|value| value.get("KHR_draco_mesh_compression"))
                        .is_some()
                })
            })
    });
    AssetSummary {
        success: true,
        mesh_count: document.meshes().len(),
        primitive_count,
        scene_count: document.scenes().len(),
        uses_draco,
        error: None,
    }
}

fn summary_to_js(summary: AssetSummary) -> JsValue {
    let object = Object::new();
    let fields = [
        ("success", JsValue::from_bool(summary.success)),
        ("meshCount", JsValue::from_f64(summary.mesh_count as f64)),
        (
            "primitiveCount",
            JsValue::from_f64(summary.primitive_count as f64),
        ),
        ("sceneCount", JsValue::from_f64(summary.scene_count as f64)),
        ("usesDraco", JsValue::from_bool(summary.uses_draco)),
        (
            "error",
            summary
                .error
                .map_or(JsValue::NULL, |error| JsValue::from_str(&error)),
        ),
    ];
    for (key, value) in fields {
        Reflect::set(&object, &JsValue::from_str(key), &value)
            .expect("writing a fresh JavaScript summary object cannot fail");
    }
    object.into()
}

fn value_array(value: Option<&JsonValue>) -> &[JsonValue] {
    value.and_then(JsonValue::as_array).unwrap_or(&[])
}

fn string(value: Option<&JsonValue>, fallback: &str) -> JsonValue {
    JsonValue::from(value.and_then(JsonValue::as_str).unwrap_or(fallback))
}

fn boolean(value: Option<&JsonValue>, fallback: bool) -> JsonValue {
    match value {
        Some(JsonValue::Bool(value)) => JsonValue::Bool(*value),
        _ => JsonValue::Bool(fallback),
    }
}

fn number(value: Option<&JsonValue>, fallback: f64) -> JsonValue {
    value
        .filter(|value| value.as_f64().is_some())
        .cloned()
        .unwrap_or_else(|| JsonValue::Number(fallback.to_string()))
}

fn index(value: Option<&JsonValue>) -> JsonValue {
    value
        .filter(|value| value.as_u64().is_some())
        .cloned()
        .unwrap_or(JsonValue::Null)
}

fn fixed_array(value: Option<&JsonValue>, length: usize, defaults: &[f64]) -> JsonValue {
    let Some(values) = value.and_then(JsonValue::as_array) else {
        return JsonValue::Array(
            defaults
                .iter()
                .map(|value| JsonValue::Number(value.to_string()))
                .collect(),
        );
    };
    if values.len() != length || values.iter().any(|value| value.as_f64().is_none()) {
        return JsonValue::Array(
            defaults
                .iter()
                .map(|value| JsonValue::Number(value.to_string()))
                .collect(),
        );
    }
    JsonValue::Array(values.to_vec())
}

fn name(value: &JsonValue, prefix: &str, index: usize) -> JsonValue {
    string(value.get("name"), &format!("{prefix}_{index}"))
}

fn preview_manifest(document: &Document) -> JsonValue {
    const SUPPORTED_EXTENSIONS: &[&str] = &["KHR_materials_unlit", "KHR_texture_transform"];
    let root = document.as_value();
    let mut warnings = Vec::new();
    let extensions_used = value_array(root.get("extensionsUsed"));
    let unsupported: Vec<_> = extensions_used
        .iter()
        .filter_map(JsonValue::as_str)
        .filter(|extension| !SUPPORTED_EXTENSIONS.contains(extension))
        .collect();
    if !unsupported.is_empty() {
        warnings.push(JsonValue::from(format!(
            "Unsupported glTF extensions ignored: {}",
            unsupported.join(", ")
        )));
    }
    if value_array(root.get("extensionsRequired"))
        .iter()
        .filter_map(JsonValue::as_str)
        .any(|extension| !SUPPORTED_EXTENSIONS.contains(&extension))
    {
        warnings.push(JsonValue::from(
            "Model requires extensions that this viewer ignores; rendering may be incomplete",
        ));
    }

    let nodes = value_array(root.get("nodes"))
        .iter()
        .enumerate()
        .map(|(node_index, node)| {
            JsonValue::object([
                ("name", name(node, "node", node_index)),
                (
                    "translation",
                    fixed_array(node.get("translation"), 3, &[0.0, 0.0, 0.0]),
                ),
                (
                    "rotation",
                    fixed_array(node.get("rotation"), 4, &[0.0, 0.0, 0.0, 1.0]),
                ),
                ("scale", fixed_array(node.get("scale"), 3, &[1.0, 1.0, 1.0])),
                (
                    "matrix",
                    node.get("matrix").cloned().unwrap_or(JsonValue::Null),
                ),
                (
                    "children",
                    node.get("children")
                        .cloned()
                        .unwrap_or(JsonValue::Array(Vec::new())),
                ),
                ("mesh", index(node.get("mesh"))),
                ("skin", index(node.get("skin"))),
            ])
        })
        .collect();

    let meshes = value_array(root.get("meshes"))
        .iter()
        .enumerate()
        .map(|(mesh_index, mesh)| {
            let primitives = value_array(mesh.get("primitives"))
                .iter()
                .map(|primitive| {
                    if primitive.get("targets").is_some() {
                        warnings.push(JsonValue::from(
                            "Morph target animation is not supported by the preview; targets are ignored",
                        ));
                    }
                    JsonValue::object([("material", index(primitive.get("material")))])
                })
                .collect();
            JsonValue::object([
                ("name", name(mesh, "mesh", mesh_index)),
                ("primitives", JsonValue::Array(primitives)),
            ])
        })
        .collect();

    let materials = value_array(root.get("materials"))
        .iter()
        .enumerate()
        .map(|(material_index, material)| {
            let pbr = material.get("pbrMetallicRoughness");
            let texture = pbr.and_then(|pbr| pbr.get("baseColorTexture"));
            let transform = texture.and_then(|texture| {
                texture
                    .get("extensions")
                    .and_then(|extensions| extensions.get("KHR_texture_transform"))
            });
            JsonValue::object([
                ("name", name(material, "material", material_index)),
                (
                    "baseColorFactor",
                    fixed_array(
                        pbr.and_then(|pbr| pbr.get("baseColorFactor")),
                        4,
                        &[1.0, 1.0, 1.0, 1.0],
                    ),
                ),
                (
                    "baseColorTexture",
                    index(texture.and_then(|texture| texture.get("index"))),
                ),
                (
                    "baseColorTexCoord",
                    index(transform.and_then(|transform| transform.get("texCoord")))
                        .as_u64()
                        .map(JsonValue::from)
                        .unwrap_or_else(|| {
                            index(texture.and_then(|texture| texture.get("texCoord")))
                        }),
                ),
                (
                    "baseColorTextureTransform",
                    JsonValue::object([
                        (
                            "offset",
                            fixed_array(
                                transform.and_then(|transform| transform.get("offset")),
                                2,
                                &[0.0, 0.0],
                            ),
                        ),
                        (
                            "scale",
                            fixed_array(
                                transform.and_then(|transform| transform.get("scale")),
                                2,
                                &[1.0, 1.0],
                            ),
                        ),
                        (
                            "rotation",
                            number(
                                transform.and_then(|transform| transform.get("rotation")),
                                0.0,
                            ),
                        ),
                    ]),
                ),
                ("doubleSided", boolean(material.get("doubleSided"), false)),
                ("alphaMode", string(material.get("alphaMode"), "OPAQUE")),
                ("alphaCutoff", number(material.get("alphaCutoff"), 0.5)),
                (
                    "unlit",
                    JsonValue::Bool(
                        material
                            .get("extensions")
                            .and_then(|extensions| extensions.get("KHR_materials_unlit"))
                            .is_some(),
                    ),
                ),
            ])
        })
        .collect();

    let images = value_array(root.get("images"))
        .iter()
        .enumerate()
        .map(|(image_index, image)| {
            JsonValue::object([
                ("name", name(image, "image", image_index)),
                ("uri", image.get("uri").cloned().unwrap_or(JsonValue::Null)),
                ("bufferView", index(image.get("bufferView"))),
                (
                    "mimeType",
                    image.get("mimeType").cloned().unwrap_or(JsonValue::Null),
                ),
            ])
        })
        .collect();
    let samplers = value_array(root.get("samplers"))
        .iter()
        .map(|sampler| {
            JsonValue::object([
                ("wrapS", number(sampler.get("wrapS"), 10497.0)),
                ("wrapT", number(sampler.get("wrapT"), 10497.0)),
                ("minFilter", number(sampler.get("minFilter"), 9987.0)),
                ("magFilter", number(sampler.get("magFilter"), 9729.0)),
            ])
        })
        .collect();
    let textures = value_array(root.get("textures"))
        .iter()
        .enumerate()
        .map(|(texture_index, texture)| {
            JsonValue::object([
                ("name", name(texture, "texture", texture_index)),
                ("source", index(texture.get("source"))),
                ("sampler", index(texture.get("sampler"))),
            ])
        })
        .collect();

    let skins = value_array(root.get("skins"))
        .iter()
        .enumerate()
        .map(|(skin_index, skin)| {
            JsonValue::object([
                ("name", name(skin, "skin", skin_index)),
                (
                    "joints",
                    skin.get("joints")
                        .cloned()
                        .unwrap_or(JsonValue::Array(Vec::new())),
                ),
                (
                    "inverseBindMatrices",
                    index(skin.get("inverseBindMatrices")),
                ),
            ])
        })
        .collect();
    let animations = value_array(root.get("animations"))
        .iter()
        .enumerate()
        .map(|(animation_index, animation)| {
            let samplers = value_array(animation.get("samplers"))
                .iter()
                .map(|sampler| {
                    JsonValue::object([
                        ("input", index(sampler.get("input"))),
                        ("output", index(sampler.get("output"))),
                        (
                            "interpolation",
                            string(sampler.get("interpolation"), "LINEAR"),
                        ),
                    ])
                })
                .collect();
            let channels = value_array(animation.get("channels"))
                .iter()
                .map(|channel| {
                    let target = channel.get("target");
                    JsonValue::object([
                        ("sampler", index(channel.get("sampler"))),
                        ("node", index(target.and_then(|target| target.get("node")))),
                        (
                            "path",
                            string(target.and_then(|target| target.get("path")), "translation"),
                        ),
                    ])
                })
                .collect();
            JsonValue::object([
                ("name", name(animation, "animation", animation_index)),
                ("samplers", JsonValue::Array(samplers)),
                ("channels", JsonValue::Array(channels)),
            ])
        })
        .collect();
    let scenes = value_array(root.get("scenes"))
        .iter()
        .map(|scene| {
            JsonValue::object([(
                "nodes",
                scene
                    .get("nodes")
                    .cloned()
                    .unwrap_or(JsonValue::Array(Vec::new())),
            )])
        })
        .collect();
    let scene_index = root.get("scene").and_then(JsonValue::as_u64).unwrap_or(0);
    let root_indices = value_array(root.get("scenes"))
        .get(scene_index as usize)
        .and_then(|scene| scene.get("nodes"))
        .cloned()
        .or_else(|| {
            value_array(root.get("scenes"))
                .first()
                .and_then(|scene| scene.get("nodes"))
                .cloned()
        })
        .unwrap_or(JsonValue::Array(Vec::new()));

    JsonValue::object([
        ("nodes", JsonValue::Array(nodes)),
        ("meshes", JsonValue::Array(meshes)),
        ("materials", JsonValue::Array(materials)),
        ("images", JsonValue::Array(images)),
        ("samplers", JsonValue::Array(samplers)),
        ("textures", JsonValue::Array(textures)),
        ("skins", JsonValue::Array(skins)),
        ("animations", JsonValue::Array(animations)),
        ("scenes", JsonValue::Array(scenes)),
        ("rootIndices", root_indices),
        ("warnings", JsonValue::Array(warnings)),
    ])
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

/// Returns the file extensions accepted by this module.
#[wasm_bindgen]
pub fn supported_extensions() -> Vec<String> {
    vec!["gltf".into(), "glb".into()]
}

/// Inspects JSON glTF or GLB without resolving external resources.
#[wasm_bindgen]
pub fn inspect_gltf(data: &[u8]) -> JsValue {
    let json = if data.len() >= 4 && &data[..4] == b"glTF" {
        match draco_io::parse_gltf_container(data) {
            Ok(container) => container.json,
            Err(error) => {
                return summary_to_js(AssetSummary {
                    error: Some(error.to_string()),
                    ..AssetSummary::default()
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
        Ok(document) => summary_to_js(asset_summary(document)),
        Err(error) => summary_to_js(AssetSummary {
            error: Some(error.to_string()),
            ..AssetSummary::default()
        }),
    }
}

#[wasm_bindgen]
impl GltfAsset {
    /// Opens JSON glTF or GLB v2/v3 with embedded or data-URI resources.
    #[wasm_bindgen(constructor)]
    pub fn new(data: &[u8], validation_profile: &str) -> Result<GltfAsset, JsValue> {
        parse(data, profile(validation_profile)?)
            .map(|import| Self {
                import,
                resolver: BrowserResourceResolver::default(),
            })
            .map_err(wasm_error)
    }

    /// Opens a document with an explicit URI-to-`Uint8Array` resource map.
    #[wasm_bindgen(js_name = withResources)]
    pub fn with_resources(
        data: &[u8],
        resources: JsValue,
        validation_profile: &str,
    ) -> Result<GltfAsset, JsValue> {
        let resolver = browser_resources(resources)?;
        parse_with_options(
            data,
            None,
            Some(&resolver),
            &ResourceLimits::default(),
            profile(validation_profile)?,
            &ExtensionRegistry::default(),
        )
        .map(|import| Self { import, resolver })
        .map_err(wasm_error)
    }

    /// Reads an ordinary or Draco-compressed primitive into packed buffers.
    #[cfg(feature = "read")]
    #[wasm_bindgen(js_name = readPrimitive)]
    pub fn read_primitive(&self, mesh: usize, primitive: usize) -> Result<PackedGeometry, JsValue> {
        self.import
            .read_primitive(PrimitiveIndex::new(draco_gltf::MeshIndex(mesh), primitive))
            .map(PackedGeometry::from_inner)
            .map_err(wasm_error)
    }

    /// Materializes any accessor into tightly packed little-endian bytes.
    ///
    /// Sparse values are applied and interleaved input is deinterleaved. The
    /// returned [`PackedAccessor`] owns its payload, so later document changes
    /// cannot invalidate JavaScript views created from it.
    #[cfg(feature = "read")]
    #[wasm_bindgen(js_name = readAccessor)]
    pub fn read_accessor(&self, index: usize) -> Result<PackedAccessor, JsValue> {
        DocumentAccessorSource::new(&self.import.document, &self.import.resources)
            .read_accessor(index)
            .map(|inner| PackedAccessor { inner })
            .map_err(wasm_error)
    }

    /// Copies one resolved buffer view while retaining its original layout.
    ///
    /// This is intended for embedded images and extension payloads. Accessor
    /// consumers should prefer [`GltfAsset::read_accessor`].
    #[cfg(feature = "read")]
    #[wasm_bindgen(js_name = bufferViewBytes)]
    pub fn buffer_view_bytes(&self, index: usize) -> Result<Vec<u8>, JsValue> {
        DocumentAccessorSource::new(&self.import.document, &self.import.resources)
            .read_buffer_view(index)
            .map_err(wasm_error)
    }

    /// Returns the number of resolved glTF buffers.
    #[cfg(feature = "raw-resources")]
    #[wasm_bindgen(js_name = bufferCount)]
    pub fn buffer_count(&self) -> usize {
        self.import.resources.buffers.len()
    }

    /// Copies one complete resolved glTF buffer into JavaScript.
    ///
    /// This operation can duplicate a large allocation. Prefer
    /// [`GltfAsset::read_accessor`] or [`GltfAsset::buffer_view_bytes`] when a
    /// narrower range is sufficient.
    #[cfg(feature = "raw-resources")]
    #[wasm_bindgen(js_name = bufferBytes)]
    pub fn buffer_bytes(&self, index: usize) -> Result<Vec<u8>, JsValue> {
        self.import
            .resources
            .buffers
            .get(index)
            .cloned()
            .ok_or_else(|| JsValue::from_str("buffer index is out of range"))
    }

    /// Returns the number of meshes in the document.
    #[cfg(feature = "read")]
    #[wasm_bindgen(js_name = meshCount)]
    pub fn mesh_count(&self) -> usize {
        self.import.document.meshes().len()
    }

    /// Returns the number of primitives in one mesh.
    #[cfg(feature = "read")]
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
    ) -> Result<GltfAsset, JsValue> {
        let profile = profile(validation_profile)?;
        let geometry = geometry.to_inner(profile)?;
        Import::from_geometry(&geometry, profile, options.inner)
            .map(|import| Self {
                import,
                resolver: BrowserResourceResolver::default(),
            })
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

    /// Returns the lossless JSON document. Untouched JSON keeps its source bytes.
    pub fn json(&self) -> Result<Vec<u8>, JsValue> {
        self.import.document.to_json_bytes().map_err(wasm_error)
    }

    /// Returns the normalized scene description consumed by the web preview.
    ///
    /// Geometry and accessors remain addressable through [`GltfAsset::read_primitive`]
    /// and [`GltfAsset::read_accessor`], while this manifest owns glTF-specific
    /// scene, material, animation, sampler, and extension interpretation.
    #[wasm_bindgen(js_name = previewManifest)]
    pub fn preview_manifest(&self) -> Vec<u8> {
        preview_manifest(&self.import.document).to_vec()
    }

    /// Returns minified JSON while preserving object order and number lexemes.
    #[wasm_bindgen(js_name = minifiedJson)]
    pub fn minified_json(&self) -> Vec<u8> {
        self.import.document.to_minified_json_bytes()
    }

    /// Serializes a GLB version 2 or 3 container.
    pub fn glb(&self, version: u32) -> Result<Vec<u8>, JsValue> {
        let output = match version {
            2 => OutputFormat::GlbV2,
            3 => OutputFormat::GlbV3,
            _ => return Err(JsValue::from_str("GLB version must be 2 or 3")),
        };
        self.import.to_bytes(output).map_err(wasm_error)
    }

    /// Strictly validates the asset with the selected glTF profile.
    pub fn validate(&self, validation_profile: &str) -> Result<(), JsValue> {
        self.import
            .document
            .validate(profile(validation_profile)?)
            .map_err(wasm_error)
    }

    /// Returns one root-array object as JSON bytes.
    #[wasm_bindgen(js_name = objectJson)]
    pub fn object_json(&self, kind: &str, index: usize) -> Result<Vec<u8>, JsValue> {
        const ROOT_ARRAYS: &[&str] = &[
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
        if !ROOT_ARRAYS.contains(&kind) {
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

    /// Explicitly loads one glTF 2.1 `files` entry using the supplied resource map.
    #[wasm_bindgen(js_name = loadAsset)]
    pub fn load_asset(
        &self,
        file: usize,
        validation_profile: &str,
        max_depth: usize,
    ) -> Result<GltfAsset, JsValue> {
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

    /// Returns mesh, scene, primitive, and Draco usage counts.
    pub fn summary(&self) -> JsValue {
        summary_to_js(asset_summary(self.import.document.clone()))
    }

    /// Materializes every Draco primitive atomically into ordinary accessors.
    #[cfg(all(feature = "write", feature = "draco-decode"))]
    pub fn decompress(&mut self) -> Result<(), JsValue> {
        self.import.decompress_in_place().map_err(wasm_error)
    }

    /// Stores one primitive with document-preserving Draco compression.
    #[cfg(feature = "draco-encode")]
    #[wasm_bindgen(js_name = compressPrimitive)]
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
}

#[cfg(feature = "read")]
#[wasm_bindgen]
impl PackedAccessor {
    /// Returns the number of accessor elements.
    pub fn count(&self) -> usize {
        self.inner.count
    }

    /// Returns the original glTF shape (`SCALAR`, `VEC*`, or `MAT*`).
    #[wasm_bindgen(js_name = accessorType)]
    pub fn accessor_type(&self) -> String {
        self.inner.accessor_type.clone()
    }

    /// Returns the number of scalar components in one element.
    pub fn components(&self) -> u8 {
        self.inner.components
    }

    /// Returns the original glTF component type code.
    #[wasm_bindgen(js_name = componentType)]
    pub fn component_type(&self) -> u32 {
        self.inner.component_type
    }

    /// Returns whether integer components use normalized interpretation.
    pub fn normalized(&self) -> bool {
        self.inner.normalized
    }

    /// Copies the tightly packed little-endian payload into JavaScript.
    /// Matrix values retain glTF's column-major component order.
    pub fn bytes(&self) -> Vec<u8> {
        self.inner.bytes.clone()
    }
}

#[cfg(feature = "read")]
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

#[cfg(feature = "read")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_manifest_normalizes_texture_transform_and_scene_links() {
        let document = Document::from_json_bytes(
            br#"{
                "asset":{"version":"2.0"},
                "extensionsUsed":["KHR_texture_transform"],
                "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{
                    "index":3,"texCoord":0,"extensions":{"KHR_texture_transform":{
                        "texCoord":1,"offset":[0.25,0.5],"scale":[2,3],"rotation":0.5
                    }}
                }}}],
                "textures":[{"source":0}],
                "images":[{"uri":"base.png"}],
                "meshes":[{"primitives":[{"material":0}]}],
                "nodes":[{"mesh":0}],
                "scenes":[{"nodes":[0]}],"scene":0
            }"#,
        )
        .unwrap();

        let manifest = preview_manifest(&document);
        assert_eq!(manifest["rootIndices"][0].as_u64(), Some(0));
        assert_eq!(
            manifest["materials"][0]["baseColorTexture"].as_u64(),
            Some(3)
        );
        assert_eq!(
            manifest["materials"][0]["baseColorTexCoord"].as_u64(),
            Some(1)
        );
        assert_eq!(
            manifest["materials"][0]["baseColorTextureTransform"]["offset"][0].as_f64(),
            Some(0.25)
        );
        assert_eq!(manifest["images"][0]["uri"].as_str(), Some("base.png"));
        assert!(manifest["warnings"].as_array().unwrap().is_empty());
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
