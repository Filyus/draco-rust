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

/// Prefix of the message [`DracoError::count_exceeds_bitstream`] builds.
///
/// The refusal has no variant of its own: `DracoError` is exhaustive, so adding
/// one is a breaking change, and this refusal is tied to a guard that
/// `hardening_status.yaml` records as unsound and due to be replaced. Marking
/// the message instead keeps the constructor and the predicate that recognises
/// it in one place, where a message is an implementation detail rather than an
/// interface.
const COUNT_EXCEEDS_BITSTREAM_PREFIX: &str = "Declared count";

impl DracoError {
    /// The decoder's refusal of a declared count larger than the remaining
    /// bitstream could describe.
    ///
    /// Gated with the decode paths that raise it; the predicate below is not,
    /// because a caller holding a `DracoError` may ask about it whatever this
    /// build compiled.
    #[cfg(feature = "decoder")]
    pub(crate) fn count_exceeds_bitstream(count: usize, remaining_bytes: usize) -> Self {
        DracoError::DracoError(format!(
            "{COUNT_EXCEEDS_BITSTREAM_PREFIX} {count} exceeds what the remaining \
             {remaining_bytes} bytes can describe"
        ))
    }

    /// Whether this is the decoder's count-vs-size preflight refusal.
    ///
    /// It is the one decode refusal a caller may legitimately want to tell
    /// apart: the bound it applies assumes at least one bit per point or face,
    /// which highly repetitive geometry can beat, so this is also the refusal
    /// that can be a false positive. See the `decoder-count-guard-is-unsound`
    /// entry in `hardening_status.yaml`.
    pub fn is_count_exceeds_bitstream(&self) -> bool {
        matches!(self, DracoError::DracoError(message)
            if message.starts_with(COUNT_EXCEEDS_BITSTREAM_PREFIX))
    }
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
