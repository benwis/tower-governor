use axum::response::{IntoResponse, Response};
use http::StatusCode;
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

#[derive(Debug, Error, Clone)]
pub enum GovernorError {
    #[error("Too Many Requests!")]
    SimplyTooManyRequests,
}
impl IntoResponse for GovernorError {
    fn into_response(self) -> Response {
        let body: String = match self {
            GovernorError::SimplyTooManyRequests => "Too Many Requests".to_string(),
        };

        // its often easiest to implement `IntoResponse` by calling other implementations
        (StatusCode::TOO_MANY_REQUESTS, body).into_response()
    }
}
