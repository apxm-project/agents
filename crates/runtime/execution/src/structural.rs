//! Structural AIR schedule derived only from typed containment and sibling order.

use std::collections::BTreeMap;

use apxm_program::air::{AirModule, SemanticOpKind, StructuralOpKind};
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
    Semantic {
        index: usize,
        loop_path: Vec<String>,
    },
    EnterLoop { static_loop_id: String },
    LoopBackEdge { static_loop_id: String },
    ProgramYield { region_id: String },
    ProgramReturn { region_id: String },
    ProgramExit { region_id: String },
}

/// Build the deterministic execution schedule for `air`.
#[must_use]
pub fn build_schedule(air: &AirModule, hook_bindings: &[HookBinding]) -> Vec<ScheduleStep> {
    let context_edges: BTreeMap<&str, &str> = air
        .context_flow
        .iter()
        .map(|edge| (edge.from_node.as_str(), edge.to_node.as_str()))
        .collect();
    let mut schedule = Vec::new();

    for binding in ordered_hooks(hook_bindings, HookPhase::Before, |hook| {
        hook.scope == HookScope::Agent
    }) {
        schedule.push(ScheduleStep::HookBefore {
            binding: binding.clone(),
        });
    }

    emit_children(
        air,
        hook_bindings,
        &context_edges,
        None,
        &[],
        &mut schedule,
    );

    for binding in ordered_hooks(hook_bindings, HookPhase::After, |hook| {
        hook.scope == HookScope::Agent
    }) {
        schedule.push(ScheduleStep::HookAfter {
            binding: binding.clone(),
        });
    }
    schedule
}

fn emit_children(
    air: &AirModule,
    hook_bindings: &[HookBinding],
    context_edges: &BTreeMap<&str, &str>,
    parent_region_id: Option<&str>,
    loop_path: &[String],
    schedule: &mut Vec<ScheduleStep>,
) {
    enum Child {
        Structural(usize),
        Semantic(usize),
    }

    let mut children = Vec::new();
    for (index, region) in air.structural_ir.iter().enumerate() {
        if region.parent_region_id.as_deref() == parent_region_id {
            children.push((region.execution_order, Child::Structural(index)));
        }
    }
    if let Some(parent_region_id) = parent_region_id {
        for (index, operation) in air.semantic_operations.iter().enumerate() {
            if operation.parent_region_id == parent_region_id {
                children.push((operation.execution_order, Child::Semantic(index)));
            }
        }
    }
    children.sort_by_key(|(order, _)| *order);

    for (_, child) in children {
        match child {
            Child::Semantic(index) => {
                emit_semantic(
                    air,
                    hook_bindings,
                    context_edges,
                    index,
                    loop_path,
                    schedule,
                );
            }
            Child::Structural(index) => {
                let region = &air.structural_ir[index];
                match region.kind {
                    StructuralOpKind::Loop => {
                        schedule.push(ScheduleStep::EnterLoop {
                            static_loop_id: region.region_id.clone(),
                        });
                        for binding in ordered_hooks(
                            hook_bindings,
                            HookPhase::Before,
                            |hook| {
                                hook.scope == HookScope::Loop
                                    && hook.target_selector == region.region_id
                            },
                        ) {
                            schedule.push(ScheduleStep::HookBefore {
                                binding: binding.clone(),
                            });
                        }
                        let mut nested_path = loop_path.to_vec();
                        nested_path.push(region.region_id.clone());
                        emit_children(
                            air,
                            hook_bindings,
                            context_edges,
                            Some(&region.region_id),
                            &nested_path,
                            schedule,
                        );
                        for binding in ordered_hooks(
                            hook_bindings,
                            HookPhase::After,
                            |hook| {
                                hook.scope == HookScope::Loop
                                    && hook.target_selector == region.region_id
                            },
                        ) {
                            schedule.push(ScheduleStep::HookAfter {
                                binding: binding.clone(),
                            });
                        }
                        schedule.push(ScheduleStep::LoopBackEdge {
                            static_loop_id: region.region_id.clone(),
                        });
                    }
                    StructuralOpKind::Yield => schedule.push(ScheduleStep::ProgramYield {
                        region_id: region.region_id.clone(),
                    }),
                    StructuralOpKind::Return => schedule.push(ScheduleStep::ProgramReturn {
                        region_id: region.region_id.clone(),
                    }),
                    StructuralOpKind::Throw => schedule.push(ScheduleStep::ProgramExit {
                        region_id: region.region_id.clone(),
                    }),
                    StructuralOpKind::Function
                    | StructuralOpKind::Region
                    | StructuralOpKind::Block
                    | StructuralOpKind::Branch
                    | StructuralOpKind::Switch
                    | StructuralOpKind::ParallelJoin
                    | StructuralOpKind::Try
                    | StructuralOpKind::Catch => emit_children(
                        air,
                        hook_bindings,
                        context_edges,
                        Some(&region.region_id),
                        loop_path,
                        schedule,
                    ),
                    StructuralOpKind::Value => {}
                }
            }
        }
    }
}

