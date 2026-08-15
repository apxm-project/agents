//! AIS operation helpers used by validate/analyze/ops.

use apxm_core::types::{AIS_OPERATIONS, OperationCategory, OperationLatency, OperationSpec};

pub(crate) fn category_str(cat: OperationCategory) -> &'static str {
    match cat {
        OperationCategory::Semantic => "semantic",
        OperationCategory::Metadata => "metadata",
        OperationCategory::Memory => "memory",
        OperationCategory::Reasoning => "reasoning",
        OperationCategory::Tools => "tools",
        OperationCategory::ControlFlow => "control_flow",
        OperationCategory::Synchronization => "synchronization",
        OperationCategory::ErrorHandling => "error_handling",
        OperationCategory::Communication => "communication",
        OperationCategory::Internal => "internal",
        OperationCategory::Coordination => "coordination",
        OperationCategory::Identity => "identity",
    }
}

pub(crate) fn find_op_spec(op: &str) -> Option<&'static OperationSpec> {
    AIS_OPERATIONS
        .iter()
        .find(|spec| spec.op_type.to_string() == op)
}

pub(crate) fn op_latency_ms(op: &str) -> u64 {
    find_op_spec(op).map_or(100, |spec| match spec.latency {
        OperationLatency::None => 10,
        OperationLatency::Low => 100,
        OperationLatency::Medium => 1000,
        OperationLatency::High => 5000,
    })
}
