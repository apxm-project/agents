use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RateLimitConfig {
    /// Maximum tokens a bucket can hold.
    pub capacity: u32,
    /// Number of tokens added every refill interval.
    pub refill_tokens: u32,
    /// How often tokens are refilled.
    pub refill_interval: Duration,
}

impl RateLimitConfig {
    pub fn validate(&self) -> Result<(), RateLimitConfigError> {
        if self.capacity == 0 {
            return Err(RateLimitConfigError::ZeroCapacity);
        }
        if self.refill_tokens == 0 {
            return Err(RateLimitConfigError::ZeroRefillTokens);
        }
        if self.refill_interval.is_zero() {
            return Err(RateLimitConfigError::ZeroRefillInterval);
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RateLimitConfigError {
    #[error("rate limit capacity must be greater than zero")]
    ZeroCapacity,
    #[error("rate limit refill_tokens must be greater than zero")]
    ZeroRefillTokens,
    #[error("rate limit refill_interval must be greater than zero")]
    ZeroRefillInterval,
}

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug, Clone)]
struct Bucket {
    tokens: u32,
    last_refill: Instant,
}

impl Bucket {
    fn new(now: Instant, capacity: u32) -> Self {
        Self {
            tokens: capacity,
            last_refill: now,
        }
    }

    fn refill(&mut self, now: Instant, cfg: &RateLimitConfig) {
        if now <= self.last_refill {
            return;
        }

        let elapsed = now.duration_since(self.last_refill);
        let interval_nanos = cfg.refill_interval.as_nanos();
        if interval_nanos == 0 {
            return;
        }

        let elapsed_nanos = elapsed.as_nanos();
        let intervals = elapsed_nanos / interval_nanos;
        if intervals == 0 {
            return;
        }

        let added = intervals.saturating_mul(cfg.refill_tokens as u128);
        let new_tokens = (self.tokens as u128).saturating_add(added);
        self.tokens = new_tokens.min(cfg.capacity as u128) as u32;

        let consumed_nanos = intervals.saturating_mul(interval_nanos);
        let consumed_nanos_u64 = consumed_nanos.min(u64::MAX as u128) as u64;
        self.last_refill += Duration::from_nanos(consumed_nanos_u64);
    }

    fn try_consume(&mut self, now: Instant, cfg: &RateLimitConfig, cost: u32) -> bool {
        self.refill(now, cfg);

        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RateLimitError {
    #[error("invalid rate limit config: {0}")]
    InvalidConfig(#[from] RateLimitConfigError),

    #[error("request cost must be greater than zero")]
    ZeroCost,

    #[error("request cost {cost} exceeds bucket capacity {capacity}")]
    CostExceedsCapacity { cost: u32, capacity: u32 },

    #[error("rate limit exceeded for key '{key}'")]
    Exceeded { key: String },
}

#[derive(Debug)]
struct Inner {
    buckets: HashMap<String, Bucket>,
}

#[derive(Debug)]
pub struct RateLimiter<C: Clock = SystemClock> {
    cfg: RateLimitConfig,
    clock: Arc<C>,
    inner: Mutex<Inner>,
}

impl RateLimiter<SystemClock> {
    pub fn new(cfg: RateLimitConfig) -> Result<Self, RateLimitError> {
        Self::with_clock(cfg, Arc::new(SystemClock))
    }
}

impl<C: Clock> RateLimiter<C> {
    pub fn with_clock(cfg: RateLimitConfig, clock: Arc<C>) -> Result<Self, RateLimitError> {
        cfg.validate()?;
        Ok(Self {
            cfg,
            clock,
            inner: Mutex::new(Inner {
                buckets: HashMap::new(),
            }),
        })
    }

    pub async fn check(&self, key: impl Into<String>) -> Result<(), RateLimitError> {
        self.check_cost(key, 1).await
    }

    pub async fn check_cost(
        &self,
        key: impl Into<String>,
        cost: u32,
    ) -> Result<(), RateLimitError> {
        if cost == 0 {
            return Err(RateLimitError::ZeroCost);
        }

        if cost > self.cfg.capacity {
            return Err(RateLimitError::CostExceedsCapacity {
                cost,
                capacity: self.cfg.capacity,
            });
        }

        let key = key.into();
        let now = self.clock.now();
        let mut guard = self.inner.lock().await;

        let bucket = guard
            .buckets
            .entry(key.clone())
            .or_insert_with(|| Bucket::new(now, self.cfg.capacity));

        if bucket.try_consume(now, &self.cfg, cost) {
            Ok(())
        } else {
            Err(RateLimitError::Exceeded { key })
        }
    }

    pub fn config(&self) -> &RateLimitConfig {
        &self.cfg
    }
}
