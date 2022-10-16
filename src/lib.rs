//! A Tower service and Axum layer that provides
//! rate-limiting backed by [governor](https://github.com/antifuchs/governor) and based heavily
//! on [actix-governor](https://github.com/AaronErhardt/actix-governor).
//!
//! # Features:
//!
//! + Simple to use
//! + High customizability
//! + High performance
//! + Robust yet flexible API
//!
//!
//! # How does it work?
//!
//! Each governor middleware has a configuration that stores a quota.
//! The quota specifies how many requests can be sent from an IP address
//! before the middleware starts blocking further requests.
//!
//! For example if the quota allowed ten requests a client could send a burst of
//! ten requests in short time before the middleware starts blocking.
//!
//! Once at least one element of the quota was used the elements of the quota
//! will be replenished after a specified period.
//!
//! For example if this period was 2 seconds and the quota was empty
//! it would take 2 seconds to replenish one element of the quota.
//! This means you could send one request every two seconds on average.
//!
//! If there was a quota that allowed ten requests with the same period
//! a client could again send a burst of ten requests and then had to wait
//! two seconds before sending further requests or 20 seconds before the full
//! quota would be replenished and he could send another burst.
//!
//! # Example
//! ```rust,no_run
//! use axum_governor::governor::{Governor, GovernorConfigBuilder};
//! use axum_governor::
//! use axum_web::{web, App, HttpServer, Responder};
//!
//! async fn hello() -> 'static str {
//!     "Hello world!"
//! }
//!
//! #[tokio::main]
//! async fn main(){
//!     // Allow bursts with up to five requests per IP address
//!     // and replenishes one element every two seconds
//!     let governor_conf = GovernorConfigBuilder::default()
//!         .per_second(2)
//!         .burst_size(5)
//!         .finish()
//!         .unwrap();
//!     // build our application with a route
//!     let app = Router::new()
//!     // `GET /` goes to `root`
//!         .route("/", get(hello))
//!         .layer()
//!
//!    // run our app with hyper
//!    // `axum::Server` is a re-export of `hyper::Server`
//!    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
//!    tracing::debug!("listening on {}", addr);
//!    axum::Server::bind(&addr)
//!        .serve(app.into_make_service())
//!        .await
//!        .unwrap();
//!    }  
//! }
//! ```
//!
//! # Configuration presets
//!
//! Instead of using the configuration builder you can use predefined presets.
//!
//! + [`GovernorConfig::default()`]: The default configuration which is suitable for most services.
//! Allows bursts with up to eight requests and replenishes one element after 500ms, based on peer IP.
//!
//! + [`GovernorConfig::secure()`]: A default configuration for security related services.
//! Allows bursts with up to two requests and replenishes one element after four seconds, based on peer IP.
//!
//! For example the secure configuration can be used as a short version of this code:
//!
//! ```rust
//! use axum_governor::governor::GovernorConfigBuilder;
//!
//! let config = GovernorConfigBuilder::default()
//!     .per_second(4)
//!     .burst_size(2)
//!     .finish()
//!     .unwrap();
//! ```
//!
//! # Customize rate limiting key
//!
//! By default, rate limiting is done using the peer IP address (i.e. the IP address of the HTTP client that requested your app: either your user or a reverse proxy, depending on your deployment setup).
//! You can configure a different behavior which:
//! 1. can be useful in itself
//! 2. allows you to setup multiple instances of this middleware based on different keys (for example, if you want to apply rate limiting with different rates on IP and API keys at the same time)
//!
//! This is achieved by defining a [KeyExtractor] and giving it to a [Governor] instance.
//! Two ready-to-use key extractors are provided:
//! - [PeerIpKeyExtractor]: this is the default
//! - [GlobalKeyExtractor]: uses the same key for all incoming requests
//!
//! Check out the [custom_key](https://github.com/AaronErhardt/axum-governor/blob/main/examples/custom_key.rs) example to see how a custom key extractor can be implemented.
//!
//!
//! Check out the [custom_key_bearer] example for more information.
//!
//! [`HttpResponseBuilder`]: axum_web::HttpResponseBuilder
//! [`HttpResponse`]: axum_web::HttpResponse
//! [custom_key_bearer]: https://github.com/AaronErhardt/axum-governor/blob/main/examples/custom_key_bearer.rs
//!
//! # Add x-ratelimit headers
//!
//! By default, `x-ratelimit-after` is enabled but if you want to enable `x-ratelimit-limit`, `x-ratelimit-whitelisted` and `x-ratelimit-remaining` use [`use_headers`] method
//!
//! [`use_headers`]: crate::GovernorConfigBuilder::use_headers()
//!
//! # Common pitfalls
//!
//! Do not construct the same configuration multiple times, unless explicitly wanted!
//! This will create an independent rate limiter for each configuration!
//!
//! Instead pass the same configuration reference into [`Governor::new()`],
//! like it is described in the example.

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
pub struct GovernorLayer<K, M>
where
    K: KeyExtractor,
    M: RateLimitingMiddleware<QuantaInstant>,
{
    pub config: GovernorConfig<K, M>,
}

