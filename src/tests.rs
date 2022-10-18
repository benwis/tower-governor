use std::net::SocketAddr;

use crate::{
    errors::display_error, governor::GovernorConfig, GovernorError, GovernorLayer, KeyExtractor,
};
use axum::{error_handling::HandleErrorLayer, routing::get, Router};
use governor::{clock::QuantaInstant, middleware::RateLimitingMiddleware};
use http::{
    header::{HeaderName, HeaderValue},
    StatusCode,
};
use http::{request::Request, response::Response};
use tower::{BoxError, ServiceBuilder};

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

async fn hello() -> &'static str {
    "Hello world"
}

async fn create_app<K, M>(config: &GovernorConfig<K, M>)
where
    K: KeyExtractor,
    M: RateLimitingMiddleware<QuantaInstant>,
{
    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(hello))
        .layer(
            ServiceBuilder::new()
                // this middleware goes above `GovernorLayer` because it will receive
                // errors returned by `GovernorLayer`
                .layer(HandleErrorLayer::new(|e: BoxError| async move {
                    display_error(e)
                }))
                .layer(GovernorLayer { config }),
        );

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_server() {
    use crate::governor::GovernorConfigBuilder;

    // Allow bursts with up to five requests per IP address
    // and replenishes one element every two seconds
    let governor_conf = GovernorConfigBuilder::default()
        .per_second(2)
        .burst_size(5)
        .finish()
        .unwrap();

    //Start Server
    create_app(&governor_conf).await;

    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 80u16);

    // First request
    let res = reqwest::get("/").await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Second request
    let res = reqwest::get("/").await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Third request -> Over limit, returns Error
    let res = reqwest::get("/").await.unwrap();
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
    let res = reqwest::get("/").await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Second request after reset -> Again over limit, returns Error
    let res = reqwest::get("/").await.unwrap();
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        res.headers()
            .get(HeaderName::from_static("x-ratelimit-after"))
            .unwrap(),
        "0"
    );
    let body = res.json::<String>().await.unwrap();
    assert_eq!(&body, "Too many requests, retry in 0s");
}

// #[tokio::test]
// async fn test_method_filter() {
//     use crate::governor::{Governor, GovernorConfigBuilder};
//     use actix_web::test;
//     use http::Method;

//     let config = GovernorConfigBuilder::default()
//         .per_millisecond(90)
//         .burst_size(2)
//         .methods(vec![Method::GET])
//         .finish()
//         .unwrap();

//     let app = test::init_service(
//         App::new()
//             .wrap(Governor::new(&config))
//             .route("/", web::get().to(hello))
//             .route("/", web::post().to(hello)),
//     )
//     .await;

//     use std::net::{IpAddr, Ipv4Addr, SocketAddr};
//     let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 80u16);

//     // First request
//     let req = reqwest::get("/").to_request();
//     let test = test::call_service(&app, req).await;
//     assert_eq!(test.status(), StatusCode::OK);

//     // Second request
//     let req = reqwest::get().peer_addr(addr).uri("/").to_request();
//     let test = test::call_service(&app, req).await;
//     assert_eq!(test.status(), StatusCode::OK);

//     // Third request -> Over limit, returns Error
//     let req = reqwest::get().peer_addr(addr).uri("/").to_request();
//     let test = app.call(req).await.unwrap_err();
//     assert_eq!(
//         test.as_response_error().status_code(),
//         StatusCode::TOO_MANY_REQUESTS
//     );
//     assert_eq!(
//         test.error_response()
//             .headers()
//             .get(HeaderName::from_static("x-ratelimit-after"))
//             .unwrap(),
//         "0"
//     );

//     // Fourth request, now a POST request
//     // This one is ignored by the ratelimit
//     let req = reqwest::post().peer_addr(addr).uri("/").to_request();
//     let test = test::call_service(&app, req).await;
//     assert_eq!(test.status(), StatusCode::OK);
// }

// #[tokio::test]
// async fn test_server_use_headers() {
//     use crate::{Governor, GovernorConfigBuilder};
//     use actix_web::test;

//     let config = GovernorConfigBuilder::default()
//         .per_millisecond(90)
//         .burst_size(2)
//         .use_headers()
//         .finish()
//         .unwrap();

//     let app = test::init_service(
//         App::new()
//             .wrap(Governor::new(&config))
//             .route("/", web::get().to(hello)),
//     )
//     .await;

