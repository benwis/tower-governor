use crate::key_extractor::{KeyExtractor, PeerIpKeyExtractor};
use governor::{
    clock::{DefaultClock, QuantaInstant},
    middleware::{NoOpMiddleware, RateLimitingMiddleware, StateInformationMiddleware},
    state::keyed::DefaultKeyedStateStore,
    Quota, RateLimiter,
};
use http::Method;
use std::{marker::PhantomData, num::NonZeroU32, sync::Arc, time::Duration};

pub const DEFAULT_PERIOD: Duration = Duration::from_millis(500);
pub const DEFAULT_BURST_SIZE: u32 = 8;

// Required by Governor's RateLimiter to share it across threads
// See Governor User Guide: https://docs.rs/governor/0.6.0/governor/_guide/index.html
pub type SharedRateLimiter<Key, M> =
    Arc<RateLimiter<Key, DefaultKeyedStateStore<Key>, DefaultClock, M>>;

/// Helper struct for building a configuration for the governor middleware.
///
/// # Example
///
/// Create a configuration with a quota of ten requests per IP address
/// that replenishes one element every minute.
///
/// ```rust
/// use tower_governor::governor::GovernorConfigBuilder;
///
/// let config = GovernorConfigBuilder::default()
///     .per_second(60)
///     .burst_size(10)
///     .finish()
///     .unwrap();
/// ```
///
/// with x-ratelimit headers
///
/// ```rust
/// use tower_governor::governor::GovernorConfigBuilder;
///
/// let config = GovernorConfigBuilder::default()
///     .per_second(60)
///     .burst_size(10)
///     .use_headers() // Add this
///     .finish()
///     .unwrap();
/// ```
#[derive(Debug, Eq, Clone, PartialEq)]
pub struct GovernorConfigBuilder<K: KeyExtractor, M: RateLimitingMiddleware<QuantaInstant>> {
    period: Duration,
    burst_size: u32,
    methods: Option<Vec<Method>>,
    key_extractor: K,
    middleware: PhantomData<M>,
}

impl Default for GovernorConfigBuilder<PeerIpKeyExtractor, NoOpMiddleware> {
    /// The default configuration which is suitable for most services.
    /// Allows burst with up to eight requests and replenishes one element after 500ms, based on peer IP.
    /// The values can be modified by calling other methods on this struct.
    fn default() -> Self {
        Self::const_default()
    }
}

/// Sets the default Governor Config and defines all the different configuration functions
/// This one is used when the default PeerIpKeyExtractor is used
impl<M: RateLimitingMiddleware<QuantaInstant>> GovernorConfigBuilder<PeerIpKeyExtractor, M> {
    pub fn const_default() -> Self {
        GovernorConfigBuilder {
            period: DEFAULT_PERIOD,
            burst_size: DEFAULT_BURST_SIZE,
            methods: None,
            key_extractor: PeerIpKeyExtractor,
            middleware: PhantomData,
        }
    }
    /// Set the interval after which one element of the quota is replenished.
    ///
    /// **The interval must not be zero.**
    pub fn const_period(mut self, duration: Duration) -> Self {
        self.period = duration;
        self
    }
    /// Set the interval after which one element of the quota is replenished in seconds.
    ///
    /// **The interval must not be zero.**
    pub fn const_per_second(mut self, seconds: u64) -> Self {
        self.period = Duration::from_secs(seconds);
        self
    }
    /// Set the interval after which one element of the quota is replenished in milliseconds.
    ///
    /// **The interval must not be zero.**
    pub fn const_per_millisecond(mut self, milliseconds: u64) -> Self {
        self.period = Duration::from_millis(milliseconds);
        self
    }
    /// Set the interval after which one element of the quota is replenished in nanoseconds.
    ///
    /// **The interval must not be zero.**
    pub fn const_per_nanosecond(mut self, nanoseconds: u64) -> Self {
        self.period = Duration::from_nanos(nanoseconds);
        self
    }
    /// Set quota size that defines how many requests can occur
    /// before the governor middleware starts blocking requests from an IP address and
    /// clients have to wait until the elements of the quota are replenished.
    ///
    /// **The burst_size must not be zero.**
    pub fn const_burst_size(mut self, burst_size: u32) -> Self {
        self.burst_size = burst_size;
        self
    }
}

/// Sets configuration options when any Key Extractor is provided
impl<K: KeyExtractor, M: RateLimitingMiddleware<QuantaInstant>> GovernorConfigBuilder<K, M> {
    /// Set the interval after which one element of the quota is replenished.
    ///
    /// **The interval must not be zero.**
    pub fn period(&mut self, duration: Duration) -> &mut Self {
        self.period = duration;
        self
    }
    /// Set the interval after which one element of the quota is replenished in seconds.
    ///
    /// **The interval must not be zero.**
    pub fn per_second(&mut self, seconds: u64) -> &mut Self {
        self.period = Duration::from_secs(seconds);
        self
    }
    /// Set the interval after which one element of the quota is replenished in milliseconds.
    ///
    /// **The interval must not be zero.**
    pub fn per_millisecond(&mut self, milliseconds: u64) -> &mut Self {
        self.period = Duration::from_millis(milliseconds);
        self
    }
    /// Set the interval after which one element of the quota is replenished in nanoseconds.
    ///
    /// **The interval must not be zero.**
    pub fn per_nanosecond(&mut self, nanoseconds: u64) -> &mut Self {
        self.period = Duration::from_nanos(nanoseconds);
        self
    }
    /// Set quota size that defines how many requests can occur
    /// before the governor middleware starts blocking requests from an IP address and
    /// clients have to wait until the elements of the quota are replenished.
    ///
    /// **The burst_size must not be zero.**
    pub fn burst_size(&mut self, burst_size: u32) -> &mut Self {
        self.burst_size = burst_size;
        self
    }

