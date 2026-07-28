use thiserror::Error;

/// Result type for RFQv2 SDK operations
pub type Result<T> = std::result::Result<T, MarketMakerError>;

/// Error types for the RFQv2 SDK
#[derive(Error, Debug)]
pub enum MarketMakerError {
    /// Connection-related errors
    #[error("Connection error: {0}")]
    Connection(#[from] tonic::transport::Error),

    /// gRPC status errors
    #[error("gRPC error: {0}")]
    Grpc(#[from] tonic::Status),

    /// Validation errors for quote data
    #[error("Validation error: {0}")]
    Validation(String),

    /// Streaming-related errors
    #[error("Streaming error: {0}")]
    Streaming(String),

    /// Timeout errors
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// Configuration errors
    #[error("Configuration error: {0}")]
    Configuration(String),
}

impl MarketMakerError {
    /// Create a validation error
    pub fn validation<S: Into<String>>(msg: S) -> Self {
        Self::Validation(msg.into())
    }

    /// Create a streaming error
    pub fn streaming<S: Into<String>>(msg: S) -> Self {
        Self::Streaming(msg.into())
    }

    /// Create a timeout error
    pub fn timeout<S: Into<String>>(msg: S) -> Self {
        Self::Timeout(msg.into())
    }

    /// Create a configuration error
    pub fn configuration<S: Into<String>>(msg: S) -> Self {
        Self::Configuration(msg.into())
    }
}
