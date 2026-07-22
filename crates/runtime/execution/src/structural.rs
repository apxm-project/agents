//! Structural AIR schedule: derive hook, context, and loop-boundary steps from
//! `AirModule::structural_ir` instead of walking semantic operations alone.

use std::collections::{BTreeMap, BTreeSet};

use apxm_program::air::{AirModule, SemanticOpKind, StructuralKind, StructuralNode};
use apxm_program::frontend_graph::{HookBinding, HookPhase, HookScope};

/// One runtime step derived from structural AIR plus the flat semantic op list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleStep {
    /// Run one exact statically compiled `before` handler.
    HookBefore { binding: HookBinding },
    /// Run one exact statically compiled `after` handler.
    HookAfter { binding: HookBinding },
    /// Commit explicit Context along a recorded context-flow edge.
    ContextEdge { from_node: String, to_node: String },
    /// Dispatch semantic operation at `index` in `air.semantic_operations`.
    Semantic { index: usize },
    /// Suspend at a compiler-provided loop continuation identity.
    LoopYield {
        continuation_id: String,
        resume_semantic_index: usize,
    },
}

/// Build the deterministic execution schedule for `air`.
///
/// When `structural_ir` is empty the schedule is a straight semantic walk, preserving
/// the legacy single-shot/resumable behavior. When structural nodes are present the
/// schedule interleaves compiled Hook callsites and explicit Context commits around
/// the authored semantic operation order and ends conversational loops with yield.
#[must_use]
pub fn build_schedule(air: &AirModule, hook_bindings: &[HookBinding]) -> Vec<ScheduleStep> {
    if air.structural_ir.is_empty() {
        return (0..air.semantic_operations.len())
            .map(|index| ScheduleStep::Semantic { index })
            .collect();
    }

    let context_edges = parse_context_edges(&air.structural_ir);
    let loop_yields = loop_yields(&air.structural_ir);

    let mut schedule = Vec::new();
    let mut seen_context: BTreeSet<(String, String)> = BTreeSet::new();

    for binding in ordered_hooks(hook_bindings, HookPhase::Before, |hook| {
        hook.scope == HookScope::Agent || hook.scope == HookScope::Loop
    }) {
        schedule.push(ScheduleStep::HookBefore {
            binding: binding.clone(),
        });
    }

    for (index, op) in air.semantic_operations.iter().enumerate() {
        for binding in ordered_hooks(hook_bindings, HookPhase::Before, |hook| {
            hook_targets_operation(hook, &op.node_id, op.op)
        }) {
            schedule.push(ScheduleStep::HookBefore {
                binding: binding.clone(),
            });
        }
        schedule.push(ScheduleStep::Semantic { index });
        if let Some(to_node) = context_edges.get(&op.node_id) {
            let key = (op.node_id.clone(), to_node.clone());
            if seen_context.insert(key.clone()) {
                schedule.push(ScheduleStep::ContextEdge {
                    from_node: key.0,
                    to_node: key.1,
                });
            }
        }
        for binding in ordered_hooks(hook_bindings, HookPhase::After, |hook| {
            hook_targets_operation(hook, &op.node_id, op.op)
        }) {
            schedule.push(ScheduleStep::HookAfter {
                binding: binding.clone(),
            });
        }
    }

    for binding in ordered_hooks(hook_bindings, HookPhase::After, |hook| {
        hook.scope == HookScope::Loop || hook.scope == HookScope::Agent
    }) {
        schedule.push(ScheduleStep::HookAfter {
            binding: binding.clone(),
        });
    }

    for continuation_id in loop_yields {
        schedule.push(ScheduleStep::LoopYield {
            continuation_id,
            resume_semantic_index: 0,
        });
    }
    schedule
}

fn ordered_hooks(
    bindings: &[HookBinding],
    phase: HookPhase,
    predicate: impl Fn(&HookBinding) -> bool,
) -> Vec<&HookBinding> {
    let mut hooks: Vec<_> = bindings
        .iter()
        .filter(|hook| hook.phase == phase && predicate(hook))
        .collect();
    hooks.sort_by(|left, right| {
        scope_rank(left.scope)
            .cmp(&scope_rank(right.scope))
            .then_with(|| left.declaration_order.cmp(&right.declaration_order))
            .then_with(|| left.hook_id.cmp(&right.hook_id))
    });
    if phase == HookPhase::After {
        hooks.reverse();
    }
    hooks
}

fn hook_targets_operation(binding: &HookBinding, node_id: &str, op: SemanticOpKind) -> bool {
    if binding.target_selector != node_id {
        return false;
    }
    match binding.scope {
        HookScope::Node => true,
        HookScope::Model => op == SemanticOpKind::ModelCall,
        HookScope::Capability => op == SemanticOpKind::CapabilityInvoke,
        HookScope::Agent | HookScope::Loop => false,
    }
}

const fn scope_rank(scope: HookScope) -> u8 {
    match scope {
        HookScope::Agent => 0,
        HookScope::Loop => 1,
        HookScope::Node => 2,
        HookScope::Model => 3,
        HookScope::Capability => 4,
    }
}

