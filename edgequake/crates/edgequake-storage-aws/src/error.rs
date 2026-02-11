//! Error types for AWS storage operations.

use thiserror::Error;

/// Result type for AWS storage operations.
pub type Result<T> = std::result::Result<T, AwsStorageError>;

/// Errors that can occur during AWS storage operations.
#[derive(Error, Debug)]
pub enum AwsStorageError {
    /// S3 operation failed
    #[error("S3 error: {0}")]
    S3Error(String),

    /// DynamoDB operation failed
    #[error("DynamoDB error: {0}")]
    DynamoDbError(String),

    /// Neptune graph database error
    #[error("Neptune error: {0}")]
    NeptuneError(String),

    /// Athena query error
    #[error("Athena error: {0}")]
    AthenaError(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Binary serialization error
    #[error("Binary serialization error: {0}")]
    BincodeError(#[from] bincode::Error),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Vector dimension mismatch
    #[error("Vector dimension mismatch: expected {expected}, got {actual}")]
    DimensionMismatch { expected: usize, actual: usize },

    /// Index not found
    #[error("Index not found: {0}")]
    IndexNotFound(String),

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Resource not found
    #[error("Resource not found: {0}")]
    NotFound(String),

    /// Generic error
    #[error("AWS storage error: {0}")]
    Other(String),
}

// Conversion from AWS SDK errors
impl<E> From<aws_smithy_runtime_api::client::result::SdkError<E>> for AwsStorageError
where
    E: std::error::Error + 'static,
{
    fn from(err: aws_smithy_runtime_api::client::result::SdkError<E>) -> Self {
        AwsStorageError::Other(err.to_string())
    }
}

// Convert to edgequake-storage::StorageError
impl From<AwsStorageError> for edgequake_storage::StorageError {
    fn from(err: AwsStorageError) -> Self {
        match err {
            AwsStorageError::NotFound(msg) => edgequake_storage::StorageError::NotFound(msg),
            AwsStorageError::DimensionMismatch { expected, actual } => {
                edgequake_storage::StorageError::InvalidDimension { expected, actual }
            }
            _ => edgequake_storage::StorageError::Database(err.to_string()),
        }
    }
}
