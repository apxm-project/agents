//! COMMUNICATE operation - Inter-agent communication
//!
//! Supports four dispatch modes selected via the `protocol` node attribute:
//!   - `local` (default): in-process sub-flow execution via FlowRegistry
//!   - `http` / `https`: POST to an external APXM agent's `/v1/receive` endpoint
//!   - `acp`: send a prompt to an ACP agent subprocess via the ProcessTable
//!   - `broadcast`: fan-out to ALL registered agents in FlowRegistry in parallel;
//!     returns an Array of all responses (non-fatal errors included as strings)
//!
//! For HTTP, `recipient` may be a full URL or an agent name resolved via the
//! agent registry at `APXM_SERVER_URL/v1/agents/{name}`.
//!
//! For ACP, `recipient` must match an agent name previously spawned via
//! `SPAWN_AGENT` and registered in the ProcessTable.

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::types::CommunicateProtocol;

mod acp;
mod broadcast;
mod http;
mod local;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let recipient = get_string_attribute(node, graph_attrs::RECIPIENT).unwrap_or_default();
    let protocol = match get_string_attribute(node, graph_attrs::PROTOCOL) {
        Ok(raw) => raw
            .parse::<CommunicateProtocol>()
            .map_err(|e| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("COMMUNICATE has invalid 'protocol' value: {e}"),
            })?,
        Err(_) => CommunicateProtocol::Local,
    };
    let message = local::resolve_message(node, protocol, &inputs)?;

    // Emit a typed COMMUNICATE_DISPATCHED event so
    // observers can render the inter-agent edge without scraping the
    // op attributes. Excerpt is capped to ~240 chars (Codex pattern).
    if let Some(emitter) = &ctx.event_emitter {
        let message_str = match &message {
            Value::String(s) => Some(s.clone()),
            other => other.to_json().ok().map(|json| json.to_string()),
        };
        let excerpt = message_str.as_deref().map(truncate_excerpt);
        emitter.emit_communicate_dispatched(
            node.id,
            &recipient,
            protocol.as_str(),
            excerpt.as_deref(),
        );
    }

    match protocol {
        CommunicateProtocol::Http | CommunicateProtocol::Https => {
            http::execute_http(ctx, node, &recipient, message).await
        }
        CommunicateProtocol::Broadcast => broadcast::execute_broadcast(ctx, node, message).await,
        CommunicateProtocol::Acp => acp::execute_acp(ctx, node, &recipient, message).await,
        CommunicateProtocol::Local => local::execute_local(ctx, node, &recipient, message).await,
    }
}

/// Cap a message excerpt at 240 chars so observer streams don't haul
/// multi-KB payloads — matches Codex CLI's `evidence_excerpt` budget.
fn truncate_excerpt(text: &str) -> String {
    const MAX: usize = 240;
    if text.len() <= MAX {
        return text.to_string();
    }
    let mut end = MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

