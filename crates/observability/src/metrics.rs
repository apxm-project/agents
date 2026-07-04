//! OBS-4 decision-5 metric set: OTLP metrics export shared by every service.
//!
//! Mirrors the trace-exporter pattern in [`crate::init`] — one OTLP endpoint
//! configures both traces and metrics, `init` degrades to "no export" (not
//! panic) on failure, and callers get back a typed handle instead of raw
//! `Meter` calls so labels can't drift into ad-hoc strings.
//!
//! The metric set (decision 5, `docs/plans/observability-and-artifacts.md`):
//! - per-service RED: `apxm.requests.total`, `apxm.requests.errors`,
//!   `apxm.request.duration`
//! - domain counters: runs started/completed/failed, op dispatches,
//!   capability invocations by [`apxm_core::types::capability::PermissionDecisionKind`]
//! - journal depth + dead-letter count (gauges)
//! - token usage (RT-8 feed)
//!
//! Quota consumption (ORG) is out of scope here — deferred, no ORG quota
//! tracker was found with confidence at OBS-4 implementation time.

use apxm_core::types::capability::PermissionDecisionKind;
use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};

/// Decision-5 metric set, built once per process from the [`Meter`] returned
/// by [`crate::init`]. Every method takes typed inputs (CM-3 enums, plain
/// numbers) so call sites cannot construct label strings ad hoc.
#[derive(Debug, Clone)]
pub struct AppMetrics {
    requests_total: Counter<u64>,
    request_errors_total: Counter<u64>,
    request_duration_seconds: Histogram<f64>,

    runs_started_total: Counter<u64>,
    runs_completed_total: Counter<u64>,
    runs_failed_total: Counter<u64>,

    op_dispatches_total: Counter<u64>,
    capability_invocations_total: Counter<u64>,

    journal_depth: Gauge<u64>,
    dead_letter_count: Gauge<u64>,

    token_usage_total: Counter<u64>,
}

impl AppMetrics {
    /// Build the full decision-5 instrument set from a [`Meter`]. Cheap:
    /// instrument creation is a one-time cost at process startup.
    pub fn new(meter: &Meter) -> Self {
        Self {
            requests_total: meter
                .u64_counter("apxm.requests.total")
                .with_description("Total requests handled by this service's entry points (RED: rate).")
                .build(),
            request_errors_total: meter
                .u64_counter("apxm.requests.errors")
                .with_description("Total requests that ended in an error (RED: errors).")
                .build(),
            request_duration_seconds: meter
                .f64_histogram("apxm.request.duration")
                .with_description("Request duration in seconds (RED: duration).")
                .with_unit("s")
                .build(),
            runs_started_total: meter
                .u64_counter("apxm.runs.started")
                .with_description("Total execution runs started.")
                .build(),
            runs_completed_total: meter
                .u64_counter("apxm.runs.completed")
                .with_description("Total execution runs completed successfully.")
                .build(),
            runs_failed_total: meter
                .u64_counter("apxm.runs.failed")
                .with_description("Total execution runs that failed.")
                .build(),
            op_dispatches_total: meter
                .u64_counter("apxm.ops.dispatched")
                .with_description("Total operation dispatches (mirrors RT-9 in-process op-usage stat).")
                .build(),
            capability_invocations_total: meter
                .u64_counter("apxm.capability.invocations")
                .with_description("Capability invocations labeled by permission decision (CM-3 PermissionDecisionKind).")
                .build(),
            journal_depth: meter
                .u64_gauge("apxm.journal.depth")
                .with_description("Current journal depth (pending entries).")
                .build(),
            dead_letter_count: meter
                .u64_gauge("apxm.journal.dead_letter_count")
                .with_description("Current dead-letter queue size.")
                .build(),
            token_usage_total: meter
                .u64_counter("apxm.tokens.usage")
                .with_description("Token usage (RT-8 token accounting fed into OTLP).")
                .build(),
        }
    }

