//! Terminal-classification drift gate for `apxm.event.v1`
//! (workspace/contracts/schemas/event.v1.json).
//!
//! `workspace/contracts` owns the public schema; its generator writes the
//! checked-in registry consumed here. A crate-only build therefore validates
//! schema parity without reading a sibling checkout or duplicating metadata.

use super::generated_event_kind_registry::SCHEMA_EVENT_KIND_REGISTRY;
use super::kind::{CORE_EVENT_KINDS, EventCategory};

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
/// matching generated registry row with the same category and
/// terminal value — the same terminal-classification invariant used by the
/// schema-parity test.
#[test]
fn core_event_kinds_match_schema_registry_table() {
    use std::collections::BTreeMap;

    let schema_table: BTreeMap<&str, (&str, bool)> = SCHEMA_EVENT_KIND_REGISTRY
        .iter()
        .map(|entry| (entry.name, (entry.category.as_str(), entry.terminal)))
        .collect();

    assert_eq!(
        schema_table.len(),
        SCHEMA_EVENT_KIND_REGISTRY.len(),
        "generated schema registry must not contain duplicate names"
    );

    let mut rust_names = std::collections::BTreeSet::new();
    for kind in CORE_EVENT_KINDS {
        rust_names.insert(kind.name());
        let Some((expected_category, expected_terminal)) = schema_table.get(kind.name()) else {
            panic!(
                "CORE_EVENT_KINDS has {:?} but the generated schema registry does not",
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
        "the generated schema registry must cover exactly CORE_EVENT_KINDS"
    );
}

/// Ambiguous-terminal sense pin (threat-model coverage): the three kinds
/// documented in schemas/event.v1.json as `terminal_sense: atomic_no_delta`
/// (no `_delta` streaming partner, but does NOT end a run/session)
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
             end a run or session"
        );
    }

    assert!(
        kind::EXECUTE_COMPLETE.is_terminal(),
        "execute_complete is terminal_sense=run_end"
    );
}
