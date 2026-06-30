//! HTTP COMMUNICATE: dispatch over HTTP to an external APXM agent's
//! `/v1/receive` endpoint, optionally resolving the recipient name via the
//! agent registry.

use super::super::{ExecutionContext, Node, Result, Value};
use crate::capability::builtins::{client_for, guard_url_ssrf_pinned, shared_client};
use apxm_core::error::RuntimeError;

/// Dispatch COMMUNICATE over HTTP to an external APXM agent.
///
/// `recipient` may be:
///   - A full URL: `http://host:port` — used directly as base URL.
///   - An agent name: looked up via `APXM_SERVER_URL/v1/agents/{name}`.
pub(super) async fn execute_http(
    ctx: &ExecutionContext,
    node: &Node,
    recipient: &str,
    message: Value,
) -> Result<Value> {
    let op_err = |msg: String| RuntimeError::Operation {
        op_type: node.op_type,
        message: msg,
    };

    let base_url = if recipient.starts_with("http://") || recipient.starts_with("https://") {
        recipient.to_string()
    } else {
        let server_url = apxm_core::env::server_url();
        let lookup_url = format!("{}/v1/agents/{}", server_url, recipient);
        // Shared hardened client (caps redirects + drops blocked-IP-literal
        // hops). Keep the lookup snappy with a per-request 10s timeout.
        let resp = shared_client()
            .get(&lookup_url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| op_err(format!("Agent registry lookup failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(op_err(format!(
                "Agent '{}' not found in registry ({})",
                recipient,
                resp.status()
            )));
        }
        let agent_info: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| op_err(format!("Failed to parse agent info: {e}")))?;
        agent_info["url"]
            .as_str()
            .ok_or_else(|| op_err(format!("Agent '{}' has no 'url' field", recipient)))?
            .to_string()
    };
    let msg_json = message
        .to_json()
        .map_err(|e| op_err(format!("Failed to serialize message: {e}")))?;

    let receive_url = format!("{}/v1/receive", base_url.trim_end_matches('/'));
    // Resolve + vet the recipient host ONCE, then pin the connect to those exact
    // addresses so a name rebound to a blocked IP between the guard and the send
    // cannot be reached (DNS-rebind TOCTOU). The guard runs against the full
    // receive URL so the vetted host matches the host we actually connect to.
    let pinned = guard_url_ssrf_pinned("communicate.http", &receive_url)
        .await
        .map_err(|error| op_err(error.to_string()))?;
    tracing::info!(
        execution_id = %ctx.execution_id,
        recipient = %recipient,
        url = %receive_url,
        "COMMUNICATE HTTP dispatch"
    );

    let client = client_for(&receive_url, &pinned);
    let resp = client
        .post(&receive_url)
        .json(&serde_json::json!({
            "from": ctx.execution_id,
            "message": msg_json
        }))
        .send()
        .await
        .map_err(|e| op_err(format!("HTTP COMMUNICATE to '{}' failed: {e}", recipient)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(op_err(format!(
            "HTTP COMMUNICATE '{}' returned {}: {}",
            recipient, status, text
        )));
    }

    let result: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| op_err(format!("Failed to parse COMMUNICATE response: {e}")))?;

    Value::try_from(result).map_err(|e| op_err(format!("Failed to convert response: {e}")))
}
