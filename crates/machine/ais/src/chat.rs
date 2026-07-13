//! Shared conversational-chat primitives — transcript rendering shared by the
//! `apxm chat` CLI and the apxm-studio backend so they stay identical.
//!
//! Compaction is NOT this crate's contract: `KEEP_RECENT_TURNS`/
//! `COMPACT_AT_TOKENS`/`estimate_tokens`/`SUMMARIZE_AIR` used to live here as
//! a chars/4-estimate duplicate with zero call sites (dead policy, never
//! wired to the dumb-pipe chat host — see `commands/chat.rs`'s doc comment).
//! They were deleted once the runtime default
//! (`ConversationMemoryMiddleware`, `apxm-runtime` crate) reached parity —
//! see the runtime compaction policy and
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

/// Escape text for safe interpolation into an AIR string literal.
pub fn escape_air_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 8);
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Retain only characters valid in an AIR routing string literal.
pub fn sanitize_route_id(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':' | '/')
        })
        .collect()
}

/// Per-turn routing and capability-group options for the conversational graph.
#[derive(Debug, Default, Clone, Copy)]
pub struct ChatAirOptions<'a> {
    /// System prompt for this turn.
    pub system_prompt: Option<&'a str>,
    /// Registered backend selected for this turn.
    pub backend: Option<&'a str>,
    /// Specific model selected for this turn.
    pub model: Option<&'a str>,
    /// Extended-thinking effort requested for this turn.
    pub effort: Option<&'a str>,
    /// Whether the web capability group is available.
    pub tools: bool,
    /// Whether the skills capability group is available.
    pub skills: bool,
    /// Whether capability discovery is available.
    pub capability_discovery: bool,
    /// Whether authoring capabilities are available.
    pub authoring: bool,
}

/// Build the canonical single-ASK conversational graph for one turn.
pub fn chat_air(options: &ChatAirOptions) -> String {
    let mut attributes = Vec::new();
    if let Some(system_prompt) = options.system_prompt.filter(|value| !value.is_empty()) {
        attributes.push(format!(
            "system_prompt = \"{}\"",
            escape_air_string(system_prompt)
        ));
    }
    for (name, value) in [
        ("backend", options.backend),
        ("model", options.model),
        ("effort", options.effort),
    ] {
        let Some(value) = value.map(sanitize_route_id).filter(|value| !value.is_empty()) else {
            continue;
        };
        if name != "effort" || value != "off" {
            attributes.push(format!("{name} = \"{value}\""));
        }
    }

    let mut capability_groups = Vec::new();
    if options.tools {
        capability_groups.push(groups::WEB);
    }
    if options.capability_discovery {
        capability_groups.push(groups::DISCOVERY);
    }
    if options.skills {
        capability_groups.push(groups::SKILLS);
    }
    if options.authoring {
        capability_groups.push(groups::AUTHORING);
    }
    if !capability_groups.is_empty() {
        let groups = capability_groups
            .iter()
            .map(|group| format!("\"{group}\""))
            .collect::<Vec<_>>()
            .join(", ");
        attributes.push(format!("capability_groups = [{groups}]"));
    }

    let attribute_dict = (!attributes.is_empty()).then(|| format!(" {{{}}}", attributes.join(", ")));
    format!(
        "module {{\n  func.func @apxm_chat(%arg0: !ais.token {{ais.param_name = \"conversation\", ais.param_type = \"str\"}}) -> !ais.token attributes {{ais.entry}} {{\n    %reply = ais.ask \"{{{{{{conversation}}}}}}\"{} : !ais.token\n    func.return %reply : !ais.token\n  }}\n}}\n",
        attribute_dict.unwrap_or_default(),
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
    fn escapes_air_string_literal_control_characters() {
        assert_eq!(
            escape_air_string("quote: \" slash: \\ newline:\n tab:\t return:\r"),
            "quote: \\\" slash: \\\\ newline:\\n tab:\\t return:\\r"
        );
    }

    #[test]
    fn chat_air_exposes_requested_capability_groups() {
        let air = chat_air(&ChatAirOptions {
            capability_discovery: true,
            skills: true,
            ..ChatAirOptions::default()
        });
        assert!(air.contains(r#"capability_groups = ["discovery", "skills"]"#));
    }
}
