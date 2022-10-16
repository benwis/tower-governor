#![doc = include_str!("../README.md")]

#[cfg(test)]
mod tests;

mod errors;
pub mod governor;
pub mod key_extractor;
use crate::governor::{Governor, GovernorConfig};
use ::governor::clock::{Clock, DefaultClock, QuantaInstant};
use ::governor::middleware::{NoOpMiddleware, RateLimitingMiddleware, StateInformationMiddleware};
use axum::response::Response;
use futures_core::ready;

use http::header::{HeaderName, HeaderValue};
use http::request::Request;
use http::{HeaderMap, StatusCode};
use key_extractor::KeyExtractor;
use pin_project::pin_project;
use std::task::{Context, Poll};
use std::{future::Future, pin::Pin};
use tower::{Layer, Service};

#[derive(Clone)]
pub struct GovernorLayer<'a, K, M>
where
    K: KeyExtractor,
    M: RateLimitingMiddleware<QuantaInstant>,
{
    pub config: &'a GovernorConfig<K, M>,
}

impl<K, M, S> Layer<S> for GovernorLayer<'_, K, M>
where
    K: KeyExtractor,
    M: RateLimitingMiddleware<QuantaInstant>,
{
    type Service = Governor<K, M, S>;

    fn layer(&self, inner: S) -> Self::Service {
        Governor::new(inner, self.config)
    }
}

// Implement Service for Governor
impl<K, S, ReqBody, ResBody> Service<Request<ReqBody>> for Governor<K, NoOpMiddleware, S>
where
    K: KeyExtractor,
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: Default,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // Our middleware doesn't care about backpressure so its ready as long
        // as the inner service is ready.
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

                    ResponseFuture {
                        inner: Kind::Error {
                            code: StatusCode::TOO_MANY_REQUESTS,
                            headers: Some(headers),
                        },
                    }
                }
            },

            Err(_) => {
                // Not sure if I should do this, but not sure how to return an Error
                // in a match arm like this
                // future::err(e.into())
                ResponseFuture {
                    inner: Kind::Error {
                        headers: None,
                        code: StatusCode::INTERNAL_SERVER_ERROR,
                    },
                }
            }
        }
    }
}

#[derive(Debug)]
#[pin_project]
/// Response future for [`Governor`].
pub struct ResponseFuture<F> {
    #[pin]
    inner: Kind<F>,
}

#[derive(Debug)]
#[pin_project(project = KindProj)]
enum Kind<F> {
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
        code: StatusCode,
        headers: Option<HeaderMap>,
    },
}

impl<F, B, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response<B>, E>>,
    B: Default,
{
    type Output = Result<Response<B>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.project().inner.project() {
            KindProj::Passthrough { future } => {
                let response: Response<B> = ready!(future.poll(cx))?;
                Poll::Ready(Ok(response))
            }
            KindProj::RateLimitHeader {
                future,
                burst_size,
                remaining_burst_capacity,
            } => {
                let mut response: Response<B> = ready!(future.poll(cx))?;

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
                let mut response: Response<B> = ready!(future.poll(cx))?;

                let headers = response.headers_mut();
                headers.insert(
                    HeaderName::from_static("x-ratelimit-whitelisted"),
                    HeaderValue::from_static("true"),
                );

                Poll::Ready(Ok(response))
            }
            KindProj::Error { code, headers } => {
                let mut response = Response::new(B::default());

                // Let's build the an error response here!
                *response.status_mut() = *code;
                if let Some(headers) = headers {
                    response.headers_mut().extend(headers.drain());
                }

                Poll::Ready(Ok(response))
            }
        }
    }
}

// Implementation of Service for StateInformationMiddleware. You can have more than one!
impl<K, S, ReqBody, ResBody> Service<Request<ReqBody>>
    for Governor<K, StateInformationMiddleware, S>
where
    K: KeyExtractor,
    S: Service<Request<ReqBody>, Response = Response<ResBody>>,
    ResBody: Default,
{
    type Response = S::Response;
    type Error = S::Error;
    // type Future = WhitelistedHeaderFut<S::Future>;
    type Future = ResponseFuture<S::Future>;

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
                    headers.insert(
                        "x-ratelimit-limit",
                        negative.quota().burst_size().get().into(),
                    );
                    headers.insert("x-ratelimit-remaining", 0.into());
                    // let response = self
                    //     .key_extractor
                    //     .exceed_rate_limit_response(&negative, response_builder);
                    ResponseFuture {
                        inner: Kind::Error {
                            headers: Some(headers),
                            code: StatusCode::TOO_MANY_REQUESTS,
                        },
                    }
                }
            },

            // Extraction failed, stop right now.
            Err(_) => {
                // Not sure if I should do this, but not sure how to return an Error
                // in a match arm like this
                // future::err(e.into())
                ResponseFuture {
                    inner: Kind::Error {
                        headers: None,
                        code: StatusCode::INTERNAL_SERVER_ERROR,
                    },
                }
            }
        }
    }
}
