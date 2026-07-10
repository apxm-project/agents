//! Shared conversational-chat primitives — transcript rendering shared by the
//! `apxm chat` CLI and the apxm-studio backend so they stay identical.
//!
//! Compaction is NOT this crate's contract: `KEEP_RECENT_TURNS`/
//! `COMPACT_AT_TOKENS`/`estimate_tokens`/`SUMMARIZE_AIR` used to live here as
//! a chars/4-estimate duplicate with zero call sites (dead policy, never
//! wired to the dumb-pipe chat host — see `commands/chat.rs`'s doc comment).
//! They were deleted once the runtime default
//! (`ConversationMemoryMiddleware`, `apxm-runtime` crate) reached parity —
//! see `docs/plans/tasks/W2.7.md` and
//! `apxm_core::constants::runtime::conversation_compaction` for the single
//! source of truth now.

use crate::capabilities::groups;

/// A chat role in the rendered transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    /// The capitalized label used in the transcript (`User`, `Assistant`, `System`).
    pub fn label(self) -> &'static str {
        match self {
            Role::System => "System",
            Role::User => "User",
            Role::Assistant => "Assistant",
        }
    }

    /// Parse a wire role string; anything unrecognized is treated as `User`.
    pub fn parse(s: &str) -> Role {
        match s {
            "system" => Role::System,
            "assistant" => Role::Assistant,
            _ => Role::User,
        }
    }
}

/// Render a message list into the flat transcript the conversational graph
/// consumes as its single `conversation` argument: each turn labeled by role,
/// then a trailing open `Assistant:` cue for the model to complete. An empty
/// list yields just the cue.
pub fn render_transcript<'a, I>(messages: I) -> String
where
    I: IntoIterator<Item = (Role, &'a str)>,
{
    let mut out = String::new();
    for (role, content) in messages {
        out.push_str(role.label());
        out.push_str(": ");
        out.push_str(content);
        out.push('\n');
    }
    out.push_str("Assistant:");
    out
}

/// Escape a string for safe interpolation into an AIR string literal.
pub fn escape_air_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Keep a routing identifier to the characters real backend/model ids use, so it
/// is always a safe MLIR string literal when interpolated into AIR.
pub fn sanitize_route_id(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/'))
        .collect()
}

/// Per-turn routing/tool options for the built-in conversational graph.
#[derive(Debug, Default, Clone, Copy)]
pub struct ChatAirOptions<'a> {
    /// System prompt for this turn. `None` falls through to the runtime default.
    pub system_prompt: Option<&'a str>,
    /// Pin this turn to a registered backend (the runtime's per-node `backend`
    /// attr). `GET /v1/models` returns backend names, so a picker value is a
    /// backend, not a model id.
    pub backend: Option<&'a str>,
    /// Pin this turn to a specific model (the per-node `model` attr).
    pub model: Option<&'a str>,
    /// Extended-thinking effort for this turn (`low`/`medium`/`high`); the
    /// runtime lowers it into a thinking token budget. `None` or `off` = no
    /// extended thinking.
    pub effort: Option<&'a str>,
    /// Add the self-enabling `web` tool group, making the reply a tool-using
    /// turn (the runtime runs independent tool calls in parallel).
    pub tools: bool,
    /// Expose the `skills` tool group so the agent can call `search_skills` to
    /// discover relevant skills by description (scoped to its visible set).
    pub skills: bool,
    /// Expose runtime capability discovery so the agent can inspect
    /// authoring-time capability templates without receiving authority.
    pub capability_discovery: bool,
    /// Expose the `authoring` tool group (`compose_workflow` / `run_workflow`) so
    /// the agent can create and run workflows. These are write-class but
    /// capability_grant_ids-gated and staging-confined (workflow-scoped admission).
    pub authoring: bool,
}

/// Per-turn routing options for a conversational ACP agent graph.
#[derive(Debug, Clone, Copy)]
pub struct ChatAcpAirOptions<'a> {
    /// ACP profile to spawn, for example `claude`.
    pub profile: &'a str,
    /// System prompt prepended to the rendered transcript.
    pub system_prompt: Option<&'a str>,
    /// Optional ACP mode set during spawn, for example `architect`.
    pub mode: Option<&'a str>,
    /// Optional ACP model requested during spawn when the profile supports it.
    pub model: Option<&'a str>,
}

