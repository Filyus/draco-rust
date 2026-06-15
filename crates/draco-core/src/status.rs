use thiserror::Error;

/// Error returned by Draco decoding, encoding, and data model operations.
#[derive(Error, Debug, Clone, PartialEq)]
pub enum DracoError {
    /// Generic Draco error with a human-readable message.
    #[error("General error: {0}")]
    DracoError(String),
    /// File or stream I/O error.
    #[error("IO error: {0}")]
    IoError(String),
    /// Invalid caller-provided parameter.
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
    /// Bitstream version is known but unsupported.
    #[error("Unsupported version: {0}")]
    UnsupportedVersion(String),
    /// Bitstream version could not be identified.
    #[error("Unknown version: {0}")]
    UnknownVersion(String),
    /// Bitstream uses a feature this crate does not support.
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
    /// Bitstream version is outside the supported range.
    #[error("Bitstream version unsupported")]
    BitstreamVersionUnsupported,
    /// Buffer read or write failed.
    #[error("Buffer decode error: {0}")]
    BufferError(String),
}

/// Convenience result type for operations that only report success or failure.
pub type Status = Result<(), DracoError>;

impl From<()> for DracoError {
    fn from(_: ()) -> Self {
        DracoError::DracoError("Unknown error".to_string())
    }
}

/// Returns a successful [`Status`].
pub fn ok_status() -> Status {
    Ok(())
}

/// Creates a generic [`DracoError`] from a message.
pub fn error_status(msg: impl Into<String>) -> DracoError {
    DracoError::DracoError(msg.into())
}
