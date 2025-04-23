use http::{HeaderMap, Response, StatusCode};
use thiserror::Error;

/// The error type returned by tower-governor.
#[derive(Debug, Error, Clone)]
pub enum GovernorError {
    #[error("Too Many Requests! Wait for {wait_time}s")]
    TooManyRequests {
        wait_time: u64,
        headers: Option<HeaderMap>,
    },
    #[error("Unable to extract key!")]
    UnableToExtractKey,
    #[error("Other Error")]
    /// Used for custom key extractors to return their own errors
    Other {
        code: StatusCode,
        msg: Option<String>,
        headers: Option<HeaderMap>,
    },
}

#[cfg(feature = "axum")]
impl From<GovernorError> for Response<axum::body::Body> {
    fn from(error: GovernorError) -> Self {
        error.into_response().map(From::from)
    }
}

#[cfg(feature = "tonic")]
impl From<GovernorError> for Response<tonic::body::Body> {
    fn from(error: GovernorError) -> Self {
        let (parts, message) = error.into_response().into_parts();
        let code = match parts.status {
            StatusCode::TOO_MANY_REQUESTS => tonic::Code::ResourceExhausted,
            StatusCode::INTERNAL_SERVER_ERROR => tonic::Code::Internal,
            _ => tonic::Code::Internal,
        };
        let mut response = tonic::Status::new(code, message).into_http();
        response.headers_mut().extend(parts.headers);
        response
    }
}

impl GovernorError {
    /// Convert self into a "default response"
    pub fn into_response(self) -> Response<String> {
        match self {
            GovernorError::TooManyRequests { wait_time, headers } => {
                let response = Response::new(format!("Too Many Requests! Wait for {}s", wait_time));
                let (mut parts, body) = response.into_parts();
                parts.status = StatusCode::TOO_MANY_REQUESTS;
                if let Some(headers) = headers {
                    parts.headers = headers;
                }
                Response::from_parts(parts, body)
            }
            GovernorError::UnableToExtractKey => {
                let response = Response::new("Unable To Extract Key!".to_string());
                let (mut parts, body) = response.into_parts();
                parts.status = StatusCode::INTERNAL_SERVER_ERROR;

                Response::from_parts(parts, body)
            }
            GovernorError::Other { msg, code, headers } => {
                let response = Response::new("Other Error!".to_string());
                let (mut parts, mut body) = response.into_parts();
                parts.status = code;
                if let Some(headers) = headers {
                    parts.headers = headers;
                }
                if let Some(msg) = msg {
                    body = msg;
                }

                Response::from_parts(parts, body)
            }
        }
    }
}
