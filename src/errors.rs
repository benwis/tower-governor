use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum SimpleKeyExtractionError {
    #[error("Too Many Requests! Wait for {0}s")]
    TooManyRequests(u64),
    #[error("Unable to extract key!")]
    UnableToExtractKey,
    #[error("{0}")]
    Other(String),
}
