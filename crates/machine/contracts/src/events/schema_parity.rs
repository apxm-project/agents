//! Generated terminal-classification drift gate for `apxm.event.v1`.
//!
//! `workspace/contracts` owns the public registry. Its code generator writes
//! `generated_event_kind_registry.rs`, so this crate can verify schema parity
//! without reading a sibling checkout or maintaining a second literal table.

use super::generated_event_kind_registry::{SCHEMA_EVENT_KIND_REGISTRY, SchemaTerminalSense};
use super::kind::{CORE_EVENT_KINDS, EventCategory, core_event_kind};

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

#[test]
fn core_event_kinds_match_generated_schema_registry() {
    use std::collections::{BTreeMap, BTreeSet};

    let schema_table: BTreeMap<_, _> = SCHEMA_EVENT_KIND_REGISTRY
        .iter()
        .map(|entry| (entry.name, entry))
        .collect();
    assert_eq!(
        schema_table.len(),
        SCHEMA_EVENT_KIND_REGISTRY.len(),
        "generated schema event registry must not contain duplicate names"
    );

    let mut rust_names = BTreeSet::new();
    for kind in CORE_EVENT_KINDS {
        rust_names.insert(kind.name());
        let entry = schema_table.get(kind.name()).unwrap_or_else(|| {
            panic!(
                "CORE_EVENT_KINDS has {:?} but the generated apxm.event.v1 registry does not",
                kind.name()
            )
        });
        assert_eq!(
            category_str(kind.category()),
            entry.category.as_str(),
            "kind {:?}: category drifted from apxm.event.v1",
            kind.name()
        );
        assert_eq!(
            kind.is_terminal(),
            entry.terminal,
            "kind {:?}: terminal flag drifted from apxm.event.v1",
            kind.name()
        );
    }

    let schema_names: BTreeSet<_> = schema_table.keys().copied().collect();
    assert_eq!(
        rust_names, schema_names,
        "apxm.event.v1 must cover exactly CORE_EVENT_KINDS"
    );
}

#[test]
fn generated_terminal_senses_preserve_live_feed_semantics() {
    for entry in SCHEMA_EVENT_KIND_REGISTRY {
        let kind = core_event_kind(entry.name)
            .unwrap_or_else(|| panic!("generated event kind {} must exist", entry.name));
        match entry.terminal_sense {
            SchemaTerminalSense::RunEnd => assert!(
                entry.terminal && kind.is_terminal(),
                "{} is run_end and must close a run-scoped feed",
                entry.name
            ),
            SchemaTerminalSense::AtomicNoDelta | SchemaTerminalSense::NA => assert!(
                !entry.terminal && !kind.is_terminal(),
                "{} does not end a run and must keep a live feed open",
                entry.name
            ),
        }
    }

    let llm_done = SCHEMA_EVENT_KIND_REGISTRY
        .iter()
        .find(|entry| entry.name == "llm_done")
        .expect("generated registry must contain llm_done");
    assert_eq!(llm_done.terminal_sense, SchemaTerminalSense::AtomicNoDelta);

    for name in ["execute_complete", "session_end", "error"] {
        let entry = SCHEMA_EVENT_KIND_REGISTRY
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("generated registry must contain {name}"));
        assert_eq!(entry.terminal_sense, SchemaTerminalSense::RunEnd);
    }
}
