//! AAM-to-ACP bridge: renders projected AAM state into formats ACP agents understand.

use apxm_core::constants::runtime::belief_keys;
use apxm_core::types::aam::AamContext;

use crate::constants::{args, fields};
use crate::registry::CapabilityServerConfig;

/// Render AAM context as a structured system prompt preamble.
///
/// Returns `None` if all three dimensions (beliefs, goals, capabilities) are empty.
pub fn render_system_prompt(ctx: &AamContext) -> Option<String> {
    let system_prompt = ctx
        .system_prompt
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    let visible_beliefs: Vec<_> = ctx
        .beliefs
        .iter()
        .filter(|(k, _)| !k.starts_with(belief_keys::INTERNAL_PREFIX))
        .collect();

    if system_prompt.is_none()
        && visible_beliefs.is_empty()
        && ctx.goals.is_empty()
        && ctx.capabilities.is_empty()
    {
        return None;
    }

    let mut sections = Vec::new();

    if let Some(text) = system_prompt {
        sections.push(text.to_string());
    }

    if !visible_beliefs.is_empty() {
        let mut belief_lines = vec!["### Beliefs".to_string()];
        for (key, value) in &visible_beliefs {
            belief_lines.push(format!("- {key}: {value}"));
        }
        sections.push(belief_lines.join("\n"));
    }

    if !ctx.goals.is_empty() {
        let mut goal_lines = vec!["### Goals (by priority)".to_string()];
        let mut sorted_goals = ctx.goals.clone();
        sorted_goals.sort_by_key(|goal| std::cmp::Reverse(goal.priority));
        for (i, goal) in sorted_goals.iter().enumerate() {
            goal_lines.push(format!(
                "{}. [P:{}] {}",
                i + 1,
                goal.priority,
                goal.description
            ));
        }
        sections.push(goal_lines.join("\n"));
    }

    if !ctx.capabilities.is_empty() {
        let mut cap_lines = vec!["### Available Capabilities".to_string()];
        for cap in &ctx.capabilities {
            cap_lines.push(format!("- {}: {}", cap.name, cap.description));
        }
        sections.push(cap_lines.join("\n"));
    }

    Some(format!(
        "## Context (from APXM controller)\n\n{}",
        sections.join("\n\n")
    ))
}

/// Build the `session/new` JSON params, rendering capabilities as `mcpServers`
/// on the wire and including `cwd`.
///
/// Note: `mcpServers` is the ACP wire key, even though we call them capabilities in APXM.
pub fn render_session_params(
    cwd: &std::path::Path,
    capabilities: &[CapabilityServerConfig],
) -> serde_json::Value {
    let mcp_servers_val =
        serde_json::to_value(capabilities).unwrap_or_else(|_| serde_json::Value::Array(vec![]));
    serde_json::json!({
        args::CWD: cwd.to_string_lossy(),
        (fields::MCP_SERVERS): mcp_servers_val,
    })
}
