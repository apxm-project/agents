//! Observability and metrics tracking for LLM requests.
//!
//! Provides request tracing, metrics collection, and performance monitoring.

use apxm_core::constants::session::metrics_keys;
use apxm_core::types::TokenUsage;
use dashmap::DashMap;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Metrics for a single request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestMetrics {
    pub backend: String,
    pub model: String,
    pub latency: Duration,
    pub usage: TokenUsage,
    pub success: bool,
    pub retry_count: usize,
    pub timestamp: std::time::SystemTime,
}

impl RequestMetrics {
    /// Create new request metrics.
    pub fn new(
        backend: String,
        model: String,
        latency: Duration,
        usage: TokenUsage,
        success: bool,
        retry_count: usize,
    ) -> Self {
        RequestMetrics {
            backend,
            model,
            latency,
            usage,
            success,
            retry_count,
            timestamp: std::time::SystemTime::now(),
        }
    }
}

/// Aggregated metrics for analysis.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AggregatedMetrics {
    pub total_requests: usize,
    pub successful_requests: usize,
    pub failed_requests: usize,
    pub total_input_tokens: usize,
    pub total_output_tokens: usize,
    pub average_latency: Duration,
    pub p50_latency: Duration,
    pub p99_latency: Duration,
    pub total_retries: usize,
}

impl AggregatedMetrics {
    /// Producer-owned wire-shape projection for `runtime.llm` in the metrics report.
    ///
    /// Flattens `Duration` fields to integer milliseconds and uses the shared
    /// `llm_keys::*` constants so the wire format stays single-sourced.
    pub fn to_metrics_json(&self) -> serde_json::Value {
        use metrics_keys::llm_keys;
        let mut llm = serde_json::Map::new();
        llm.insert(
            llm_keys::TOTAL_REQUESTS.to_owned(),
            self.total_requests.into(),
        );
        llm.insert(
            llm_keys::TOTAL_INPUT_TOKENS.to_owned(),
            self.total_input_tokens.into(),
        );
        llm.insert(
            llm_keys::TOTAL_OUTPUT_TOKENS.to_owned(),
            self.total_output_tokens.into(),
        );
        llm.insert(
            llm_keys::AVG_LATENCY_MS.to_owned(),
            (self.average_latency.as_millis() as u64).into(),
        );
        llm.insert(
            llm_keys::P50_LATENCY_MS.to_owned(),
            (self.p50_latency.as_millis() as u64).into(),
        );
        llm.insert(
            llm_keys::P99_LATENCY_MS.to_owned(),
            (self.p99_latency.as_millis() as u64).into(),
        );
        serde_json::Value::Object(llm)
    }
}

/// Metrics tracker for observability.
#[derive(Clone)]
pub struct MetricsTracker {
    inner: Arc<Mutex<MetricsTrackerInner>>,
}

struct MetricsTrackerInner {
    requests: Vec<RequestMetrics>,
    backend_metrics: DashMap<String, Vec<RequestMetrics>>,
    model_metrics: DashMap<String, Vec<RequestMetrics>>,
}

impl MetricsTracker {
    /// Create a new metrics tracker.
    pub fn new() -> Self {
        MetricsTracker {
            inner: Arc::new(Mutex::new(MetricsTrackerInner {
                requests: Vec::new(),
                backend_metrics: DashMap::new(),
                model_metrics: DashMap::new(),
            })),
        }
    }

    /// Record a request.
    pub fn record(&self, metrics: RequestMetrics) {
        let mut inner = self.inner.lock();

        inner
            .backend_metrics
            .entry(metrics.backend.clone())
            .or_default()
            .push(metrics.clone());

        inner
            .model_metrics
            .entry(metrics.model.clone())
            .or_default()
            .push(metrics.clone());

        inner.requests.push(metrics);
    }

