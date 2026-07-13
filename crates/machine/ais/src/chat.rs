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
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

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

/// Validation errors for the typed compile-service process contract.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompileServiceOptionsError {
    /// A backend or model id is empty or contains a character outside the
    /// routing-id grammar.
    #[error(
        "invalid {field} {value:?}: expected a non-empty routing id containing only ASCII letters, digits, '.', '_', '-', ':', or '/'"
    )]
    InvalidRouteId {
        /// Contract field being validated.
        field: &'static str,
        /// Rejected value, preserved exactly as supplied by the caller.
        value: String,
    },
    /// Extended-thinking effort is outside the closed contract vocabulary.
    #[error("invalid effort {0:?}: expected one of 'off', 'low', 'medium', or 'high'")]
    InvalidEffort(String),
}

fn validate_route_id(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), CompileServiceOptionsError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_empty()
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/'))
    {
        return Err(CompileServiceOptionsError::InvalidRouteId {
            field,
            value: value.to_string(),
        });
    }
    Ok(())
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
    /// Context-planning profile selected for this turn.
    pub context_profile: Option<&'a str>,
    /// Resolved output-token reservation for this turn.
    pub max_output_tokens: Option<usize>,
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

fn deserialize_required_nullable_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

fn deserialize_required_nullable_usize<'de, D>(deserializer: D) -> Result<Option<usize>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<usize>::deserialize(deserializer)
}

/// Owned, serializable process contract for `apxm compile-service`.
///
/// Nullable routing fields must still be present in JSON so Server and the
/// compiler cannot silently drift to different defaults. Unknown fields fail
/// closed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileServiceOptions {
    #[serde(deserialize_with = "deserialize_required_nullable_string")]
    pub system_prompt: Option<String>,
    #[serde(deserialize_with = "deserialize_required_nullable_string")]
    pub backend: Option<String>,
    #[serde(deserialize_with = "deserialize_required_nullable_string")]
    pub model: Option<String>,
    #[serde(deserialize_with = "deserialize_required_nullable_string")]
    pub context_profile: Option<String>,
    #[serde(deserialize_with = "deserialize_required_nullable_usize")]
    pub max_output_tokens: Option<usize>,
    #[serde(deserialize_with = "deserialize_required_nullable_string")]
    pub effort: Option<String>,
    pub tools: bool,
    pub skills: bool,
    pub capability_discovery: bool,
    pub authoring: bool,
}

impl CompileServiceOptions {
    /// Validate routing and effort values without normalizing or rewriting
    /// caller input.
    pub fn validate(&self) -> Result<(), CompileServiceOptionsError> {
        validate_route_id("backend", self.backend.as_deref())?;
        validate_route_id("model", self.model.as_deref())?;
        validate_route_id("context_profile", self.context_profile.as_deref())?;
        if let Some(effort) = self.effort.as_deref()
            && !matches!(effort, "off" | "low" | "medium" | "high")
        {
            return Err(CompileServiceOptionsError::InvalidEffort(
                effort.to_string(),
            ));
        }
        Ok(())
    }
}

impl From<&ChatAirOptions<'_>> for CompileServiceOptions {
    fn from(options: &ChatAirOptions<'_>) -> Self {
        Self {
            system_prompt: options.system_prompt.map(ToOwned::to_owned),
            backend: options.backend.map(ToOwned::to_owned),
            model: options.model.map(ToOwned::to_owned),
            context_profile: options.context_profile.map(ToOwned::to_owned),
            max_output_tokens: options.max_output_tokens,
            effort: options.effort.map(ToOwned::to_owned),
            tools: options.tools,
            skills: options.skills,
            capability_discovery: options.capability_discovery,
            authoring: options.authoring,
        }
    }
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
        ("profile", options.context_profile),
        ("effort", options.effort),
    ] {
        let Some(value) = value
            .map(sanitize_route_id)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if name != "effort" || value != "off" {
            attributes.push(format!("{name} = \"{value}\""));
        }
    }
    if let Some(max_output_tokens) = options.max_output_tokens {
        attributes.push(format!("token_budget = {max_output_tokens} : i64"));
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

    let attribute_dict =
        (!attributes.is_empty()).then(|| format!(" {{{}}}", attributes.join(", ")));
    format!(
        "module {{\n  func.func @apxm_chat(%arg0: !ais.token {{ais.param_name = \"conversation\", ais.param_type = \"str\"}}) -> !ais.token attributes {{ais.entry}} {{\n    %reply = ais.ask \"{{{{{{conversation}}}}}}\"{} : !ais.token\n    func.return %reply : !ais.token\n  }}\n}}\n",
        attribute_dict.unwrap_or_default(),
    )
}

