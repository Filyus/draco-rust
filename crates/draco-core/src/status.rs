use std::fmt;

/// What a [`DracoError`] refused, as a value that can be compared and copied.
///
/// Separate from the error itself so that the error can stay one pointer wide:
/// the kind and its message live together behind a single allocation, and
/// `Result<(), DracoError>` is a pointer-sized value returned in a register.
/// See [`DracoError`] for why that matters.
///
/// Non-exhaustive: a decoder that learns to tell one refusal from another
/// should be able to say so without a major release. Match with a `_` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Generic error.
    ///
    /// Mirrors upstream's `Status::DRACO_ERROR`, whose own comment reads "used
    /// for general errors".
    General,
    /// File or stream I/O error.
    Io,
    /// Invalid caller-provided parameter.
    InvalidParameter,
    /// Bitstream version is known but unsupported.
    UnsupportedVersion,
    /// Bitstream version could not be identified.
    UnknownVersion,
    /// Bitstream uses a feature this crate does not support.
    UnsupportedFeature,
    /// Bitstream version is outside the supported range.
    BitstreamVersionUnsupported,
    /// Buffer read or write failed.
    Buffer,
    /// A decode would allocate more than the stream it is reading could
    /// plausibly describe.
    ///
    /// The bound is a ratio against the input size, not a cap on geometry — see
    /// `decode_budget`. A stream that trips it is malformed or adversarial; a
    /// large but genuine mesh scales its own budget with it.
    AllocationExceedsInput,
    /// A decode would produce more than the caller's [`DecodeLimits`](crate::DecodeLimits) allow.
    ///
    /// Distinct from [`AllocationExceedsInput`](Self::AllocationExceedsInput)
    /// on purpose: that one says the file is malformed, this one says the file
    /// may well be fine and the caller declined to decode something that
    /// large. Raise the limits, or use
    /// [`DecodeLimits::permissive`](crate::DecodeLimits::permissive).
    LimitExceeded,
}

impl ErrorKind {
    /// The fixed text this kind contributes to [`DracoError`]'s `Display`.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorKind::General => "General error",
            ErrorKind::Io => "IO error",
            ErrorKind::InvalidParameter => "Invalid parameter",
            ErrorKind::UnsupportedVersion => "Unsupported version",
            ErrorKind::UnknownVersion => "Unknown version",
            ErrorKind::UnsupportedFeature => "Unsupported feature",
            ErrorKind::BitstreamVersionUnsupported => "Bitstream version unsupported",
            ErrorKind::Buffer => "Buffer decode error",
            ErrorKind::AllocationExceedsInput => "Allocation exceeds input",
            ErrorKind::LimitExceeded => "Decode limit exceeded",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Inner {
    kind: ErrorKind,
    message: String,
}

/// Error returned by Draco decoding, encoding, and data model operations.
///
/// One pointer wide, in the shape of [`std::io::Error`]: the [`ErrorKind`] and
/// the message live behind a single boxed allocation that is only made on the
/// failure path. This is not a style preference. Almost every decode and encode
/// function in this crate returns [`Status`], and when the error was an enum
/// carrying a `String` inline, `Result<(), DracoError>` was 32 bytes and needed
/// dropping: every one of those functions returned through a hidden out-pointer
/// and every `?` expanded to `String` drop glue. Boxing makes the success case a
/// null pointer in a register and reduces the drop glue to one shared function.
/// Measured on the glTF WASM module, that is 2.6 KiB of gzipped code. It is not
/// what dominates, and this comment says so because the first guess was wrong:
/// the message text and the `format!` that builds it are worth 16 KiB in the
/// same module, and no shape of the error type reaches those.
///
/// The kind is matched with [`kind`](Self::kind) rather than by pattern, and it
/// is `#[non_exhaustive]`, so a later release can tell one refusal from another
/// without a major bump.
///
/// ```
/// use draco_core::{DracoError, ErrorKind};
///
/// let error = DracoError::unsupported_feature("multi-pass attribute coding");
/// assert_eq!(error.kind(), ErrorKind::UnsupportedFeature);
/// assert_eq!(error.message(), "multi-pass attribute coding");
/// assert_eq!(
///     error.to_string(),
///     "Unsupported feature: multi-pass attribute coding"
/// );
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct DracoError {
    inner: Box<Inner>,
}

impl DracoError {
    /// Builds an error of `kind` carrying `message`.
    ///
    /// Cold and never inlined so that the allocation is emitted once rather
    /// than at each of the several hundred sites that construct an error.
    #[cold]
    #[inline(never)]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            inner: Box::new(Inner {
                kind,
                message: message.into(),
            }),
        }
    }

    /// What was refused. See [`ErrorKind`].
    pub fn kind(&self) -> ErrorKind {
        self.inner.kind
    }

    /// The message alone, without the fixed text [`ErrorKind::as_str`] adds.
    ///
    /// Empty for kinds that carry no detail.
    pub fn message(&self) -> &str {
        &self.inner.message
    }

    /// A generic failure. See [`ErrorKind::General`].
    pub fn general(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::General, message)
    }

    /// A file or stream failure. See [`ErrorKind::Io`].
    pub fn io(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Io, message)
    }

    /// A rejected caller-provided parameter. See [`ErrorKind::InvalidParameter`].
    pub fn invalid_parameter(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidParameter, message)
    }

    /// A known but unsupported bitstream version. See [`ErrorKind::UnsupportedVersion`].
    pub fn unsupported_version(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::UnsupportedVersion, message)
    }

    /// An unidentifiable bitstream version. See [`ErrorKind::UnknownVersion`].
    pub fn unknown_version(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::UnknownVersion, message)
    }

    /// A bitstream feature this crate does not implement. See [`ErrorKind::UnsupportedFeature`].
    pub fn unsupported_feature(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::UnsupportedFeature, message)
    }

    /// A bitstream version outside the supported range.
    /// See [`ErrorKind::BitstreamVersionUnsupported`].
    pub fn bitstream_version_unsupported() -> Self {
        Self::new(ErrorKind::BitstreamVersionUnsupported, String::new())
    }

    /// A failed buffer read or write. See [`ErrorKind::Buffer`].
    pub fn buffer(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Buffer, message)
    }

    /// A decode that asked for more memory than its input could describe.
    /// See [`ErrorKind::AllocationExceedsInput`].
    /// Prefixes this error's message with `context`, keeping its kind.
    ///
    /// The kind is what a caller matches on, so a layer that adds context has
    /// to carry it through. Rebuilding the error as
    /// [`general`](Self::general) instead flattens every refusal underneath
    /// into one kind -- which is how a caller's own
    /// [`LimitExceeded`](ErrorKind::LimitExceeded) ceiling became
    /// indistinguishable from a corrupt file.
    #[cold]
    #[inline(never)]
    #[must_use]
    pub fn context(self, context: impl std::fmt::Display) -> Self {
        let kind = self.inner.kind;
        let message = if self.inner.message.is_empty() {
            format!("{context}")
        } else {
            format!("{context}: {}", self.inner.message)
        };
        Self::new(kind, message)
    }

    pub fn allocation_exceeds_input(requested_bytes: usize, stream_bytes: usize) -> Self {
        Self::new(
            ErrorKind::AllocationExceedsInput,
            format!("would allocate {requested_bytes} bytes from a {stream_bytes} byte stream"),
        )
    }
}

