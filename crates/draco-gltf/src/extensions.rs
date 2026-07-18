//! Extension contracts for the lossless document model.

use std::sync::Arc;

use crate::json::Value;
use draco_core::Mesh;
#[cfg(feature = "draco-decode")]
use draco_core::{DecoderBuffer, MeshDecoder};

use crate::{Document, Error, PrimitiveRef, Result};

pub const KHR_DRACO_MESH_COMPRESSION: &str = "KHR_draco_mesh_compression";

/// Resolved binary resources indexed by glTF buffer index.
#[derive(Clone, Debug, Default)]
pub struct ResourceStore {
    pub buffers: Vec<Vec<u8>>,
}

/// Narrow validation permissions granted by an extension.
#[derive(Default)]
pub struct ExtensionValidationContext {
    accessors_without_buffer_view: Vec<usize>,
}

impl ExtensionValidationContext {
    pub fn allow_accessor_without_buffer_view(&mut self, index: usize) {
        if !self.accessors_without_buffer_view.contains(&index) {
            self.accessors_without_buffer_view.push(index);
        }
    }
    pub fn allows_accessor_without_buffer_view(&self, index: usize) -> bool {
        self.accessors_without_buffer_view.contains(&index)
    }
}

/// A registered glTF extension with optional geometry decoding.
pub trait ExtensionHandler: Send + Sync {
    fn name(&self) -> &'static str;
    fn validate(
        &self,
        _document: &Document,
        _context: &mut ExtensionValidationContext,
    ) -> Result<()> {
        Ok(())
    }
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
pub struct ExtensionRegistry {
    handlers: Vec<Arc<dyn ExtensionHandler>>,
}
impl ExtensionRegistry {
    pub fn new() -> Self {
        Self::default()
    }
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
    pub fn contains(&self, name: &str) -> bool {
        self.handlers.iter().any(|handler| handler.name() == name)
    }
    pub fn validate(&self, document: &Document) -> Result<ExtensionValidationContext> {
        let mut context = ExtensionValidationContext::default();
        for handler in &self.handlers {
            handler.validate(document, &mut context)?;
        }
        Ok(context)
    }
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
    fn validate(
        &self,
        document: &Document,
        context: &mut ExtensionValidationContext,
    ) -> Result<()> {
        let accessors = document
            .as_value()
            .get("accessors")
            .and_then(Value::as_array)
            .map(|values| values)
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

#[cfg_attr(
    not(any(feature = "draco-decode", feature = "compact")),
    allow(dead_code)
)]
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
        registry
    }
}
