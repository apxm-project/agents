//! `DispatchIrV1` — the first internal version of APXM's compiled-graph
//! intent contract for graph-aware inference backends.
//!
//! This module is internal to `apxm-runtime`. It is not re-exported from the
//! crate root and is not part of any public ABI. Promotion to a public
//! contract is gated on benchmark proof and a serious second backend per
//! `.apxm/docs/design/dispatch-ir.md`.

mod lower;
mod plan;


use std::collections::{BTreeSet, HashMap};

use apxm_core::constants::llm::apxm::dispatch_fields as df;
use apxm_core::types::{BackendGraphCapabilities, GraphStatusSnapshot, PinMode};

#[allow(unused_imports)]
pub(crate) use lower::{derive_apxm_hints, lower_graph};
#[allow(unused_imports)]
pub(crate) use plan::{
    BackendCapabilityRequirements, DISPATCH_IR_V1_SCHEMA_VERSION, DispatchIrV1, GraphDispatchPlan,
    NodeDispatchPlan, TelemetryContract,
};

/// One record per backend that the runtime gated OUT of graph-aware
/// dispatch because its static capability table is missing one or more
/// `BackendCapabilityRequirements.required` fields.
///
/// The runtime records this in `dispatch_ir_metrics.fallbacks` so a
/// claim cannot quietly inherit "we sent X" semantics from a backend
/// that never received X. Runtime-time capability gating keeps this
/// as an explicit documented fallback path.
#[derive(Debug, Clone)]
pub(crate) struct DispatchFallback {
    pub backend_name: String,
    pub missing_required: Vec<String>,
    pub reason: String,
}

impl DispatchFallback {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "backend": self.backend_name,
            "missing_required": self.missing_required,
            "reason": self.reason,
        })
    }
}

/// Decide whether `backend_caps` meets the IR's `required` capabilities.
/// Returns `Some(DispatchFallback)` if it does not — the caller should
/// skip `register_graph` and flat-HTTP-dispatch for that backend.
pub(crate) fn evaluate_required_capabilities(
    backend_name: &str,
    backend_caps: &BackendGraphCapabilities,
    ir: &DispatchIrV1,
) -> Option<DispatchFallback> {
    let missing = backend_caps
        .unsupported_dispatch_fields(ir.requirements.required.iter().map(String::as_str));
    if missing.is_empty() {
        return None;
    }
    let reason = format!(
        "backend {} declares it cannot honor required dispatch \
         field(s) {}; runtime is falling back to flat-HTTP dispatch \
         (no graph registration, no vllm_xargs.apxm injection) for this \
         backend; this is the documented degraded path, \
         not a silent skip",
        backend_name,
        missing.join(", "),
    );
    Some(DispatchFallback {
        backend_name: backend_name.to_owned(),
        missing_required: missing,
        reason,
    })
}

