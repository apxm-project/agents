//! Opt-in, fail-closed bearer authentication for mutating routes.
//!
//! # Security model
//!
//! This middleware is **off by default** (`ServerConfig::auth.require_auth`
//! defaults to `false`). When enabled — via config or the
//! `APXM_SERVER_REQUIRE_AUTH` env — every request to a *mutating* route must
//! present an `Authorization: Bearer <token>` header whose token matches the
//! bearer resolved at request time. Any mismatch, missing header, or missing
//! token source yields `401 Unauthorized` (fail-closed).
//!
//! The bearer is re-resolved **per request** (never cached) so that apxm-auth
//! key rotation — which rewrites `auth.bearer` — does not strand in-flight
//! callers against a stale token.
//!
//! Token resolution order:
//!   1. `ServerAuthConfig.bearer` (explicit override / `APXM_SERVER_BEARER`).
//!   2. `ServerAuthConfig.bearer_file` if set, else
//!      `$XDG_STATE_HOME/apxm/auth/auth.bearer`, falling back to
//!      `$HOME/.local/state/apxm/auth/auth.bearer`.
//!
//! Only mutating routes are guarded (see [`is_protected`]); read-only and
//! discovery routes (health, models, runs, agent card, etc.) remain open so
//! the studio proxy and observers keep working on loopback.

use std::path::PathBuf;

use apxm_driver::ServerAuthConfig;
use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::principal::PrincipalId;
use crate::routes;

/// Auth policy resolved at startup from bind address + config.
#[derive(Clone)]
pub(crate) struct AuthPolicy {
    pub(crate) config: ServerAuthConfig,
    pub(crate) effective_require_auth: bool,
}
/// Whether a request requires the bearer when auth is effectively enabled.
/// Default-deny: everything is protected except a small public read surface
/// (liveness, model discovery, metrics, validation) and CORS preflight.
fn is_protected(method: &axum::http::Method, path: &str) -> bool {
    if method == axum::http::Method::OPTIONS {
        // Never gate CORS preflight; it carries no Authorization header.
        return false;
    }
    // Public, read-only endpoints that stay open even with auth enabled.
    let public = path == routes::HEALTH
        || path == routes::METRICS
        || path == routes::MODELS
        || path == routes::BACKENDS
        || (path.starts_with(routes::SKILLS) && path.ends_with("/validate"));
    !public
}

/// Resolve the expected bearer at request time. Returns `None` when no token
/// source is configured/readable — callers must treat that as fail-closed.
pub(crate) fn resolve_expected_bearer(config: &ServerAuthConfig) -> Option<String> {
    if let Some(token) = config
        .bearer
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        return Some(token.to_string());
    }

    let path = bearer_file_path(config)?;
    let contents = std::fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn bearer_file_path(config: &ServerAuthConfig) -> Option<PathBuf> {
    if let Some(explicit) = config
        .bearer_file
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(PathBuf::from(explicit));
    }

    let base = std::env::var("XDG_STATE_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(base.join("apxm/auth/auth.bearer"))
}

/// Extract the bearer token from an `Authorization: Bearer <token>` header.
fn presented_bearer(req: &Request<Body>) -> Option<String> {
    let value = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let trimmed = token.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Constant-time comparison to avoid leaking token length/content via timing.
fn tokens_match(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn unauthorized(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

/// Some authority-minting routes are protected even on loopback when global auth
/// is relaxed. Loopback is not a sufficient boundary for grants that can turn
/// into filesystem writes.
fn always_requires_bearer(path: &str) -> bool {
    path == routes::CAPABILITY_DELEGATE
        || (path.starts_with("/v1/capabilities/") && path.ends_with("/revoke"))
}

/// Axum middleware enforcing bearer auth on protected routes.
///
/// When `effective_require_auth` is `false` this is a transparent pass-through
/// for ordinary routes, but authority-minting delegated capability routes still
/// fail closed. When enabled, protected routes fail closed and attach a
/// [`PrincipalId`] extension after successful validation.
pub(crate) async fn require_bearer(
    State(policy): State<AuthPolicy>,
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let path = req.uri().path();
    let must_authenticate = policy.effective_require_auth && is_protected(req.method(), path)
        || always_requires_bearer(path);

    if !must_authenticate {
        req.extensions_mut().insert(PrincipalId::anonymous());
        return next.run(req).await;
    }

    let Some(expected) = resolve_expected_bearer(&policy.config) else {
        return unauthorized("server auth is required but no bearer token is configured");
    };

    let Some(presented) = presented_bearer(&req) else {
        return unauthorized("missing or malformed Authorization: Bearer <token> header");
    };

    if !tokens_match(&presented, &expected) {
        return unauthorized("invalid bearer token");
    }

    req.extensions_mut()
        .insert(PrincipalId::from_bearer(&presented));
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::{always_requires_bearer, is_protected};
    use axum::http::Method;

    #[test]
    fn delegated_capability_authority_routes_are_always_bearer_protected() {
        assert!(always_requires_bearer("/v1/capabilities/delegate"));
        assert!(always_requires_bearer("/v1/capabilities/cap_123/revoke"));
        assert!(!always_requires_bearer("/v1/capability-templates"));
    }

    #[test]
    fn discovery_routes_remain_public_when_auth_is_enabled() {
        assert!(!is_protected(&Method::GET, crate::routes::HEALTH));
        assert!(!is_protected(&Method::GET, crate::routes::MODELS));
        assert!(is_protected(&Method::POST, crate::routes::EXECUTE));
    }
}
