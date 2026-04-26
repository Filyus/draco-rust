use thiserror::Error;

#[derive(Error, Debug, Clone, PartialEq)]
pub enum DracoError {
    #[error("General error: {0}")]
    DracoError(String),
    #[error("IO error: {0}")]
    IoError(String),
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
    #[error("Unsupported version: {0}")]
    UnsupportedVersion(String),
    #[error("Unknown version: {0}")]
    UnknownVersion(String),
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
    #[error("Bitstream version unsupported")]
    BitstreamVersionUnsupported,
    #[error("Buffer decode error: {0}")]
    BufferError(String),
}

pub type Status = Result<(), DracoError>;

impl From<()> for DracoError {
    fn from(_: ()) -> Self {
        DracoError::DracoError("Unknown error".to_string())
    }
}

pub fn ok_status() -> Status {
    Ok(())
}

pub fn error_status(msg: impl Into<String>) -> DracoError {
    DracoError::DracoError(msg.into())
}