/// Build the built-in single-ASK conversational graph. With default options the
/// output is the canonical bare chat graph (one ASK over the `conversation`
/// param); `backend`/`model`/`tools` ride the ASK attr-dict the runtime reads.
/// `{{{conversation}}}` is the parameter placeholder (single-pass substitution).
pub fn chat_air(opts: &ChatAirOptions) -> String {
    let mut attrs: Vec<String> = Vec::new();
    if let Some(sp) = opts.system_prompt
        && !sp.is_empty()
    {
        attrs.push(format!("system_prompt = \"{}\"", escape_air_string(sp)));
    }
    if let Some(b) = opts.backend {
        let safe = sanitize_route_id(b);
        if !safe.is_empty() {
            attrs.push(format!("backend = \"{safe}\""));
        }
    }
    if let Some(m) = opts.model {
        let safe = sanitize_route_id(m);
        if !safe.is_empty() {
            attrs.push(format!("model = \"{safe}\""));
        }
    }
    if let Some(e) = opts.effort {
        let safe = sanitize_route_id(e);
        if !safe.is_empty() && safe != "off" {
            attrs.push(format!("effort = \"{safe}\""));
        }
    }
    let mut groups: Vec<&str> = Vec::new();
    if opts.tools {
        groups.push(groups::WEB);
    }
    if opts.capability_discovery {
        groups.push(groups::DISCOVERY);
    }
    if opts.skills {
        groups.push(groups::SKILLS);
    }
    if opts.authoring {
        groups.push(groups::AUTHORING);
    }
    if !groups.is_empty() {
        let list = groups
            .iter()
            .map(|g| format!("\"{g}\""))
            .collect::<Vec<_>>()
            .join(", ");
        attrs.push(format!("capability_groups = [{list}]"));
    }
    let attr_dict = if attrs.is_empty() {
        String::new()
    } else {
        format!(" {{{}}}", attrs.join(", "))
    };

    let mut air = String::new();
    air.push_str("module {\n");
    air.push_str("  func.func @apxm_chat(%arg0: !ais.token {ais.param_name = \"conversation\", ais.param_type = \"str\"}) -> !ais.token attributes {ais.entry} {\n");
    air.push_str("    %reply = ais.ask \"{{{conversation}}}\"");
    air.push_str(&attr_dict);
    air.push_str(" : !ais.token\n");
    air.push_str("    func.return %reply : !ais.token\n");
    air.push_str("  }\n}\n");
    air
}

/// Build a conversational graph that spawns one ACP profile and sends the
/// rendered transcript to it. The CLI still owns the conversation loop; this
/// graph makes the per-turn assistant a real APXM-managed ACP worker.
pub fn acp_chat_air(opts: &ChatAcpAirOptions) -> String {
    let profile = sanitize_route_id(opts.profile);
    let mut spawn_attrs = vec![format!("profile = \"{profile}\"")];
    if let Some(mode) = opts.mode {
        let safe = sanitize_route_id(mode);
        if !safe.is_empty() {
            spawn_attrs.push(format!("mode = \"{safe}\""));
        }
    }
    if let Some(model) = opts.model {
        let safe = sanitize_route_id(model);
        if !safe.is_empty() {
            spawn_attrs.push(format!("model = \"{safe}\""));
        }
    }

    let message = match opts.system_prompt.filter(|prompt| !prompt.is_empty()) {
        Some(system_prompt) => format!(
            "System instructions:\n{}\n\nConversation:\n{{conversation}}",
            system_prompt
        ),
        None => "{conversation}".to_string(),
    };

    format!(
        "module {{\n  func.func @apxm_chat(%arg0: !ais.token {{ais.param_name = \"conversation\", ais.param_type = \"str\"}}) -> !ais.token attributes {{ais.entry}} {{\n    %spawn = ais.spawn_agent \"apxm_chat_orchestrator\" {{{}}} : !ais.token\n    %reply = ais.communicate \"{}\" to \"apxm_chat_orchestrator\" (%arg0, %spawn : !ais.token, !ais.token) {{protocol = \"acp\", input_names = [\"conversation\"]}} : !ais.token\n    func.return %reply : !ais.token\n  }}\n}}\n",
        spawn_attrs.join(", "),
        escape_air_string(&message),
    )
}

/// Parse the tool binding / capability id out of an admission-denial message.
/// Matches server preflight, runtime INV_CAP admission, spawn admission, and
/// python-backed handler checks.
pub fn parse_denied_capability(body: &str) -> Option<String> {
    let is_denial = body.contains("performs writes")
        || body.contains("missing a capability grant")
        || body.contains("performs process spawning");
    if !is_denial {
        return None;
    }
    for prefix in [
        "capability '",
        "python-backed capability '",
        "missing a capability grant for '",
    ] {
        if let Some(cap) = quoted_after(body, prefix) {
            return Some(cap);
        }
    }
    None
}

fn quoted_after(body: &str, prefix: &str) -> Option<String> {
    let after = body.split_once(prefix)?.1;
    let cap = after.split_once('\'')?.0;
    (!cap.is_empty()).then(|| cap.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_denied_capability_matches_write_spawn_and_python_denials() {
        assert_eq!(
            parse_denied_capability(
                "capability 'slack.post_message' performs writes and is missing a capability grant"
            ),
            Some("slack.post_message".to_string())
        );
        assert_eq!(
            parse_denied_capability(
                "SpawnAgent performs process spawning and is missing a capability grant for 'SPAWN_AGENT'"
            ),
            Some("SPAWN_AGENT".to_string())
        );
        assert_eq!(
            parse_denied_capability(
                "python-backed capability 'python_tools' is missing a capability grant"
            ),
            Some("python_tools".to_string())
        );
        assert!(parse_denied_capability("unrelated server fault").is_none());
    }

    #[test]
    fn chat_air_can_expose_capability_discovery_group() {
        let air = chat_air(&ChatAirOptions {
            capability_discovery: true,
            skills: true,
            ..ChatAirOptions::default()
        });
        assert!(air.contains(r#"capability_groups = ["discovery", "skills"]"#));
    }
}
