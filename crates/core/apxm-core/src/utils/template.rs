//! Shared template-placeholder parsing.
//!
//! Templates throughout APXM use the named form `{name}`. Both the compiler
//! validator and the runtime handlers resolve each name against the node's
//! `input_names` parallel attribute (matching incoming Data edges) or against
//! the module's declared parameters. Numeric placeholders are not allowed in
//! authored templates; `is_numeric_placeholder` exists so the validator can
//! produce a precise diagnostic when one slips through.

/// Parse all `{name}` placeholders in `s` and return the names in order
/// of appearance, preserving duplicates.
///
/// Names are matched by the regex `\{(\w+)\}` — only ASCII word characters
/// (letters, digits, underscore). Names may *contain* digits but cannot
/// start with one for them to be interpreted as parameter references; the
/// digit-only case is detected by `is_numeric_placeholder` for diagnostics.
pub fn parse_placeholder_names(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b'}' {
                // SAFETY: `\w` characters are valid UTF-8 boundaries.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_names_in_order() {
        assert_eq!(parse_placeholder_names("Hello {a} and {b}"), vec!["a", "b"]);
    }

    #[test]
    fn parse_preserves_duplicates() {
        assert_eq!(parse_placeholder_names("{x} again {x}"), vec!["x", "x"]);
    }

    #[test]
    fn parse_no_placeholders() {
        assert!(parse_placeholder_names("plain text").is_empty());
    }

    #[test]
    fn parse_ignores_non_word_braces() {
        assert!(parse_placeholder_names("{ }").is_empty());
        assert!(parse_placeholder_names("{a-b}").is_empty());
        assert!(parse_placeholder_names("{a b}").is_empty());
    }

    #[test]
    fn parse_picks_up_digits_in_names() {
        assert_eq!(parse_placeholder_names("{topic1} {0}"), vec!["topic1", "0"]);
    }

    #[test]
    fn numeric_placeholder_detection() {
        assert!(is_numeric_placeholder("0"));
        assert!(is_numeric_placeholder("12"));
        assert!(!is_numeric_placeholder("a"));
        assert!(!is_numeric_placeholder("a0"));
        assert!(!is_numeric_placeholder(""));
    }
}