    /// Record one request against a named route/entry point. `route` should
    /// be a low-cardinality, static-ish label (e.g. the axum route pattern,
    /// not the raw path with IDs interpolated) — takes `&str` rather than
    /// `&'static str` so callers can pass a matched-route string without
    /// leaking memory per request.
    pub fn record_request(&self, route: &str, method: &'static str, is_error: bool, duration_seconds: f64) {
        let attrs = [
            KeyValue::new("route", route.to_string()),
            KeyValue::new("method", method),
        ];
        self.requests_total.add(1, &attrs);
        self.request_duration_seconds.record(duration_seconds, &attrs);
        if is_error {
            self.request_errors_total.add(1, &attrs);
        }
    }

    pub fn record_run_started(&self) {
        self.runs_started_total.add(1, &[]);
    }

    pub fn record_run_completed(&self) {
        self.runs_completed_total.add(1, &[]);
    }

    pub fn record_run_failed(&self) {
        self.runs_failed_total.add(1, &[]);
    }

    /// Record `count` operation dispatches, optionally labeled by op kind
    /// (e.g. the node/operation name). Pass `None` to keep the label unset
    /// for high-cardinality dispatch sites.
    pub fn record_op_dispatches(&self, count: u64, op_kind: Option<&'static str>) {
        match op_kind {
            Some(kind) => self
                .op_dispatches_total
                .add(count, &[KeyValue::new("op_kind", kind)]),
            None => self.op_dispatches_total.add(count, &[]),
        }
    }

    /// Record a capability invocation labeled by its permission decision.
    /// The label value is always `PermissionDecisionKind::as_str()` — CM-3
    /// enums are the only source of this label, per OBS-4 decision-5.
    pub fn record_capability_invocation(&self, decision: PermissionDecisionKind) {
        self.capability_invocations_total
            .add(1, &[KeyValue::new("decision", decision.as_str())]);
    }

    /// Same as [`Self::record_capability_invocation`] but additionally
    /// labels the capability name (low-to-medium cardinality; keep capability
    /// counts bounded per deployment).
    pub fn record_capability_invocation_named(
        &self,
        decision: PermissionDecisionKind,
        capability: &str,
    ) {
        self.capability_invocations_total.add(
            1,
            &[
                KeyValue::new("decision", decision.as_str()),
                KeyValue::new("capability", capability.to_string()),
            ],
        );
    }

    pub fn set_journal_depth(&self, depth: u64) {
        self.journal_depth.record(depth, &[]);
    }

    pub fn set_dead_letter_count(&self, count: u64) {
        self.dead_letter_count.record(count, &[]);
    }

