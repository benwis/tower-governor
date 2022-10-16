use axum::{routing::get, Router};
use axum_governor::{
    governor::{Governor, GovernorConfigBuilder},
    GovernorLayer,
};
// use axum_web::{web, App, HttpServer, Responder};
use std::net::SocketAddr;

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
    let addr = SocketAddr::from(([127, 0, 0, 1], 8080));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}
