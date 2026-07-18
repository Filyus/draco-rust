//! Full, lossless glTF 2.0 / pinned 2.1-draft model with Draco geometry.
//!
//! [`Document`] is the public scene model, and all unknown JSON remains part
//! of that model.

use std::path::Path;

use thiserror::Error;

#[cfg(feature = "geometry")]
mod accessor;
#[cfg(feature = "transform")]
mod compression;
pub mod document;
#[cfg(feature = "geometry")]
pub use accessor::{AccessorData, DocumentAccessorSource};
#[cfg(feature = "transform")]
pub use compression::{CompressionMode, CompressionOptions, CompressionReport};
mod json;
pub use document::{
    Accessor, AccessorIndex, Animation, AnimationIndex, BoundingVolume, Buffer, BufferIndex,
    BufferView, BufferViewIndex, Camera, CameraIndex, ComponentType, Document, ExternalAsset,
    ExternalAssetIndex, File, FileIndex, Image, ImageIndex, Material, MaterialIndex, Mesh,
    MeshIndex, Node, NodeIndex, PrimitiveRef, Sampler, SamplerIndex, Scene, SceneIndex, Shape,
    ShapeIndex, Skin, SkinIndex, Texture, TextureIndex, ValidationProfile,
};
pub use json::Value as JsonValue;
pub mod extensions;
pub use extensions::{
    DracoExtension, ExtensionHandler, ExtensionRegistry, ExtensionValidationContext, ResourceStore,
    KHR_DRACO_MESH_COMPRESSION,
};
#[cfg(feature = "compact")]
pub mod compact;
#[cfg(feature = "compact")]
pub use compact::{CompactDocument, CompactMeshRange};
#[cfg(feature = "compact")]
pub use draco_io::{PackedAttribute, PackedPrimitive};
mod import;
#[cfg(not(target_arch = "wasm32"))]
pub use import::open;
pub use import::{
    parse, parse_with_options, GltfOutput, GltfResource, Import, DEFAULT_EXTERNAL_ASSET_DEPTH,
};

pub use draco_io::{
    ExternalFilePolicy, FileResourceResolver, GlbRangeReader, GltfContainerFormat, GltfError,
    ResourceLimits, ResourceResolver,
};

/// Container representation selected when serializing an import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    SameAsInput,
    GltfJson,
    GlbV2,
    GlbV3,
}

/// Errors returned by glTF parsing and Draco operations.
#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(String),
    #[error("Draco decode error: {0}")]
    Decode(#[from] draco_core::DracoError),
    #[error("draco-io error: {0}")]
    DracoIo(#[from] GltfError),
    #[error("extension error: {0}")]
    Extension(String),
    #[error("glTF validation failed: {0:?}")]
    Validation(Vec<String>),
    #[error("resource quota exceeded: {0}")]
    ResourceLimit(String),
}
pub type Result<T> = std::result::Result<T, Error>;

/// Import options.
pub struct ImportOptions<'a> {
    pub base_path: Option<&'a Path>,
    pub external_file_policy: ExternalFilePolicy,
    pub resolver: Option<&'a dyn ResourceResolver>,
    pub limits: ResourceLimits,
    pub profile: ValidationProfile,
    pub extensions: ExtensionRegistry,
}
impl Default for ImportOptions<'_> {
    fn default() -> Self {
        Self {
            base_path: None,
            external_file_policy: ExternalFilePolicy::Deny,
            resolver: None,
            limits: ResourceLimits::default(),
            profile: ValidationProfile::Gltf21Draft,
            extensions: ExtensionRegistry::default(),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn import(path: impl AsRef<Path>) -> Result<Import> {
    open(path, ValidationProfile::Gltf21Draft)
}

pub fn import_slice(bytes: &[u8], base: Option<&Path>) -> Result<Import> {
    let options = ImportOptions {
        base_path: base,
        external_file_policy: if base.is_some() {
            ExternalFilePolicy::Allow
        } else {
            ExternalFilePolicy::Deny
        },
        ..ImportOptions::default()
    };
    import_slice_with_options(bytes, &options)
}

pub fn import_slice_with_options(bytes: &[u8], options: &ImportOptions<'_>) -> Result<Import> {
    let file_resolver = options
        .base_path
        .map(|base| FileResourceResolver::new(base, options.external_file_policy));
    let resolver = options.resolver.or_else(|| {
        file_resolver
            .as_ref()
            .map(|value| value as &dyn ResourceResolver)
    });
    parse_with_options(
        bytes,
        options.base_path,
        resolver,
        &options.limits,
        options.profile,
        &options.extensions,
    )
}

/// Validates a document against the pinned draft profile.
pub fn validate(document: &Document) -> Result<()> {
    document.validate(ValidationProfile::Gltf21Draft)
}

#[cfg(test)]
mod document_tests;
