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
    /// Splice an inner DAG into the live execution.
    async fn splice_dag(
        &self,
        inner_dag: ExecutionDag,
        token_connections: HashMap<TokenId, TokenId>,
    ) -> SpliceResult;

    /// Mark tokens as delegated to a spliced sub-DAG.
    ///
    /// The delegator node's publish is skipped; the spliced sub-DAG produces the actual value.
    fn mark_tokens_delegated(&self, delegator_node_id: u64, token_ids: &[TokenId]);

    /// Condense a sub-DAG into a single replacement node (reverse of `splice_dag`).
    ///
    /// Removes the given nodes and inserts a replacement, reconnecting external edges.
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