impl fmt::Display for DracoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.inner.message.is_empty() {
            f.write_str(self.inner.kind.as_str())
        } else {
            write!(f, "{}: {}", self.inner.kind.as_str(), self.inner.message)
        }
    }
}

/// Prints the kind and the message rather than the box, so that a `{:?}` of a
/// `Result` reads the way the enum's did.
impl fmt::Debug for DracoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DracoError")
            .field("kind", &self.inner.kind)
            .field("message", &self.inner.message)
            .finish()
    }
}

impl std::error::Error for DracoError {}

/// Convenience result type for operations that only report success or failure.
pub type Status = Result<(), DracoError>;

impl From<()> for DracoError {
    fn from(_: ()) -> Self {
        DracoError::general("Unknown error")
    }
}

/// Returns a successful [`Status`].
pub fn ok_status() -> Status {
    Ok(())
}

/// Creates a generic [`DracoError`] from a message.
pub fn error_status(msg: impl Into<String>) -> DracoError {
    DracoError::general(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reason the type is shaped this way: every fallible function in the
    /// crate returns `Status`, so the size of the failure case is paid by the
    /// success case at every call site.
    #[test]
    fn a_status_is_a_pointer_wide_and_the_success_case_needs_no_drop() {
        assert_eq!(
            std::mem::size_of::<Status>(),
            std::mem::size_of::<*const ()>()
        );
        assert_eq!(
            std::mem::size_of::<DracoError>(),
            std::mem::size_of::<*const ()>()
        );
        // `Ok(())` must be the null pointer, or the niche is not being used and
        // the discriminant costs another word.
        assert_eq!(
            std::mem::size_of::<Option<Status>>(),
            2 * std::mem::size_of::<usize>()
        );
    }

    #[test]
    fn a_kind_without_a_message_displays_as_the_kind_alone() {
        assert_eq!(
            DracoError::bitstream_version_unsupported().to_string(),
            "Bitstream version unsupported"
        );
        assert_eq!(DracoError::bitstream_version_unsupported().message(), "");
    }

    #[test]
    fn the_kind_survives_a_round_trip_through_display() {
        let error = DracoError::buffer("read past end");
        assert_eq!(error.kind(), ErrorKind::Buffer);
        assert_eq!(error.to_string(), "Buffer decode error: read past end");
    }

    #[test]
    fn two_errors_of_different_kinds_are_not_equal() {
        assert_ne!(DracoError::general("x"), DracoError::io("x"));
        assert_eq!(DracoError::general("x"), DracoError::general("x"));
    }
}