pub(crate) fn dispatch_ir_accounting_json(
    ir: Option<&DispatchIrV1>,
    backend_capabilities: &HashMap<String, BackendGraphCapabilities>,
    graph_status_snapshots: &[GraphStatusSnapshot],
    fallbacks: &[DispatchFallback],
    fields_honored_by_backend: &HashMap<String, Vec<String>>,
) -> serde_json::Value {
    let Some(ir) = ir else {
        return serde_json::Value::Null;
    };

    let fields_sent = dispatch_fields_sent(ir);
    // Single pass over `backend_capabilities` produces both partitions
    // of `fields_sent` for every backend. The two maps together cover
    // every (backend, field) cell — `unsupported` and
    // `capability_supported` are inverses by construction. Distinct
    // from `fields_honored`, which is the per-request runtime-evidence
    // signal sourced from the x-apxm-fields-honored response header
    // and is therefore not derivable from the static
    // capability table at all.
    let mut unsupported_by_backend: HashMap<String, Vec<String>> =
        HashMap::with_capacity(backend_capabilities.len());
    let mut capability_supported_by_backend: HashMap<String, Vec<String>> =
        HashMap::with_capacity(backend_capabilities.len());
    for (backend, capabilities) in backend_capabilities {
        unsupported_by_backend.insert(
            backend.clone(),
            capabilities.unsupported_dispatch_fields(fields_sent.iter().map(String::as_str)),
        );
        capability_supported_by_backend.insert(
            backend.clone(),
            capabilities
                .dispatch_fields_capability_supported(fields_sent.iter().map(String::as_str)),
        );
    }

    let priority_backends = backend_capabilities
        .iter()
        .filter_map(|(backend, capabilities)| {
            capabilities
                .supports_priority
                .then(|| serde_json::Value::String(backend.clone()))
        })
        .collect::<Vec<_>>();

    let pinned_blocks_peak_max = graph_status_snapshots
        .iter()
        .map(|status| status.pinned_blocks_peak.max(status.pinned_blocks))
        .max()
        .unwrap_or_default();

    let fallback_triggered = !fallbacks.is_empty();
    let fallback_records: Vec<serde_json::Value> =
        fallbacks.iter().map(DispatchFallback::to_json).collect();

    serde_json::json!({
        "schema_version": ir.schema_version,
        "graph_id": ir.graph.graph_id,
        "fields_sent": fields_sent,
        "fields_unsupported_by_backend": unsupported_by_backend,
        "fields_capability_supported_by_backend": capability_supported_by_backend,
        // Per-request runtime honor evidence union'd by backend.
        // Populated by the LLM handler from each per-node response's
        // `metadata["fields_honored"]`, which the OpenAI backend parses
        // from the vLLM fork's `x-apxm-fields-honored` response header.
        // Distinct from `fields_capability_supported_by_backend`, which
        // is a static-capability statement, not runtime evidence.
        "fields_honored": fields_honored_by_backend,
        // Fields APXM declares it sends but the v1 vLLM scheduler does
        // not act on. They are present in `fields_sent`
        // only to keep the wire-payload round-trippable; they MUST NOT
        // be counted toward "the backend honored APXM hints" claims.
        "fields_passthrough_only": PASSTHROUGH_ONLY_FIELDS,
        "fallback_triggered": fallback_triggered,
        "fallbacks": fallback_records,
        "evidence": {
            "priority_capable_backends": priority_backends,
            "graph_status_count": graph_status_snapshots.len(),
            "registered_graph_status_count": graph_status_snapshots
                .iter()
                .filter(|status| status.registered)
                .count(),
            "pinned_blocks_peak_max": pinned_blocks_peak_max,
        }
    })
}

/// Fields APXM sends in `vllm_xargs.apxm.graph_metrics` that the v1
/// vLLM scheduler does not consume in scheduling decisions. These are
/// tagged passthrough so the `fields_sent` /
/// `fields_honored` partition stays honest. Wire one of them into a
/// scheduling decision on the fork to remove it from this list.
pub(crate) const PASSTHROUGH_ONLY_FIELDS: &[&str] = &[
    "latency_class",
    "batch_group",
    "fanout_count",
    "remaining_path_len",
    "stage_index",
    "estimated_dynamic_tokens",
];

fn dispatch_fields_sent(ir: &DispatchIrV1) -> Vec<String> {
    let mut fields = BTreeSet::new();
    fields.insert(df::DISPATCH_IR_V1_INTERNAL);
    fields.insert(df::GRAPH_REGISTRATION);
    fields.insert(df::REQUEST_HINTS);

    for node in &ir.nodes {
        if node.priority_class.is_some() {
            fields.insert(df::PRIORITY);
        }
        if node.prefix_cohort.is_some() || node.reuse_group.is_some() {
            fields.insert(df::PREFIX_COHORTS);
        }
        if node.pin_policy.mode == PinMode::Prefix {
            fields.insert(df::PIN_RELEASE);
            fields.insert(df::BACKEND_CACHE_STATE);
        }
    }

    fields.into_iter().map(ToOwned::to_owned).collect()
}
