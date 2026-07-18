//! Shared errors for glTF container, resource, and geometry operations.

use std::io;

use thiserror::Error;

/// Errors that can occur when reading, transforming, or decoding glTF data.
#[derive(Error, Debug)]
pub enum GltfError {
    /// Underlying stream or filesystem error.
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    /// Malformed GLB header or chunk table.
    #[error("Invalid GLB: {0}")]
    InvalidGlb(String),
    /// Malformed glTF structure or accessor data.
    #[error("Invalid glTF: {0}")]
    InvalidGltf(String),
    /// Draco bitstream decoding failed.
    #[error("Draco decode error: {0}")]
    DracoDecode(#[source] draco_core::DracoError),
    /// Draco bitstream encoding failed.
    #[error("Draco encode error: {0}")]
    DracoEncode(#[source] draco_core::DracoError),
    /// The requested format operation is outside this crate's contract.
    #[error("Unsupported feature: {0}")]
    Unsupported(String),
    /// An external resource was denied by the resolver policy.
    #[error("External resource denied: {0}")]
    ExternalResourceDenied(String),
    /// A configured resource quota was exceeded.
    #[error("Resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),
    /// A binary reference could not be safely interpreted or remapped.
    #[error("Opaque binary reference: {0}")]
    OpaqueBinaryReference(String),
    /// Caller-supplied compression options are invalid.
    #[error("Invalid compression options: {0}")]
    InvalidOptions(String),
}

/// Result type for low-level glTF container and geometry operations.
pub type Result<T> = std::result::Result<T, GltfError>;