    /// Set the HTTP methods this configuration should apply to.
    /// By default this is all methods.
    pub fn methods(&mut self, methods: Vec<Method>) -> &mut Self {
        self.methods = Some(methods);
        self
    }

    /// Set the key extractor this configuration should use.
    /// By default this is using the [PeerIpKeyExtractor].
    pub fn key_extractor<K2: KeyExtractor>(
        &mut self,
        key_extractor: K2,
    ) -> GovernorConfigBuilder<K2, M> {
        GovernorConfigBuilder {
            period: self.period,
            burst_size: self.burst_size,
            methods: self.methods.to_owned(),
            key_extractor,
            middleware: PhantomData,
        }
    }
    /// Set x-ratelimit headers to response, the headers is
    /// - `x-ratelimit-limit`       - Request limit
    /// - `x-ratelimit-remaining`   - The number of requests left for the time window
    /// - `x-ratelimit-after`       - Number of seconds in which the API will become available after its rate limit has been exceeded
    /// - `x-ratelimit-whitelisted` - If the request method not in methods, this header will be add it, use [`methods`] to add methods
    ///
    /// By default `x-ratelimit-after` is enabled, with [`use_headers`] will enable `x-ratelimit-limit`, `x-ratelimit-whitelisted` and `x-ratelimit-remaining`
    ///
    /// [`methods`]: crate::GovernorConfigBuilder::methods()
    /// [`use_headers`]: Self::use_headers
    pub fn use_headers(&mut self) -> GovernorConfigBuilder<K, StateInformationMiddleware> {
        GovernorConfigBuilder {
            period: self.period,
            burst_size: self.burst_size,
            methods: self.methods.to_owned(),
            key_extractor: self.key_extractor.clone(),
            middleware: PhantomData,
        }
    }

    /// Finish building the configuration and return the configuration for the middleware.
    /// Returns `None` if either burst size or period interval are zero.
    pub fn finish(&mut self) -> Option<GovernorConfig<K, M>> {
        if self.burst_size != 0 && self.period.as_nanos() != 0 {
            Some(GovernorConfig {
                key_extractor: self.key_extractor.clone(),
                limiter: Arc::new(
                    RateLimiter::keyed(
                        Quota::with_period(self.period)
                            .unwrap()
                            .allow_burst(NonZeroU32::new(self.burst_size).unwrap()),
                    )
                    .with_middleware::<M>(),
                ),
                methods: self.methods.clone(),
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
/// Configuration for the Governor middleware.
pub struct GovernorConfig<K: KeyExtractor, M: RateLimitingMiddleware<QuantaInstant>> {
    key_extractor: K,
    limiter: SharedRateLimiter<K::Key, M>,
    methods: Option<Vec<Method>>,
}

impl Default for GovernorConfig<PeerIpKeyExtractor, NoOpMiddleware> {
    /// The default configuration which is suitable for most services.
    /// Allows bursts with up to eight requests and replenishes one element after 500ms, based on peer IP.
    fn default() -> Self {
        GovernorConfigBuilder::default().finish().unwrap()
    }
}

impl<M: RateLimitingMiddleware<QuantaInstant>> GovernorConfig<PeerIpKeyExtractor, M> {
    /// A default configuration for security related services.
    /// Allows bursts with up to two requests and replenishes one element after four seconds, based on peer IP.
    ///
    /// This prevents brute-forcing passwords or security tokens
    /// yet allows to quickly retype a wrong password once before the quota is exceeded.
    pub fn secure() -> Self {
        GovernorConfigBuilder {
            period: Duration::from_secs(4),
            burst_size: 2,
            methods: None,
            key_extractor: PeerIpKeyExtractor,
            middleware: PhantomData,
        }
        .finish()
        .unwrap()
    }
}

/// Governor middleware factory. Hand this a GovernorConfig and it'll create this struct, which
/// contains everything needed to implement a middleware
/// https://stegosaurusdormant.com/understanding-derive-clone/
#[derive(Debug)]
pub struct Governor<K: KeyExtractor, M: RateLimitingMiddleware<QuantaInstant>, S> {
    pub key_extractor: K,
    pub limiter: SharedRateLimiter<K::Key, M>,
    pub methods: Option<Vec<Method>>,
    pub inner: S,
}
impl<K: KeyExtractor, M: RateLimitingMiddleware<QuantaInstant>, S: Clone> Clone
    for Governor<K, M, S>
{
    fn clone(&self) -> Self {
        Self {
            key_extractor: self.key_extractor.clone(),
            limiter: self.limiter.clone(),
            methods: self.methods.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<K: KeyExtractor, M: RateLimitingMiddleware<QuantaInstant>, S> Governor<K, M, S> {
    /// Create new governor middleware factory from configuration.
    pub fn new(inner: S, config: &GovernorConfig<K, M>) -> Self {
        Governor {
            key_extractor: config.key_extractor.clone(),
            limiter: config.limiter.clone(),
            methods: config.methods.clone(),
            inner,
        }
    }
}
