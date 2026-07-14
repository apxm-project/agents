//! Conservative effect and authority facts for artifact optimization evidence.

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::compiler::{Determinism, EffectAuthoritySummary, ReplaySafety};
use apxm_core::types::execution::Node;
use apxm_core::types::{AISOperationType, PermissionOperation, Value};

use super::backend_legality;

/// Derive facts that must fail closed when an operation's authority is unknown.
pub(super) fn summarize(node: &Node) -> EffectAuthoritySummary {
    let mut summary = EffectAuthoritySummary {
        determinism: Determinism::Unknown,
        replay_safety: ReplaySafety::Unknown,
        ..Default::default()
    };

    match node.op_type {
        AISOperationType::Agent
        | AISOperationType::Jump
        | AISOperationType::BranchOnValue
        | AISOperationType::Return
        | AISOperationType::Switch
        | AISOperationType::Merge
        | AISOperationType::Fence
        | AISOperationType::WaitAll
        | AISOperationType::TryCatch
        | AISOperationType::Err
        | AISOperationType::Nop
        | AISOperationType::Identity
        | AISOperationType::ConstStr
        | AISOperationType::Yield => mark_pure(&mut summary),
        AISOperationType::Ask | AISOperationType::Think => {
            summary.reads.push("memory".to_string());
            if backend_legality::may_invoke_tools(node) {
                summary.writes.push("capability".to_string());
                mark_authority_effect(&mut summary, PermissionOperation::Execute);
            } else if has_explicit_zero_temperature(node) {
                summary.determinism = Determinism::Proven;
                summary.idempotent = true;
                summary.replay_safety = ReplaySafety::Safe;
            } else {
                summary.idempotent = true;
            }
        }
        AISOperationType::Reason => {
            summary.reads.push("memory".to_string());
            summary.writes.push("beliefs".to_string());
            summary.writes.push("goals".to_string());
            mark_authority_effect(&mut summary, PermissionOperation::Write);
            if backend_legality::may_invoke_tools(node) {
                summary.writes.push("capability".to_string());
                summary
                    .permission_operations
                    .push(PermissionOperation::Execute);
            }
        }
        AISOperationType::Plan | AISOperationType::UpdateGoal => {
            summary.writes.push("goals".to_string());
            mark_authority_effect(&mut summary, PermissionOperation::Write);
        }
        AISOperationType::Reflect => {
            summary.reads.push("episodic-memory".to_string());
            summary.writes.push("beliefs".to_string());
            mark_authority_effect(&mut summary, PermissionOperation::Write);
        }
        AISOperationType::Verify | AISOperationType::QMem => {
            summary.reads.push("memory".to_string());
            summary
                .permission_operations
                .push(PermissionOperation::Read);
            summary.idempotent = true;
        }
        AISOperationType::UMem => {
            summary.writes.push("memory".to_string());
            mark_authority_effect(&mut summary, PermissionOperation::Write);
        }
        AISOperationType::InvCap => {
            let capability = node
                .get_attribute(graph_attrs::CAPABILITY)
                .and_then(Value::as_str)
                .map_or_else(
                    || "capability".to_string(),
                    |value| format!("capability:{value}"),
                );
            summary.reads.push("capability-registry".to_string());
            summary.writes.push(capability);
            mark_authority_effect(&mut summary, PermissionOperation::Execute);
        }
        AISOperationType::Exc => {
            summary.writes.push("confined-execution".to_string());
            mark_authority_effect(&mut summary, PermissionOperation::Execute);
        }
        AISOperationType::Print => {
            summary.writes.push("stdout".to_string());
            summary.replay_safety = ReplaySafety::RequiresCheckpoint;
        }
        AISOperationType::FlowCall
        | AISOperationType::WorkflowSpawn
        | AISOperationType::Delegate
        | AISOperationType::SpawnAgent
        | AISOperationType::Autonomous => {
            summary.writes.push("execution".to_string());
            mark_authority_effect(&mut summary, PermissionOperation::Execute);
        }
        AISOperationType::Communicate | AISOperationType::Handoff => {
            summary.writes.push("communication".to_string());
            mark_authority_effect(&mut summary, PermissionOperation::Send);
        }
        AISOperationType::Pause | AISOperationType::Resume => {
            summary.writes.push("execution-control".to_string());
            mark_authority_effect(&mut summary, PermissionOperation::Approve);
        }
        AISOperationType::RegisterCapability | AISOperationType::RegisterHook => {
            summary.writes.push("capability-registry".to_string());
            mark_authority_effect(&mut summary, PermissionOperation::Write);
        }
        AISOperationType::Checkpoint => {
            summary.writes.push("checkpoint".to_string());
            summary
                .permission_operations
                .push(PermissionOperation::Write);
            summary.replay_safety = ReplaySafety::Safe;
        }
    }

    summary
}

/// Mark an operation as deterministic, idempotent, and replay-safe.
fn mark_pure(summary: &mut EffectAuthoritySummary) {
    summary.determinism = Determinism::Proven;
    summary.idempotent = true;
    summary.replay_safety = ReplaySafety::Safe;
}

/// Record the common authority and replay boundary for an observable effect.
fn mark_authority_effect(summary: &mut EffectAuthoritySummary, operation: PermissionOperation) {
    summary.permission_operations.push(operation);
    summary.approval_required = true;
    summary.replay_safety = ReplaySafety::RequiresCheckpoint;
}

/// Whether the artifact explicitly requests deterministic sampling.
fn has_explicit_zero_temperature(node: &Node) -> bool {
    node.get_attribute(graph_attrs::TEMPERATURE)
        .and_then(Value::as_number)
        .is_some_and(|temperature| temperature.as_f64() == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::WIRE_INDEXED_OPERATIONS;

    #[test]
    fn every_wire_operation_has_explicit_effect_or_purity_evidence() {
        for (wire_index, operation) in WIRE_INDEXED_OPERATIONS {
            let summary = summarize(&Node::new(u64::from(*wire_index), *operation));
            let proven_pure = summary.determinism == Determinism::Proven
                && summary.idempotent
                && summary.replay_safety == ReplaySafety::Safe;
            let effect_or_authority = !summary.reads.is_empty()
                || !summary.writes.is_empty()
                || !summary.permission_operations.is_empty()
                || summary.approval_required
                || summary.replay_safety != ReplaySafety::Unknown;
            assert!(
                proven_pure || effect_or_authority,
                "{operation} has no explicit effect, authority, or purity classification"
            );
        }
    }
}
