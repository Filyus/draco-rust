//! Extension contracts for the lossless document model.

use std::sync::Arc;

use crate::json::Value;
use draco_core::Mesh;
#[cfg(feature = "draco-decode")]
use draco_core::{DecoderBuffer, MeshDecoder};

use crate::{Document, Error, PrimitiveRef, Result};

/// Extension name for the Khronos Draco mesh compression contract.
pub const KHR_DRACO_MESH_COMPRESSION: &str = "KHR_draco_mesh_compression";

/// Extension name for the meshoptimizer buffer view compression contract.
///
/// The import path decodes it eagerly into the fallback buffers, so the rest of
/// the crate never sees a compressed buffer view.
pub const EXT_MESHOPT_COMPRESSION: &str = "EXT_meshopt_compression";

/// The name gltfpack wrote before the extension was ratified under the `EXT_`
/// vendor prefix.
///
/// The extension object, the bitstream and the fallback-buffer convention are
/// identical, so assets carrying the older name decode through exactly the same
/// path. Refusing them means refusing a file over its spelling.
pub const KHR_MESHOPT_COMPRESSION: &str = "KHR_meshopt_compression";

/// Reads a `extensions` object's meshopt entry under either spelling.
pub fn meshopt_extension(extensions: Option<&Value>) -> Option<(&'static str, &Value)> {
    let extensions = extensions?;
    for name in [EXT_MESHOPT_COMPRESSION, KHR_MESHOPT_COMPRESSION] {
        if let Some(value) = extensions.get(name) {
            return Some((name, value));
        }
    }
    None
}

/// The mutable form of [`meshopt_extension`].
pub fn meshopt_extension_mut(extensions: Option<&mut Value>) -> Option<(&'static str, &mut Value)> {
    let extensions = extensions?;
    let name = if extensions.get(EXT_MESHOPT_COMPRESSION).is_some() {
        EXT_MESHOPT_COMPRESSION
    } else if extensions.get(KHR_MESHOPT_COMPRESSION).is_some() {
        KHR_MESHOPT_COMPRESSION
    } else {
        return None;
    };
    extensions.get_mut(name).map(|value| (name, value))
}

/// Extensions whose specifications name no accessor and no buffer view.
///
/// Every entry is an assertion about a published specification, not a guess
/// from the extension's prefix: the JSON these define is factors, colors,
/// names, enum values and indices into `materials`, `textures` or their own
/// root arrays — never into `accessors` or `bufferViews`. A binary transform
/// therefore cannot invalidate them, and nothing has to be remapped.
///
/// The list matters because the safety check in `Import` is whole-document: an
/// unregistered extension anywhere refuses Draco compression for the entire
/// file. Before this list existed that refused 21 of the 70 corpus assets over
/// extensions that describe how a surface is lit.
pub const BINARY_FREE_EXTENSIONS: &[&str] = &[
    // The layered material model. None of these reach past `materials`.
    "KHR_materials_unlit",
    "KHR_materials_emissive_strength",
    "KHR_materials_ior",
    "KHR_materials_specular",
    "KHR_materials_anisotropy",
    "KHR_materials_transmission",
    "KHR_materials_dispersion",
    "KHR_materials_volume",
    "KHR_materials_iridescence",
    "KHR_materials_sheen",
    "KHR_materials_clearcoat",
    // Archived by Khronos, still present in assets, and equally binary-free.
    "KHR_materials_pbrSpecularGlossiness",
    // Rides on a texture binding: offset, scale, rotation and a texCoord set.
    "KHR_texture_transform",
    // Name an alternate `images[]` entry; the image itself is an ordinary one.
    "EXT_texture_webp",
    "KHR_texture_basisu",
    // Scene-level, and both stay in their own index spaces: lights[] and
    // variants[] are root arrays this crate never compacts.
    "KHR_lights_punctual",
    "KHR_materials_variants",
    // A permission rather than a payload: it widens the component types an
    // accessor may use, and names none of them.
    "KHR_mesh_quantization",
    // A Cesium vendor extension holding one origin offset, `center: [x, y, z]`.
    "CESIUM_RTC",
];