//     use std::net::{IpAddr, Ipv4Addr, SocketAddr};
//     let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 80u16);

//     // First request
//     let req = reqwest::get().peer_addr(addr).uri("/").to_request();
//     let test = test::call_service(&app, req).await;
//     assert_eq!(test.status(), StatusCode::OK);
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-limit"))
//             .unwrap(),
//         "2"
//     );
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-remaining"))
//             .unwrap(),
//         "1"
//     );
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-after"))
//         .is_none());
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-whitelisted"))
//         .is_none());

//     // Second request
//     let req = reqwest::get().peer_addr(addr).uri("/").to_request();
//     let test = test::call_service(&app, req).await;
//     assert_eq!(test.status(), StatusCode::OK);
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-limit"))
//             .unwrap(),
//         "2"
//     );
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-remaining"))
//             .unwrap(),
//         "0"
//     );
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-after"))
//         .is_none());
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-whitelisted"))
//         .is_none());

//     // Third request -> Over limit, returns Error
//     let req = reqwest::get().peer_addr(addr).uri("/").to_request();
//     let test = app.call(req).await.unwrap_err();
//     let err_response: Response = test.error_response();
//     assert_eq!(err_response.status(), StatusCode::TOO_MANY_REQUESTS);
//     assert_eq!(
//         err_response
//             .headers()
//             .get(HeaderName::from_static("x-ratelimit-after"))
//             .unwrap(),
//         "0"
//     );
//     assert_eq!(
//         err_response
//             .headers()
//             .get(HeaderName::from_static("x-ratelimit-limit"))
//             .unwrap(),
//         "2"
//     );
//     assert_eq!(
//         err_response
//             .headers()
//             .get(HeaderName::from_static("x-ratelimit-remaining"))
//             .unwrap(),
//         "0"
//     );
//     assert!(err_response
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-whitelisted"))
//         .is_none());

//     // Replenish one element by waiting for >90ms
//     let sleep_time = std::time::Duration::from_millis(100);
//     std::thread::sleep(sleep_time);

//     // First request after reset
//     let req = reqwest::get().peer_addr(addr).uri("/").to_request();
//     let test = test::call_service(&app, req).await;
//     assert_eq!(test.status(), StatusCode::OK);
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-limit"))
//             .unwrap(),
//         "2"
//     );
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-remaining"))
//             .unwrap(),
//         "0"
//     );
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-after"))
//         .is_none());
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-whitelisted"))
//         .is_none());

//     // Second request after reset -> Again over limit, returns Error
//     let req = reqwest::get().peer_addr(addr).uri("/").to_request();
//     let test = app.call(req).await.unwrap_err();
//     let err_response: Response = test.error_response();
//     assert_eq!(err_response.status(), StatusCode::TOO_MANY_REQUESTS);
//     assert_eq!(
//         err_response
//             .headers()
//             .get(HeaderName::from_static("x-ratelimit-after"))
//             .unwrap(),
//         "0"
//     );
//     assert_eq!(
//         err_response
//             .headers()
//             .get(HeaderName::from_static("x-ratelimit-limit"))
//             .unwrap(),
//         "2"
//     );
//     assert_eq!(
//         err_response
//             .headers()
//             .get(HeaderName::from_static("x-ratelimit-remaining"))
//             .unwrap(),
//         "0"
//     );
//     assert!(err_response
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-whitelisted"))
//         .is_none());

//     let body = actix_web::body::to_bytes(err_response.into_body())
//         .await
//         .unwrap();
//     assert_eq!(body, "Too many requests, retry in 0s");
// }

// #[tokio::test]
// async fn test_method_filter_use_headers() {
//     use crate::{Governor, GovernorConfigBuilder, Method};
//     use actix_web::test;

//     let config = GovernorConfigBuilder::default()
//         .per_millisecond(90)
//         .burst_size(2)
//         .methods(vec![Method::GET])
//         .use_headers()
//         .finish()
//         .unwrap();

//     let app = test::init_service(
//         App::new()
//             .wrap(Governor::new(&config))
//             .route("/", web::get().to(hello))
//             .route("/", web::post().to(hello)),
//     )
//     .await;

//     use std::net::{IpAddr, Ipv4Addr, SocketAddr};
//     let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 80u16);

