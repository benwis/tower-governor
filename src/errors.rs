use http::{HeaderMap, Response, StatusCode};
use std::mem;
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

impl GovernorError {
    pub(crate) fn as_response<ResB>(&mut self) -> Response<ResB>
    where
        ResB: From<String>,
    {
        match mem::replace(self, Self::UnableToExtractKey) {
            GovernorError::TooManyRequests { wait_time, headers } => {
                let response = Response::new(format!("Too Many Requests! Wait for {}s", wait_time));
                let (mut parts, body) = response.into_parts();
                parts.status = StatusCode::TOO_MANY_REQUESTS;
                if let Some(headers) = headers {
                    parts.headers = headers;
                }
                Response::from_parts(parts, ResB::from(body))
            }
            GovernorError::UnableToExtractKey => {
                let response = Response::new("Unable To Extract Key!".to_string());
                let (mut parts, body) = response.into_parts();
                parts.status = StatusCode::INTERNAL_SERVER_ERROR;

                Response::from_parts(parts, ResB::from(body))
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

                Response::from_parts(parts, ResB::from(body))
            }
        }
    }
}
