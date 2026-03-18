//! DAG splicing interface for runtime
//!
//! This module provides the interface for operation handlers to splice
//! inner plan DAGs into the live execution, enabling inner/outer plan unification.

use apxm_core::{error::RuntimeError, types::execution::ExecutionDag};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use apxm_core::types::{Node, NodeId, TokenId};

/// Result type for DAG splicing operations
pub type SpliceResult = Result<HashMap<TokenId, TokenId>, RuntimeError>;

/// Trait for splicing DAGs into live execution
///
/// This enables dynamic inner/outer plan unification where inner plan
/// DAGs generated during execution are merged into the currently executing
/// outer plan DAG.
#[async_trait]
pub trait DagSplicer: Send + Sync {
    /// Splice an inner DAG into the live execution
    ///
    /// # Arguments
    ///
    /// * `inner_dag` - The inner DAG to splice in
    /// * `token_connections` - Mapping from inner DAG input tokens to outer DAG output tokens
    ///
    /// # Returns
    ///
    /// Mapping from original inner DAG token IDs to remapped token IDs
    ///
    /// # Errors
    ///
    /// Returns error if splicing fails (e.g., invalid token connections)
    async fn splice_dag(
        &self,
        inner_dag: ExecutionDag,
        token_connections: HashMap<TokenId, TokenId>,
    ) -> SpliceResult;

    /// Mark tokens as delegated to a spliced sub-DAG
    ///
    /// When a token is marked as delegated by a node, that node's publish
    /// will be skipped, allowing the spliced sub-DAG to produce the actual value.
    ///
    /// # Arguments
    ///
    /// * `delegator_node_id` - The node that is delegating (e.g., the Switch node)
    /// * `token_ids` - The tokens to mark as delegated
    fn mark_tokens_delegated(&self, delegator_node_id: u64, token_ids: &[TokenId]);

    /// Condense a sub-DAG into a single replacement node.
    ///
    /// This is the reverse of `splice_dag`: it takes a set of node IDs that form
    /// a connected sub-graph, removes them, and inserts a single replacement node.
    /// External edges (edges crossing the boundary of the sub-graph) are reconnected
    /// to the replacement node.
    ///
    /// # Arguments
    ///
    /// * `node_ids` - The set of nodes to condense (must all exist in the DAG)
    /// * `replacement` - The single node that replaces the sub-graph
    ///
    /// # Returns
    ///
    /// `Ok(())` on success, or a `RuntimeError` if condensation fails.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Any node in `node_ids` does not exist
    /// - The operation is not supported in this context
    async fn condense_subdag(
        &self,
        node_ids: &[NodeId],
        replacement: Arc<Node>,
    ) -> Result<(), RuntimeError> {
        let _ = (node_ids, replacement);
        Err(RuntimeError::State(
            "DAG condensation not supported in this context".to_string(),
        ))
    }
}

/// No-op splicer for contexts that don't support dynamic splicing
pub struct NoOpSplicer;

#[async_trait]
impl DagSplicer for NoOpSplicer {
    async fn splice_dag(
        &self,
        _inner_dag: ExecutionDag,
        _token_connections: HashMap<TokenId, TokenId>,
    ) -> SpliceResult {
        Err(RuntimeError::State(
            "Dynamic DAG splicing not supported in this context".to_string(),
        ))
    }

    fn mark_tokens_delegated(&self, _delegator_node_id: u64, _token_ids: &[TokenId]) {
        // No-op: tokens cannot be delegated without scheduler state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::operations::AISOperationType;

    #[tokio::test]
    async fn test_noop_splicer() {
        let splicer = NoOpSplicer;
        let dag = ExecutionDag::new();
        let connections = HashMap::new();

        let result = splicer.splice_dag(dag, connections).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_noop_splicer_condense_returns_error() {
        let splicer = NoOpSplicer;
        let replacement = Arc::new(Node::new(99, AISOperationType::Return));
        let result = splicer.condense_subdag(&[1, 2], replacement).await;
        assert!(result.is_err());
    }
}
