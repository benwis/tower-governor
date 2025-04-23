#![doc = include_str!("../README.md")]

#[cfg(test)]
mod tests;

pub mod errors;
pub mod governor;
pub mod key_extractor;
use crate::governor::{Governor, GovernorConfig};
use ::governor::clock::{Clock, DefaultClock, QuantaInstant};
use ::governor::middleware::{NoOpMiddleware, RateLimitingMiddleware, StateInformationMiddleware};
pub use errors::GovernorError;
use governor::ErrorHandler;
use http::response::Response;

use http::header::{HeaderName, HeaderValue};
use http::request::Request;
use http::HeaderMap;
use key_extractor::KeyExtractor;
use pin_project::pin_project;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::{future::Future, pin::Pin, task::ready};
use tower::{Layer, Service};

/// The Layer type that implements tower::Layer and is passed into `.layer()`
pub struct GovernorLayer<K, M, RespBody>
where
    K: KeyExtractor,
    M: RateLimitingMiddleware<QuantaInstant>,
{
    config: Arc<GovernorConfig<K, M>>,
    error_handler: Option<ErrorHandler<RespBody>>,
}

impl<K, M, RespBody> GovernorLayer<K, M, RespBody>
where
    K: KeyExtractor,
    M: RateLimitingMiddleware<QuantaInstant>,
{
    /// Create a new layer from config
    pub fn new(config: impl Into<Arc<GovernorConfig<K, M>>>) -> Self {
        Self {
            config: config.into(),
            error_handler: None,
        }
    }

    /// Set custom error handler for governor errors [`GovernorError`]
    ///
    /// If the handler is not set, the response will be created via the conversion `RespBody:
    /// From<GovernorError>`.
    pub fn error_handler(
        mut self,
        handler: impl Fn(GovernorError) -> Response<RespBody> + Send + Sync + 'static,
    ) -> Self {
        self.error_handler = Some(ErrorHandler::new(handler));
        self
    }
}

impl<K, M, S, RespBody> Layer<S> for GovernorLayer<K, M, RespBody>
where
    K: KeyExtractor,
    M: RateLimitingMiddleware<QuantaInstant>,
{
    type Service = Governor<K, M, S, RespBody>;

    fn layer(&self, inner: S) -> Self::Service {
        let mut service = Governor::new(inner, &self.config);
        service.set_error_handler(self.error_handler.clone());
        service
    }
}

/// https://stegosaurusdormant.com/understanding-derive-clone/
impl<K: KeyExtractor, M: RateLimitingMiddleware<QuantaInstant>, RespBody> Clone
    for GovernorLayer<K, M, RespBody>
{
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            error_handler: self.error_handler.clone(),
        }
    }
}

// Implement tower::Service for Governor
impl<K, S, ReqBody, RespBody> Service<Request<ReqBody>> for Governor<K, NoOpMiddleware, S, RespBody>
where
    K: KeyExtractor,
    S: Service<Request<ReqBody>, Response = Response<RespBody>>,
    S::Response: From<GovernorError>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future, RespBody>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        if let Some(configured_methods) = &self.methods {
            if !configured_methods.contains(req.method()) {
                // The request method is not configured, we're ignoring this one.
                let future = self.inner.call(req);
                return ResponseFuture {
                    inner: Kind::Passthrough { future },
                };
            }
        }
        // Use the provided key extractor to extract the rate limiting key from the request.
        match self.key_extractor.extract(&req) {
            // Extraction worked, let's check if rate limiting is needed.
            Ok(key) => match self.limiter.check_key(&key) {
                Ok(_) => {
                    let future = self.inner.call(req);
                    ResponseFuture {
                        inner: Kind::Passthrough { future },
                    }
                }

                Err(negative) => {
                    let wait_time = negative
                        .wait_time_from(DefaultClock::default().now())
                        .as_secs();

                    #[cfg(feature = "tracing")]
                    {
                        let key_name = match self.key_extractor.key_name(&key) {
                            Some(n) => format!(" [{}]", &n),
                            None => "".to_owned(),
                        };
                        tracing::info!(
                            "Rate limit exceeded for {}{}, quota reset in {}s",
                            self.key_extractor.name(),
                            key_name,
                            &wait_time
                        );
                    }
                    let mut headers = HeaderMap::new();
                    headers.insert("x-ratelimit-after", wait_time.into());
                    headers.insert("retry-after", wait_time.into());

                    let error_response = self.handle_error(GovernorError::TooManyRequests {
                        wait_time,
                        headers: Some(headers),
                    });

                    ResponseFuture {
                        inner: Kind::Error {
                            error_response: Some(error_response),
                        },
                    }
                }
            },

            Err(e) => ResponseFuture {
                inner: Kind::Error {
                    error_response: Some(self.handle_error(e)),
                },
            },
        }
    }
}

