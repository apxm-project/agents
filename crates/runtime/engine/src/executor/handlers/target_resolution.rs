//! Platform target-grammar resolution for `DELEGATE`/`COMMUNICATE`.
//!
//! The platform target grammar accepts, at every entry point:
//!
//! ```text
//!  name:<agent>          → stage 1 only (explicit, exact lookup)
//!  topic:<subject>       → stages 2→4→5 (topic → capability filter → score → admission)
//!  capability:<cap-id>   → stages 3→4→5 (capability filter → score → admission)
//!  (bare utterance)      → stage 2 classification, then as topic:
//! ```
//!
//! This module implements the `topic:`/`capability:` half of that grammar
//! for the in-graph `DELEGATE`/`COMMUNICATE` ops, resolving a target string
//! into a concrete registered agent id **before** the existing exact-id
//! lookup (`delegate.rs`, `communicate/local.rs`) runs. Anything that
//! doesn't match one of these two grammar forms (including bare `name:` or
//! an unprefixed exact id) is left untouched and falls through to that
//! unchanged exact lookup.
//!
//! ## Local routing contract
//!
//! This repo has no local subject/topic router; that service is seeded from
//! org `routing.toml` and lives in the `os` repo. `topic:` resolution here is
//! scored against each registered agent's declared
//! **discoverable** names (`AgentMetadata::discoverable` — "discoverable
//! capability/tool names", the closest local analog to "subjects this agent
//! answers to", platform.md rule 3: "agents declare what they
//! answer"). `capability:` resolution is scored against each agent's
//! declared **capabilities** (`AgentMetadata::capabilities`). Both reuse the
//! shared  scoring engine (`agent_scoring`) for the
//! capability-fit/least-used/preferred/order tie-break (stage 4) over the
//! candidate set filtered by the requested subject/capability (stage 2/3).
//!
//! Admission (`delegates_to`/`directory_policy`) and org-default fallback are
//! OS-owned topology concerns. This module does not enforce them. Locally, zero
//! eligible candidates always yields the typed [`RuntimeError::NoRouteFound`]
//! error, never a silent broadcast to every candidate.

use std::collections::HashSet;

use apxm_core::error::RuntimeError;
use apxm_core::types::Agent;

use crate::agent_scoring::{self, ScoringCandidate};
use crate::capability::flow_registry::FlowRegistry;

/// A parsed platform target-grammar expression that requires resolution
/// through the routing pipeline (as opposed to `name:`/bare exact lookup,
/// which is left to the existing unchanged code path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutedTarget {
    /// `topic:<subject>` — resolved against declared discoverable subjects.
    Topic(String),
    /// `capability:<cap-id>` — resolved against declared capabilities.
    Capability(String),
}

/// Parse a raw `DELEGATE`/`COMMUNICATE` target string for the two
/// grammar-based forms this module resolves. Returns `None` for anything
/// else (bare ids, `name:<agent>`, empty strings) so the caller falls
/// through to the existing exact lookup unchanged.
pub fn parse_routed_target(target: &str) -> Option<RoutedTarget> {
    if let Some(rest) = target.strip_prefix("topic:") {
        let subject = rest.trim();
        return (!subject.is_empty()).then(|| RoutedTarget::Topic(subject.to_string()));
    }
    if let Some(rest) = target.strip_prefix("capability:") {
        let cap = rest.trim();
        return (!cap.is_empty()).then(|| RoutedTarget::Capability(cap.to_string()));
    }
    None
}

/// One registered agent as a scoreable routing candidate. The `scoring_set`
/// is selected by the caller (discoverable subjects for `topic:`,
/// capabilities for `capability:`) so the same candidate shape serves both
/// grammar forms without conflating the two local tables.
struct AgentTargetCandidate {
    name: String,
    scoring_set: HashSet<String>,
    index: usize,
}

impl ScoringCandidate for AgentTargetCandidate {
    fn scoring_id(&self) -> &str {
        &self.name
    }

    fn scoring_capabilities(&self) -> &HashSet<String> {
        &self.scoring_set
    }

    fn scoring_index(&self) -> usize {
        self.index
    }
}

/// Resolve a parsed [`RoutedTarget`] into a concrete registered agent name
/// using the org-member candidate set from the [`FlowRegistry`] and the
/// shared  scoring engine.
///
/// Returns the chosen agent's name, or [`RuntimeError::NoRouteFound`] when
/// no registered agent declares the requested subject/capability. Never
/// returns more than one candidate — this is a resolution, not a broadcast.
pub fn resolve_routed_target(
    flow_registry: &FlowRegistry,
    target: &RoutedTarget,
) -> Result<String, RuntimeError> {
    let (target_str, required) = match target {
        RoutedTarget::Topic(subject) => (format!("topic:{subject}"), vec![subject.clone()]),
        RoutedTarget::Capability(cap) => (format!("capability:{cap}"), vec![cap.clone()]),
    };
    let required = agent_scoring::normalize_capabilities(&required);

    // Deterministic candidate order: FlowRegistry's underlying map iteration
    // order is not guaranteed, so sort agent names before assigning the
    // declaration-order tie-break index (platform.md rule 2: routing must be
    // deterministic and explainable).
    let mut agent_names = flow_registry.list_agents();
    agent_names.sort();

    let candidates: Vec<AgentTargetCandidate> = agent_names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            let agent = flow_registry.get_agent(name)?;
            let scoring_set = scoring_set_for(&agent, target);
            Some(AgentTargetCandidate {
                name: name.clone(),
                scoring_set,
                index,
            })
        })
        .collect();

    let eligible = agent_scoring::matching_candidates(&candidates, &required);
    if eligible.is_empty() {
        return Err(RuntimeError::NoRouteFound {
            target: target_str,
            reason: if candidates.is_empty() {
                "no agents are registered".to_string()
            } else {
                format!(
                    "no registered agent declares the requested {}",
                    match target {
                        RoutedTarget::Topic(subject) => format!("topic '{subject}'"),
                        RoutedTarget::Capability(cap) => format!("capability '{cap}'"),
                    }
                )
            },
        });
    }

    let chosen = agent_scoring::select_best(&eligible, required.len(), &[], &Default::default());
    Ok(chosen.name.clone())
}