/// An extension that owns no binary references.
///
/// Opting into binary transforms with the trait's own empty
/// [`ExtensionHandler::collect_binary_references`] and
/// [`ExtensionHandler::remap_binary_references`] is exactly the statement
/// "this extension participates and owns nothing": there is nothing to keep
/// alive and nothing to rewrite.
#[derive(Clone, Copy, Debug)]
pub struct BinaryFreeExtension(pub &'static str);
impl ExtensionHandler for BinaryFreeExtension {
    fn name(&self) -> &'static str {
        self.0
    }
    fn allows_binary_transform(&self) -> bool {
        true
    }
}

/// Resolved binary resources indexed by glTF buffer index.
#[derive(Clone, Debug, Default)]
pub struct ResourceStore {
    /// Resolved bytes indexed by glTF `buffers[]` position.
    pub buffers: Vec<Vec<u8>>,
}

/// Narrow validation permissions granted by an extension.
#[derive(Default)]
pub struct ExtensionValidationContext {
    accessors_without_buffer_view: Vec<usize>,
}

impl ExtensionValidationContext {
    /// Allows a registered extension to omit a buffer view for one accessor.
    pub fn allow_accessor_without_buffer_view(&mut self, index: usize) {
        if !self.accessors_without_buffer_view.contains(&index) {
            self.accessors_without_buffer_view.push(index);
        }
    }
    /// Returns whether an accessor has received that narrow exemption.
    pub fn allows_accessor_without_buffer_view(&self, index: usize) -> bool {
        self.accessors_without_buffer_view.contains(&index)
    }
}

/// A registered glTF extension with optional geometry decoding.
pub trait ExtensionHandler: Send + Sync {
    /// Returns the exact glTF extension name handled by this implementation.
    fn name(&self) -> &'static str;
    /// Performs extension-specific strict validation and records narrowly
    /// scoped core-validation exemptions in `context`.
    fn validate(
        &self,
        _document: &Document,
        _context: &mut ExtensionValidationContext,
    ) -> Result<()> {
        Ok(())
    }
    /// Whether a transform may replace accessor and buffer-view binary data
    /// while preserving this extension. Handlers must opt in explicitly after
    /// validating their binary-reference semantics.
    fn allows_binary_transform(&self) -> bool {
        false
    }
    /// Marks every accessor and buffer-view reference owned by this extension.
    ///
    /// A handler that opts into binary transforms must implement this together
    /// with [`Self::remap_binary_references`]. Unknown extension JSON is never
    /// inspected or rewritten by the core document transformer.
    fn collect_binary_references(
        &self,
        _document: &Document,
        _accessors: &mut [bool],
        _buffer_views: &mut [bool],
    ) -> Result<()> {
        Ok(())
    }
    /// Applies the maps produced by binary compaction to references owned by
    /// this extension. This is called only for handlers that explicitly allow
    /// binary transforms.
    fn remap_binary_references(
        &self,
        _document: &mut Document,
        _accessors: &[Option<usize>],
        _buffer_views: &[Option<usize>],
    ) -> Result<()> {
        Ok(())
    }
    /// Decodes geometry for `primitive`, or returns `None` when this handler
    /// does not own that primitive.
    fn decode_primitive(
        &self,
        _document: &Document,
        _resources: &ResourceStore,
        _primitive: PrimitiveRef<'_>,
    ) -> Option<Result<Mesh>> {
        None
    }
}

