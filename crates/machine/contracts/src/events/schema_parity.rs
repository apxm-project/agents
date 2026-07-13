//! Terminal-classification drift gate for `apxm.event.v1`
//! (workspace/contracts/schemas/event.v1.json).
//!
//! `workspace/contracts` owns the public schema; this crate owns the typed
//! Rust subset it supports. This table mirrors the matching entries from the
//! schema's `$defs.EventKindRegistryTable.const` (64 entries at the time this
//! test was written). Other registered kinds retain the event system's
//! unknown-event fallback. If `CORE_EVENT_KINDS` changes — a kind added,
//! removed, or given a different `category`/`terminal` — this test fails until
//! this table and the public schema agree. There is deliberately no filesystem
//! read of the sibling `contracts` repo here, since a consumer building only
//! this crate must not need that repo checked out.

use super::kind::{CORE_EVENT_KINDS, EventCategory};

/// `(wire name, category, terminal)` for every `CORE_EVENT_KINDS` entry,
/// mirroring `schemas/event.v1.json`'s `EventKindRegistryTable` exactly.
const SCHEMA_EVENT_KIND_TABLE: &[(&str, &str, bool)] = &[
    ("agent_message", "agent", false),
    ("agent_route_decision", "observability", false),
    ("agent_spawned", "agent", false),
    ("approval_request", "agent", false),
    ("approval_resolved", "agent", false),
    ("capability_effect_receipt", "observability", false),
    ("cancelled", "error", true),
    ("checkpoint_restored", "lifecycle", false),
    ("checkpoint_saved", "lifecycle", false),
    ("citation", "observability", false),
    ("communicate_dispatched", "agent", false),
    ("context_compacted", "observability", false),
    ("context_window_warning", "error", false),
    ("error", "error", true),
    ("execute_complete", "lifecycle", true),
    ("execution_started", "lifecycle", false),
    ("gpu_utilization", "observability", false),
    ("graph_edge", "topology", false),
    ("head_of_line_block", "observability", false),
    ("llm_done", "lifecycle", true),
    ("llm_prompt", "observability", false),
    ("loop_detected", "error", false),
    ("memoization_hit", "observability", false),
    ("memory_read", "observability", false),
    ("memory_write", "observability", false),
    ("model_context_metrics", "observability", false),
    ("model_rerouted", "lifecycle", false),
    ("model_route_decision", "observability", false),
    ("node_metrics", "observability", false),
    ("node_output", "observability", false),
    ("operation_end", "lifecycle", false),
    ("operation_start", "lifecycle", false),
    ("plan_created", "lifecycle", false),
    ("plan_step_completed", "lifecycle", false),
    ("plan_step_started", "lifecycle", false),
    ("plan_workflow_emitted", "lifecycle", false),
    ("provider_event", "observability", false),
    ("retry", "error", false),
    ("scheduler_decision", "observability", false),
    ("session_end", "lifecycle", true),
    ("session_start", "lifecycle", false),
    ("subagent_done", "agent", true),
    ("subagent_failed", "agent", true),
    ("subagent_llm_call_begin", "agent", false),
    ("subagent_llm_call_end", "agent", false),
    ("subagent_spawn_begin", "agent", false),
    ("subagent_spawn_end", "agent", false),
    ("thought", "stream", false),
    ("token", "stream", false),
    ("token_usage", "observability", false),
    ("tool_call", "lifecycle", false),
    ("tool_call_begin", "agent", false),
    ("tool_call_end", "agent", false),
    ("tool_end", "lifecycle", false),
    ("tool_start", "lifecycle", false),
    ("turn_aborted", "lifecycle", true),
    ("turn_boundary", "lifecycle", false),
    ("turn_complete", "lifecycle", true),
    ("turn_started", "lifecycle", false),
    ("usage", "observability", false),
    ("warning", "error", false),
    ("workflow_finished", "lifecycle", false),
    ("workflow_started", "lifecycle", false),
    ("workflow_step_completed", "lifecycle", false),
    ("workflow_step_started", "lifecycle", false),
];

fn category_str(category: EventCategory) -> &'static str {
    match category {
        EventCategory::Stream => "stream",
        EventCategory::Lifecycle => "lifecycle",
        EventCategory::Error => "error",
        EventCategory::Observability => "observability",
        EventCategory::UserAction => "user_action",
        EventCategory::Agent => "agent",
        EventCategory::Topology => "topology",
    }
}

/// Positive/drift gate: every `CORE_EVENT_KINDS` entry has exactly one
/// matching row in `SCHEMA_EVENT_KIND_TABLE` with the same category and
/// terminal value — the same terminal-classification invariant used by the
/// schema-parity test.
#[test]
fn core_event_kinds_match_schema_registry_table() {
    use std::collections::BTreeMap;

    let schema_table: BTreeMap<&str, (&str, bool)> = SCHEMA_EVENT_KIND_TABLE
        .iter()
        .map(|(name, category, terminal)| (*name, (*category, *terminal)))
        .collect();

    assert_eq!(
        schema_table.len(),
        SCHEMA_EVENT_KIND_TABLE.len(),
        "SCHEMA_EVENT_KIND_TABLE must not contain duplicate names"
    );

    let mut rust_names = std::collections::BTreeSet::new();
    for kind in CORE_EVENT_KINDS {
        rust_names.insert(kind.name());
        let Some((expected_category, expected_terminal)) = schema_table.get(kind.name()) else {
            panic!(
                "CORE_EVENT_KINDS has {:?} but SCHEMA_EVENT_KIND_TABLE does \
                 not — update the public schema and this table alongside \
                 kind.rs",
                kind.name()
            );
        };
        assert_eq!(
            category_str(kind.category()),
            *expected_category,
            "kind {:?}: category drifted from schemas/event.v1.json",
            kind.name()
        );
        assert_eq!(
            kind.is_terminal(),
            *expected_terminal,
            "kind {:?}: terminal drifted from schemas/event.v1.json",
            kind.name()
        );
    }

    let schema_names: std::collections::BTreeSet<&str> = schema_table.keys().copied().collect();
    assert_eq!(
        rust_names, schema_names,
        "SCHEMA_EVENT_KIND_TABLE must cover exactly CORE_EVENT_KINDS, no more, \
         no less"
    );
}

/// Ambiguous-terminal sense pin (threat-model coverage): the three kinds
/// documented in schemas/event.v1.json as `terminal_sense: atomic_no_delta`
/// (no `_delta` streaming partner, but does NOT end a run/session/turn)
/// must stay `terminal: false`, and a genuine run-ending kind stays
/// `terminal: true` — regression pin for the exact misclassification the
/// ambiguous terminal classifications.
#[test]
fn ambiguous_terminal_kinds_stay_non_terminal() {
    use super::kind;

    for name in ["agent_spawned", "communicate_dispatched", "graph_edge"] {
        let kind = super::kind::core_event_kind(name)
            .unwrap_or_else(|| panic!("{name} must be a CORE_EVENT_KINDS entry"));
        assert!(
            !kind.is_terminal(),
            "{name} carries terminal_sense=atomic_no_delta in schemas/event.v1.json \
             (no _delta streaming partner) and must stay terminal=false — it does not \
             end a run/session/turn"
        );
    }

    assert!(
        kind::TURN_COMPLETE.is_terminal(),
        "turn_complete is terminal_sense=run_end"
    );
}
