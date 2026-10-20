use axum::{routing::get, Router};
use governor::nanos::Nanos;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::state::StateStore;
use http::{request::Request, StatusCode};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tower_governor::{
    errors::GovernorError, governor::GovernorConfigBuilder, key_extractor::KeyExtractor,
    GovernorLayer,
};

/// Key every request by its bearer token.
#[derive(Debug, Clone, Eq, PartialEq)]
struct UserToken;

impl KeyExtractor for UserToken {
    type Key = String;

    fn extract<B>(&self, req: &Request<B>) -> Result<Self::Key, GovernorError> {
        req.headers()
            .get("Authorization")
            .and_then(|token| token.to_str().ok())
            .and_then(|token| token.strip_prefix("Bearer "))
            .map(|token| token.trim().to_owned())
            .ok_or(GovernorError::Other {
                code: StatusCode::UNAUTHORIZED,
                msg: Some("You don't have permission to access".to_string()),
                headers: None,
            })
    }

    fn key_name(&self, key: &Self::Key) -> Option<String> {
        Some(key.clone())
    }

    fn name(&self) -> &'static str {
        "UserToken"
    }
}

/// A state store that keeps `unlimited` out of the bucket table and forwards
/// everything else to the default in-memory store.
///
/// This is the supported way to express "this caller is exempt": the middleware
/// still asks the store about every request, but returning without storing
/// anything means no bucket is ever created for the exempt key, so it costs no
/// memory and can never be throttled.
#[derive(Default)]
struct ExemptStore {
    inner: DefaultKeyedStateStore<String>,
}

impl StateStore for ExemptStore {
    type Key = String;

    fn measure_and_replace<T, F, E>(&self, key: &Self::Key, f: F) -> Result<T, E>
    where
        F: Fn(Option<Nanos>) -> Result<(T, Nanos), E>,
    {
        if key == "unlimited" {
            // No stored state means "fresh bucket": the closure admits the request
            // and the updated state it computes is simply dropped.
            return f(None).map(|(decision, _state)| decision);
        }

        self.inner.measure_and_replace(key, f)
    }
}

async fn hello() -> &'static str {
    "Hello world"
}

#[tokio::main]
async fn main() {
    // Configure tracing if desired
    // construct a subscriber that prints formatted traces to stdout
    let subscriber = tracing_subscriber::FmtSubscriber::new();
    // use this subscriber to process traces emitted after this point
    tracing::subscriber::set_global_default(subscriber).unwrap();

    // Five requests per token, with one token ("unlimited") exempt from the limit.
    let governor_conf = GovernorConfigBuilder::default()
        .per_second(20)
        .burst_size(5)
        .key_extractor(UserToken)
        .use_headers()
        .finish_with(ExemptStore::default())
        .expect("valid configuration");

    // build our application with a route
    let app = Router::new()
        .route("/", get(hello))
        .layer(GovernorLayer::new(governor_conf));

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    let listener = TcpListener::bind(addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();

    // Try it:
    //   curl -H 'Authorization: Bearer alice' localhost:3000   # limited
    //   curl -H 'Authorization: Bearer unlimited' localhost:3000  # never limited
}