fn emit_semantic(
    air: &AirModule,
    hook_bindings: &[HookBinding],
    context_edges: &BTreeMap<&str, &str>,
    index: usize,
    loop_path: &[String],
    schedule: &mut Vec<ScheduleStep>,
) {
    let operation = &air.semantic_operations[index];
    for binding in ordered_hooks(hook_bindings, HookPhase::Before, |hook| {
        hook_targets_operation(hook, &operation.node_id, operation.op)
    }) {
        schedule.push(ScheduleStep::HookBefore {
            binding: binding.clone(),
        });
    }
    schedule.push(ScheduleStep::Semantic {
        index,
        loop_path: loop_path.to_vec(),
    });
    if let Some(to_node) = context_edges.get(operation.node_id.as_str()) {
        schedule.push(ScheduleStep::ContextEdge {
            from_node: operation.node_id.clone(),
            to_node: (*to_node).to_string(),
        });
    }
    for binding in ordered_hooks(hook_bindings, HookPhase::After, |hook| {
        hook_targets_operation(hook, &operation.node_id, operation.op)
    }) {
        schedule.push(ScheduleStep::HookAfter {
            binding: binding.clone(),
        });
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_program::air::AirModule;
    use serde_json::json;

    #[test]
    fn non_loop_yield_is_not_scheduled_as_a_committed_back_edge() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [],
            "structural_ir": [
                {
                    "region_id": "region.program.yield",
                    "kind": "yield",
                    "execution_order": 0
                }
            ],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        }))
        .expect("program yield AIR");

        assert!(
            build_schedule(&air, &[])
                .iter()
                .all(|step| !matches!(step, ScheduleStep::LoopBackEdge { .. }))
        );
        assert!(matches!(
            build_schedule(&air, &[]).as_slice(),
            [ScheduleStep::ProgramYield { .. }]
        ));
    }

    #[test]
    fn sibling_and_nested_loops_have_independent_ordered_back_edges() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [
                {
                    "node_id": "node.outer.before",
                    "op": "model.call",
                    "parent_region_id": "loop.outer",
                    "execution_order": 0
                },
                {
                    "node_id": "node.inner",
                    "op": "capability.invoke",
                    "parent_region_id": "loop.inner",
                    "execution_order": 0
                },
                {
                    "node_id": "node.outer.after",
                    "op": "model.call",
                    "parent_region_id": "loop.outer",
                    "execution_order": 2
                },
                {
                    "node_id": "node.sibling",
                    "op": "model.call",
                    "parent_region_id": "loop.sibling",
                    "execution_order": 0
                }
            ],
            "structural_ir": [
                {"region_id": "region.root", "kind": "region", "execution_order": 0},
                {
                    "region_id": "loop.outer",
                    "kind": "ais.loop",
                    "parent_region_id": "region.root",
                    "execution_order": 0
                },
                {
                    "region_id": "loop.inner",
                    "kind": "ais.loop",
                    "parent_region_id": "loop.outer",
                    "execution_order": 1
                },
                {
                    "region_id": "loop.sibling",
                    "kind": "ais.loop",
                    "parent_region_id": "region.root",
                    "execution_order": 1
                }
            ],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": [
                    {"region_id": "loop.outer", "annotation": "structural_loop"},
                    {"region_id": "loop.inner", "annotation": "structural_loop"},
                    {"region_id": "loop.sibling", "annotation": "structural_loop"}
                ]
            }
        }))
        .expect("nested loop AIR");

        let schedule = build_schedule(&air, &[]);
        let control: Vec<String> = schedule
            .iter()
            .filter_map(|step| match step {
                ScheduleStep::EnterLoop { static_loop_id } => {
                    Some(format!("enter:{static_loop_id}"))
                }
                ScheduleStep::Semantic { index, loop_path } => Some(format!(
                    "node:{}:{}",
                    air.semantic_operations[*index].node_id,
                    loop_path.join(">")
                )),
                ScheduleStep::LoopBackEdge { static_loop_id } => {
                    Some(format!("back:{static_loop_id}"))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            control,
            [
                "enter:loop.outer",
                "node:node.outer.before:loop.outer",
                "enter:loop.inner",
                "node:node.inner:loop.outer>loop.inner",
                "back:loop.inner",
                "node:node.outer.after:loop.outer",
                "back:loop.outer",
                "enter:loop.sibling",
                "node:node.sibling:loop.sibling",
                "back:loop.sibling",
            ]
        );
    }
}
