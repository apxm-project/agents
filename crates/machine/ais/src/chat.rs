//! Shared text utilities used by thin chat clients and AIR-producing callers.
//!
//! This module deliberately contains no graph generation, package compilation,
//! session-loop, capability-selection, or Agent Skill activation behavior.

/// A participant role in a rendered client transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::User => "User",
            Self::Assistant => "Assistant",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value {
            "system" => Self::System,
            "assistant" => Self::Assistant,
            _ => Self::User,
        }
    }
}

/// Render client transcript data. This is data formatting only; the package
/// entry and host context assembly own model instructions and execution.
pub fn render_transcript<'a, I>(messages: I) -> String
where
    I: IntoIterator<Item = (Role, &'a str)>,
{
    let mut output = String::new();
    for (role, content) in messages {
        output.push_str(role.label());
        output.push_str(": ");
        output.push_str(content);
        output.push('\n');
    }
    output.push_str("Assistant:");
    output
}

/// Escape text for a caller that is already authoring an AIR string literal.
#[must_use]
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

/// Extract a denied capability id from a typed admission-denial message.
#[must_use]
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
        if let Some(capability) = quoted_after(body, prefix) {
            return Some(capability);
        }
    }
    None
}

fn quoted_after(body: &str, prefix: &str) -> Option<String> {
    let after = body.split_once(prefix)?.1;
    let capability = after.split_once('\'')?.0;
    (!capability.is_empty()).then(|| capability.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_air_string_literal_control_characters() {
        assert_eq!(
            escape_air_string("quote: \" slash: \\ newline:\n tab:\t return:\r"),
            "quote: \\\" slash: \\\\ newline:\\n tab:\\t return:\\r"
        );
    }

    #[test]
    fn parses_capability_denials() {
        assert_eq!(
            parse_denied_capability(
                "capability 'slack.post_message' performs writes and is missing a capability grant"
            ),
            Some("slack.post_message".to_string())
        );
        assert!(parse_denied_capability("unrelated server fault").is_none());
    }
}
