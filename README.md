 A Tower service and layer that provides a rate-limiting backed by [governor](https://github.com/antifuchs/governor). Based heavily on the work done for [actix-governor](https://github.com/AaronErhardt/actix-governor). Works with Axum, Hyper, Tower, Tonic, and anything else based on Tower!

 # Features:

 + Rate limit requests based on peer IP address, IP address headers, globally, or via custom keys
 + Custom traffic limiting criteria per second, or to certain bursts
 + Simple to use
 + High customizability
 + High performance
 + Robust yet flexible API


 # How does it work?

 Each governor middleware has a configuration that stores a quota.
 The quota specifies how many requests can be sent from an IP address
 before the middleware starts blocking further requests.

 For example if the quota allowed ten requests a client could send a burst of
 ten requests in short time before the middleware starts blocking.

 Once at least one element of the quota was used the elements of the quota
 will be replenished after a specified period.

 For example if this period was 2 seconds and the quota was empty
 it would take 2 seconds to replenish one element of the quota.
 This means you could send one request every two seconds on average.

 If there was a quota that allowed ten requests with the same period
 a client could again send a burst of ten requests and then had to wait
 two seconds before sending further requests or 20 seconds before the full
 quota would be replenished and he could send another burst.

 # Example
 ```rust,no_run
 use axum::{routing::get, Router};
use tower_governor::{
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
            config: &governor_conf,
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
 ```

 # Configuration presets

 Instead of using the configuration builder you can use predefined presets.

 + [`GovernorConfig::default()`]: The default configuration which is suitable for most services.
 Allows bursts with up to eight requests and replenishes one element after 500ms, based on peer IP.

 + [`GovernorConfig::secure()`]: A default configuration for security related services.
 Allows bursts with up to two requests and replenishes one element after four seconds, based on peer IP.

 For example the secure configuration can be used as a short version of this code:

 ```rust
 use tower_governor::governor::GovernorConfigBuilder;

 let config = GovernorConfigBuilder::default()
     .per_second(4)
     .burst_size(2)
     .finish()
     .unwrap();
 ```

 # Customize rate limiting key

 By default, rate limiting is done using the peer IP address (i.e. the IP address of the HTTP client that requested your app: either your user or a reverse proxy, depending on your deployment setup).
 You can configure a different behavior which:
 1. can be useful in itself
 2. allows you to setup multiple instances of this middleware based on different keys (for example, if you want to apply rate limiting with different rates on IP and API keys at the same time)

 This is achieved by defining a [KeyExtractor] and giving it to a [Governor] instance.
 Three ready-to-use key extractors are provided:
 - [PeerIpKeyExtractor]: this is the default
 - [SmartIpKeyExtractor]: Looks for common IP identification headers usually provided by CDNs and reverse proxies in order(x-forwarded-for,x-real-ip, forwarded) and falls back to the peer IP address.
 - [GlobalKeyExtractor]: uses the same key for all incoming requests

 Check out the [custom_key_bearer] example for more information.
 [custom_key_bearer]: https://github.com/benwis/tower-governor/blob/main/examples/src/custom_key_bearer.rs

 # Add x-ratelimit headers

 By default, `x-ratelimit-after` is enabled but if you want to enable `x-ratelimit-limit`, `x-ratelimit-whitelisted` and `x-ratelimit-remaining` use [`use_headers`] method

 [`use_headers`]: crate::governor::GovernorConfigBuilder::use_headers()

 # Common pitfalls

 1. Do not construct the same configuration multiple times, unless explicitly wanted!
 This will create an independent rate limiter for each configuration! Instead pass the same configuration reference into [`Governor::new()`], like it is described in the example.

 2. Be careful to create your server with `.into_make_service_with_connection_info::<SocketAddr>` instead of `.into_make_service()` if you are using the default PeerIpKeyExtractor. Otherwise there will be no peer ip address for Tower to find!