fn scoring_set_for(agent: &Agent, target: &RoutedTarget) -> HashSet<String> {
    match target {
        RoutedTarget::Topic(_) => {
            agent_scoring::normalize_capabilities(&agent.metadata.discoverable)
                .into_iter()
                .collect()
        }
        RoutedTarget::Capability(_) => agent_scoring::normalize_capabilities(
            &agent
                .metadata
                .capabilities
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>(),
        )
        .into_iter()
        .collect(),
    }
}

/// Convenience used by `DELEGATE`/`COMMUNICATE` handlers: if `target` matches
/// the routed-target grammar, resolve it to a concrete registered agent name
/// through the routing pipeline; otherwise return the original string
/// unchanged so the caller's existing exact lookup runs exactly as before.
pub fn resolve_target_or_passthrough(
    flow_registry: &FlowRegistry,
    target: &str,
) -> Result<String, RuntimeError> {
    match parse_routed_target(target) {
        Some(routed) => resolve_routed_target(flow_registry, &routed),
        None => Ok(target.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::{AgentMetadata, CapabilityDeclaration};

    fn agent_with(name: &str, capabilities: &[&str], discoverable: &[&str]) -> Agent {
        Agent {
            name: name.to_string(),
            metadata: AgentMetadata {
                memories: Vec::new(),
                capabilities: capabilities
                    .iter()
                    .map(|c| CapabilityDeclaration {
                        name: c.to_string(),
                        description: None,
                    })
                    .collect(),
                tools: Vec::new(),
                discoverable: discoverable.iter().map(|d| d.to_string()).collect(),
                context: None,
            },
            flows: Default::default(),
        }
    }

    fn registry_with(agents: Vec<Agent>) -> FlowRegistry {
        let registry = FlowRegistry::new();
        for agent in agents {
            registry.register_agent(agent);
        }
        registry
    }

    #[test]
    fn parse_routed_target_recognizes_topic_and_capability() {
        assert_eq!(
            parse_routed_target("topic:receivables"),
            Some(RoutedTarget::Topic("receivables".to_string()))
        );
        assert_eq!(
            parse_routed_target("capability:invoice_lookup"),
            Some(RoutedTarget::Capability("invoice_lookup".to_string()))
        );
    }

    #[test]
    fn parse_routed_target_ignores_exact_and_name_targets() {
        assert_eq!(parse_routed_target("researcher"), None);
        assert_eq!(parse_routed_target("name:researcher"), None);
        assert_eq!(parse_routed_target("topic:"), None);
        assert_eq!(parse_routed_target("capability:"), None);
    }

    #[test]
    fn resolve_topic_target_picks_declaring_agent() {
        let registry = registry_with(vec![
            agent_with("billing", &[], &["receivables"]),
            agent_with("support", &[], &["tickets"]),
        ]);
        let resolved =
            resolve_target_or_passthrough(&registry, "topic:receivables").expect("resolves");
        assert_eq!(resolved, "billing");
    }

    #[test]
    fn resolve_capability_target_picks_declaring_agent() {
        let registry = registry_with(vec![
            agent_with("writer", &["draft_report"], &[]),
            agent_with("reviewer", &["review_report"], &[]),
        ]);
        let resolved =
            resolve_target_or_passthrough(&registry, "capability:review_report").expect("resolves");
        assert_eq!(resolved, "reviewer");
    }

    #[test]
    fn resolve_returns_no_route_found_when_no_candidate_declares_it() {
        let registry = registry_with(vec![agent_with("billing", &[], &["receivables"])]);
        let err = resolve_target_or_passthrough(&registry, "topic:nonexistent")
            .expect_err("no candidate declares this topic");
        match err {
            RuntimeError::NoRouteFound { target, .. } => {
                assert_eq!(target, "topic:nonexistent");
            }
            other => panic!("expected NoRouteFound, got {other:?}"),
        }
    }

    #[test]
    fn resolve_returns_no_route_found_when_registry_is_empty() {
        let registry = registry_with(vec![]);
        let err = resolve_target_or_passthrough(&registry, "capability:anything")
            .expect_err("empty registry has no candidates");
        assert!(matches!(err, RuntimeError::NoRouteFound { .. }));
    }

    #[test]
    fn passthrough_leaves_exact_and_unprefixed_targets_untouched() {
        let registry = registry_with(vec![agent_with("billing", &[], &["receivables"])]);
        assert_eq!(
            resolve_target_or_passthrough(&registry, "billing").unwrap(),
            "billing"
        );
        assert_eq!(
            resolve_target_or_passthrough(&registry, "name:billing").unwrap(),
            "name:billing"
        );
    }

    #[test]
    fn resolve_topic_uses_scoring_engine_for_ties() {
        // Both declare the topic; "idle" has never been used so with no
        // usage counts supplied both start at zero — the tighter capability
        // fit (fewer extra discoverable entries) wins per  precedence.
        let registry = registry_with(vec![
            agent_with("wide", &[], &["receivables", "payables", "invoices"]),
            agent_with("tight", &[], &["receivables"]),
        ]);
        let resolved =
            resolve_target_or_passthrough(&registry, "topic:receivables").expect("resolves");
        assert_eq!(resolved, "tight");
    }
}
