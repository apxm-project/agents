//! `DispatchIrV1` — the first internal version of APXM's compiled-graph
//! intent contract for graph-aware inference backends.
//!
//! This module is internal to `apxm-runtime`. It is not re-exported from the
//! crate root and is not part of any public ABI. Promotion to a public
//! contract is gated on benchmark proof and a serious second backend per
//! `.apxm/docs/design/dispatch-ir.md`.

mod lower;
mod plan;

#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashMap};

use apxm_core::types::{BackendGraphCapabilities, GraphStatusSnapshot, PinMode};

#[allow(unused_imports)]
pub(crate) use lower::{derive_apxm_hints, lower_graph};
#[allow(unused_imports)]
pub(crate) use plan::{
    BackendCapabilityRequirements, DISPATCH_IR_V1_SCHEMA_VERSION, DispatchIrV1, GraphDispatchPlan,
    NodeDispatchPlan, TelemetryContract,
};

pub(crate) fn dispatch_ir_accounting_json(
    ir: Option<&DispatchIrV1>,
    backend_capabilities: &HashMap<String, BackendGraphCapabilities>,
    graph_status_snapshots: &[GraphStatusSnapshot],
) -> serde_json::Value {
    let Some(ir) = ir else {
        return serde_json::Value::Null;
    };

    let fields_sent = dispatch_fields_sent(ir);
    let unsupported_by_backend = backend_capabilities
        .iter()
        .map(|(backend, capabilities)| {
            let unsupported =
                capabilities.unsupported_dispatch_fields(fields_sent.iter().map(String::as_str));
            (backend.clone(), unsupported)
        })
        .collect::<HashMap<_, _>>();

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

    serde_json::json!({
        "schema_version": ir.schema_version,
        "graph_id": ir.graph.graph_id,
        "fields_sent": fields_sent,
        "fields_unsupported_by_backend": unsupported_by_backend,
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

fn dispatch_fields_sent(ir: &DispatchIrV1) -> Vec<String> {
    let mut fields = BTreeSet::new();
    fields.insert("dispatch_ir_v1_internal");
    fields.insert("graph_registration");
    fields.insert("request_hints");

    for node in &ir.nodes {
        if node.priority_class.is_some() {
            fields.insert("priority");
        }
        if node.prefix_cohort.is_some() || node.reuse_group.is_some() {
            fields.insert("prefix_cohorts");
        }
        if node.pin_policy.mode == PinMode::Prefix {
            fields.insert("pin_release");
            fields.insert("backend_cache_state");
        }
    }

    fields.into_iter().map(ToOwned::to_owned).collect()
}
