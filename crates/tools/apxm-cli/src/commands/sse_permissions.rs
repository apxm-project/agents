//! Shared SSE permission prompt handling for chat and watch.

use anyhow::Result;
use apxm_client::{
    Client,
    events::{parse_approval_prompt, prompt_permission_decision},
    types::{PermissionResponse, PermissionResponseDecision},
};
use serde_json::Value as JsonValue;

/// If `json` carries an `approval_request`, prompt on the terminal and reply on
/// the server permission endpoint. Returns `true` when a prompt was handled.
pub async fn maybe_answer_permission(client: &Client, json: &JsonValue) -> Result<bool> {
    let Some(prompt) = parse_approval_prompt(json) else {
        return Ok(false);
    };
    let decision = prompt_permission_decision(&prompt)?;
    client
        .respond_permission(&prompt.approval_id, &PermissionResponse { decision })
        .await
        .map_err(|e| anyhow::anyhow!("permission respond failed: {e}"))?;
    if decision == PermissionResponseDecision::Deny {
        eprintln!("[permission] denied '{}'", prompt.tool_name);
    } else {
        eprintln!("[permission] approved '{}'", prompt.tool_name);
    }
    Ok(true)
}