//     // First request
//     let req = reqwest::get().peer_addr(addr).uri("/").to_request();
//     let test = test::call_service(&app, req).await;
//     assert_eq!(test.status(), StatusCode::OK);
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-limit"))
//             .unwrap(),
//         "2"
//     );
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-remaining"))
//             .unwrap(),
//         "1"
//     );
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-after"))
//         .is_none());
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-whitelisted"))
//         .is_none());

//     // Second request
//     let req = reqwest::get().peer_addr(addr).uri("/").to_request();
//     let test = test::call_service(&app, req).await;
//     assert_eq!(test.status(), StatusCode::OK);
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-limit"))
//             .unwrap(),
//         "2"
//     );
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-remaining"))
//             .unwrap(),
//         "0"
//     );
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-after"))
//         .is_none());
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-whitelisted"))
//         .is_none());

//     // Third request -> Over limit, returns Error
//     let req = reqwest::get().peer_addr(addr).uri("/").to_request();
//     let test = app.call(req).await.unwrap_err();
//     let err_response: Response = test.error_response();
//     assert_eq!(err_response.status(), StatusCode::TOO_MANY_REQUESTS);
//     assert_eq!(
//         err_response
//             .headers()
//             .get(HeaderName::from_static("x-ratelimit-after"))
//             .unwrap(),
//         "0"
//     );
//     assert_eq!(
//         err_response
//             .headers()
//             .get(HeaderName::from_static("x-ratelimit-limit"))
//             .unwrap(),
//         "2"
//     );
//     assert_eq!(
//         err_response
//             .headers()
//             .get(HeaderName::from_static("x-ratelimit-remaining"))
//             .unwrap(),
//         "0"
//     );
//     assert!(err_response
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-whitelisted"))
//         .is_none());

//     // Fourth request, now a POST request
//     // This one is ignored by the ratelimit
//     let req = reqwest::post().peer_addr(addr).uri("/").to_request();
//     let test = test::call_service(&app, req).await;
//     assert_eq!(test.status(), StatusCode::OK);
//     assert_eq!(
//         test.headers()
//             .get(HeaderName::from_static("x-ratelimit-whitelisted"))
//             .unwrap(),
//         "true"
//     );
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-limit"))
//         .is_none());
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-remaining"))
//         .is_none());
//     assert!(test
//         .headers()
//         .get(HeaderName::from_static("x-ratelimit-after"))
//         .is_none());
// }

// #[tokio::test]
// async fn test_json_error_response() {
//     use crate::{governor::GovernorConfigBuilder, Governor};
//     use actix_web::test;

//     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//     struct FooKeyExtractor;

//     impl KeyExtractor for FooKeyExtractor {
//         type Key = String;
//         type KeyExtractionError = GovernorError;

//         fn extract<B>(&self, _req: &Request<B>) -> Result<Self::Key, Self::KeyExtractionError> {
//             Ok("test".to_owned())
//         }
//     }

//     let config = GovernorConfigBuilder::default()
//         .burst_size(2)
//         .per_second(3)
//         .key_extractor(FooKeyExtractor)
//         .finish()
//         .unwrap();
//     let app = test::init_service(
//         App::new()
//             .wrap(Governor::new(&config))
//             .route("/", web::get().to(hello)),
//     )
//     .await;

//     // First request
//     let req = reqwest::get().uri("/").to_request();
//     assert_eq!(test::call_service(&app, req).await.status(), StatusCode::OK);
//     // Second request
//     let req = reqwest::get().uri("/").to_request();
//     assert_eq!(test::call_service(&app, req).await.status(), StatusCode::OK);
//     // Third request
//     let err_req = reqwest::get().uri("/").to_request();
//     let err_res: Response = app.call(err_req).await.unwrap_err().error_response();
//     assert_eq!(
//         err_res.headers().get(header::CONTENT_TYPE).unwrap(),
//         HeaderValue::from_static("application/json")
//     );
//     let body = actix_web::body::to_bytes(err_res.into_body())
//         .await
//         .unwrap();
//     assert_eq!(body, "{\"msg\":\"Test\"}".to_owned());
// }

// #[tokio::test]
// async fn test_forbidden_response_error() {
//     use crate::{Governor, GovernorConfigBuilder};
//     use actix_web::test;

//     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//     struct FooKeyExtractor;

//     impl KeyExtractor for FooKeyExtractor {
//         type Key = String;
//         type KeyExtractionError = GovernorError<&'static str>;

