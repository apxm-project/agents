//! RESUME operation — wait for a PAUSE checkpoint to be resumed.
//!
//! PARKS (yields its worker lane + concurrency permit) on the named checkpoint
//! id. When a human resumes it (`POST /v1/checkpoints/{id}/resume`), the server
//! calls `park_registry::wake(checkpoint_id, human_input)`, which delivers the
//! `human_input` as this node's output for downstream nodes to consume — the
//! same event-driven mechanism PAUSE uses, with no polling and no held worker.
//!
//! ## AIS usage
//! ```ais
//! pause(checkpoint: "review_plan", message: "Please review: " + plan) -> _
//! resume(checkpoint: "review_plan") -> human_input
//! ask("Implement approved plan. Human notes: " + human_input) -> code
//! ```

use super::{ExecutionContext, Node, Result, Value};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::error::RuntimeError;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let checkpoint_id = node
        .attributes
        .get(graph_attrs::CHECKPOINT)
        .and_then(|v| v.as_string().map(|s| s.to_string()))
        .ok_or_else(|| RuntimeError::Operation {
            op_type: node.op_type,
            message: "RESUME requires a `checkpoint` attribute".to_string(),
        })?;

    // Record the resume intent in AAM before parking (preserved from the old
    // polling path; the wake delivers the value, this is just bookkeeping).
    let label = crate::aam::TransitionLabel::operation(node.id, node.op_type);
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::RESUME_PREFIX, checkpoint_id),
        Value::String("resumed".to_string()),
        label,
    );

    tracing::info!(
        execution_id = %ctx.execution_id,
        checkpoint_id = %checkpoint_id,
        "RESUME parking until checkpoint is resumed"
    );

    // Park: the scheduler yields this worker + permit and re-injects the node
    // (with the human_input as its output) when the checkpoint is resumed.
    Err(RuntimeError::OperationParked {
        wait_key: checkpoint_id,
    })
}

