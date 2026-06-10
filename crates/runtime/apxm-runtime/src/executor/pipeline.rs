//! Token pipelining support (research prototype, disabled by default).
//!
//! This module provides utilities for detecting and executing pipeline-eligible
//! node pairs where producer output can stream to consumer before completion.
//!
//! **Status**: Research feature, disabled by default.
//!
//! # Current limitations
//!
//! - Only supports ASK→ASK chains
//! - Requires {0} placeholder in consumer template
//! - No tools or structured output
//! - Single producer → single consumer
//!
//! # Future work
//!
//! - Actual streaming transport to backend providers
//! - Support for THINK/REASON ops
//! - Multi-consumer fan-out
//! - Integration with KV cache pinning

use apxm_core::types::{AISOperationType, Value};
use std::collections::HashMap;

const ATTR_PIPELINE_CANDIDATE: &str = "pipeline_candidate";
const ATTR_PIPELINE_CONSUMER_ID: &str = "pipeline_consumer_id";
const ATTR_PIPELINE_PRODUCER_ID: &str = "pipeline_producer_id";

/// Check if a node is pipeline-eligible based on graph attributes
pub fn is_pipeline_candidate(node_attrs: &HashMap<String, Value>) -> bool {
    node_attrs
        .get(ATTR_PIPELINE_CANDIDATE)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Get the consumer node ID for a pipeline producer
pub fn get_pipeline_consumer_id(node_attrs: &HashMap<String, Value>) -> Option<u64> {
    node_attrs
        .get(ATTR_PIPELINE_CONSUMER_ID)
        .and_then(|v| v.as_u64())
}

/// Get the producer node ID for a pipeline consumer
pub fn get_pipeline_producer_id(node_attrs: &HashMap<String, Value>) -> Option<u64> {
    node_attrs
        .get(ATTR_PIPELINE_PRODUCER_ID)
        .and_then(|v| v.as_u64())
}

/// Check if node is a pure LLM operation eligible for pipelining
pub fn is_pure_llm_op(op: &AISOperationType) -> bool {
    matches!(
        op,
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
    )
}

/// Is this a long-WAITING op that holds its worker while blocking on an external
/// event, rather than consuming CPU or an LLM slot?
///
/// Such ops do ~no compute while waiting, so the scheduler routes them to a
/// separate, generous concurrency pool — a burst of them must never exhaust the
/// compute or LLM permits and stall real work. PAUSE/RESUME are NOT here: they
/// PARK (return `OperationParked`, yielding their worker + permit immediately —
/// see the scheduler park path), so they hold nothing while waiting. Only
/// AUTONOMOUS in its `mode = "recv"` form still long-polls while holding a worker.
pub fn is_blocking_wait_op(node: &apxm_core::types::Node) -> bool {
    match node.op_type {
        AISOperationType::Autonomous => node
            .attributes
            .get("mode")
            .and_then(|v| v.as_str())
            .map(|m| m == "recv")
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(op: AISOperationType) -> apxm_core::types::Node {
        apxm_core::types::Node {
            id: 1,
            op_type: op,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        }
    }

    #[test]
    fn blocking_wait_ops_classified() {
        // PAUSE/RESUME PARK (yield their worker), so they are NOT blocking-pool ops.
        assert!(!is_blocking_wait_op(&node(AISOperationType::Pause)));
        assert!(!is_blocking_wait_op(&node(AISOperationType::Resume)));
        // Compute/LLM ops are not blocking-pool ops either.
        assert!(!is_blocking_wait_op(&node(AISOperationType::Ask)));
        assert!(!is_blocking_wait_op(&node(AISOperationType::InvTool)));
        // AUTONOMOUS holds its worker only in recv mode (it still long-polls).
        let mut recv = node(AISOperationType::Autonomous);
        assert!(
            !is_blocking_wait_op(&recv),
            "plain autonomous is compute/LLM-bound"
        );
        recv.attributes
            .insert("mode".to_string(), Value::String("recv".to_string()));
        assert!(
            is_blocking_wait_op(&recv),
            "autonomous mode=recv holds a worker while waiting"
        );
    }

    #[test]
    fn test_pipeline_candidate_detection() {
        let mut attrs = HashMap::new();
        assert!(!is_pipeline_candidate(&attrs));

        attrs.insert(ATTR_PIPELINE_CANDIDATE.to_string(), Value::Bool(true));
        assert!(is_pipeline_candidate(&attrs));

        attrs.insert(ATTR_PIPELINE_CANDIDATE.to_string(), Value::Bool(false));
        assert!(!is_pipeline_candidate(&attrs));
    }

    #[test]
    fn test_consumer_id_extraction() {
        let mut attrs = HashMap::new();
        assert_eq!(get_pipeline_consumer_id(&attrs), None);

        attrs.insert(
            ATTR_PIPELINE_CONSUMER_ID.to_string(),
            Value::Number(42.into()),
        );
        assert_eq!(get_pipeline_consumer_id(&attrs), Some(42));
    }

    #[test]
    fn test_pure_llm_op_check() {
        assert!(is_pure_llm_op(&AISOperationType::Ask));
        assert!(is_pure_llm_op(&AISOperationType::Think));
        assert!(is_pure_llm_op(&AISOperationType::Reason));
        assert!(!is_pure_llm_op(&AISOperationType::QMem));
        assert!(!is_pure_llm_op(&AISOperationType::InvTool));
    }
}