//         fn extract(
//             &self,
//             _req: &actix_web::dev::ServiceRequest,
//         ) -> Result<Self::Key, Self::KeyExtractionError> {
//             Err(GovernorError::new("test").set_status_code(StatusCode::FORBIDDEN))
//         }
//     }

//     let config = GovernorConfigBuilder::default()
//         .burst_size(2)
//         .per_second(3)
//         .key_extractor(FooKeyExtractor)
//         .finish()
//         .unwrap();
//     let app = test::init_service(
//         App::new()
//             .wrap(Governor::new(&config))
//             .route("/", web::get().to(hello)),
//     )
//     .await;

//     // First request
//     let req = reqwest::get().uri("/").to_request();
//     let err_res = app.call(req).await.unwrap_err();
//     assert_eq!(
//         err_res.as_response_error().status_code(),
//         StatusCode::FORBIDDEN
//     );
// }

// #[tokio::test]
// async fn test_html_error_response() {
//     use crate::{Governor, GovernorConfigBuilder};
//     use actix_web::test;

//     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//     struct FooKeyExtractor;

//     impl KeyExtractor for FooKeyExtractor {
//         type Key = String;
//         type KeyExtractionError = GovernorError<String>;

//         fn extract(
//             &self,
//             _req: &actix_web::dev::ServiceRequest,
//         ) -> Result<Self::Key, Self::KeyExtractionError> {
//             Ok("test".to_owned())
//         }

//         fn exceed_rate_limit_response(
//             &self,
//             _negative: &governor::NotUntil<governor::clock::QuantaInstant>,
//             mut response: ResponseBuilder,
//         ) -> Response {
//             response.content_type(ContentType::html()).body(
//                 r#"<!DOCTYPE html><html lang="en"><head></head><body><h1>Rate limit error</h1></body></html>"#
//             )
//         }
//     }

//     let config = GovernorConfigBuilder::default()
//         .burst_size(2)
//         .per_second(3)
//         .key_extractor(FooKeyExtractor)
//         .finish()
//         .unwrap();
//     let app = test::init_service(
//         App::new()
//             .wrap(Governor::new(&config))
//             .route("/", web::get().to(hello)),
//     )
//     .await;

//     // First request
//     let req = reqwest::get().uri("/").to_request();
//     assert_eq!(test::call_service(&app, req).await.status(), StatusCode::OK);
//     // Second request
//     let req = reqwest::get().uri("/").to_request();
//     assert_eq!(test::call_service(&app, req).await.status(), StatusCode::OK);
//     // Third request
//     let err_req = reqwest::get().uri("/").to_request();
//     let err_res = app.call(err_req).await.unwrap_err().error_response();
//     assert_eq!(
//         err_res.headers().get(header::CONTENT_TYPE).unwrap(),
//         HeaderValue::from_static("text/html; charset=utf-8")
//     );
//     let body = actix_web::body::to_bytes(err_res.into_body())
//         .await
//         .unwrap();
//     assert_eq!(body,"<!DOCTYPE html><html lang=\"en\"><head></head><body><h1>Rate limit error</h1></body></html>".to_owned());
// }

// #[tokio::test]
// async fn test_network_authentication_required_response_error() {
//     use crate::{Governor, GovernorConfigBuilder};
//     use actix_web::test;

//     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//     struct FooKeyExtractor;

//     impl KeyExtractor for FooKeyExtractor {
//         type Key = String;
//         type KeyExtractionError = GovernorError;

//         fn extract<B>(&self, _req: &Request<B>) -> Result<Self::Key, Self::KeyExtractionError> {
//             Err(GovernorError::new("test")
//                 .set_status_code(StatusCode::NETWORK_AUTHENTICATION_REQUIRED))
//         }
//     }

//     let config = GovernorConfigBuilder::default()
//         .burst_size(2)
//         .per_second(3)
//         .key_extractor(FooKeyExtractor)
//         .finish()
//         .unwrap();
//     let app = test::init_service(
//         App::new()
//             .wrap(Governor::new(&config))
//             .route("/", web::get().to(hello)),
//     )
//     .await;

//     // First request
//     let req = reqwest::get().uri("/").to_request();
//     let err_res = app.call(req).await.unwrap_err();
//     assert_eq!(
//         err_res.as_response_error().status_code(),
//         StatusCode::NETWORK_AUTHENTICATION_REQUIRED
//     );
// }