    /// Feed RT-8 token accounting into OTLP. `token_kind` should be
    /// `"input"`, `"output"`, `"cached_input"`, or `"reasoning_output"` —
    /// these are wire-stable names from
    /// `apxm_runtime::executor::token_accounting::TokenUsageSummary`, not a
    /// CM-3 enum (there isn't one for token kind), so a `&'static str`
    /// constant is the correct label source here.
    pub fn record_token_usage(&self, count: u64, token_kind: &'static str) {
        self.token_usage_total
            .add(count, &[KeyValue::new("token_kind", token_kind)]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::capability::{
        ApprovalPosture, OperationClass, PermissionScopeKind, RiskLevel,
    };
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::metrics::data::Sum;
    use opentelemetry_sdk::testing::metrics::InMemoryMetricExporter;

    fn test_metrics() -> (AppMetrics, InMemoryMetricExporter, SdkMeterProvider) {
        let exporter = InMemoryMetricExporter::default();
        let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(
            exporter.clone(),
            opentelemetry_sdk::runtime::Tokio,
        )
        .build();
        let provider = SdkMeterProvider::builder().with_reader(reader).build();
        let meter = provider.meter("apxm-observability-tests");
        (AppMetrics::new(&meter), exporter, provider)
    }

    fn find_sum_datapoint_labels(
        exporter: &InMemoryMetricExporter,
        metric_name: &str,
    ) -> Vec<Vec<(String, String)>> {
        let mut out = Vec::new();
        for resource_metrics in exporter.get_finished_metrics().unwrap() {
            for scope_metrics in &resource_metrics.scope_metrics {
                for metric in &scope_metrics.metrics {
                    if metric.name != metric_name {
                        continue;
                    }
                    if let Some(sum) = metric.data.as_any().downcast_ref::<Sum<u64>>() {
                        for dp in &sum.data_points {
                            let mut labels: Vec<(String, String)> = dp
                                .attributes
                                .iter()
                                .map(|kv| (kv.key.to_string(), kv.value.to_string()))
                                .collect();
                            labels.sort();
                            out.push(labels);
                        }
                    }
                }
            }
        }
        out
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capability_invocation_labels_use_cm3_enum_values() {
        let (metrics, exporter, provider) = test_metrics();

        metrics.record_capability_invocation(PermissionDecisionKind::Allow);
        metrics.record_capability_invocation(PermissionDecisionKind::RequireApproval);
        metrics.record_capability_invocation(PermissionDecisionKind::Deny);

        provider.force_flush().unwrap();

        let datapoints = find_sum_datapoint_labels(&exporter, "apxm.capability.invocations");
        assert_eq!(datapoints.len(), 3, "expected one data point per decision label");

        let has_label = |value: &str| {
            datapoints
                .iter()
                .any(|labels| labels.contains(&("decision".to_string(), value.to_string())))
        };
        assert!(has_label("allow"), "missing decision=allow data point");
        assert!(
            has_label("require_approval"),
            "missing decision=require_approval data point"
        );
        assert!(has_label("deny"), "missing decision=deny data point");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn request_error_increments_errors_total_with_route_label() {
        let (metrics, exporter, provider) = test_metrics();

        metrics.record_request("/v1/generate", "POST", true, 0.042);

        provider.force_flush().unwrap();

        let requests = find_sum_datapoint_labels(&exporter, "apxm.requests.total");
        let errors = find_sum_datapoint_labels(&exporter, "apxm.requests.errors");
        assert_eq!(requests.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains(&("route".to_string(), "/v1/generate".to_string()))
        );
        assert!(errors[0].contains(&("method".to_string(), "POST".to_string())));
    }

    /// Round-trip every `PermissionDecisionKind` variant through `as_str()`
    /// — catches a stringly-typed regression if a variant's label drifts
    /// from its serde wire value.
    #[test]
    fn permission_decision_kind_as_str_matches_wire_values() {
        assert_eq!(PermissionDecisionKind::Allow.as_str(), "allow");
        assert_eq!(PermissionDecisionKind::Deny.as_str(), "deny");
        assert_eq!(
            PermissionDecisionKind::RequireApproval.as_str(),
            "require_approval"
        );
    }

    #[test]
    fn cm3_enum_as_str_matches_serde_wire_values() {
        for (value, expected) in [
            (OperationClass::Read.as_str(), "read"),
            (OperationClass::Write.as_str(), "write"),
            (OperationClass::Destructive.as_str(), "destructive"),
        ] {
            assert_eq!(value, expected);
        }
        for (value, expected) in [
            (RiskLevel::Low.as_str(), "low"),
            (RiskLevel::Medium.as_str(), "medium"),
            (RiskLevel::High.as_str(), "high"),
            (RiskLevel::Critical.as_str(), "critical"),
        ] {
            assert_eq!(value, expected);
        }
        for (value, expected) in [
            (ApprovalPosture::Auto.as_str(), "auto"),
            (ApprovalPosture::Confirm.as_str(), "confirm"),
            (ApprovalPosture::DualControl.as_str(), "dual_control"),
            (
                ApprovalPosture::ExternalSignoff.as_str(),
                "external_signoff",
            ),
        ] {
            assert_eq!(value, expected);
        }
        for (value, expected) in [
            (PermissionScopeKind::Session.as_str(), "session"),
            (PermissionScopeKind::Workspace.as_str(), "workspace"),
            (PermissionScopeKind::Org.as_str(), "org"),
            (PermissionScopeKind::Global.as_str(), "global"),
        ] {
            assert_eq!(value, expected);
        }
    }
}
