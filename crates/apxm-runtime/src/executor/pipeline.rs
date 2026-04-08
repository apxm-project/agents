//! Token pipelining support (Phase 4 research prototype).
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
//! - Actual streaming transport to vLLM
//! - Support for THINK/REASON ops
//! - Multi-consumer fan-out
//! - Integration with KV cache pinning

use apxm_core::types::{AISOperationType, Value};
use std::collections::HashMap;

/// Check if a node is pipeline-eligible based on graph attributes
pub fn is_pipeline_candidate(node_attrs: &HashMap<String, Value>) -> bool {
    node_attrs
        .get("pipeline_candidate")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Get the consumer node ID for a pipeline producer
pub fn get_pipeline_consumer_id(node_attrs: &HashMap<String, Value>) -> Option<u64> {
    node_attrs
        .get("pipeline_consumer_id")
        .and_then(|v| v.as_u64())
}

/// Get the producer node ID for a pipeline consumer
pub fn get_pipeline_producer_id(node_attrs: &HashMap<String, Value>) -> Option<u64> {
    node_attrs
        .get("pipeline_producer_id")
        .and_then(|v| v.as_u64())
}

/// Check if node is a pure LLM operation eligible for pipelining
pub fn is_pure_llm_op(op: &AISOperationType) -> bool {
    matches!(
        op,
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_candidate_detection() {
        let mut attrs = HashMap::new();
        assert!(!is_pipeline_candidate(&attrs));

        attrs.insert("pipeline_candidate".to_string(), Value::Bool(true));
        assert!(is_pipeline_candidate(&attrs));

        attrs.insert("pipeline_candidate".to_string(), Value::Bool(false));
        assert!(!is_pipeline_candidate(&attrs));
    }

    #[test]
    fn test_consumer_id_extraction() {
        let mut attrs = HashMap::new();
        assert_eq!(get_pipeline_consumer_id(&attrs), None);

        attrs.insert(
            "pipeline_consumer_id".to_string(),
            Value::Number(42.into()),
        );
        assert_eq!(get_pipeline_consumer_id(&attrs), Some(42));
    }

    #[test]
    fn test_pure_llm_op_check() {
        assert!(is_pure_llm_op(&AISOperationType::Ask));
        assert!(is_pure_llm_op(&AISOperationType::Think));
        assert!(is_pure_llm_op(&AISOperationType::Reason));
        assert!(!is_pure_llm_op(&AISOperationType::QMEM));
        assert!(!is_pure_llm_op(&AISOperationType::Inv));
    }
}
