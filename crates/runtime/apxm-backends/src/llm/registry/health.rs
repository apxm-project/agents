//! Health monitoring for backends.
//!
//! Tracks backend health status based on recent request success/failure rates
//! and response latencies.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Health status of a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum HealthStatus {
    /// Backend is healthy (success rate >= 90%)
    Healthy,
    /// Backend is degraded (success rate 50-90%)
    Degraded,
    /// Backend is unhealthy (success rate < 50%)
    Unhealthy,
    /// Health status unknown (insufficient data)
    #[default]
    Unknown,
}

/// Health statistics for a backend.
#[derive(Debug, Clone)]
struct HealthStats {
    /// Total requests
    total_requests: usize,
    /// Successful requests
    successful_requests: usize,
    /// Failed requests
    failed_requests: usize,
    /// Recent latencies (last 10 requests)
    recent_latencies: Vec<Duration>,
    /// Last update timestamp
    last_updated: Instant,
    /// Explicit status override
    status_override: Option<HealthStatus>,
}

impl HealthStats {
    fn new() -> Self {
        HealthStats {
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            recent_latencies: Vec::new(),
            last_updated: Instant::now(),
            status_override: None,
        }
    }

    fn record_success(&mut self, latency: Duration) {
        self.total_requests += 1;
        self.successful_requests += 1;
        self.add_latency(latency);
        self.last_updated = Instant::now();
    }

    fn record_failure(&mut self, latency: Duration) {
        self.total_requests += 1;
        self.failed_requests += 1;
        self.add_latency(latency);
        self.last_updated = Instant::now();
    }

    fn add_latency(&mut self, latency: Duration) {
        self.recent_latencies.push(latency);
        if self.recent_latencies.len() > 10 {
            self.recent_latencies.remove(0);
        }
    }

    fn success_rate(&self) -> f64 {
        if self.total_requests == 0 {
            return 0.0;
        }
        (self.successful_requests as f64) / (self.total_requests as f64)
    }

    fn average_latency(&self) -> Option<Duration> {
        if self.recent_latencies.is_empty() {
            return None;
        }

        let total: Duration = self.recent_latencies.iter().sum();
        Some(total / self.recent_latencies.len() as u32)
    }

    fn compute_status(&self) -> HealthStatus {
        // Use override if set
        if let Some(status) = self.status_override {
            return status;
        }

        // Need at least 3 requests for meaningful health check
        if self.total_requests < 3 {
            return HealthStatus::Unknown;
        }

        let success_rate = self.success_rate();

        if success_rate >= 0.9 {
            HealthStatus::Healthy
        } else if success_rate >= 0.5 {
            HealthStatus::Degraded
        } else {
            HealthStatus::Unhealthy
        }
    }
}

/// Health monitor tracks backend health.
pub struct HealthMonitor {
    /// Health statistics per backend
    stats: Arc<DashMap<String, parking_lot::Mutex<HealthStats>>>,
}

impl HealthMonitor {
    /// Create a new health monitor.
    pub fn new() -> Self {
        HealthMonitor {
            stats: Arc::new(DashMap::new()),
        }
    }

    /// Register a backend for health tracking.
    pub fn register_backend(&self, name: &str) {
        self.stats.insert(
            name.to_string(),
            parking_lot::Mutex::new(HealthStats::new()),
        );
    }

    /// Unregister a backend from health tracking.
    pub fn unregister_backend(&self, name: &str) {
        self.stats.remove(name);
    }

    /// Record a successful request.
    pub fn record_success(&self, name: &str, latency: Duration) {
        if let Some(entry) = self.stats.get(name) {
            entry.value().lock().record_success(latency);
        }
    }

    /// Record a failed request.
    pub fn record_failure(&self, name: &str, latency: Duration) {
        if let Some(entry) = self.stats.get(name) {
            entry.value().lock().record_failure(latency);
        }
    }

    /// Get the health status of a backend.
    pub fn status(&self, name: &str) -> HealthStatus {
        self.stats
            .get(name)
            .map(|entry| entry.value().lock().compute_status())
            .unwrap_or(HealthStatus::Unknown)
    }

    /// Explicitly set the health status of a backend (override).
    pub fn set_status(&self, name: &str, status: HealthStatus) {
        if let Some(entry) = self.stats.get(name) {
            entry.value().lock().status_override = Some(status);
        }
    }

    /// Get success rate for a backend.
    pub fn success_rate(&self, name: &str) -> Option<f64> {
        self.stats
            .get(name)
            .map(|entry| entry.value().lock().success_rate())
    }

    /// Get average latency for a backend.
    pub fn average_latency(&self, name: &str) -> Option<Duration> {
        self.stats
            .get(name)
            .and_then(|entry| entry.value().lock().average_latency())
    }

    /// Get total request count for a backend.
    pub fn total_requests(&self, name: &str) -> Option<usize> {
        self.stats
            .get(name)
            .map(|entry| entry.value().lock().total_requests)
    }

    /// Reset statistics for a backend.
    pub fn reset(&self, name: &str) {
        if let Some(entry) = self.stats.get(name) {
            *entry.value().lock() = HealthStats::new();
        }
    }

    /// Get all backend names being monitored.
    pub fn monitored_backends(&self) -> Vec<String> {
        self.stats.iter().map(|entry| entry.key().clone()).collect()
    }
}

impl Default for HealthMonitor {
    fn default() -> Self {
        Self::new()
    }
}
