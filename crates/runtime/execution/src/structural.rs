//! Structural AIR schedule derived only from typed containment and sibling order.

use std::collections::BTreeMap;

use apxm_program::air::{AirModule, ControlPredicate, StructuralOpKind};
use apxm_program::frontend_graph::{HookBinding, HookPhase};

/// One runtime step derived from structural AIR plus the flat semantic op list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduleStep {
    /// Snapshot the selected target before walking one captured Hook body.
    ///
    /// This is separate from the eventual Hook step because the captured body
    /// is executable AIR and may change the driver's last-result accumulator.
    HookBodyBegin {
        binding: HookBinding,
    },
    /// Run one exact statically compiled `before` handler.
    HookBefore {
        binding: HookBinding,
    },
    /// Run one exact statically compiled `after` handler.
    HookAfter {
        binding: HookBinding,
    },
    /// Commit explicit Context along a recorded context-flow edge.
    ContextEdge {
        from_node: String,
        to_node: String,
        value_id: String,
    },
    /// Dispatch semantic operation at `index` in `air.semantic_operations`.
    Semantic {
        index: usize,
        loop_path: Vec<String>,
    },
    EnterLoop {
        static_loop_id: String,
        predicate: Option<ControlPredicate>,
    },
    LoopBackEdge {
        static_loop_id: String,
    },
    BranchDecision {
        static_branch_id: String,
        predicate: Option<ControlPredicate>,
    },
    BranchArm {
        static_branch_id: String,
        arm_index: usize,
    },
    BranchArmEnd {
        static_branch_id: String,
        arm_index: usize,
    },
    BranchEnd {
        static_branch_id: String,
    },
    ProgramYield {
        region_id: String,
        resume_value_id: Option<String>,
    },
    ProgramReturn {
        region_id: String,
    },
    ProgramExit {
        region_id: String,
    },
}

/// Build the deterministic execution schedule for `air`.
///
/// Hook placement is read off structural AIR rather than recomputed here. Each
/// Hook's captured body is a region lowering already positioned by scope, phase,
/// and declaration order, so walking that region runs the body's operations and
/// then applies the Hook's declared return contract. A Hook whose binding is
/// not supplied contributes its body but no Hook step, which is what makes
/// passing an artifact's `hook_bindings` the thing that brings Hooks to life.
#[must_use]
pub fn build_schedule(air: &AirModule, hook_bindings: &[HookBinding]) -> Vec<ScheduleStep> {
    let context_edges: BTreeMap<&str, Vec<(&str, &str)>> =
        air.context_flow
            .iter()
            .fold(BTreeMap::new(), |mut edges, edge| {
                edges
                    .entry(edge.to_node.as_str())
                    .or_default()
                    .push((edge.from_node.as_str(), edge.value_id.as_str()));
                edges
            });
    let hooks: BTreeMap<&str, &HookBinding> = hook_bindings
        .iter()
        .map(|binding| (binding.body_region_id.as_str(), binding))
        .collect();
    let mut schedule = Vec::new();
    emit_children(air, &hooks, &context_edges, None, &[], &mut schedule);
    schedule
}

