//! PAUSE operation — suspend execution and await human-in-the-loop input.
//!
//! Creates a checkpoint at the APXM server, then polls until the checkpoint
//! is resumed via `POST /v1/checkpoints/:id/resume`. The human_input from
//! that resume call is returned as the output value.
//!
//! The agent's current execution is blocked (async polling) until resumed
//! or until `timeout_ms` elapses.
//!
//! ## Attributes
//! - `message`          (required): human-readable message explaining the pause
//! - `checkpoint_id`    (optional): stable ID (auto-generated UUID if omitted)
//! - `timeout_ms`       (optional): max wait time in ms (default: 0 = indefinite)
//! - `poll_interval_ms` (optional): polling interval (default: 2000)
//! - `notification_url` (optional): webhook to fire on checkpoint creation
//! - `server_url`       (optional): override APXM_SERVER_URL
//!
//! ## AIS usage
//! ```ais
//! pause(
//!   message: "Please review findings before proceeding",
//!   checkpoint_id: "human_review_1",
//!   timeout_ms: 3600000,
//!   notification_url: "https://hooks.slack.com/..."
//! ) <- findings -> human_input
//! ```

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute,
    get_optional_u64_attribute, get_string_attribute,
};
use apxm_core::constants::defaults;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::error::RuntimeError;

const DEFAULT_POLL_INTERVAL_MS: u64 = 2_000;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let message = get_string_attribute(node, graph_attrs::MESSAGE)?;

    let checkpoint_id = get_optional_string_attribute(node, graph_attrs::CHECKPOINT_ID)?
        .unwrap_or_else(|| format!("chk_{}", &uuid::Uuid::new_v4().to_string()[..8]));

    let timeout_ms = get_optional_u64_attribute(node, graph_attrs::TIMEOUT_MS)?.unwrap_or(0);
    let poll_interval_ms = get_optional_u64_attribute(node, graph_attrs::POLL_INTERVAL_MS)?
        .unwrap_or(DEFAULT_POLL_INTERVAL_MS);
    let notification_url = get_optional_string_attribute(node, graph_attrs::NOTIFICATION_URL)?;

    let server_url = get_optional_string_attribute(node, graph_attrs::SERVER_URL)?
        .or_else(|| std::env::var("APXM_SERVER_URL").ok())
        .unwrap_or_else(|| defaults::DEFAULT_SERVER_URL.to_string());

    let base_url = server_url.trim_end_matches('/');

    // Serialize the input (display_data for human review)
    let display_data = inputs
        .first()
        .and_then(|v| v.to_json().ok())
        .unwrap_or(serde_json::Value::Null);

    tracing::info!(
        execution_id = %ctx.execution_id,
        checkpoint_id = %checkpoint_id,
        message = %message,
        timeout_ms = %timeout_ms,
        "Executing PAUSE — creating checkpoint"
    );

    // Record pause in AAM
    let label = crate::aam::TransitionLabel::operation(node.id, node.op_type);
    ctx.aam.set_belief(
        format!("{}{}", belief_keys::PAUSE_PREFIX, checkpoint_id),
        Value::String(message.clone()),
        label,
    );

    // 1. Create checkpoint at the server
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Failed to build HTTP client for PAUSE: {}", e),
        })?;

    let create_url = format!("{}/v1/checkpoints", base_url);
    let mut create_body = serde_json::json!({
        "checkpoint_id": checkpoint_id,
        "message": message,
        "display_data": display_data
    });
    if let Some(ref url) = notification_url {
        create_body["notification_url"] = serde_json::Value::String(url.clone());
    }

    let create_resp = client
        .post(&create_url)
        .json(&create_body)
        .send()
        .await
        .map_err(|e| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("PAUSE failed to create checkpoint: {}", e),
        })?;

    if !create_resp.status().is_success() {
        let body = create_resp.text().await.unwrap_or_default();
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("PAUSE checkpoint creation failed: {}", body),
        });
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        checkpoint_id = %checkpoint_id,
        "Checkpoint created — polling for resume"
    );

    // 2. Poll until resumed or timeout
    let get_url = format!("{}/v1/checkpoints/{}", base_url, checkpoint_id);
    let poll_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Failed to build poll client: {}", e),
        })?;

    let start_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    loop {
        tokio::time::sleep(std::time::Duration::from_millis(poll_interval_ms)).await;

        // Check timeout
        if timeout_ms > 0 {
            let elapsed_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64
                - start_ms;
            if elapsed_ms >= timeout_ms {
                return Err(RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!(
                        "PAUSE checkpoint '{}' timed out after {}ms",
                        checkpoint_id, timeout_ms
                    ),
                });
            }
        }

        // Poll checkpoint status
        let poll_resp = match poll_client.get(&get_url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    checkpoint_id = %checkpoint_id,
                    error = %e,
                    "Checkpoint poll request failed — retrying"
                );
                continue;
            }
        };

        if !poll_resp.status().is_success() {
            tracing::warn!(
                checkpoint_id = %checkpoint_id,
                status = %poll_resp.status(),
                "Checkpoint poll returned non-success — retrying"
            );
            continue;
        }

        let cp_json: serde_json::Value = match poll_resp.json().await {
            Ok(j) => j,
            Err(_) => continue,
        };

        let status = cp_json
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("pending");

        match status {
            "resumed" => {
                let human_input = cp_json
                    .get("human_input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);

                tracing::info!(
                    execution_id = %ctx.execution_id,
                    checkpoint_id = %checkpoint_id,
                    "PAUSE resumed with human input"
                );

                return Value::try_from(human_input).map_err(|e| RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!(
                        "Failed to convert human_input from checkpoint '{}': {}",
                        checkpoint_id, e
                    ),
                });
            }
            "expired" => {
                return Err(RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!("Checkpoint '{}' expired", checkpoint_id),
                });
            }
            "pending" => {
                tracing::debug!(
                    checkpoint_id = %checkpoint_id,
                    "Checkpoint still pending — continuing to poll"
                );
                // Continue polling
            }
            other => {
                tracing::warn!(
                    checkpoint_id = %checkpoint_id,
                    status = %other,
                    "Unknown checkpoint status"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use std::sync::Arc;

    fn make_pause_node(message: &str, server_url: &str) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::Pause,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::MESSAGE.to_string(),
            Value::String(message.to_string()),
        );
        node.attributes.insert(
            graph_attrs::SERVER_URL.to_string(),
            Value::String(server_url.to_string()),
        );
        node.attributes.insert(
            graph_attrs::CHECKPOINT_ID.to_string(),
            Value::String("test_checkpoint".to_string()),
        );
        node
    }

    #[tokio::test]
    async fn test_pause_records_aam_belief_before_http() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        // Use an unreachable server URL so the HTTP call will fail
        let node = make_pause_node("Review findings", "http://127.0.0.1:1");
        let result = execute(
            &ctx,
            &node,
            vec![Value::String("findings data".to_string())],
        )
        .await;

        // The HTTP request to port 1 should fail
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("PAUSE"),
            "Expected PAUSE-related error, got: {}",
            err_msg
        );

        // But the AAM belief should have been recorded before the HTTP call
        let beliefs = aam.beliefs();
        let key = format!("{}test_checkpoint", belief_keys::PAUSE_PREFIX);
        assert_eq!(
            beliefs.get(&key),
            Some(&Value::String("Review findings".to_string()))
        );
    }

    #[tokio::test]
    async fn test_pause_missing_message_attribute() {
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
            op_type: AISOperationType::Pause,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
    }
}
