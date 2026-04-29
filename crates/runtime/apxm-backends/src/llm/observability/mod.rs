//! Observability and metrics tracking for LLM requests.
//!
//! Provides request tracing, metrics collection, and performance monitoring.

use apxm_core::constants::session::metrics_keys;
use apxm_core::metrics::MetricsSource;
use apxm_core::types::{GraphStatusSnapshot, TokenUsage};
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

        // Add to per-backend metrics
        inner
            .backend_metrics
            .entry(metrics.backend.clone())
            .or_default()
            .push(metrics.clone());

        // Add to per-model metrics
        inner
            .model_metrics
            .entry(metrics.model.clone())
            .or_default()
            .push(metrics.clone());

        // Add to global metrics
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
        for entry in inner.backend_metrics.iter() {
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

/// Collected backend metrics for the unified metrics report.
pub struct BackendMetricsSource {
    pub aggregate: AggregatedMetrics,
    pub per_backend: HashMap<String, AggregatedMetrics>,
    pub graph_status_snapshots: Vec<GraphStatusSnapshot>,
}

impl MetricsSource for BackendMetricsSource {
    fn section_name(&self) -> &'static str {
        metrics_keys::SECTION_BACKENDS
    }

    fn collect(&self) -> serde_json::Value {
        if self.aggregate.total_requests == 0
            && self.per_backend.is_empty()
            && self.graph_status_snapshots.is_empty()
        {
            return serde_json::Value::Null;
        }

        let mut map = serde_json::Map::new();
        map.insert(
            metrics_keys::BACKENDS_AGGREGATE.to_owned(),
            serde_json::to_value(&self.aggregate).unwrap_or_default(),
        );
        map.insert(
            metrics_keys::BACKENDS_PER_BACKEND.to_owned(),
            serde_json::to_value(&self.per_backend).unwrap_or_default(),
        );
        let graph_statuses: Vec<_> = self
            .graph_status_snapshots
            .iter()
            .map(GraphStatusSnapshot::to_metrics_json)
            .collect();
        if !graph_statuses.is_empty() {
            map.insert(
                metrics_keys::BACKENDS_GRAPHS.to_owned(),
                serde_json::Value::Array(graph_statuses),
            );
        }
        serde_json::Value::Object(map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_tracker() {
        let tracker = MetricsTracker::new();

        let metrics1 = RequestMetrics::new(
            "openai".to_string(),
            "gpt-4".to_string(),
            Duration::from_millis(500),
            TokenUsage::new(100, 50),
            true,
            0,
        );

        let metrics2 = RequestMetrics::new(
            "openai".to_string(),
            "gpt-4".to_string(),
            Duration::from_millis(300),
            TokenUsage::new(200, 75),
            true,
            1,
        );

        tracker.record(metrics1);
        tracker.record(metrics2);

        let aggregated = tracker.aggregate();
        assert_eq!(aggregated.total_requests, 2);
        assert_eq!(aggregated.successful_requests, 2);
        assert_eq!(aggregated.total_input_tokens, 300);
        assert_eq!(aggregated.total_output_tokens, 125);
        assert_eq!(aggregated.total_retries, 1);
    }

    #[test]
    fn test_backend_specific_metrics() -> Result<(), Box<dyn std::error::Error>> {
        let tracker = MetricsTracker::new();

        let openai_metrics = RequestMetrics::new(
            "openai".to_string(),
            "gpt-4".to_string(),
            Duration::from_millis(500),
            TokenUsage::new(100, 50),
            true,
            0,
        );

        let anthropic_metrics = RequestMetrics::new(
            "anthropic".to_string(),
            "claude-3".to_string(),
            Duration::from_millis(600),
            TokenUsage::new(150, 60),
            true,
            0,
        );

        tracker.record(openai_metrics);
        tracker.record(anthropic_metrics);

        let openai_agg = match tracker.aggregate_backend("openai") {
            Some(agg) => agg,
            None => return Err("expected aggregated metrics for 'openai'".into()),
        };
        assert_eq!(openai_agg.total_requests, 1);
        assert_eq!(openai_agg.total_input_tokens, 100);

        let anthropic_agg = match tracker.aggregate_backend("anthropic") {
            Some(agg) => agg,
            None => return Err("expected aggregated metrics for 'anthropic'".into()),
        };
        assert_eq!(anthropic_agg.total_requests, 1);
        assert_eq!(anthropic_agg.total_input_tokens, 150);

        Ok(())
    }

    #[test]
    fn test_success_rate() {
        let tracker = MetricsTracker::new();

        tracker.record(RequestMetrics::new(
            "test".to_string(),
            "model".to_string(),
            Duration::from_millis(100),
            TokenUsage::new(10, 10),
            true,
            0,
        ));

        tracker.record(RequestMetrics::new(
            "test".to_string(),
            "model".to_string(),
            Duration::from_millis(100),
            TokenUsage::new(10, 10),
            false,
            0,
        ));

        assert!((tracker.success_rate() - 50.0).abs() < 0.1);
    }

    #[test]
    fn test_request_tracer() {
        let mut tracer = RequestTracer::start("test".to_string(), "model".to_string());
        tracer.record_retry();
        tracer.record_retry();

        let metrics = tracer.finish(TokenUsage::new(100, 50), true);

        assert_eq!(metrics.backend, "test");
        assert_eq!(metrics.model, "model");
        assert_eq!(metrics.retry_count, 2);
        assert!(metrics.success);
    }

    #[test]
    fn test_aggregate_per_backend() {
        let tracker = MetricsTracker::new();

        tracker.record(RequestMetrics::new(
            "openai".to_string(),
            "gpt-4".to_string(),
            Duration::from_millis(500),
            TokenUsage::new(100, 50),
            true,
            0,
        ));
        tracker.record(RequestMetrics::new(
            "anthropic".to_string(),
            "claude-3".to_string(),
            Duration::from_millis(600),
            TokenUsage::new(150, 60),
            true,
            0,
        ));

        let per_backend = tracker.aggregate_per_backend();
        assert_eq!(per_backend.len(), 2);
        assert_eq!(per_backend["openai"].total_requests, 1);
        assert_eq!(per_backend["anthropic"].total_requests, 1);
    }

    #[test]
    fn backend_metrics_source_null_when_empty() {
        let source = BackendMetricsSource {
            aggregate: AggregatedMetrics::default(),
            per_backend: HashMap::new(),
            graph_status_snapshots: vec![],
        };
        assert!(source.collect().is_null());
    }

    #[test]
    fn backend_metrics_source_emits_all_keys_when_populated() {
        let mut per_backend = HashMap::new();
        per_backend.insert(
            "vllm-fork".to_string(),
            AggregatedMetrics {
                total_requests: 3,
                ..Default::default()
            },
        );

        let source = BackendMetricsSource {
            aggregate: AggregatedMetrics {
                total_requests: 3,
                ..Default::default()
            },
            per_backend,
            graph_status_snapshots: vec![
                GraphStatusSnapshot::graph_aware("g1").with_pin_counts(0, 12),
            ],
        };

        let val = source.collect();
        let obj = val.as_object().expect("collect must return an object");

        assert!(obj.contains_key(metrics_keys::BACKENDS_AGGREGATE));
        assert!(obj.contains_key(metrics_keys::BACKENDS_PER_BACKEND));
        assert!(obj.contains_key(metrics_keys::BACKENDS_GRAPHS));

        let graphs = obj[metrics_keys::BACKENDS_GRAPHS].as_array().unwrap();
        assert_eq!(graphs.len(), 1);
    }

    #[test]
    fn backend_metrics_source_omits_graphs_when_no_graph_snapshots() {
        let source = BackendMetricsSource {
            aggregate: AggregatedMetrics {
                total_requests: 1,
                ..Default::default()
            },
            per_backend: HashMap::new(),
            graph_status_snapshots: vec![],
        };

        let val = source.collect();
        let obj = val.as_object().expect("collect must return an object");

        assert!(obj.contains_key(metrics_keys::BACKENDS_AGGREGATE));
        assert!(!obj.contains_key(metrics_keys::BACKENDS_GRAPHS));
    }

    #[test]
    fn backend_metrics_source_section_name() {
        let source = BackendMetricsSource {
            aggregate: AggregatedMetrics::default(),
            per_backend: HashMap::new(),
            graph_status_snapshots: vec![],
        };
        assert_eq!(source.section_name(), metrics_keys::SECTION_BACKENDS);
    }
}
