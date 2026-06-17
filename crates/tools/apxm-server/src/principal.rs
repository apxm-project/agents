//! Per-principal identity derived from validated bearer tokens (v0 single-tenant).

use axum::http::Request;

/// Stable principal fingerprint for rate limiting and structured logs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PrincipalId(pub(crate) String);

impl PrincipalId {
    /// Derive a stable id from a bearer secret. Uses blake3 hex prefix — not
    /// reversible and safe to log.
    pub(crate) fn from_bearer(token: &str) -> Self {
        let digest = blake3::hash(token.as_bytes());
        let hex = digest.to_hex();
        Self(hex[..16].to_string())
    }

    /// Anonymous traffic (no validated bearer).
    pub(crate) fn anonymous() -> Self {
        Self("anonymous".to_string())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) fn principal_from_request<B>(req: &Request<B>) -> PrincipalId {
    req.extensions()
        .get::<PrincipalId>()
        .cloned()
        .unwrap_or_else(PrincipalId::anonymous)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_fingerprint_is_stable() {
        let a = PrincipalId::from_bearer("secret-token");
        let b = PrincipalId::from_bearer("secret-token");
        assert_eq!(a, b);
        assert_ne!(a, PrincipalId::from_bearer("other"));
    }
}