fn parse_context_edges(structural_ir: &[StructuralNode]) -> BTreeMap<String, String> {
    let mut edges = BTreeMap::new();
    for node in structural_ir {
        if node.kind != StructuralKind::Value {
            continue;
        }
        let Some((from_node, to_node)) = parse_context_value_id(&node.region_id) else {
            continue;
        };
        edges.insert(from_node.clone(), to_node);
    }
    edges
}

fn parse_context_value_id(region_id: &str) -> Option<(String, String)> {
    let rest = region_id.strip_prefix("value.context.")?;
    let positions: Vec<usize> = rest.match_indices(".node.").map(|(pos, _)| pos).collect();
    for &pos in positions.iter().rev() {
        if pos == 0 {
            continue;
        }
        let from_node = rest[..pos].to_string();
        let to_node = rest[pos + 1..].to_string();
        if from_node.starts_with("node.") && to_node.starts_with("node.") {
            return Some((from_node, to_node));
        }
    }
    None
}

fn loop_yields(structural_ir: &[StructuralNode]) -> Vec<String> {
    structural_ir
        .iter()
        .filter(|node| node.kind == StructuralKind::Yield)
        .map(|node| node.region_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_program::air::AirModule;
    use apxm_program::frontend_graph::{HookBinding, HookPhase, HookReturnMode, HookScope};
    use serde_json::json;

    fn gao_air() -> AirModule {
        serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {"node_id": "node.turn.model", "op": "model.call", "operands": {"model_target_ref": "model.default"}},
                {"node_id": "node.turn.tool", "op": "capability.invoke", "operands": {"capability_ref": "cap.search"}},
                {"node_id": "node.turn.await", "op": "await.event", "operands": {"event_ref": "session-input:s1"}}
            ],
            "structural_ir": [
                {"region_id": "region.loop.turn", "kind": "loop"},
                {"region_id": "region.region.loop.turn.yield", "kind": "yield"},
                {"region_id": "value.hook.before.hook.before.turn", "kind": "value"},
                {"region_id": "value.context.node.turn.model.node.turn.tool", "kind": "value"},
                {"region_id": "value.hook.after.hook.after.model", "kind": "value"}
            ],
            "source_map": {"schema_version": "apxm.source-map.v1", "source_language": "python", "node_spans": [], "region_annotations": []}
        }))
        .expect("gao air")
    }

    #[test]
    fn schedule_interleaves_hooks_context_and_yield() {
        let hooks = vec![
            hook("hook.agent.before", HookScope::Agent, HookPhase::Before, "region.fn.Gao", 0),
            hook("hook.loop.before", HookScope::Loop, HookPhase::Before, "region.loop.turn", 1),
            hook("hook.node.before", HookScope::Node, HookPhase::Before, "node.turn.model", 2),
            hook("hook.model.before", HookScope::Model, HookPhase::Before, "node.turn.model", 3),
            hook("hook.node.after", HookScope::Node, HookPhase::After, "node.turn.model", 4),
            hook("hook.model.after", HookScope::Model, HookPhase::After, "node.turn.model", 5),
            hook("hook.loop.after", HookScope::Loop, HookPhase::After, "region.loop.turn", 6),
            hook("hook.agent.after", HookScope::Agent, HookPhase::After, "region.fn.Gao", 7),
        ];
        let schedule = build_schedule(&gao_air(), &hooks);
        let hook_ids: Vec<_> = schedule
            .iter()
            .filter_map(|step| match step {
                ScheduleStep::HookBefore { binding } | ScheduleStep::HookAfter { binding } => {
                    Some(binding.hook_id.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            hook_ids,
            vec![
                "hook.agent.before",
                "hook.loop.before",
                "hook.node.before",
                "hook.model.before",
                "hook.model.after",
                "hook.node.after",
                "hook.loop.after",
                "hook.agent.after",
            ]
        );
        assert!(schedule.iter().any(|step| matches!(step, ScheduleStep::ContextEdge { .. })));
        assert!(schedule.iter().any(|step| matches!(step, ScheduleStep::LoopYield { .. })));
        assert_eq!(
            schedule
                .iter()
                .filter(|step| matches!(step, ScheduleStep::Semantic { .. }))
                .count(),
            3
        );
    }

    #[test]
    fn empty_structural_ir_preserves_linear_semantic_walk() {
        let mut air = gao_air();
        air.structural_ir.clear();
        let schedule = build_schedule(&air, &[]);
        assert_eq!(
            schedule,
            vec![
                ScheduleStep::Semantic { index: 0 },
                ScheduleStep::Semantic { index: 1 },
                ScheduleStep::Semantic { index: 2 },
            ]
        );
    }

    fn hook(
        hook_id: &str,
        scope: HookScope,
        phase: HookPhase,
        target_selector: &str,
        declaration_order: u32,
    ) -> HookBinding {
        HookBinding {
            hook_id: hook_id.to_string(),
            scope,
            phase,
            target_selector: target_selector.to_string(),
            declaration_order,
            handler_ref: format!("handlers.{hook_id}"),
            handler_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            input_type_ref: "Context".to_string(),
            output_type_ref: "Context".to_string(),
            return_mode: HookReturnMode::Observe,
        }
    }
}