#[derive(Debug)]
#[pin_project]
/// Response future for [`Governor`].
pub struct ResponseFuture<F, RespBody> {
    #[pin]
    inner: Kind<F, RespBody>,
}

#[derive(Debug)]
#[pin_project(project = KindProj)]
enum Kind<F, RespBody> {
    Passthrough {
        #[pin]
        future: F,
    },
    RateLimitHeader {
        #[pin]
        future: F,
        #[pin]
        burst_size: u32,
        #[pin]
        remaining_burst_capacity: u32,
    },
    WhitelistedHeader {
        #[pin]
        future: F,
    },
    Error {
        error_response: Option<Response<RespBody>>,
    },
}

impl<F, E, RespBody> Future for ResponseFuture<F, RespBody>
where
    F: Future<Output = Result<Response<RespBody>, E>>,
{
    type Output = Result<Response<RespBody>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project().inner.project() {
            KindProj::Passthrough { future } => future.poll(cx),
            KindProj::RateLimitHeader {
                future,
                burst_size,
                remaining_burst_capacity,
            } => {
                let mut response = ready!(future.poll(cx))?;

                let mut headers = HeaderMap::new();
                headers.insert(
                    HeaderName::from_static("x-ratelimit-limit"),
                    HeaderValue::from(*burst_size),
                );
                headers.insert(
                    HeaderName::from_static("x-ratelimit-remaining"),
                    HeaderValue::from(*remaining_burst_capacity),
                );
                response.headers_mut().extend(headers.drain());

                Poll::Ready(Ok(response))
            }
            KindProj::WhitelistedHeader { future } => {
                let mut response = ready!(future.poll(cx))?;

                let headers = response.headers_mut();
                headers.insert(
                    HeaderName::from_static("x-ratelimit-whitelisted"),
                    HeaderValue::from_static("true"),
                );

                Poll::Ready(Ok(response))
            }
            KindProj::Error { error_response } => Poll::Ready(Ok(error_response.take().expect("
                <Governor as Service<Request<_>>>::call must produce Response<String> when GovernorError occurs.
            "))),
        }
    }
}

// Implementation of Service for Governor using the StateInformationMiddleware.
impl<K, S, ReqBody, RespBody> Service<Request<ReqBody>>
    for Governor<K, StateInformationMiddleware, S, RespBody>
where
    K: KeyExtractor,
    S: Service<Request<ReqBody>, Response = Response<RespBody>>,
    S::Response: From<GovernorError>,
    // Body type of response must impl From<String> trait to convert potential error
    // produced by governor to re
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future, RespBody>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Our middleware doesn't care about backpressure so its ready as long
        // as the inner service is ready.
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        if let Some(configured_methods) = &self.methods {
            if !configured_methods.contains(req.method()) {
                // The request method is not configured, we're ignoring this one.
                let fut = self.inner.call(req);
                return ResponseFuture {
                    inner: Kind::WhitelistedHeader { future: fut },
                };
            }
        }
        // Use the provided key extractor to extract the rate limiting key from the request.
        match self.key_extractor.extract(&req) {
            // Extraction worked, let's check if rate limiting is needed.
            Ok(key) => match self.limiter.check_key(&key) {
                Ok(snapshot) => {
                    let fut = self.inner.call(req);
                    ResponseFuture {
                        inner: Kind::RateLimitHeader {
                            future: fut,
                            burst_size: snapshot.quota().burst_size().get(),
                            remaining_burst_capacity: snapshot.remaining_burst_capacity(),
                        },
                    }
                }

                Err(negative) => {
                    let wait_time = negative
                        .wait_time_from(DefaultClock::default().now())
                        .as_secs();

                    #[cfg(feature = "tracing")]
                    {
                        let key_name = match self.key_extractor.key_name(&key) {
                            Some(n) => format!(" [{}]", &n),
                            None => "".to_owned(),
                        };
                        tracing::info!(
                            "Rate limit exceeded for {}{}, quota reset in {}s",
                            self.key_extractor.name(),
                            key_name,
                            &wait_time
                        );
                    }

                    let mut headers = HeaderMap::new();
                    headers.insert("x-ratelimit-after", wait_time.into());
                    headers.insert("retry-after", wait_time.into());
                    headers.insert(
                        "x-ratelimit-limit",
                        negative.quota().burst_size().get().into(),
                    );
                    headers.insert("x-ratelimit-remaining", 0.into());

                    let error_response = self.handle_error(GovernorError::TooManyRequests {
                        wait_time,
                        headers: Some(headers),
                    });

                    ResponseFuture {
                        inner: Kind::Error {
                            error_response: Some(error_response),
                        },
                    }
                }
            },

            // Extraction failed, stop right now.
            Err(e) => ResponseFuture {
                inner: Kind::Error {
                    error_response: Some(self.handle_error(e)),
                },
            },
        }
    }
}
