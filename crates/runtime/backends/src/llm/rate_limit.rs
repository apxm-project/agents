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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RateLimitMode {
    Requests,
    Tokens,
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
        if self.token_based
            && (!(self.default_token_estimate.is_finite()) || self.default_token_estimate <= 0.0)
        {
            return Err(RateLimitConfigError::InvalidDefaultTokenEstimate(
                self.default_token_estimate,
            ));
        }
        Ok(())
    }

    fn mode(&self) -> RateLimitMode {
        if self.token_based {
            RateLimitMode::Tokens
        } else {
            RateLimitMode::Requests
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum RateLimitConfigError {
    #[error("rate limit capacity must be > 0")]
    ZeroCapacity,

    #[error("tokens_per_second must be finite and > 0, got {0}")]
    InvalidTokensPerSecond(f64),

    #[error("default_token_estimate must be finite and > 0 in token-based mode, got {0}")]
    InvalidDefaultTokenEstimate(f64),
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

#[derive(Debug, Clone)]
struct RateLimitState {
    bucket: TokenBucket,
    mode: RateLimitMode,
    default_token_estimate: f64,
}

impl RateLimitState {
    fn new(config: &RateLimitConfig, now: Instant) -> Self {
        Self {
            bucket: TokenBucket::new(config, now),
            mode: config.mode(),
            default_token_estimate: config.default_token_estimate,
        }
    }

    fn request_cost(&self, estimated_tokens: Option<f64>) -> f64 {
        match self.mode {
            RateLimitMode::Requests => 1.0,
            RateLimitMode::Tokens => estimated_tokens
                .filter(|tokens| tokens.is_finite() && *tokens > 0.0)
                .unwrap_or(self.default_token_estimate),
        }
    }
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
    buckets: Mutex<HashMap<String, RateLimitState>>,
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
            buckets.insert(backend, RateLimitState::new(&config, now));
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
    /// * `cost` - The token cost to consume
    pub fn check_and_consume(&self, backend: &str, cost: f64) -> Result<(), RateLimitError> {
        let now = self.clock.now();
        let mut guard = self.buckets.lock().expect("rate limiter mutex poisoned");

        match guard.get_mut(backend) {
            Some(state) => {
                if state.bucket.try_consume(now, cost) {
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

    /// Enforce the configured request mode for a backend.
    ///
    /// Request-count limiters always consume one unit per request. Token-based
    /// limiters consume the provided token estimate, falling back to the
    /// backend's configured default estimate when no estimate is available.
    /// Returns the cost that was admitted so the caller can reconcile later.
    pub fn check_and_consume_request(
        &self,
        backend: &str,
        estimated_tokens: Option<f64>,
    ) -> Result<f64, RateLimitError> {
        let now = self.clock.now();
        let mut guard = self.buckets.lock().expect("rate limiter mutex poisoned");

        match guard.get_mut(backend) {
            Some(state) => {
                let cost = state.request_cost(estimated_tokens);
                if state.bucket.try_consume(now, cost) {
                    Ok(cost)
                } else {
                    Err(RateLimitError::Limited {
                        backend: backend.to_string(),
                    })
                }
            }
            None => Ok(1.0),
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

        if let Some(state) = guard.get_mut(backend) {
            let delta = actual - estimated;
            state.bucket.adjust(now, delta);
        }
    }

    /// Reconcile actual token usage after a request completes.
    ///
    /// Per-request limiters intentionally do not reconcile because the admitted
    /// cost is the request itself, not the resulting token count.
    pub fn reconcile_request(&self, backend: &str, estimated: f64, actual_tokens: Option<f64>) {
        let now = self.clock.now();
        let mut guard = self.buckets.lock().expect("rate limiter mutex poisoned");

        if let Some(state) = guard.get_mut(backend)
            && state.mode == RateLimitMode::Tokens
        {
            let actual = actual_tokens
                .filter(|tokens| tokens.is_finite() && *tokens > 0.0)
                .unwrap_or(estimated);
            let delta = actual - estimated;
            state.bucket.adjust(now, delta);
        }
    }

    #[cfg(test)]
    fn available_tokens(&self, backend: &str) -> Option<f64> {
        let now = self.clock.now();
        let guard = self.buckets.lock().expect("rate limiter mutex poisoned");
        guard
            .get(backend)
            .map(|state| state.bucket.available_tokens(now))
    }
}