#[derive(Clone)]
/// Registry of unique extension handlers used by document validation/transforms.
pub struct ExtensionRegistry {
    handlers: Vec<Arc<dyn ExtensionHandler>>,
}
impl ExtensionRegistry {
    /// Creates the registry containing the built-in Draco handler.
    pub fn new() -> Self {
        Self::default()
    }
    /// Registers one extension handler. Extension names must be unique.
    pub fn register<H: ExtensionHandler + 'static>(&mut self, handler: H) -> Result<()> {
        if self
            .handlers
            .iter()
            .any(|existing| existing.name() == handler.name())
        {
            return Err(Error::Extension(format!(
                "extension handler {} is already registered",
                handler.name()
            )));
        }
        self.handlers.push(Arc::new(handler));
        Ok(())
    }
    /// Returns whether a handler is registered for `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.handlers.iter().any(|handler| handler.name() == name)
    }
    /// Returns whether `name` explicitly supports binary-reference transforms.
    pub fn allows_binary_transform(&self, name: &str) -> bool {
        self.handlers
            .iter()
            .any(|handler| handler.name() == name && handler.allows_binary_transform())
    }
    /// Validates every registered extension against `document`.
    pub fn validate(&self, document: &Document) -> Result<ExtensionValidationContext> {
        let mut context = ExtensionValidationContext::default();
        for handler in &self.handlers {
            handler.validate(document, &mut context)?;
        }
        Ok(context)
    }
    #[cfg(feature = "draco-encode")]
    pub(crate) fn collect_binary_references(
        &self,
        document: &Document,
        accessors: &mut [bool],
        buffer_views: &mut [bool],
    ) -> Result<()> {
        for handler in &self.handlers {
            if handler.allows_binary_transform() {
                handler.collect_binary_references(document, accessors, buffer_views)?;
            }
        }
        Ok(())
    }
    #[cfg(feature = "draco-encode")]
    pub(crate) fn remap_binary_references(
        &self,
        document: &mut Document,
        accessors: &[Option<usize>],
        buffer_views: &[Option<usize>],
    ) -> Result<()> {
        for handler in &self.handlers {
            if handler.allows_binary_transform() {
                handler.remap_binary_references(document, accessors, buffer_views)?;
            }
        }
        Ok(())
    }
    /// Dispatches geometry decoding to the handler that owns `primitive`.
    pub fn decode_primitive(
        &self,
        document: &Document,
        resources: &ResourceStore,
        primitive: PrimitiveRef<'_>,
    ) -> Result<Mesh> {
        for handler in &self.handlers {
            if let Some(result) = handler.decode_primitive(document, resources, primitive) {
                return result;
            }
        }
        Err(Error::Extension(
            "primitive has no registered geometry extension decoder".into(),
        ))
    }
}

