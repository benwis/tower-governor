use std::net::SocketAddr;

use axum::{error_handling::HandleErrorLayer, routing::get, Router};
use tower::{BoxError, ServiceBuilder};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{errors::display_error, governor::GovernorConfigBuilder, GovernorLayer};

#[tokio::main]
async fn _main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "example_testing=debug,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], 3000));

    tracing::debug!("listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    axum::serve(listener, app().into_make_service())
        .await
        .unwrap();
}

/// Having a function that produces our app makes it easy to call it from tests
/// without having to create an HTTP server.
#[allow(dead_code)]
fn app() -> Router {
    let config = Box::new(
        GovernorConfigBuilder::default()
            .per_millisecond(90)
            .burst_size(2)
            .finish()
            .unwrap(),
    );

    Router::new()
        // `GET /` goes to `root`
        .route(
            "/",
            get(|| async { "Hello, World!" }).post(|| async { "Hello, Post World!" }),
        )
        .layer(
            ServiceBuilder::new()
                // this middleware goes above `GovernorLayer` because it will receive
                // errors returned by `GovernorLayer`
                .layer(HandleErrorLayer::new(|e: BoxError| async move {
                    display_error(e)
                }))
                .layer(GovernorLayer {
                    config: Box::leak(config),
                }),
        )
        .layer(TraceLayer::new_for_http())
}

#[cfg(test)]
mod governor_tests {
    use super::*;
    use axum::http;
    use reqwest::header::HeaderName;
    use reqwest::StatusCode;
    use std::net::SocketAddr;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn hello_world() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let app = app();
            tx.send(()).unwrap();
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        rx.await.unwrap();

        let client = reqwest::Client::new();

        let res = client.get(format!("http://{}", addr)).send().await.unwrap();
        let res2 = client.get(format!("http://{}", addr)).send().await.unwrap();

        let body = res.text().await.unwrap();
        let body2 = res2.text().await.unwrap();