fn emit_children(
    air: &AirModule,
    hooks: &BTreeMap<&str, &HookBinding>,
    context_edges: &BTreeMap<&str, Vec<(&str, &str)>>,
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
                emit_semantic(air, context_edges, index, loop_path, schedule);
            }
            Child::Structural(index) => {
                let region = &air.structural_ir[index];
                emit_context_before(context_edges, &region.region_id, schedule);
                match region.kind {
                    StructuralOpKind::Loop => {
                        schedule.push(ScheduleStep::EnterLoop {
                            static_loop_id: region.region_id.clone(),
                            predicate: region.predicate.clone(),
                        });
                        let mut nested_path = loop_path.to_vec();
                        nested_path.push(region.region_id.clone());
                        emit_children(
                            air,
                            hooks,
                            context_edges,
                            Some(&region.region_id),
                            &nested_path,
                            schedule,
                        );
                        schedule.push(ScheduleStep::LoopBackEdge {
                            static_loop_id: region.region_id.clone(),
                        });
                    }
                    StructuralOpKind::Yield => schedule.push(ScheduleStep::ProgramYield {
                        region_id: region.region_id.clone(),
                        resume_value_id: region
                            .block_arguments
                            .first()
                            .map(|argument| argument.value_id.clone()),
                    }),
                    StructuralOpKind::Return => schedule.push(ScheduleStep::ProgramReturn {
                        region_id: region.region_id.clone(),
                    }),
                    StructuralOpKind::Throw => schedule.push(ScheduleStep::ProgramExit {
                        region_id: region.region_id.clone(),
                    }),
                    StructuralOpKind::Branch | StructuralOpKind::Switch => {
                        schedule.push(ScheduleStep::BranchDecision {
                            static_branch_id: region.region_id.clone(),
                            predicate: region.predicate.clone(),
                        });
                        let mut arms: Vec<_> = air
                            .structural_ir
                            .iter()
                            .filter(|child| {
                                child.parent_region_id.as_deref() == Some(region.region_id.as_str())
                            })
                            .collect();
                        arms.sort_by_key(|arm| arm.execution_order);
                        for (arm_index, arm) in arms.iter().enumerate() {
                            schedule.push(ScheduleStep::BranchArm {
                                static_branch_id: region.region_id.clone(),
                                arm_index,
                            });
                            emit_children(
                                air,
                                hooks,
                                context_edges,
                                Some(&arm.region_id),
                                loop_path,
                                schedule,
                            );
                            schedule.push(ScheduleStep::BranchArmEnd {
                                static_branch_id: region.region_id.clone(),
                                arm_index,
                            });
                        }
                        schedule.push(ScheduleStep::BranchEnd {
                            static_branch_id: region.region_id.clone(),
                        });
                    }
                    StructuralOpKind::Function
                    | StructuralOpKind::Region
                    | StructuralOpKind::Block
                    | StructuralOpKind::ParallelJoin
                    | StructuralOpKind::Try
                    | StructuralOpKind::Catch => {
                        if let Some(binding) = hooks.get(region.region_id.as_str()) {
                            schedule.push(ScheduleStep::HookBodyBegin {
                                binding: (*binding).clone(),
                            });
                        }
                        emit_children(
                            air,
                            hooks,
                            context_edges,
                            Some(&region.region_id),
                            loop_path,
                            schedule,
                        );
                        emit_hook_step(hooks, &region.region_id, schedule);
                    }
                    StructuralOpKind::Value => {}
                }
            }
        }
    }
}

fn emit_semantic(
    air: &AirModule,
    context_edges: &BTreeMap<&str, Vec<(&str, &str)>>,
    index: usize,
    loop_path: &[String],
    schedule: &mut Vec<ScheduleStep>,
) {
    let operation = &air.semantic_operations[index];
    emit_context_before(context_edges, &operation.node_id, schedule);
    schedule.push(ScheduleStep::Semantic {
        index,
        loop_path: loop_path.to_vec(),
    });
}

/// Apply one Hook's declared return contract immediately after its captured
/// body has run, so a Hook never claims to have executed before its own
/// operations did.
fn emit_hook_step(
    hooks: &BTreeMap<&str, &HookBinding>,
    region_id: &str,
    schedule: &mut Vec<ScheduleStep>,
) {
    let Some(binding) = hooks.get(region_id) else {
        return;
    };
    let binding = (*binding).clone();
    schedule.push(match binding.phase {
        HookPhase::Before => ScheduleStep::HookBefore { binding },
        HookPhase::After => ScheduleStep::HookAfter { binding },
    });
}