/// Build a conversational graph that spawns one ACP profile and sends the
/// rendered transcript to it. The CLI owns the conversation loop; this graph
/// makes the per-turn assistant an APXM-managed ACP worker.
pub fn acp_chat_air(options: &ChatAcpAirOptions) -> String {
    let profile = sanitize_route_id(options.profile);
    let mut spawn_attributes = vec![format!("profile = \"{profile}\"")];
    if let Some(mode) = options.mode {
        let safe = sanitize_route_id(mode);
        if !safe.is_empty() {
            spawn_attributes.push(format!("mode = \"{safe}\""));
        }
    }
    if let Some(model) = options.model {
        let safe = sanitize_route_id(model);
        if !safe.is_empty() {
            spawn_attributes.push(format!("model = \"{safe}\""));
        }
    }

    let message = match options
        .system_prompt
        .filter(|system_prompt| !system_prompt.is_empty())
    {
        Some(system_prompt) => format!(
            "System instructions:\n{}\n\nConversation:\n{{conversation}}",
            system_prompt
        ),
        None => "{conversation}".to_string(),
    };

    format!(
        "module {{\n  func.func @apxm_chat(%arg0: !ais.token {{ais.param_name = \"conversation\", ais.param_type = \"str\"}}) -> !ais.token attributes {{ais.entry}} {{\n    %spawn = ais.spawn_agent \"apxm_chat_orchestrator\" {{{}}} : !ais.token\n    %reply = ais.communicate \"{}\" to \"apxm_chat_orchestrator\" (%arg0, %spawn : !ais.token, !ais.token) {{protocol = \"acp\", input_names = [\"conversation\"]}} : !ais.token\n    func.return %reply : !ais.token\n  }}\n}}\n",
        spawn_attributes.join(", "),
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

    #[test]
    fn chat_air_exposes_caller_supplied_routing_and_planning_attributes() {
        let air = chat_air(&ChatAirOptions {
            backend: Some("configured-backend"),
            model: Some("configured-model"),
            context_profile: Some("configured-profile"),
            max_output_tokens: Some(256),
            ..ChatAirOptions::default()
        });

        assert!(air.contains(r#"backend = "configured-backend""#));
        assert!(air.contains(r#"model = "configured-model""#));
        assert!(air.contains(r#"profile = "configured-profile""#));
        assert!(air.contains("token_budget = 256 : i64"));
    }

    #[test]
    fn compile_service_options_validate_exact_route_ids_and_effort() {
        let valid = CompileServiceOptions {
            backend: Some("gateway:v1/primary".to_string()),
            model: Some("openai/gpt-4.1-mini".to_string()),
            effort: Some("high".to_string()),
            ..CompileServiceOptions::default()
        };
        valid.validate().expect("valid options");

        let invalid_backend = CompileServiceOptions {
            backend: Some("gateway primary".to_string()),
            ..CompileServiceOptions::default()
        };
        assert_eq!(
            invalid_backend.validate(),
            Err(CompileServiceOptionsError::InvalidRouteId {
                field: "backend",
                value: "gateway primary".to_string(),
            })
        );

        let invalid_effort = CompileServiceOptions {
            effort: Some("HIGH".to_string()),
            ..CompileServiceOptions::default()
        };
        assert_eq!(
            invalid_effort.validate(),
            Err(CompileServiceOptionsError::InvalidEffort(
                "HIGH".to_string()
            ))
        );
    }
}
