//! Shared conversational-chat primitives — transcript rendering, the chat /
//! summarize AIR, and the compaction budget — used by both the `apxm chat` CLI
//! and the apxm-studio backend so they stay identical. (The TS frontend mirrors
//! this contract; these values are the spec.)

/// Turns kept verbatim during compaction; older turns fold into the summary.
pub const KEEP_RECENT_TURNS: usize = 4;
/// Transcript token budget (chars/4 estimate) above which compaction triggers.
/// ~0.6 of a 32k window — conservative.
pub const COMPACT_AT_TOKENS: usize = 20_000;

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

/// Estimate the token count of a string (chars/4 heuristic). The same estimate
/// is used for the compaction budget on every surface.
pub fn estimate_tokens(s: &str) -> usize {
    s.len() / 4
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
    if let Some(sp) = opts.system_prompt {
        if !sp.is_empty() {
            attrs.push(format!("system_prompt = \"{}\"", escape_air_string(sp)));
        }
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
        groups.push("web");
    }
    if opts.skills {
        groups.push("skills");
    }
    if !groups.is_empty() {
        let list = groups
            .iter()
            .map(|g| format!("\"{g}\""))
            .collect::<Vec<_>>()
            .join(", ");
        attrs.push(format!("tool_groups = [{list}]"));
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

/// Parse the capability name out of a write-denial message. Matches BOTH the
/// server's static pre-flight wording (`capability '<cap>' performs writes and
/// was not granted; …`) and the runtime's invoke-site wording (`write capability
/// '<cap>' is not admitted by this execution's grant`). Shared by the CLI REPL
/// and the studio backend so both fire the HITL grant prompt on either form.
pub fn parse_denied_capability(body: &str) -> Option<String> {
    let is_write_denial =
        body.contains("performs writes") || body.contains("is not admitted by this execution");
    if !is_write_denial {
        return None;
    }
    let after = body.split_once("capability '")?.1;
    let cap = after.split_once('\'')?.0;
    (!cap.is_empty()).then(|| cap.to_string())
}

/// Single-ASK summarize graph used by the compaction post-hook to fold older
/// turns into a running summary. One source of truth for both surfaces.
pub const SUMMARIZE_AIR: &str = r#"module {
  func.func @apxm_summarize(%arg0: !ais.token {ais.param_name = "to_summarize", ais.param_type = "str"}) -> !ais.token attributes {ais.entry} {
    %summary = ais.ask "Summarize the following conversation excerpt into a concise running summary that preserves decisions, facts, names, and open tasks. Be terse.\n\n{{{to_summarize}}}" : !ais.token
    func.return %summary : !ais.token
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_labels_roles_and_leaves_assistant_open() {
        let msgs = [
            (Role::System, "You are APXM."),
            (Role::User, "hi"),
            (Role::Assistant, "hello"),
            (Role::User, "17 + 25?"),
        ];
        assert_eq!(
            render_transcript(msgs),
            "System: You are APXM.\nUser: hi\nAssistant: hello\nUser: 17 + 25?\nAssistant:"
        );
    }

    #[test]
    fn empty_transcript_is_just_the_cue() {
        let empty: [(Role, &str); 0] = [];
        assert_eq!(render_transcript(empty), "Assistant:");
    }

    #[test]
    fn default_air_is_the_bare_single_ask() {
        let air = chat_air(&ChatAirOptions::default());
        assert!(air.contains("ais.ask \"{{{conversation}}}\" : !ais.token"));
        assert!(!air.contains("backend ="));
        assert!(!air.contains("tool_groups"));
        assert!(air.contains("@apxm_chat"));
    }

    #[test]
    fn acp_chat_air_spawns_profile_with_model_and_templates_conversation() {
        let air = acp_chat_air(&ChatAcpAirOptions {
            profile: "claude",
            system_prompt: Some("You are the orchestrator."),
            mode: Some("architect"),
            model: Some("claude-3-5-haiku-latest"),
        });
        assert!(air.contains("ais.spawn_agent \"apxm_chat_orchestrator\""));
        assert!(air.contains("profile = \"claude\""));
        assert!(air.contains("mode = \"architect\""));
        assert!(air.contains("model = \"claude-3-5-haiku-latest\""));
        assert!(air.contains("ais.communicate"));
        assert!(air.contains("protocol = \"acp\""));
        assert!(air.contains("input_names = [\"conversation\"]"));
        assert!(air.contains("System instructions:\\nYou are the orchestrator."));
        assert!(air.contains("{conversation}"));
    }

    #[test]
    fn denial_parser_matches_server_and_runtime_wordings() {
        let server = "capability 'fs.write' performs writes and was not granted";
        assert_eq!(parse_denied_capability(server).as_deref(), Some("fs.write"));
        let runtime = "write capability 'fs.write' is not admitted by this execution's grant";
        assert_eq!(
            parse_denied_capability(runtime).as_deref(),
            Some("fs.write")
        );
        assert_eq!(
            parse_denied_capability("capability 'x' is not registered"),
            None
        );
        assert_eq!(parse_denied_capability("unrelated"), None);
    }

    #[test]
    fn air_pins_backend_model_and_tools() {
        let air = chat_air(&ChatAirOptions {
            system_prompt: None,
            backend: Some("amd"),
            model: Some("claude-sonnet-4-6"),
            effort: Some("medium"),
            tools: true,
            skills: false,
        });
        assert!(air.contains("backend = \"amd\""));
        assert!(air.contains("model = \"claude-sonnet-4-6\""));
        assert!(air.contains("effort = \"medium\""));
        assert!(air.contains("tool_groups = [\"web\"]"));
    }

    #[test]
    fn air_exposes_skills_group_for_discovery() {
        let air = chat_air(&ChatAirOptions {
            skills: true,
            ..Default::default()
        });
        assert!(air.contains("tool_groups = [\"skills\"]"), "{air}");
        let both = chat_air(&ChatAirOptions {
            tools: true,
            skills: true,
            ..Default::default()
        });
        assert!(
            both.contains("tool_groups = [\"web\", \"skills\"]"),
            "{both}"
        );
    }

    #[test]
    fn air_omits_effort_when_off_or_unset() {
        let off = chat_air(&ChatAirOptions {
            effort: Some("off"),
            ..Default::default()
        });
        assert!(!off.contains("effort"));
        let unset = chat_air(&ChatAirOptions::default());
        assert!(!unset.contains("effort"));
    }

    #[test]
    fn air_injects_escaped_system_prompt() {
        let air = chat_air(&ChatAirOptions {
            system_prompt: Some("You are APXM.\nProject: \"acme\""),
            ..Default::default()
        });
        // Newlines and quotes are escaped so the AIR string literal stays valid.
        assert!(air.contains("system_prompt = \"You are APXM.\\nProject: \\\"acme\\\"\""));
    }

    #[test]
    fn route_id_sanitized() {
        assert_eq!(sanitize_route_id("amd\" injected"), "amdinjected");
        assert_eq!(
            sanitize_route_id("vendor/model-1.5:turbo"),
            "vendor/model-1.5:turbo"
        );
    }

    #[test]
    fn role_parse_defaults_to_user() {
        assert_eq!(Role::parse("system"), Role::System);
        assert_eq!(Role::parse("assistant"), Role::Assistant);
        assert_eq!(Role::parse("weird"), Role::User);
    }
}
