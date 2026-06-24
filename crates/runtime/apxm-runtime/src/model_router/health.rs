//! Circuit breaker for LLM backend health management.
//!
//! Implements a standard three-state circuit breaker per backend:
//!
//! ```text
//! Closed ──(failures >= threshold)──► Open
//!   ▲                                   │
//!   │                                   │ (timeout elapsed)
//!   │                                   ▼
//!   └────(success in half-open)──── HalfOpen
//! ```
//!
//! - **Closed**: Normal operation. Failures accumulate toward the trip threshold.
//! - **Open**: Backend is tripped; requests are rejected immediately.
//! - **HalfOpen**: Cooldown expired; one probe request is allowed through.
//!   Success resets to Closed; failure returns to Open.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Circuit breaker state for a single backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CircuitState {
    /// Normal operation.
    #[default]
    Closed,
    /// Backend tripped; requests rejected.
    Open,
    /// Cooldown expired; one probe allowed.
    HalfOpen,
}

/// Configuration for the circuit breaker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures to trip the circuit.
    pub failure_threshold: usize,
    /// How long to stay Open before moving to HalfOpen.
    pub open_duration: Duration,
    /// Minimum requests before health evaluation begins.
    pub min_requests: usize,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        CircuitBreakerConfig {
            failure_threshold: 5,
            open_duration: Duration::from_secs(30),
            min_requests: 3,
        }
    }
}

/// Per-backend circuit breaker state.
struct BreakerState {
    state: CircuitState,
    consecutive_failures: usize,
    total_requests: usize,
    total_successes: usize,
    tripped_at: Option<Instant>,
    config: CircuitBreakerConfig,
}

impl BreakerState {
    fn new(config: CircuitBreakerConfig) -> Self {
        BreakerState {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            total_requests: 0,
            total_successes: 0,
            tripped_at: None,
            config,
        }
    }

    /// Compute effective current state (may auto-transition Open → HalfOpen).
    fn effective_state(&self) -> CircuitState {
        if self.state == CircuitState::Open {
            if let Some(tripped_at) = self.tripped_at {
                if tripped_at.elapsed() >= self.config.open_duration {
                    return CircuitState::HalfOpen;
                }
            }
        }
        self.state
    }

    fn record_success(&mut self) {
        self.total_requests += 1;
        self.total_successes += 1;
        self.consecutive_failures = 0;
        // HalfOpen probe succeeded → close the circuit
        if self.state == CircuitState::Open {
            if let Some(tripped_at) = self.tripped_at {
                if tripped_at.elapsed() >= self.config.open_duration {
                    self.state = CircuitState::Closed;
                    self.tripped_at = None;
                }
            }
        } else {
            self.state = CircuitState::Closed;
        }
    }

    fn record_failure(&mut self) {
        self.total_requests += 1;
        self.consecutive_failures += 1;
        // Check if we should trip
        if self.total_requests >= self.config.min_requests
            && self.consecutive_failures >= self.config.failure_threshold
        {
            self.state = CircuitState::Open;
            self.tripped_at = Some(Instant::now());
        }
    }

    fn success_rate(&self) -> f64 {
        if self.total_requests == 0 {
            return 1.0; // Optimistic before first request
        }
        self.total_successes as f64 / self.total_requests as f64
    }
}

/// Health snapshot for a backend (returned to callers / CLI).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendHealth {
    pub name: String,
    pub state: CircuitState,
    pub consecutive_failures: usize,
    pub total_requests: usize,
    pub success_rate: f64,
    pub is_available: bool,
}

/// Circuit breaker registry — one breaker per registered backend.
pub struct CircuitBreakerRegistry {
    breakers: Arc<DashMap<String, parking_lot::Mutex<BreakerState>>>,
    default_config: CircuitBreakerConfig,
}

impl CircuitBreakerRegistry {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        CircuitBreakerRegistry {
            breakers: Arc::new(DashMap::new()),
            default_config: config,
        }
    }

    /// Register a backend with this registry.
    pub fn register(&self, name: &str) {
        self.breakers.entry(name.to_string()).or_insert_with(|| {
            parking_lot::Mutex::new(BreakerState::new(self.default_config.clone()))
        });
    }

    /// Check whether a backend is available to receive a request.
    ///
    /// - `Closed` → available
    /// - `HalfOpen` → available (probe request)
    /// - `Open` → not available
    pub fn is_available(&self, name: &str) -> bool {
        self.breakers
            .get(name)
            .map(|entry| entry.lock().effective_state() != CircuitState::Open)
            .unwrap_or(true) // Unknown backends are assumed available
    }

    /// Record a successful request to a backend.
    pub fn record_success(&self, name: &str) {
        if let Some(entry) = self.breakers.get(name) {
            entry.lock().record_success();
        }
    }

    /// Record a failed request to a backend.
    pub fn record_failure(&self, name: &str) {
        if let Some(entry) = self.breakers.get(name) {
            entry.lock().record_failure();
        }
    }

    /// Get health snapshot for a backend.
    pub fn health(&self, name: &str) -> Option<BackendHealth> {
        self.breakers.get(name).map(|entry| {
            let state = entry.lock();
            BackendHealth {
                name: name.to_string(),
                state: state.effective_state(),
                consecutive_failures: state.consecutive_failures,
                total_requests: state.total_requests,
                success_rate: state.success_rate(),
                is_available: state.effective_state() != CircuitState::Open,
            }
        })
    }

    /// Get health snapshots for all registered backends.
    pub fn all_health(&self) -> Vec<BackendHealth> {
        self.breakers
            .iter()
            .map(|entry| {
                let state = entry.lock();
                BackendHealth {
                    name: entry.key().clone(),
                    state: state.effective_state(),
                    consecutive_failures: state.consecutive_failures,
                    total_requests: state.total_requests,
                    success_rate: state.success_rate(),
                    is_available: state.effective_state() != CircuitState::Open,
                }
            })
            .collect()
    }

    /// Reset the circuit breaker for a backend (force-close).
    pub fn reset(&self, name: &str) {
        if let Some(entry) = self.breakers.get(name) {
            let mut state = entry.lock();
            state.state = CircuitState::Closed;
            state.consecutive_failures = 0;
            state.tripped_at = None;
        }
    }
}

impl Default for CircuitBreakerRegistry {
    fn default() -> Self {
        Self::new(CircuitBreakerConfig::default())
    }
}
