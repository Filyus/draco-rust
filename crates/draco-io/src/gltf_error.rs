//! Shared errors for glTF container, resource, and geometry operations.

use std::io;

use thiserror::Error;

/// Errors that can occur when reading, transforming, or decoding glTF data.
#[derive(Error, Debug)]
pub enum GltfError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid GLB: {0}")]
    InvalidGlb(String),
    #[error("Invalid glTF: {0}")]
    InvalidGltf(String),
    #[error("Draco decode error: {0}")]
    DracoDecode(#[source] draco_core::DracoError),
    #[error("Draco encode error: {0}")]
    DracoEncode(#[source] draco_core::DracoError),
    #[error("Unsupported feature: {0}")]
    Unsupported(String),
    #[error("External resource denied: {0}")]
    ExternalResourceDenied(String),
    #[error("Resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),
    #[error("Opaque binary reference: {0}")]
    OpaqueBinaryReference(String),
    #[error("Invalid compression options: {0}")]
    InvalidOptions(String),
}

pub type Result<T> = std::result::Result<T, GltfError>;
