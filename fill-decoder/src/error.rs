//! Error types for the fill-decoder crate.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, FillDecoderError>;

/// Errors produced by the fill-decoder crate.
#[derive(Error, Debug)]
#[error("{0}")]
pub struct FillDecoderError(String);

impl FillDecoderError {
    /// Create a validation / format error (wrong discriminator, missing accounts).
    pub fn validation<S: Into<String>>(msg: S) -> Self {
        Self(format!("validation error: {}", msg.into()))
    }

    /// Create a generic error (overflow, truncated data, …).
    pub fn other<S: Into<String>>(msg: S) -> Self {
        Self(msg.into())
    }
}