fn emit_context_before(
    context_edges: &BTreeMap<&str, Vec<(&str, &str)>>,
    to_node: &str,
    schedule: &mut Vec<ScheduleStep>,
) {
    if let Some(edges) = context_edges.get(to_node) {
        for (from_node, value_id) in edges {
            schedule.push(ScheduleStep::ContextEdge {
                from_node: (*from_node).to_string(),
                to_node: to_node.to_string(),
                value_id: (*value_id).to_string(),
            });
        }
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
            "schema_version": "apxm.air",
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
                "schema_version": "apxm.source-map",
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
            "schema_version": "apxm.air",
            "semantic_operations": [
                {
                    "node_id": "node.outer.before",
                    "op": "model.call",
                    "parent_region_id": "loop.outer",
                    "execution_order": 0,
                    "operands": [{"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.outer.before.request", "type_ref": "ModelRequest"}],
                    "result": {"value_id": "value.outer.before.output", "type_ref": "ModelOutput"}
                },
                {
                    "node_id": "node.inner",
                    "op": "capability.invoke",
                    "parent_region_id": "loop.inner",
                    "execution_order": 0,
                    "operands": [{"slot": "capability_ref", "value_id": "capability.inner", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.inner.arguments", "type_ref": "CapabilityArguments"}],
                    "result": {"value_id": "value.inner.output", "type_ref": "CapabilityOutput"}
                },
                {
                    "node_id": "node.outer.after",
                    "op": "model.call",
                    "parent_region_id": "loop.outer",
                    "execution_order": 2,
                    "operands": [{"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.outer.after.request", "type_ref": "ModelRequest"}],
                    "result": {"value_id": "value.outer.after.output", "type_ref": "ModelOutput"}
                },
                {
                    "node_id": "node.sibling",
                    "op": "model.call",
                    "parent_region_id": "loop.sibling",
                    "execution_order": 0,
                    "operands": [{"slot": "model_ref", "value_id": "model.target", "type_ref": "ModelTargetRef"}, {"slot": "request", "value_id": "value.sibling.request", "type_ref": "ModelRequest"}],
                    "result": {"value_id": "value.sibling.output", "type_ref": "ModelOutput"}
                }
            ],
            "structural_ir": [
                {
                    "region_id": "region.root",
                    "kind": "region",
                    "execution_order": 0,
                    "block_arguments": [
                        {"value_id": "value.outer.before.request", "type_ref": "ModelRequest"},
                        {"value_id": "value.outer.after.request", "type_ref": "ModelRequest"},
                        {"value_id": "value.sibling.request", "type_ref": "ModelRequest"}
                    ]
                },
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
            "value_assemblies": [{"value_id": "value.inner.arguments", "expression": {"kind": "object", "fields": [{"name": "query", "value": {"kind": "string", "value": "nested"}}]}}],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map",
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
        assert!(air.verify().is_accepted());

        let schedule = build_schedule(&air, &[]);
        let control: Vec<String> = schedule
            .iter()
            .filter_map(|step| match step {
                ScheduleStep::EnterLoop { static_loop_id, .. } => {
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

    /// A loop Hook belongs to the loop it names. Its captured body is a child of
    /// that loop's region, so it is scheduled between that loop's entry and its
    /// back edge — once per iteration of the inner loop, not once around the
    /// outer one.
    #[test]
    fn a_loop_hook_body_runs_inside_the_loop_it_names() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air",
            "semantic_operations": [
                {"node_id": "node.inner", "op": "capability.invoke", "parent_region_id": "loop.inner", "execution_order": 500000, "operands": [{"slot": "capability_ref", "value_id": "capability.inner", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.inner.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.inner.output", "type_ref": "CapabilityOutput"}},
                {"node_id": "node.audit", "op": "capability.invoke", "parent_region_id": "hook.audit.body", "execution_order": 500000, "operands": [{"slot": "capability_ref", "value_id": "capability.audit", "type_ref": "CapabilityRef"}, {"slot": "arguments", "value_id": "value.inner.arguments", "type_ref": "CapabilityArguments"}], "result": {"value_id": "value.audit.output", "type_ref": "CapabilityOutput"}}
            ],
            "structural_ir": [
                {"region_id": "region.root", "kind": "region", "execution_order": 0},
                {"region_id": "loop.outer", "kind": "ais.loop", "parent_region_id": "region.root", "execution_order": 500000},
                {"region_id": "loop.inner", "kind": "ais.loop", "parent_region_id": "loop.outer", "execution_order": 1500000},
                {"region_id": "hook.audit.body", "kind": "region", "parent_region_id": "loop.inner", "execution_order": 0}
            ],
            "value_assemblies": [{"value_id": "value.inner.arguments", "expression": {"kind": "string", "value": "audit"}}],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": [
                    {"region_id": "loop.outer", "annotation": "structural_loop"},
                    {"region_id": "loop.inner", "annotation": "structural_loop"}
                ]
            }
        }))
        .expect("nested loop AIR with a Hook on the inner loop");
        assert!(air.verify().is_accepted());

        let binding: HookBinding = serde_json::from_value(json!({
            "hook_id": "hook.audit",
            "scope": "loop",
            "phase": "before",
            "target_selector": "loop.inner",
            "declaration_order": 0,
            "handler_ref": "AuditInnerIteration",
            "handler_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "input_type_ref": "AgentFacade",
            "output_type_ref": "Unit",
            "return_mode": "observe",
            "body_region_id": "hook.audit.body"
        }))
        .expect("loop Hook binding");

        let trace: Vec<String> = build_schedule(&air, std::slice::from_ref(&binding))
            .iter()
            .filter_map(|step| match step {
                ScheduleStep::EnterLoop { static_loop_id, .. } => {
                    Some(format!("enter:{static_loop_id}"))
                }
                ScheduleStep::LoopBackEdge { static_loop_id } => {
                    Some(format!("back:{static_loop_id}"))
                }
                ScheduleStep::Semantic { index, .. } => {
                    Some(format!("node:{}", air.semantic_operations[*index].node_id))
                }
                ScheduleStep::HookBefore { binding } => Some(format!("hook:{}", binding.hook_id)),
                _ => None,
            })
            .collect();
        assert_eq!(
            trace,
            [
                "enter:loop.outer",
                "enter:loop.inner",
                "node:node.audit",
                "hook:hook.audit",
                "node:node.inner",
                "back:loop.inner",
                "back:loop.outer",
            ]
        );

        // Without the binding the body still runs, but nothing claims a Hook
        // executed: the schedule only knows a Hook it was handed.
        assert!(
            build_schedule(&air, &[])
                .iter()
                .all(|step| !matches!(step, ScheduleStep::HookBefore { .. }))
        );
    }

    #[test]
    fn multiple_context_edges_to_one_consumer_are_all_scheduled() {
        let air: AirModule = serde_json::from_value(json!({
            "schema_version": "apxm.air",
            "semantic_operations": [{
                "node_id": "node.consumer",
                "op": "model.call",
                "parent_region_id": "region.root",
                "execution_order": 0
            }],
            "structural_ir": [{
                "region_id": "region.root",
                "kind": "region",
                "execution_order": 0
            }],
            "context_flow": [
                {
                    "from_node": "region.first",
                    "to_node": "node.consumer",
                    "context_type_ref": "Context",
                    "value_id": "context.first"
                },
                {
                    "from_node": "region.second",
                    "to_node": "node.consumer",
                    "context_type_ref": "Context",
                    "value_id": "context.second"
                }
            ],
            "source_map": {
                "schema_version": "apxm.source-map",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        }))
        .expect("duplicate-destination context edges");

        let context_edges: Vec<_> = build_schedule(&air, &[])
            .into_iter()
            .filter_map(|step| match step {
                ScheduleStep::ContextEdge {
                    from_node,
                    to_node,
                    value_id,
                } => Some((from_node, to_node, value_id)),
                _ => None,
            })
            .collect();
        assert_eq!(
            context_edges,
            [
                (
                    "region.first".to_string(),
                    "node.consumer".to_string(),
                    "context.first".to_string()
                ),
                (
                    "region.second".to_string(),
                    "node.consumer".to_string(),
                    "context.second".to_string()
                )
            ]
        );
    }
}
