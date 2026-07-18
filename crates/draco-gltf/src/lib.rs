//! Full, lossless glTF 2.0 / pinned 2.1-draft model with Draco geometry.
//!
//! [`Document`] is the public scene model, and all unknown JSON remains part
//! of that model.

#![deny(missing_docs)]

use std::path::Path;

use thiserror::Error;

#[cfg(feature = "geometry")]
mod accessor;
#[cfg(feature = "draco-encode")]
mod compression;
/// Lossless document model and typed views for glTF scenes.
pub mod document;
#[cfg(feature = "geometry")]
pub use accessor::{AccessorData, DocumentAccessorSource};
#[cfg(feature = "draco-encode")]
pub use compression::{CompressionMode, CompressionOptions, CompressionReport};
mod json;
pub use document::{
    Accessor, AccessorIndex, Animation, AnimationIndex, BoundingVolume, Buffer, BufferIndex,
    BufferView, BufferViewIndex, Camera, CameraIndex, ComponentType, Document, ExternalAsset,
    ExternalAssetIndex, File, FileIndex, Image, ImageIndex, Material, MaterialIndex, Mesh,
    MeshIndex, Node, NodeIndex, PrimitiveIndex, PrimitiveRef, Sampler, SamplerIndex, Scene,
    SceneIndex, Shape, ShapeIndex, Skin, SkinIndex, Texture, TextureIndex, ValidationProfile,
};
#[cfg(feature = "geometry")]
mod packed;
#[cfg(feature = "geometry")]
pub use packed::{GeometryError, PackedAttribute, PackedGeometry, PackedIndices, PrimitiveMode};
#[cfg(feature = "write")]
mod writer;
/// Lossless JSON value used by the document model.
pub use json::Value as JsonValue;
#[cfg(feature = "write")]
pub use writer::{GeometryEncoding, GeometryWriteOptions, GeometryWriteReport, PreserveReason};
/// Extension contracts and resource storage used by document transforms.
pub mod extensions;
pub use extensions::{
    DracoExtension, ExtensionHandler, ExtensionRegistry, ExtensionValidationContext, ResourceStore,
    KHR_DRACO_MESH_COMPRESSION,
};
#[cfg(feature = "compact")]
/// Geometry-oriented facade over the same document model.
pub mod compact;
#[cfg(feature = "compact")]
pub use compact::{CompactDocument, CompactMeshRange};
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
    /// Preserve the input container kind.
    SameAsInput,
    /// Emit a JSON glTF document and no materialized companion buffers.
    GltfJson,
    /// Emit a GLB version 2 container.
    GlbV2,
    /// Emit a draft GLB version 3 container.
    GlbV3,
}

/// Errors returned by glTF parsing and Draco operations.
#[derive(Debug, Error)]
pub enum Error {
    /// An operating-system or stream error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    /// The JSON chunk could not be parsed.
    #[error("JSON error: {0}")]
    Json(String),
    /// Draco bitstream decoding failed.
    #[error("Draco decode error: {0}")]
    Decode(#[from] draco_core::DracoError),
    /// A low-level container or resource operation failed.
    #[error("draco-io error: {0}")]
    DracoIo(#[from] GltfError),
    /// Materialized primitive geometry is invalid or unsupported.
    #[cfg(feature = "geometry")]
    #[error("geometry error: {0}")]
    Geometry(#[from] GeometryError),
    /// An extension contract rejected the document or transform.
    #[error("extension error: {0}")]
    Extension(String),
    /// Strict document validation failed.
    #[error("glTF validation failed: {0:?}")]
    Validation(Vec<String>),
    /// A configured resource or graph quota was exceeded.
    #[error("resource quota exceeded: {0}")]
    ResourceLimit(String),
}
/// Result type returned by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Options controlling document loading, resource resolution and validation.
pub struct ImportOptions<'a> {
    /// Base directory used for relative external resources.
    pub base_path: Option<&'a Path>,
    /// Policy applied by the default filesystem resolver.
    pub external_file_policy: ExternalFilePolicy,
    /// Optional caller-provided synchronous resource resolver.
    pub resolver: Option<&'a dyn ResourceResolver>,
    /// Resource and graph quotas applied during loading.
    pub limits: ResourceLimits,
    /// Profile used for strict validation after parsing.
    pub profile: ValidationProfile,
    /// Extension handlers available to validation and transforms.
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
/// Opens a glTF or GLB file using the draft validation profile.
pub fn import(path: impl AsRef<Path>) -> Result<Import> {
    open(path, ValidationProfile::Gltf21Draft)
}

/// Parses glTF or GLB bytes using the draft validation profile.
///
/// ```
/// # use draco_gltf::import_slice;
/// let input = br#"{"asset":{"version":"2.0"},"meshes":[]}"#;
/// let scene = import_slice(input, None)?;
/// assert_eq!(scene.document.meshes().len(), 0);
/// # Ok::<(), draco_gltf::Error>(())
/// ```
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

/// Parses glTF or GLB bytes with explicit loading options.
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
