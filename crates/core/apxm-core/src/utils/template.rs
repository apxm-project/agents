//! Shared template-placeholder parsing.
//!
//! Templates throughout APXM use the named form `{name}` with optional dotted
//! JSON selectors, e.g. `{data.event.subject}`. Validators resolve the root
//! name against the node's `input_names` parallel attribute (matching incoming
//! Data edges) or against the module's declared parameters. Runtime handlers
//! then navigate the remaining dotted path inside JSON values.

/// Parse all `{name}` placeholders in `s` and return the names in order of
/// appearance, preserving duplicates.
///
/// Names are matched by the regex-like shape `\{([A-Za-z0-9_.]+)\}`. Dotted
/// selectors are returned whole; use [`placeholder_root`] for validation
/// against declared names.
pub fn parse_placeholder_names(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            i += 2;
            continue;
        }
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'.')
            {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b'}' {
                // SAFETY: ASCII placeholder characters are valid UTF-8 boundaries.
                out.push(&s[start..end]);
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Returns true if `name` is a digit-only placeholder like `0`, `12`.
pub fn is_numeric_placeholder(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

/// Return the declared input/parameter name that owns a placeholder.
///
/// For `{data.event.subject}`, this returns `data`; for `{answer}` it returns
/// `answer`.
pub fn placeholder_root(name: &str) -> &str {
    name.split_once('.').map_or(name, |(root, _)| root)
}

#[cfg(test)]
mod tests {
    use super::{parse_placeholder_names, placeholder_root};

    #[test]
    fn parses_dotted_placeholders() {
        assert_eq!(
            parse_placeholder_names("reply to {data.event.subject} with {answer}"),
            vec!["data.event.subject", "answer"]
        );
        assert_eq!(placeholder_root("data.event.subject"), "data");
        assert_eq!(placeholder_root("answer"), "answer");
    }
}
