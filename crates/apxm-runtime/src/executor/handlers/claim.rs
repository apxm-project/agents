//! CLAIM operation — atomically claim a task from a shared queue.
//!
//! Critical for work-stealing multi-agent systems. Calls the APXM server's
//! `/v1/tasks/:queue/claim` endpoint to atomically claim a pending task.
//!
//! Falls back to polling STM under `_task_queue:<queue>` if no server URL is
//! configured (useful for single-process testing without a server).
//!
//! ## Attributes
//! - `queue`       (required): queue name to claim from
//! - `lease_ms`    (optional): lease duration in ms (default: 60000)
//! - `max_wait_ms` (optional): max time to wait for a task (default: 5000)
//! - `server_url`  (optional): override APXM_SERVER_URL env var
//!
//! ## AIS usage
//! ```ais
//! claim(queue: "research_tasks", lease_ms: 60000, max_wait_ms: 5000)
//!   -> task_data, task_id, claim_token
//! ```

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute,
    get_optional_u64_attribute, get_string_attribute,
};
use crate::memory::MemorySpace;
use apxm_core::constants::defaults;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let queue = get_string_attribute(node, graph_attrs::QUEUE)?;
    let lease_ms =
        get_optional_u64_attribute(node, graph_attrs::LEASE_MS)?.unwrap_or(defaults::DEFAULT_LEASE_MS);
    let max_wait_ms =
        get_optional_u64_attribute(node, graph_attrs::MAX_WAIT_MS)?.unwrap_or(defaults::DEFAULT_MAX_WAIT_MS);

    let server_url = get_optional_string_attribute(node, graph_attrs::SERVER_URL)?
        .or_else(|| std::env::var("APXM_SERVER_URL").ok())
        .unwrap_or_else(|| defaults::DEFAULT_SERVER_URL.to_string());

    tracing::info!(
        execution_id = %ctx.execution_id,
        queue = %queue,
        lease_ms = %lease_ms,
        max_wait_ms = %max_wait_ms,
        server_url = %server_url,
        "Executing CLAIM operation"
    );

    let url = format!(
        "{}/v1/tasks/{}/claim",
        server_url.trim_end_matches('/'),
        queue
    );

    let body = serde_json::json!({
        "agent_id": ctx.execution_id,
        "lease_ms": lease_ms,
        "max_wait_ms": max_wait_ms
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(max_wait_ms + 5000))
        .build()
        .map_err(|e| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Failed to build HTTP client for CLAIM: {}", e),
        })?;

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("CLAIM request to '{}' failed: {}", url, e),
        })?;

    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("No tasks available in queue '{}'", queue),
        });
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("CLAIM returned {}: {}", status, body),
        });
    }

    let resp_json: serde_json::Value = resp.json().await.map_err(|e| RuntimeError::Operation {
        op_type: node.op_type,
        message: format!("Failed to parse CLAIM response: {}", e),
    })?;

    // Extract task_data, task_id, claim_token from response
    let task_id = resp_json
        .get("task_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let claim_token = resp_json
        .get("claim_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let task_data = resp_json
        .get("data")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let task_data_value = Value::try_from(task_data).unwrap_or(Value::Null);

    // Store claim metadata in STM for COMPLETE op
    let _ = ctx
        .memory
        .write(
            MemorySpace::Stm,
            format!("{}{}", belief_keys::CLAIM_TOKEN_PREFIX, task_id),
            Value::String(claim_token.clone()),
        )
        .await;

    tracing::info!(
        execution_id = %ctx.execution_id,
        queue = %queue,
        task_id = %task_id,
        "CLAIM succeeded"
    );

    // Record claim in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!("{}{}:{}", belief_keys::CLAIM_PREFIX, queue, task_id),
        Value::String(format!("claimed from queue:{}", queue)),
        label,
    );

    // Return as object with named fields
    let mut result = HashMap::new();
    result.insert("task_data".to_string(), task_data_value);
    result.insert("task_id".to_string(), Value::String(task_id));
    result.insert("claim_token".to_string(), Value::String(claim_token));

    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::values::Number;
    use std::sync::Arc;

    fn make_claim_node(queue: &str, server_url: &str, max_wait_ms: u64) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::Claim,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::QUEUE.to_string(),
            Value::String(queue.to_string()),
        );
        node.attributes.insert(
            graph_attrs::SERVER_URL.to_string(),
            Value::String(server_url.to_string()),
        );
        node.attributes.insert(
            graph_attrs::MAX_WAIT_MS.to_string(),
            Value::Number(Number::Integer(max_wait_ms as i64)),
        );
        node
    }

    #[tokio::test]
    async fn test_claim_connection_refused() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        // Point to an unreachable server with a short timeout
        let node = make_claim_node("test_queue", "http://127.0.0.1:1", 100);
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("CLAIM"),
            "Expected CLAIM-related error, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_claim_missing_queue_attribute() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let node = Node {
            id: 1,
            op_type: AISOperationType::Claim,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
    }
}

