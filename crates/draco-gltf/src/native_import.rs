use std::path::Path;

use serde_json::Value;

use crate::{
    Document, Error, ExtensionRegistry, FileIndex, ResourceStore, Result, ValidationProfile,
};
use draco_io::{
    parse_gltf_container, resolve_gltf_buffers, ExternalFilePolicy, FileResourceResolver,
    GltfBufferReference, GltfContainerFormat, ResourceLimits, ResourceResolver,
};

/// Native, lossless glTF import independent of `gltf-rs`.
pub struct NativeImport {
    pub document: Document,
    pub resources: ResourceStore,
    pub input_format: GltfContainerFormat,
    profile: ValidationProfile,
}

impl NativeImport {
    pub fn validate(&self, extensions: &ExtensionRegistry) -> Result<()> {
        self.document.validate(self.profile)?;
        extensions.validate(&self.document)?;
        Ok(())
    }

    /// Lists declared glTF 2.1 `files` entries without resolving them.
    pub fn external_assets(&self) -> impl Iterator<Item = FileIndex> + '_ {
        self.document.files().into_iter().map(|file| file.index())
    }

    /// Explicitly resolves and parses one nested glTF file.
    pub fn load_asset(
        &self,
        file: FileIndex,
        resolver: &dyn ResourceResolver,
        limits: &ResourceLimits,
        profile: ValidationProfile,
        extensions: &ExtensionRegistry,
    ) -> Result<Self> {
        let uri = self
            .document
            .files()
            .get(file)
            .and_then(|file| file.value().get("uri"))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Extension(format!("file {} has no external URI", file.0)))?;
        let bytes = draco_io::resolve_resource_uri(uri, Some(resolver), limits.max_resource_bytes)?;
        parse_native_with_options(&bytes, None, Some(resolver), limits, profile, extensions)
    }
}

pub fn parse_native(bytes: &[u8], profile: ValidationProfile) -> Result<NativeImport> {
    parse_native_with_options(
        bytes,
        None,
        None,
        &ResourceLimits::default(),
        profile,
        &ExtensionRegistry::default(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub fn open_native(path: impl AsRef<Path>, profile: ValidationProfile) -> Result<NativeImport> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)?;
    let resolver = FileResourceResolver::new(
        path.parent().unwrap_or_else(|| Path::new(".")),
        ExternalFilePolicy::ConfineToBase,
    );
    parse_native_with_options(
        &bytes,
        path.parent(),
        Some(&resolver),
        &ResourceLimits::default(),
        profile,
        &ExtensionRegistry::default(),
    )
}

pub fn parse_native_with_options(
    bytes: &[u8],
    _base: Option<&Path>,
    resolver: Option<&dyn ResourceResolver>,
    limits: &ResourceLimits,
    profile: ValidationProfile,
    extensions: &ExtensionRegistry,
) -> Result<NativeImport> {
    let container = parse_gltf_container(bytes)?;
    let document = Document::from_json_bytes(container.json)?;
    document.validate(profile)?;
    extensions.validate(&document)?;
    let mut references = Vec::new();
    for buffer in document.buffers() {
        let uri = buffer.value().get("uri").and_then(Value::as_str);
        let byte_length = buffer
            .value()
            .get("byteLength")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                Error::Validation(vec![format!(
                    "buffer {} byteLength is invalid",
                    buffer.index().0
                )])
            })?;
        references.push(GltfBufferReference { uri, byte_length });
    }
    let buffers = resolve_gltf_buffers(
        &references,
        container.format,
        container.bin,
        resolver,
        limits,
    )?;
    Ok(NativeImport {
        document,
        resources: ResourceStore { buffers },
        input_format: container.format,
        profile,
    })
}
