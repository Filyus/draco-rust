use thiserror::Error;

/// Error returned by Draco decoding, encoding, and data model operations.
///
/// Non-exhaustive: a decoder that learns to tell one refusal from another
/// should be able to say so without a major release. Match with a `_` arm.
#[derive(Error, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum DracoError {
    /// Generic error with a human-readable message.
    ///
    /// Mirrors upstream's `Status::DRACO_ERROR`, whose own comment reads "used
    /// for general errors". Named for the meaning rather than the spelling:
    /// upstream qualifies it as `Status::DRACO_ERROR`, while here the enum is
    /// already `DracoError`, so the literal port stuttered as
    /// `DracoError::DracoError`. The `Display` text has always said "General
    /// error", and this is now what the variant is called.
    #[error("General error: {0}")]
    General(String),
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
    /// A decode would allocate more than the stream it is reading could
    /// plausibly describe.
    ///
    /// The bound is a ratio against the input size, not a cap on geometry — see
    /// `decode_budget`. A stream that trips it is malformed or adversarial; a
    /// large but genuine mesh scales its own budget with it.
    #[error("Decode would allocate {requested_bytes} bytes from a {stream_bytes} byte stream")]
    AllocationExceedsInput {
        /// Bytes the decode was about to reserve.
        requested_bytes: usize,
        /// Size of the stream being decoded.
        stream_bytes: usize,
    },
}

/// Convenience result type for operations that only report success or failure.
pub type Status = Result<(), DracoError>;

impl From<()> for DracoError {
    fn from(_: ()) -> Self {
        DracoError::General("Unknown error".to_string())
    }
}

/// Returns a successful [`Status`].
pub fn ok_status() -> Status {
    Ok(())
}

/// Creates a generic [`DracoError`] from a message.
pub fn error_status(msg: impl Into<String>) -> DracoError {
    DracoError::General(msg.into())
}
