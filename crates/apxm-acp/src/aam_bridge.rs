//! AAM-to-ACP bridge: renders projected AAM state into formats ACP agents understand.

use apxm_core::constants::acp::session_params;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::types::aam::AamContext;

use crate::registry::CapabilityServerConfig;

/// Render AAM context as a structured system prompt preamble.
///
/// Returns `None` if all three dimensions (beliefs, goals, capabilities) are empty.
pub fn render_system_prompt(ctx: &AamContext) -> Option<String> {
    let visible_beliefs: Vec<_> = ctx
        .beliefs
        .iter()
        .filter(|(k, _)| !k.starts_with(belief_keys::INTERNAL_PREFIX))
        .collect();

    if visible_beliefs.is_empty() && ctx.goals.is_empty() && ctx.capabilities.is_empty() {
        return None;
    }

    let mut sections = Vec::new();

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
        sorted_goals.sort_by(|a, b| b.priority.cmp(&a.priority));
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
        "## Context (from APXM orchestrator)\n\n{}",
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
        session_params::CWD: cwd.to_string_lossy(),
        (session_params::MCP_SERVERS): mcp_servers_val,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::aam::{CapabilityProjection, GoalProjection};

    #[test]
    fn empty_context_returns_none() {
        let ctx = AamContext::default();
        assert!(render_system_prompt(&ctx).is_none());
    }

    #[test]
    fn renders_beliefs() {
        let mut ctx = AamContext::default();
        ctx.beliefs.insert(
            "user".to_string(),
            serde_json::Value::String("Alice".to_string()),
        );
        let result = render_system_prompt(&ctx).unwrap();
        assert!(result.contains("### Beliefs"));
        assert!(result.contains("user"));
        assert!(result.contains("Alice"));
    }

    #[test]
    fn filters_internal_beliefs() {
        let mut ctx = AamContext::default();
        ctx.beliefs.insert(
            "_internal".to_string(),
            serde_json::Value::String("hidden".to_string()),
        );
        assert!(render_system_prompt(&ctx).is_none());

        // Add a visible belief — internal one should still be absent
        ctx.beliefs.insert(
            "visible".to_string(),
            serde_json::Value::String("shown".to_string()),
        );
        let result = render_system_prompt(&ctx).unwrap();
        assert!(result.contains("visible"));
        assert!(!result.contains("_internal"));
    }

    #[test]
    fn renders_goals_sorted_by_priority() {
        let mut ctx = AamContext::default();
        ctx.goals.push(GoalProjection {
            description: "low priority".to_string(),
            priority: 10,
        });
        ctx.goals.push(GoalProjection {
            description: "high priority".to_string(),
            priority: 90,
        });
        let result = render_system_prompt(&ctx).unwrap();
        assert!(result.contains("### Goals (by priority)"));
        // High priority should come first
        let high_pos = result.find("high priority").unwrap();
        let low_pos = result.find("low priority").unwrap();
        assert!(high_pos < low_pos);
    }

    #[test]
    fn renders_capabilities() {
        let mut ctx = AamContext::default();
        ctx.capabilities.push(CapabilityProjection {
            name: "web_search".to_string(),
            description: "Search the web".to_string(),
        });
        let result = render_system_prompt(&ctx).unwrap();
        assert!(result.contains("### Available Capabilities"));
        assert!(result.contains("web_search: Search the web"));
    }

    #[test]
    fn session_params_has_mcp_servers_wire_key() {
        let cwd = std::path::Path::new("/tmp/test");
        let caps = vec![];
        let params = render_session_params(cwd, &caps);
        assert!(params.get(session_params::MCP_SERVERS).is_some());
        assert!(params.get(session_params::CWD).is_some());
    }

    #[test]
    fn session_params_includes_capabilities() {
        let cwd = std::path::Path::new("/tmp/test");
        let caps = vec![CapabilityServerConfig {
            name: "test-mcp".to_string(),
            command: "npx test-mcp".to_string(),
            args: vec![],
            env: Default::default(),
        }];
        let params = render_session_params(cwd, &caps);
        let servers = params
            .get(session_params::MCP_SERVERS)
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["name"], "test-mcp");
    }
}
