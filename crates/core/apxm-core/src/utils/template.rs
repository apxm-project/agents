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
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            i += 2;
            continue;
        }
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

