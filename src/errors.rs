use http::{HeaderMap, Response, StatusCode};
use std::mem;
use thiserror::Error;
use tower::BoxError;

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
/// Used in the Error Handler Middleware(for Axum) to convert GovernorError into a Response
/// This one returns a String Body with the error message, and applies a HTTP Status Code, Headers,
/// and msg body from the Error, if included.
/// Feel free to use your own, as long as it returns a Response
pub fn display_error(mut e: BoxError) -> Response<String> {
    if e.is::<GovernorError>() {
        // It shouldn't be possible for this to panic, since we already know it's a GovernorError
        e.downcast_mut::<GovernorError>().unwrap().as_response()
    } else {
        let response = Response::new("Internal Server Error".to_string());
        let (mut parts, body) = response.into_parts();
        parts.status = StatusCode::INTERNAL_SERVER_ERROR;

        Response::from_parts(parts, body)
    }
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