        assert!(body.starts_with("Hello, World!"));
        assert!(body2.starts_with("Hello, World!"));
    }

    // #[test]
    // fn builder_test() {
    //     use crate::governor::GovernorConfigBuilder;

    //     let mut builder = GovernorConfigBuilder::default();
    //     builder
    //         .period(crate::governor::DEFAULT_PERIOD)
    //         .burst_size(crate::governor::DEFAULT_BURST_SIZE);

    //     assert_eq!(GovernorConfigBuilder::default(), builder);

    //     let mut builder1 = builder.clone();
    //     builder1.per_millisecond(5000);
    //     let builder2 = builder.per_second(5);

    //     assert_eq!(&builder1, builder2);
    // }

    #[tokio::test]
    async fn test_server() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let app = app();
            tx.send(()).unwrap();
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        rx.await.unwrap();

        let client = reqwest::Client::new();

        // First request
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Second request
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Third request -> Over limit, returns Error
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-after"))
                .unwrap(),
            "0"
        );

        // Replenish one element by waiting for >90ms
        let sleep_time = std::time::Duration::from_millis(100);
        std::thread::sleep(sleep_time);

        // First request after reset
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Second request after reset -> Again over limit, returns Error
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-after"))
                .unwrap(),
            "0"
        );
        let body = res.text().await.unwrap();
        assert_eq!(&body, "Too Many Requests! Wait for 0s");
    }
    #[tokio::test]
    async fn test_method_filter() {
        use crate::governor::GovernorConfigBuilder;
        use http::Method;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let config = Box::new(
                GovernorConfigBuilder::default()
                    .per_millisecond(90)
                    .burst_size(2)
                    .methods(vec![Method::GET])
                    .finish()
                    .unwrap(),
            );

            let app = Router::new()
                // `GET /` goes to `root`
                .route(
                    "/",
                    get(|| async { "Hello, World!" }).post(|| async { "Hello, Post World!" }),
                )
                .layer(
                    ServiceBuilder::new()
                        // this middleware goes above `GovernorLayer` because it will receive
                        // errors returned by `GovernorLayer`
                        .layer(HandleErrorLayer::new(|e: BoxError| async move {
                            display_error(e)
                        }))
                        .layer(GovernorLayer {
                            config: Box::leak(config),
                        }),
                )
                .layer(TraceLayer::new_for_http());
            tx.send(()).unwrap();
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        rx.await.unwrap();

        let client = reqwest::Client::new();

        // First request
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Second request
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // Third request -> Over limit, returns Error
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-after"))
                .unwrap(),
            "0"
        );

        // Fourth request. POST should be ignored by the method filter
        let res = client.post(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_server_use_headers() {
        use crate::governor::GovernorConfigBuilder;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let config = Box::new(
                GovernorConfigBuilder::default()
                    .per_millisecond(90)
                    .burst_size(2)
                    .use_headers()
                    .finish()
                    .unwrap(),
            );

            let app = Router::new()
                // `GET /` goes to `root`
                .route(
                    "/",
                    get(|| async { "Hello, World!" }).post(|| async { "Hello, Post World!" }),
                )
                .layer(
                    ServiceBuilder::new()
                        // this middleware goes above `GovernorLayer` because it will receive
                        // errors returned by `GovernorLayer`
                        .layer(HandleErrorLayer::new(|e: BoxError| async move {
                            display_error(e)
                        }))
                        .layer(GovernorLayer {
                            config: Box::leak(config),
                        }),
                )
                .layer(TraceLayer::new_for_http());
            tx.send(()).unwrap();
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        rx.await.unwrap();

        let client = reqwest::Client::new();

        // First request
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-limit"))
                .unwrap(),
            "2"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-remaining"))
                .unwrap(),
            "0" //TODO: Should this be 1?!?
        );
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-after"))
            .is_none());
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-whitelisted"))
            .is_none());

        // Second request
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-limit"))
                .unwrap(),
            "2"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-remaining"))
                .unwrap(),
            "0"
        );
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-after"))
            .is_none());
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-whitelisted"))
            .is_none());

        // Third request -> Over limit, returns Error
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-after"))
                .unwrap(),
            "0"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-limit"))
                .unwrap(),
            "2"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-remaining"))
                .unwrap(),
            "0"
        );
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-whitelisted"))
            .is_none());

        // Replenish one element by waiting for >90ms
        let sleep_time = std::time::Duration::from_millis(100);
        std::thread::sleep(sleep_time);

        // First request after reset
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-limit"))
                .unwrap(),
            "2"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-remaining"))
                .unwrap(),
            "0"
        );
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-after"))
            .is_none());
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-whitelisted"))
            .is_none());

        // Second request after reset -> Again over limit, returns Error
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-after"))
                .unwrap(),
            "0"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-limit"))
                .unwrap(),
            "2"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-remaining"))
                .unwrap(),
            "0"
        );
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-whitelisted"))
            .is_none());

        let body = res.text().await.unwrap();
        assert_eq!(&body, "Too Many Requests! Wait for 0s");
    }

    #[tokio::test]
    async fn test_method_filter_use_headers() {
        use crate::governor::GovernorConfigBuilder;
        use http::Method;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);

        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let config = Box::new(
                GovernorConfigBuilder::default()
                    .per_millisecond(90)
                    .burst_size(2)
                    .methods(vec![Method::GET])
                    .use_headers()
                    .finish()
                    .unwrap(),
            );

            let app = Router::new()
                // `GET /` goes to `root`
                .route(
                    "/",
                    get(|| async { "Hello, World!" }).post(|| async { "Hello, Post World!" }),
                )
                .layer(
                    ServiceBuilder::new()
                        // this middleware goes above `GovernorLayer` because it will receive
                        // errors returned by `GovernorLayer`
                        .layer(HandleErrorLayer::new(|e: BoxError| async move {
                            display_error(e)
                        }))
                        .layer(GovernorLayer {
                            config: Box::leak(config),
                        }),
                )
                .layer(TraceLayer::new_for_http());
            tx.send(()).unwrap();
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        rx.await.unwrap();

        let client = reqwest::Client::new();

        // First request
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-limit"))
                .unwrap(),
            "2"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-remaining"))
                .unwrap(),
            "0" //TODO: Should this be 1?!?
        );
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-after"))
            .is_none());
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-whitelisted"))
            .is_none());

        // Second request
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-limit"))
                .unwrap(),
            "2"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-remaining"))
                .unwrap(),
            "0"
        );
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-after"))
            .is_none());
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-whitelisted"))
            .is_none());

        // Third request -> Over limit, returns Error
        let res = client.get(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-after"))
                .unwrap(),
            "0"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-limit"))
                .unwrap(),
            "2"
        );
        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-remaining"))
                .unwrap(),
            "0"
        );
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-whitelisted"))
            .is_none());

        // Fourth request, ignored because POST
        let res = client.post(&url).send().await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        assert_eq!(
            res.headers()
                .get(HeaderName::from_static("x-ratelimit-whitelisted"))
                .unwrap(),
            "true"
        );
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-limit"))
            .is_none());
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-remaining"))
            .is_none());
        assert!(res
            .headers()
            .get(HeaderName::from_static("x-ratelimit-after"))
            .is_none());

        let body = res.text().await.unwrap();
        assert_eq!(&body, "Hello, Post World!");
    }
}
