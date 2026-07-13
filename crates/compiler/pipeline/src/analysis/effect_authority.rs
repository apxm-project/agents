//! Conservative effect and authority facts for artifact optimization evidence.

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::compiler::{Determinism, EffectAuthoritySummary, ReplaySafety};
use apxm_core::types::execution::Node;
use apxm_core::types::{AISOperationType, PermissionOperation, Value};

/// Derive facts that must fail closed when an operation's authority is unknown.
pub(super) fn summarize(node: &Node) -> EffectAuthoritySummary {
    let mut summary = EffectAuthoritySummary {
        determinism: Determinism::Unknown,
        replay_safety: ReplaySafety::Unknown,
        ..Default::default()
    };

    match node.op_type {
        AISOperationType::Nop
        | AISOperationType::Identity
        | AISOperationType::ConstStr
        | AISOperationType::Merge
        | AISOperationType::WaitAll => {
            summary.determinism = Determinism::Proven;
            summary.idempotent = true;
            summary.replay_safety = ReplaySafety::Safe;
        }
        AISOperationType::QMem => {
            summary.reads.push("memory".to_string());
            summary
                .permission_operations
                .push(PermissionOperation::Read);
        }
        AISOperationType::UMem => {
            summary.writes.push("memory".to_string());
            summary
                .permission_operations
                .push(PermissionOperation::Write);
            summary.approval_required = true;
            summary.replay_safety = ReplaySafety::RequiresCheckpoint;
        }
        AISOperationType::InvCap => {
            let capability = node
                .get_attribute(graph_attrs::CAPABILITY)
                .and_then(Value::as_str)
                .unwrap_or("unknown-capability");
            summary.writes.push(format!("capability:{capability}"));
            summary
                .permission_operations
                .push(PermissionOperation::Execute);
            summary.approval_required = true;
            summary.replay_safety = ReplaySafety::RequiresCheckpoint;
        }
        AISOperationType::RegisterCapability | AISOperationType::RegisterHook => {
            summary.writes.push("capability-registry".to_string());
            summary
                .permission_operations
                .push(PermissionOperation::Write);
            summary.approval_required = true;
            summary.replay_safety = ReplaySafety::RequiresCheckpoint;
        }
        AISOperationType::Checkpoint => {
            summary.writes.push("checkpoint".to_string());
            summary
                .permission_operations
                .push(PermissionOperation::Write);
            summary.replay_safety = ReplaySafety::Safe;
        }
        _ => {}
    }

    summary
}
