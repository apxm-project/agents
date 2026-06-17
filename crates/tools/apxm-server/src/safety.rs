//! HTTP safety middleware: per-principal rate limiting and request body caps.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use apxm_driver::ServerSafetyConfig;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::principal::principal_from_request;

#[derive(Clone)]
pub(crate) struct SafetyState {
    limiter: Option<Arc<RateLimiter>>,
}

impl SafetyState {
    pub(crate) fn from_config(config: &ServerSafetyConfig) -> Self {
        let limiter = config.rate_limit_rps.filter(|rps| *rps > 0).map(|rps| {
            let burst = config.rate_limit_burst.unwrap_or(rps).max(1);
            Arc::new(RateLimiter::new(rps, burst))
        });
        Self { limiter }
    }
}

struct RateLimiter {
    rps: f64,
    burst: f64,
    buckets: Mutex<HashMap<String, TokenBucket>>,
}

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    fn new(rps: u32, burst: u32) -> Self {
        Self {
            rps: f64::from(rps),
            burst: f64::from(burst),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    fn check(&self, key: &str) -> bool {
        let mut buckets = self.buckets.lock().expect("rate limiter mutex");
        let now = Instant::now();
        let bucket = buckets
            .entry(key.to_owned())
            .or_insert_with(|| TokenBucket {
                tokens: self.burst,
                last_refill: now,
            });
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.rps).min(self.burst);
        bucket.last_refill = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

fn rate_limit_exceeded() -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(serde_json::json!({
            "class": "server_fault",
            "code": "rate_limit_exceeded",
            "message": "request rate limit exceeded for this principal"
        })),
    )
        .into_response()
}

/// Per-principal token-bucket rate limiter. Transparent when disabled.
pub(crate) async fn rate_limit_middleware(
    State(state): State<SafetyState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(limiter) = state.limiter.as_ref() else {
        return next.run(req).await;
    };

    let principal = principal_from_request(&req);
    if limiter.check(principal.as_str()) {
        next.run(req).await
    } else {
        rate_limit_exceeded()
    }
}

/// Build the optional axum body-limit layer from config.
pub(crate) fn body_limit_layer(
    config: &ServerSafetyConfig,
) -> Option<tower_http::limit::RequestBodyLimitLayer> {
    config
        .max_body_bytes
        .filter(|bytes| *bytes > 0)
        .map(tower_http::limit::RequestBodyLimitLayer::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bucket_enforces_burst() {
        let limiter = RateLimiter::new(2, 2);
        assert!(limiter.check("a"));
        assert!(limiter.check("a"));
        assert!(!limiter.check("a"));
    }

    #[test]
    fn principals_are_isolated() {
        let limiter = RateLimiter::new(1, 1);
        assert!(limiter.check("alice"));
        assert!(!limiter.check("alice"));
        assert!(limiter.check("bob"));
    }
}
