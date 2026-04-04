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
    pub fn check_and_consume(&self, backend: &str) -> Result<(), RateLimitError> {
        let now = self.clock.now();
        let mut guard = self.buckets.lock().expect("rate limiter mutex poisoned");

        match guard.get_mut(backend) {
            Some(bucket) => {
                if bucket.try_consume(now, 1.0) {
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

        assert_eq!(limiter.check_and_consume("openai"), Ok(()));
        assert_eq!(limiter.check_and_consume("openai"), Ok(()));
        assert_eq!(limiter.check_and_consume("openai"), Ok(()));
    }

    #[test]
    fn rejects_when_capacity_exhausted() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("anthropic".to_string(), config(2, 1.0));

        let limiter = RateLimiter::new(configs, clock).unwrap();

        assert_eq!(limiter.check_and_consume("anthropic"), Ok(()));
        assert_eq!(limiter.check_and_consume("anthropic"), Ok(()));
        assert_eq!(
            limiter.check_and_consume("anthropic"),
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

        assert_eq!(limiter.check_and_consume("openai"), Ok(()));
        assert_eq!(limiter.check_and_consume("openai"), Ok(()));
        assert!(limiter.check_and_consume("openai").is_err());

        clock.advance(Duration::from_secs(1));
        assert_eq!(limiter.check_and_consume("openai"), Ok(()));
        assert!(limiter.check_and_consume("openai").is_err());
    }

    #[test]
    fn caps_refill_at_capacity() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("openai".to_string(), config(3, 10.0));

        let limiter = RateLimiter::new(configs, clock.clone()).unwrap();

        assert_eq!(limiter.check_and_consume("openai"), Ok(()));
        assert_eq!(limiter.check_and_consume("openai"), Ok(()));

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
            assert_eq!(limiter.check_and_consume("unconfigured"), Ok(()));
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

        assert_eq!(limiter.check_and_consume("a"), Ok(()));
        assert!(limiter.check_and_consume("a").is_err());

        assert_eq!(limiter.check_and_consume("b"), Ok(()));
        assert_eq!(limiter.check_and_consume("b"), Ok(()));
        assert!(limiter.check_and_consume("b").is_err());
    }

    #[test]
    fn fractional_refill_works() {
        let start = Instant::now();
        let clock = Arc::new(ManualClock::new(start));
        let mut configs = HashMap::new();
        configs.insert("x".to_string(), config(1, 2.0));

        let limiter = RateLimiter::new(configs, clock.clone()).unwrap();

        assert_eq!(limiter.check_and_consume("x"), Ok(()));
        assert!(limiter.check_and_consume("x").is_err());

        clock.advance(Duration::from_millis(400));
        assert!(limiter.check_and_consume("x").is_err());

        clock.advance(Duration::from_millis(100));
        assert_eq!(limiter.check_and_consume("x"), Ok(()));
    }
}
