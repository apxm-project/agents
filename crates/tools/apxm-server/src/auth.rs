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

use crate::routes;

/// Whether a request requires the bearer when `require_auth` is enabled.
/// Default-deny: everything is protected except a small public read surface
/// (liveness, model discovery, validation) and CORS preflight, so a newly added
/// route is protected unless deliberately listed.
fn is_protected(method: &axum::http::Method, path: &str) -> bool {
    if method == axum::http::Method::OPTIONS {
        // Never gate CORS preflight; it carries no Authorization header.
        return false;
    }
    // Public, read-only endpoints that stay open even with auth enabled.
    let public = path == routes::HEALTH
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

/// Axum middleware enforcing bearer auth on mutating routes when enabled.
///
/// When `require_auth` is `false` (the default) this is a transparent
/// pass-through for *all* routes. When enabled, protected routes fail closed.
pub(crate) async fn require_bearer(
    State(config): State<ServerAuthConfig>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if !config.require_auth {
        return next.run(req).await;
    }

    if !is_protected(req.method(), req.uri().path()) {
        return next.run(req).await;
    }

    let Some(expected) = resolve_expected_bearer(&config) else {
        // Auth is required but no token source is available: fail closed.
        return unauthorized("server auth is required but no bearer token is configured");
    };

    let Some(presented) = presented_bearer(&req) else {
        return unauthorized("missing or malformed Authorization: Bearer <token> header");
    };

    if !tokens_match(&presented, &expected) {
        return unauthorized("invalid bearer token");
    }

    next.run(req).await
}