    /// Get aggregated metrics for all requests.
    pub fn aggregate(&self) -> AggregatedMetrics {
        let inner = self.inner.lock();
        Self::compute_aggregated(&inner.requests)
    }

    /// Get aggregated metrics for a specific backend.
    pub fn aggregate_backend(&self, backend: &str) -> Option<AggregatedMetrics> {
        let inner = self.inner.lock();
        inner
            .backend_metrics
            .get(backend)
            .map(|metrics| Self::compute_aggregated(metrics.value()))
    }

    /// Get aggregated metrics for a specific model.
    pub fn aggregate_model(&self, model: &str) -> Option<AggregatedMetrics> {
        let inner = self.inner.lock();
        inner
            .model_metrics
            .get(model)
            .map(|metrics| Self::compute_aggregated(metrics.value()))
    }

    /// Compute aggregated metrics from a list of requests.
    fn compute_aggregated(requests: &[RequestMetrics]) -> AggregatedMetrics {
        if requests.is_empty() {
            return AggregatedMetrics::default();
        }

        let total_requests = requests.len();
        let successful_requests = requests.iter().filter(|r| r.success).count();
        let failed_requests = total_requests - successful_requests;

        let total_input_tokens: usize = requests.iter().map(|r| r.usage.input_tokens).sum();
        let total_output_tokens: usize = requests.iter().map(|r| r.usage.output_tokens).sum();

        let total_latency: Duration = requests.iter().map(|r| r.latency).sum();
        let average_latency = total_latency / total_requests as u32;

        let total_retries: usize = requests.iter().map(|r| r.retry_count).sum();

        // Calculate percentiles
        let mut latencies: Vec<Duration> = requests.iter().map(|r| r.latency).collect();
        latencies.sort();

        let p50_index = (total_requests as f64 * 0.5) as usize;
        let p99_index = (total_requests as f64 * 0.99) as usize;

        let p50_latency = latencies.get(p50_index).copied().unwrap_or_default();
        let p99_latency = latencies.get(p99_index).copied().unwrap_or_default();

        AggregatedMetrics {
            total_requests,
            successful_requests,
            failed_requests,
            total_input_tokens,
            total_output_tokens,
            average_latency,
            p50_latency,
            p99_latency,
            total_retries,
        }
    }

    /// Aggregate metrics per backend name.
    pub fn aggregate_per_backend(&self) -> HashMap<String, AggregatedMetrics> {
        let inner = self.inner.lock();
        let mut result = HashMap::new();
        for entry in &inner.backend_metrics {
            result.insert(entry.key().clone(), Self::compute_aggregated(entry.value()));
        }
        result
    }

    /// Get success rate as a percentage.
    pub fn success_rate(&self) -> f64 {
        let metrics = self.aggregate();
        if metrics.total_requests == 0 {
            return 0.0;
        }
        (metrics.successful_requests as f64 / metrics.total_requests as f64) * 100.0
    }

    /// Reset all metrics.
    pub fn reset(&self) {
        let mut inner = self.inner.lock();
        inner.requests.clear();
        inner.backend_metrics.clear();
        inner.model_metrics.clear();
    }
}

impl Default for MetricsTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Request tracer for debugging and logging.
pub struct RequestTracer {
    start: Instant,
    backend: String,
    model: String,
    retry_count: usize,
}

impl RequestTracer {
    /// Start tracing a new request.
    pub fn start(backend: String, model: String) -> Self {
        RequestTracer {
            start: Instant::now(),
            backend,
            model,
            retry_count: 0,
        }
    }

    /// Record a retry attempt.
    pub fn record_retry(&mut self) {
        self.retry_count += 1;
    }

    /// Finish tracing and return metrics.
    pub fn finish(self, usage: TokenUsage, success: bool) -> RequestMetrics {
        let latency = self.start.elapsed();
        RequestMetrics::new(
            self.backend,
            self.model,
            latency,
            usage,
            success,
            self.retry_count,
        )
    }
}
