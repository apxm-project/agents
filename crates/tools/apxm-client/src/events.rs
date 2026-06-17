//! SSE event helpers for permission prompts (spec 0002 US4/US7).

use serde_json::Value as JsonValue;

use crate::types;

/// Parsed server permission prompt from an SSE `ApxmEvent` JSON frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalPrompt {
    pub approval_id: String,
    pub tool_name: String,
    pub agent_code: String,
    pub risk_level: String,
}

/// Extract event kind from either a bare payload or `{ meta, payload }` envelope.
pub fn event_kind(json: &JsonValue) -> Option<&str> {
    json.get("kind")
        .and_then(|v| v.as_str())
        .or_else(|| json.pointer("/payload/kind").and_then(|v| v.as_str()))
}

/// Detect an `approval_request` frame and return the prompt fields.
pub fn parse_approval_prompt(json: &JsonValue) -> Option<ApprovalPrompt> {
    if event_kind(json) != Some("approval_request") {
        return None;
    }
    let payload = json.get("payload").unwrap_or(json);
    Some(ApprovalPrompt {
        approval_id: payload.get("approval_id")?.as_str()?.to_string(),
        tool_name: payload.get("tool_name")?.as_str()?.to_string(),
        agent_code: payload
            .get("agent_code")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        risk_level: payload
            .get("risk_level")
            .and_then(|v| v.as_str())
            .unwrap_or("medium")
            .to_string(),
    })
}

/// Blocking terminal prompt for a permission decision (host pipe only).
pub fn prompt_permission_decision(
    prompt: &ApprovalPrompt,
) -> std::io::Result<types::PermissionResponseDecision> {
    use std::io::Write as _;
    eprintln!(
        "\n[permission] tool '{}' requested by {} (risk={})",
        prompt.tool_name, prompt.agent_code, prompt.risk_level
    );
    eprint!("  approve [y], deny [n], approve for session [s]? [y/N/s] ");
    let _ = std::io::stderr().flush();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(match answer.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => types::PermissionResponseDecision::Approve,
        "s" | "session" | "approve_for_session" => {
            types::PermissionResponseDecision::ApproveForSession
        }
        _ => types::PermissionResponseDecision::Deny,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_enveloped_approval_request() {
        let json = serde_json::json!({
            "meta": { "seq": 1 },
            "payload": {
                "kind": "approval_request",
                "approval_id": "perm-abc",
                "tool_name": "write_file",
                "agent_code": "sess-1",
                "risk_level": "medium"
            }
        });
        let prompt = parse_approval_prompt(&json).expect("prompt");
        assert_eq!(prompt.approval_id, "perm-abc");
        assert_eq!(prompt.tool_name, "write_file");
    }
}
