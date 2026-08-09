//! Contract identifier and digest grammar, checked without a regex dependency.
//!
//! These mirror `apxm.contract-common.v1#/$defs/Identifier` and `/$defs/Digest`
//! exactly. The crate consumes those closed primitives; it does not redefine
//! them.

/// `^[A-Za-z0-9][A-Za-z0-9._:/@-]{0,255}$`
#[must_use]
pub fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    if value.len() > 256 {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '-'))
}

/// `^apxm\.[a-z0-9]+(?:-[a-z0-9]+)*\.v[0-9]+$`
#[must_use]
pub fn is_schema_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("apxm.") else {
        return false;
    };
    let Some((body, version)) = rest.rsplit_once(".v") else {
        return false;
    };
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    if body.is_empty() {
        return false;
    }
    body.split('-').all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

/// `^sha256:[0-9a-f]{64}$`
#[must_use]
pub fn is_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_grammar() {
        assert!(is_identifier("node.model.1"));
        assert!(is_identifier("Specialist"));
        assert!(is_identifier("hooks.before_model"));
        assert!(!is_identifier(""));
        assert!(!is_identifier(".leading-dot"));
        assert!(!is_identifier("has space"));
    }

    #[test]
    fn schema_id_grammar() {
        assert!(is_schema_id("apxm.air.v2"));
        assert!(is_schema_id("apxm.model-context-envelope.v1"));
        assert!(!is_schema_id("apxm.Air.v1"));
        assert!(!is_schema_id("apxm.a.b.v1"));
        assert!(!is_schema_id("air.v1"));
        assert!(!is_schema_id("apxm.air.v"));
    }

    #[test]
    fn digest_grammar() {
        assert!(is_digest(&format!("sha256:{}", "a".repeat(64))));
        assert!(!is_digest(&format!("sha256:{}", "A".repeat(64))));
        assert!(!is_digest("sha256:abc"));
        assert!(!is_digest("blake3:0000"));
    }
}
