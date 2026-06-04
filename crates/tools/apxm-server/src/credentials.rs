//! Credential resolution against **apxm-auth** (the credential plane).
//!
//! At `inv_tool` dispatch the runtime turns a connection id / credential path
//! into a live access token by calling apxm-auth — the kernel never stores or
//! refreshes secrets itself. Wiring this into `HttpCapability::execute` (inject
//! `Authorization: Bearer <token>`) is the live integration step; the resolver
//! client itself lives here and is unit-tested.

// Live: consumed by `execute::inject_resolved_credentials` (F12/F13 wiring).

use serde::Deserialize;

/// Client for apxm-auth's resolve endpoint.
pub(crate) struct CredentialResolver {
    base: String,
    bearer: Option<String>,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
}

impl CredentialResolver {
    pub(crate) fn new(base: impl Into<String>) -> Self {
        Self { base: base.into(), bearer: read_bearer(), http: reqwest::Client::new() }
    }

    /// Resolve from `APXM_AUTH_URL` (default `http://127.0.0.1:18810`).
    pub(crate) fn from_env() -> Self {
        Self::new(std::env::var("APXM_AUTH_URL").unwrap_or_else(|_| "http://127.0.0.1:18810".to_string()))
    }

    /// `GET /v1/connections/{id}/token` — returns the current access token.
    ///
    /// apxm-auth requires an `owner` query parameter and scopes the connection
    /// to that tenant. When the dispatch context carries an owner it is threaded
    /// through `owner`; otherwise the `APXM_AUTH_OWNER` env (default `"default"`)
    /// is used. The service bearer authenticates the service; owner scopes the
    /// tenant — both are sent.
    pub(crate) async fn resolve(
        &self,
        connection_id: &str,
        owner: Option<&str>,
    ) -> anyhow::Result<String> {
        let url = format!(
            "{}/v1/connections/{}/token?owner={}",
            self.base,
            enc(connection_id),
            enc(&resolve_owner(owner))
        );
        let mut req = self.http.get(url);
        if let Some(b) = &self.bearer {
            req = req.bearer_auth(b);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("apxm-auth resolve returned {}: {}", resp.status(), resp.text().await.unwrap_or_default());
        }
        Ok(resp.json::<TokenResp>().await?.access_token)
    }
}

/// Resolve the owner/tenant for an apxm-auth request: prefer an owner carried
/// by the dispatch context, else `APXM_AUTH_OWNER`, else the `"default"`
/// convention (matching apxm-auth's own oauth_start default).
fn resolve_owner(owner: Option<&str>) -> String {
    owner
        .map(str::to_string)
        .unwrap_or_else(|| std::env::var("APXM_AUTH_OWNER").unwrap_or_else(|_| "default".to_string()))
}

/// Read apxm-auth's per-run bearer (written 0600 by `apxm-auth serve`).
fn read_bearer() -> Option<String> {
    let dir = std::env::var("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| std::path::PathBuf::from(h).join(".local/state")))
        .ok()?;
    std::fs::read_to_string(dir.join("apxm/auth/auth.bearer")).ok().map(|s| s.trim().to_string())
}

/// Percent-encode a path segment (SSRF/path-injection hardening on the id).
fn enc(seg: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(seg.len());
    for b in seg.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => {
                out.push('%');
                out.push(HEX[(b >> 4) as usize] as char);
                out.push(HEX[(b & 0x0f) as usize] as char);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_holds_base() {
        let r = CredentialResolver::new("http://127.0.0.1:18810");
        assert_eq!(r.base, "http://127.0.0.1:18810");
    }

    #[test]
    fn path_segment_is_encoded() {
        assert_eq!(enc("a/b c"), "a%2Fb%20c");
        assert_eq!(enc("ok-id_1.2~"), "ok-id_1.2~");
    }
}