impl<K, M, S> Layer<S> for GovernorLayer<K, M>
where
    K: KeyExtractor,
    M: RateLimitingMiddleware<QuantaInstant>,
{
    type Service = Governor<K, M, S>;

    fn layer(&self, inner: S) -> Self::Service {
        Governor::new(inner, &self.config)
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
    // type Future = RateLimitHeaderFut<S::Future>;
    //type Future = future::Either<future::Ready<Result<Response<B>, Self::Error>>, S::Future>;
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
                // return future::Either::Right(fut);
            }
        }
        // Use the provided key extractor to extract the rate limiting key from the request.
        match self.key_extractor.extract(&req) {
            // Extraction worked, let's check if rate limiting is needed.
            Ok(key) => match self.limiter.check_key(&key) {
                Ok(_) => {
                    let future = self.inner.call(req);
                    // return future::Either::Right(fut);
                    ResponseFuture {
                        inner: Kind::Passthrough { future },
                    }
                }

                Err(negative) => {
                    let wait_time = negative
                        .wait_time_from(DefaultClock::default().now())
                        .as_secs();

                    #[cfg(feature = "log")]
                    {
                        let key_name = match self.key_extractor.key_name(&key) {
                            Some(n) => format!(" [{}]", &n),
                            None => "".to_owned(),
                        };
                        log::info!(
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
                    // future::Either::Left(future::err(
                    //     error::InternalError::from_response("TooManyRequests", response).into(),
                    // ))
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
        println!("POLLING RESPONSE FUTURE");
        match self.project().inner.project() {
            KindProj::Passthrough { future } => {
                println!("Passthrough!");
                let response: Response<B> = ready!(future.poll(cx))?;
                Poll::Ready(Ok(response))
            }
            KindProj::RateLimitHeader {
                future,
                burst_size,
                remaining_burst_capacity,
            } => {
                println!("RateLimit!");
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
                println!("WhiteList!");
                let mut response: Response<B> = ready!(future.poll(cx))?;

                let headers = response.headers_mut();
                headers.insert(
                    HeaderName::from_static("x-ratelimit-whitelisted"),
                    HeaderValue::from_static("true"),
                );

                Poll::Ready(Ok(response))
            }
            KindProj::Error { code, headers } => {
                println!("Error!");
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
                // return future::Either::Right(future::Either::Right(WhitelistedHeaderFut {
                //     response_future: fut,
                // }));
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
                    // future::Either::Right(future::Either::Left(RateLimitHeaderFut {
                    //     response_future: fut,
                    //     burst_size: snapshot.quota().burst_size().get(),
                    //     remaining_burst_capacity: snapshot.remaining_burst_capacity(),
                    // }))
                }

                Err(negative) => {
                    let wait_time = negative
                        .wait_time_from(DefaultClock::default().now())
                        .as_secs();

                    #[cfg(feature = "log")]
                    {
                        let key_name = match self.key_extractor.key_name(&key) {
                            Some(n) => format!(" [{}]", &n),
                            None => "".to_owned(),
                        };
                        log::info!(
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
