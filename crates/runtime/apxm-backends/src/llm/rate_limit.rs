use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A source of time used for deterministic testing.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

#[derive(Debug, Clone, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug)]
pub struct ManualClock {
    inner: Mutex<Instant>,
}

impl ManualClock {
    pub fn new(start: Instant) -> Self {
        Self {
            inner: Mutex::new(start),
        }
    }

    pub fn advance(&self, duration: Duration) {
        let mut guard = self.inner.lock().expect("manual clock mutex poisoned");
        *guard += duration;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.inner.lock().expect("manual clock mutex poisoned")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Maximum burst size.
    pub capacity: u32,
    /// Refill rate in tokens per second.
    pub tokens_per_second: f64,
    /// If true, rate limit by estimated token count instead of request count.
    #[serde(default)]
    pub token_based: bool,
    /// Default token estimate when actual count is unknown (used in token_based mode).
    #[serde(default = "default_token_estimate")]
    pub default_token_estimate: f64,
}

fn default_token_estimate() -> f64 {
    1.0
}

impl RateLimitConfig {
    pub fn validate(&self) -> Result<(), RateLimitConfigError> {
        if self.capacity == 0 {
            return Err(RateLimitConfigError::ZeroCapacity);
        }
        if !(self.tokens_per_second.is_finite()) || self.tokens_per_second <= 0.0 {
            return Err(RateLimitConfigError::InvalidTokensPerSecond(
                self.tokens_per_second,
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum RateLimitConfigError {
    #[error("rate limit capacity must be > 0")]
    ZeroCapacity,

    #[error("tokens_per_second must be finite and > 0, got {0}")]
    InvalidTokensPerSecond(f64),
}

#[derive(Debug, Error, PartialEq)]
pub enum RateLimitError {
    #[error("backend '{backend}' is rate limited")]
    Limited { backend: String },
}

#[derive(Debug, Clone)]
struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_rate_per_sec: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(config: &RateLimitConfig, now: Instant) -> Self {
        Self {
            capacity: config.capacity as f64,
            tokens: config.capacity as f64,
            refill_rate_per_sec: config.tokens_per_second,
            last_refill: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_refill);
        let added = elapsed.as_secs_f64() * self.refill_rate_per_sec;
        self.tokens = (self.tokens + added).min(self.capacity);
        self.last_refill = now;
    }

    fn try_consume(&mut self, now: Instant, amount: f64) -> bool {
        self.refill(now);
        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }

    fn adjust(&mut self, now: Instant, delta: f64) {
        self.refill(now);
        // Delta is positive to debit more, negative to credit back
        self.tokens = (self.tokens - delta).max(0.0).min(self.capacity);
    }

    #[cfg(test)]
    fn available_tokens(&self, now: Instant) -> f64 {
        let mut clone = self.clone();
        clone.refill(now);
        clone.tokens
    }
}

/// Rate limiter with per-backend token buckets.
///
/// Future extension point: The current key is backend name and may be
/// generalized later to support per-model or per-tenant rate limiting.
#[derive(Debug)]
pub struct RateLimiter<C: Clock> {
    clock: Arc<C>,
    buckets: Mutex<HashMap<String, TokenBucket>>,
}

impl<C: Clock> RateLimiter<C> {
    pub fn new(
        configs: HashMap<String, RateLimitConfig>,
        clock: Arc<C>,
    ) -> Result<Self, RateLimitConfigError> {
        let now = clock.now();
        let mut buckets = HashMap::with_capacity(configs.len());

        for (backend, config) in configs {
            config.validate()?;
            buckets.insert(backend, TokenBucket::new(&config, now));
        }

        Ok(Self {
            clock,
            buckets: Mutex::new(buckets),
        })
    }

    /// If backend has a configured bucket, enforce it.
    /// If backend is unconfigured, allow by default.
    ///
    /// # Arguments
    /// * `backend` - The backend name
    /// * `cost` - The token cost to consume (defaults to 1.0 for backward compatibility)
    pub fn check_and_consume(&self, backend: &str, cost: f64) -> Result<(), RateLimitError> {
        let now = self.clock.now();
        let mut guard = self.buckets.lock().expect("rate limiter mutex poisoned");

        match guard.get_mut(backend) {
            Some(bucket) => {
                if bucket.try_consume(now, cost) {
                    Ok(())
                } else {
                    Err(RateLimitError::Limited {
                        backend: backend.to_string(),
                    })
                }
            }
            None => Ok(()),
        }
    }

    /// Adjust token consumption based on actual usage after a request completes.
    ///
    /// This allows for post-request reconciliation when the actual token count
    /// differs from the initial estimate. If actual > estimated, debits additional
    /// tokens; if actual < estimated, credits tokens back.
    ///
    /// # Arguments
    /// * `backend` - The backend name
    /// * `estimated` - The token cost that was initially consumed
    /// * `actual` - The actual token cost after request completion
    pub fn reconcile(&self, backend: &str, estimated: f64, actual: f64) {
        let now = self.clock.now();
        let mut guard = self.buckets.lock().expect("rate limiter mutex poisoned");

        if let Some(bucket) = guard.get_mut(backend) {
            let delta = actual - estimated;
            bucket.adjust(now, delta);
        }
    }

    #[cfg(test)]
    fn available_tokens(&self, backend: &str) -> Option<f64> {
        let now = self.clock.now();
        let guard = self.buckets.lock().expect("rate limiter mutex poisoned");
        guard.get(backend).map(|b| b.available_tokens(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(capacity: u32, tokens_per_second: f64) -> RateLimitConfig {
        RateLimitConfig {
            capacity,
            tokens_per_second,
            token_based: false,
            default_token_estimate: 1.0,
        }
    }

    fn token_based_config(
        capacity: u32,
        tokens_per_second: f64,
        default_estimate: f64,
    ) -> RateLimitConfig {
        RateLimitConfig {
            capacity,
            tokens_per_second,
            token_based: true,
            default_token_estimate: default_estimate,
        }
    }

    #[test]
    fn validates_good_config() {
        let cfg = config(5, 2.0);
        assert_eq!(cfg.validate(), Ok(()));
    }

    #[test]
    fn rejects_zero_capacity() {
        let cfg = config(0, 1.0);
        assert_eq!(cfg.validate(), Err(RateLimitConfigError::ZeroCapacity));
    }

    #[test]
    fn rejects_zero_tokens_per_second() {
        let cfg = config(1, 0.0);
        assert_eq!(
            cfg.validate(),
            Err(RateLimitConfigError::InvalidTokensPerSecond(0.0))
        );
    }

    #[test]
    fn rejects_negative_tokens_per_second() {
        let cfg = config(1, -2.0);
        assert_eq!(
            cfg.validate(),
            Err(RateLimitConfigError::InvalidTokensPerSecond(-2.0))
        );
    }

    #[test]
    fn rejects_nan_tokens_per_second() {
        let cfg = config(1, f64::NAN);
        match cfg.validate() {
            Err(RateLimitConfigError::InvalidTokensPerSecond(v)) => assert!(v.is_nan()),
            other => panic!("unexpected result: {:?}", other),
        }
    }

    #[test]
    fn allows_requests_within_capacity() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("openai".to_string(), config(3, 1.0));

        let limiter = RateLimiter::new(configs, clock).unwrap();

        assert_eq!(limiter.check_and_consume("openai", 1.0), Ok(()));
        assert_eq!(limiter.check_and_consume("openai", 1.0), Ok(()));
        assert_eq!(limiter.check_and_consume("openai", 1.0), Ok(()));
    }

    #[test]
    fn rejects_when_capacity_exhausted() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("anthropic".to_string(), config(2, 1.0));

        let limiter = RateLimiter::new(configs, clock).unwrap();

        assert_eq!(limiter.check_and_consume("anthropic", 1.0), Ok(()));
        assert_eq!(limiter.check_and_consume("anthropic", 1.0), Ok(()));
        assert_eq!(
            limiter.check_and_consume("anthropic", 1.0),
            Err(RateLimitError::Limited {
                backend: "anthropic".to_string()
            })
        );
    }

    #[test]
    fn refills_after_time_passes() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("openai".to_string(), config(2, 1.0));

        let limiter = RateLimiter::new(configs, clock.clone()).unwrap();

        assert_eq!(limiter.check_and_consume("openai", 1.0), Ok(()));
        assert_eq!(limiter.check_and_consume("openai", 1.0), Ok(()));
        assert!(limiter.check_and_consume("openai", 1.0).is_err());

        clock.advance(Duration::from_secs(1));
        assert_eq!(limiter.check_and_consume("openai", 1.0), Ok(()));
        assert!(limiter.check_and_consume("openai", 1.0).is_err());
    }

    #[test]
    fn caps_refill_at_capacity() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("openai".to_string(), config(3, 10.0));

        let limiter = RateLimiter::new(configs, clock.clone()).unwrap();

        assert_eq!(limiter.check_and_consume("openai", 1.0), Ok(()));
        assert_eq!(limiter.check_and_consume("openai", 1.0), Ok(()));

        clock.advance(Duration::from_secs(10));

        let tokens = limiter.available_tokens("openai").unwrap();
        assert!(tokens <= 3.0);
        assert!(tokens > 2.9);
    }

    #[test]
    fn unknown_backend_is_unlimited_by_default() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let configs = HashMap::new();

        let limiter = RateLimiter::new(configs, clock).unwrap();

        for _ in 0..100 {
            assert_eq!(limiter.check_and_consume("unconfigured", 1.0), Ok(()));
        }
    }

    #[test]
    fn separate_backends_have_independent_buckets() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("a".to_string(), config(1, 1.0));
        configs.insert("b".to_string(), config(2, 1.0));

        let limiter = RateLimiter::new(configs, clock).unwrap();

        assert_eq!(limiter.check_and_consume("a", 1.0), Ok(()));
        assert!(limiter.check_and_consume("a", 1.0).is_err());

        assert_eq!(limiter.check_and_consume("b", 1.0), Ok(()));
        assert_eq!(limiter.check_and_consume("b", 1.0), Ok(()));
        assert!(limiter.check_and_consume("b", 1.0).is_err());
    }

    #[test]
    fn fractional_refill_works() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("x".to_string(), config(1, 2.0));

        let limiter = RateLimiter::new(configs, clock.clone()).unwrap();

        assert_eq!(limiter.check_and_consume("x", 1.0), Ok(()));
        assert!(limiter.check_and_consume("x", 1.0).is_err());

        clock.advance(Duration::from_millis(400));
        assert!(limiter.check_and_consume("x", 1.0).is_err());

        clock.advance(Duration::from_millis(100));
        assert_eq!(limiter.check_and_consume("x", 1.0), Ok(()));
    }

