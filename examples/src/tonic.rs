//! Serves a tonic gRPC service at /Greeter/<anything>
//!
//! Example request mit curl:
//!
//! ```bash
//! curl http://localhost:50051/Greeter/hello --http2-prior-knowledge
//! ```

use std::{
    convert::Infallible,
    future::{self, Ready},
    net::SocketAddr,
    task::{Context, Poll},
};

use http::{Request, Response};
use tonic::{
    body::Body,
    server::NamedService,
    service::{Interceptor, InterceptorLayer},
    Status,
};
use tower::Service;
use tower_governor::{
    governor::GovernorConfigBuilder, key_extractor::SmartIpKeyExtractor, GovernorLayer,
};

#[derive(Debug, Clone)]
struct GreeterService;

/// A tonic service (usually generated from a proto file).
impl Service<Request<Body>> for GreeterService {
    type Response = Response<Body>;
    type Error = Infallible;
    type Future = Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _request: Request<Body>) -> Self::Future {
        let response = Response::new(Body::new("Hello, World!".to_owned()));
        future::ready(Ok(response))
    }
}

impl NamedService for GreeterService {
    const NAME: &'static str = "Greeter";
}

// Extracts the IP for the `key_extractor` being able to use it
#[derive(Debug, Clone)]
struct ConnectInfoInterceptor;

impl Interceptor for ConnectInfoInterceptor {
    fn call(&mut self, mut request: tonic::Request<()>) -> Result<tonic::Request<()>, Status> {
        let addr = request.remote_addr().expect("not running on a TCP socket");
        // We use the standard `forwarded` header to pass the IP address to the smart ip key
        // extractor. If running behind a reverse proxy, the IP address will be the proxy's IP
        request
            .metadata_mut()
            .insert("forwarded", format!("for={addr}").try_into().unwrap());
        Ok(request)
    }
}

#[tokio::main]
async fn main() {
    let config = GovernorConfigBuilder::default()
        .key_extractor(SmartIpKeyExtractor)
        .per_second(2)
        .burst_size(2)
        .finish()
        .unwrap();

    let listen_addr: SocketAddr = "0.0.0.0:50051".parse().unwrap();

    println!("Listening on {listen_addr}");
    tonic::transport::Server::builder()
        .layer(InterceptorLayer::new(ConnectInfoInterceptor))
        .layer(GovernorLayer::new(config))
        .add_service(GreeterService)
        .serve(listen_addr)
        .await
        .unwrap();
}
