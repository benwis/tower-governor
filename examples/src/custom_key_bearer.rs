use tokio_governor::governor::{Governor, GovernorConfigBuilder, KeyExtractor, SimpleKeyExtractionError};
use http::StatusCode;
use axum::{routing::get, Router};
use std::net::SocketAddr;
use governor::clock::{Clock, DefaultClock};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
struct UserToken;

impl KeyExtractor for UserToken {
    type Key = String;
    type KeyExtractionError = SimpleKeyExtractionError<&'static str>;

    #[cfg(feature = "tracing")]
    fn name(&self) -> &'static str {
        "Bearer token"
    }

    fn extract(&self, req: &Request) -> Result<Self::Key, Self::KeyExtractionError> {
        req.headers()
            .get("Authorization")
            .and_then(|token| token.to_str().ok())
            .and_then(|token| token.strip_prefix("Bearer "))
            .and_then(|token| Some(token.trim().to_owned()))
            .ok_or(
                Self::KeyExtractionError::new(
                    r#"{ "code": 401, "msg": "You don't have permission to access"}"#,
                )
                .set_content_type(ContentType::json())
                .set_status_code(StatusCode::UNAUTHORIZED),
            )
    }

    #[cfg(feature = "tracing")]
    fn key_name(&self, key: &Self::Key) -> Option<String> {
        Some("String".to_owned())
    }
}

async fn hello() -> &'static str {
    "Hello world"
}

#[tokio::main]
async fn main() {
    // Allow bursts with up to five requests per IP address
    // and replenishes one element every two seconds
    let governor_conf = GovernorConfigBuilder::default()
        .per_second(2)
        .burst_size(5)
        .finish()
        .unwrap();
    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(hello))
        .layer(GovernorLayer {
            config: governor_conf,
        });

    // run our app with hyper
    // `axum::Server` is a re-export of `hyper::Server`
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}