    #[test]
    fn token_based_mode_uses_actual_cost() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert(
            "token-backend".to_string(),
            token_based_config(1000, 100.0, 100.0),
        );

        let limiter = RateLimiter::new(configs, clock).unwrap();

        // Consume 100 tokens - should succeed
        assert_eq!(limiter.check_and_consume("token-backend", 100.0), Ok(()));
        assert_eq!(limiter.available_tokens("token-backend").unwrap(), 900.0);

        // Consume 500 tokens - should succeed
        assert_eq!(limiter.check_and_consume("token-backend", 500.0), Ok(()));
        assert_eq!(limiter.available_tokens("token-backend").unwrap(), 400.0);

        // Try to consume 500 tokens - should fail (only 400 available)
        assert!(limiter.check_and_consume("token-backend", 500.0).is_err());
    }

    #[test]
    fn reconcile_debits_when_actual_exceeds_estimate() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("backend".to_string(), config(1000, 100.0));

        let limiter = RateLimiter::new(configs, clock).unwrap();

        // Consume estimated 100 tokens
        assert_eq!(limiter.check_and_consume("backend", 100.0), Ok(()));
        assert_eq!(limiter.available_tokens("backend").unwrap(), 900.0);

        // Actual usage was 150 tokens - reconcile should debit 50 more
        limiter.reconcile("backend", 100.0, 150.0);
        assert_eq!(limiter.available_tokens("backend").unwrap(), 850.0);
    }

    #[test]
    fn reconcile_credits_when_actual_less_than_estimate() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("backend".to_string(), config(1000, 100.0));

        let limiter = RateLimiter::new(configs, clock).unwrap();

        // Consume estimated 200 tokens
        assert_eq!(limiter.check_and_consume("backend", 200.0), Ok(()));
        assert_eq!(limiter.available_tokens("backend").unwrap(), 800.0);

        // Actual usage was only 100 tokens - reconcile should credit back 100
        limiter.reconcile("backend", 200.0, 100.0);
        assert_eq!(limiter.available_tokens("backend").unwrap(), 900.0);
    }

    #[test]
    fn reconcile_does_not_exceed_capacity() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("backend".to_string(), config(1000, 100.0));

        let limiter = RateLimiter::new(configs, clock).unwrap();

        // Consume 50 tokens
        assert_eq!(limiter.check_and_consume("backend", 50.0), Ok(()));
        assert_eq!(limiter.available_tokens("backend").unwrap(), 950.0);

        // Actual usage was 10 tokens - reconcile credits back 40
        // But should cap at capacity (1000)
        limiter.reconcile("backend", 50.0, 10.0);
        assert_eq!(limiter.available_tokens("backend").unwrap(), 990.0);

        // Credit back more - should cap at 1000
        limiter.reconcile("backend", 100.0, 50.0);
        assert_eq!(limiter.available_tokens("backend").unwrap(), 1000.0);
    }

    #[test]
    fn reconcile_does_not_go_negative() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("backend".to_string(), config(100, 10.0));

        let limiter = RateLimiter::new(configs, clock).unwrap();

        // Consume all tokens
        assert_eq!(limiter.check_and_consume("backend", 100.0), Ok(()));
        assert_eq!(limiter.available_tokens("backend").unwrap(), 0.0);

        // Actual usage was even more - should debit but not go negative
        limiter.reconcile("backend", 100.0, 150.0);
        assert_eq!(limiter.available_tokens("backend").unwrap(), 0.0);
    }

    #[test]
    fn reconcile_with_unconfigured_backend_is_noop() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let configs = HashMap::new();

        let limiter = RateLimiter::new(configs, clock).unwrap();

        // Should not panic
        limiter.reconcile("unconfigured", 100.0, 150.0);
    }

    #[test]
    fn backward_compatible_per_request_mode() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        // Create config with token_based = false (default)
        configs.insert("legacy".to_string(), config(3, 1.0));

        let limiter = RateLimiter::new(configs, clock).unwrap();

        // Using cost of 1.0 per request (backward compatible)
        assert_eq!(limiter.check_and_consume("legacy", 1.0), Ok(()));
        assert_eq!(limiter.check_and_consume("legacy", 1.0), Ok(()));
        assert_eq!(limiter.check_and_consume("legacy", 1.0), Ok(()));
        assert!(limiter.check_and_consume("legacy", 1.0).is_err());
    }
}