/// Default decoder for `KHR_draco_mesh_compression`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DracoExtension;
impl ExtensionHandler for DracoExtension {
    fn name(&self) -> &'static str {
        KHR_DRACO_MESH_COMPRESSION
    }
    fn allows_binary_transform(&self) -> bool {
        true
    }
    fn collect_binary_references(
        &self,
        document: &Document,
        _accessors: &mut [bool],
        buffer_views: &mut [bool],
    ) -> Result<()> {
        if buffer_views.is_empty() {
            return Ok(());
        }
        for mesh in document.meshes() {
            for primitive in mesh
                .value()
                .get("primitives")
                .and_then(Value::as_array)
                .unwrap_or(&[])
            {
                let Some(extension) = primitive
                    .get("extensions")
                    .and_then(|value| value.get(KHR_DRACO_MESH_COMPRESSION))
                else {
                    continue;
                };
                let index = extension
                    .get("bufferView")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|index| *index < buffer_views.len())
                    .ok_or_else(|| Error::Extension("Draco bufferView is invalid".into()))?;
                buffer_views[index] = true;
            }
        }
        Ok(())
    }
    fn remap_binary_references(
        &self,
        document: &mut Document,
        _accessors: &[Option<usize>],
        buffer_views: &[Option<usize>],
    ) -> Result<()> {
        if buffer_views.is_empty() {
            return Ok(());
        }
        let Some(meshes) = document
            .as_value_mut()
            .get_mut("meshes")
            .and_then(Value::as_array_mut)
        else {
            return Ok(());
        };
        for mesh in meshes {
            let Some(primitives) = mesh.get_mut("primitives").and_then(Value::as_array_mut) else {
                continue;
            };
            for primitive in primitives {
                let Some(value) = primitive
                    .get_mut("extensions")
                    .and_then(|value| value.get_mut(KHR_DRACO_MESH_COMPRESSION))
                    .and_then(|value| value.get_mut("bufferView"))
                else {
                    continue;
                };
                remap_reference(value, buffer_views, "Draco bufferView")?;
            }
        }
        Ok(())
    }
    fn validate(
        &self,
        document: &Document,
        context: &mut ExtensionValidationContext,
    ) -> Result<()> {
        let accessors = document
            .as_value()
            .get("accessors")
            .and_then(Value::as_array)
            .unwrap_or(&[]);
        for mesh in document.meshes() {
            for primitive_index in mesh
                .value()
                .get("primitives")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let primitive = primitive_index.1;
                let Some(_parsed) = parse_draco_extension(
                    primitive
                        .get("extensions")
                        .and_then(|extensions| extensions.get(KHR_DRACO_MESH_COMPRESSION)),
                )?
                else {
                    continue;
                };
                for accessor in primitive
                    .get("attributes")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flat_map(|attrs| attrs.iter().map(|(_, value)| value))
                    .chain(primitive.get("indices"))
                {
                    if let Some(index) = accessor
                        .as_u64()
                        .and_then(|value| usize::try_from(value).ok())
                    {
                        if accessors.get(index).is_some_and(|value| {
                            value.get("bufferView").is_none() && value.get("sparse").is_none()
                        }) {
                            context.allow_accessor_without_buffer_view(index);
                        }
                    }
                }
            }
        }
        Ok(())
    }
    #[cfg(feature = "draco-decode")]
    fn decode_primitive(
        &self,
        document: &Document,
        resources: &ResourceStore,
        primitive: PrimitiveRef<'_>,
    ) -> Option<Result<Mesh>> {
        let extension = primitive.extension(self.name())?;
        Some((|| {
            let parsed = parse_draco_extension(Some(extension))?
                .ok_or_else(|| Error::Extension("missing Draco extension".into()))?;
            let view = document.as_value()["bufferViews"]
                .as_array()
                .and_then(|views| views.get(parsed.buffer_view))
                .ok_or_else(|| Error::Extension("Draco bufferView out of range".into()))?;
            let buffer = view
                .get("buffer")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .and_then(|index| resources.buffers.get(index))
                .ok_or_else(|| Error::Extension("Draco buffer is not resolved".into()))?;
            let start = view.get("byteOffset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let length = view
                .get("byteLength")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| Error::Extension("Draco bufferView length is invalid".into()))?;
            let end = start
                .checked_add(length)
                .filter(|end| *end <= buffer.len())
                .ok_or_else(|| Error::Extension("Draco bufferView out of bounds".into()))?;
            let mut mesh = Mesh::new();
            MeshDecoder::new()
                .decode(&mut DecoderBuffer::new(&buffer[start..end]), &mut mesh)
                .map_err(Error::Decode)?;
            Ok(mesh)
        })())
    }
}

fn remap_reference(value: &mut Value, map: &[Option<usize>], kind: &str) -> Result<()> {
    let old = value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| Error::Extension(format!("{kind} is invalid")))?;
    let new = map
        .get(old)
        .and_then(|value| *value)
        .ok_or_else(|| Error::Extension(format!("{kind} was removed")))?;
    *value = Value::from(new);
    Ok(())
}

#[cfg_attr(not(feature = "draco-decode"), allow(dead_code))]
#[derive(Clone, Debug)]
pub(crate) struct DracoContract {
    pub buffer_view: usize,
    pub attributes: Vec<(String, u32)>,
}

pub(crate) fn parse_draco_extension(value: Option<&Value>) -> Result<Option<DracoContract>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let buffer_view = value
        .get("bufferView")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| Error::Extension("Draco bufferView is invalid".into()))?;
    let attributes = value
        .get("attributes")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::Extension("Draco attributes is invalid".into()))?
        .iter()
        .map(|(name, value)| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .map(|value| (name.clone(), value))
                .ok_or_else(|| Error::Extension(format!("Draco attribute {name} is invalid")))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(DracoContract {
        buffer_view,
        attributes,
    }))
}

impl Default for ExtensionRegistry {
    fn default() -> Self {
        let mut registry = Self {
            handlers: Vec::new(),
        };
        registry
            .register(DracoExtension)
            .expect("built-in extension names are unique");
        for name in BINARY_FREE_EXTENSIONS {
            registry
                .register(BinaryFreeExtension(name))
                .expect("built-in extension names are unique");
        }
        registry
    }
}
