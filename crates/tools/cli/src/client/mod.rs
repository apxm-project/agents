//! Typed HTTP client for [`apxm-server`](https://github.com/apxm-project/server).
//!
//! Session/permission bindings are generated from `openapi/openapi-session-v1.yaml`
//! (see `build.rs`). Execute-stream helpers live in [`execute`] until the
//! OpenAPI contract grows those paths.
//!
//! Folded into `apxm-cli` — this was previously the standalone
//! `apxm-client` crate; it has no consumer outside this binary.

include!(concat!(env!("OUT_DIR"), "/apxm_client_codegen.rs"));

#[cfg(feature = "driver")]
pub mod events;
pub mod execute;

/// Environment variable naming the apxm-server base URL.
#[cfg(feature = "driver")]
pub const APXM_SERVER_BASE_ENV: &str = "APXM_SERVER_BASE";

/// Resolve the apxm-server base URL, failing closed. The address must be given
/// explicitly (`--server`) or via `APXM_SERVER_BASE`; there is no hardcoded
/// localhost default, so the CLI never silently talks to the wrong endpoint.
#[cfg(feature = "driver")]
pub fn resolve_server_base(explicit: Option<&str>) -> anyhow::Result<String> {
    if let Some(base) = explicit.filter(|value| !value.is_empty()) {
        return Ok(base.to_string());
    }
    match std::env::var(APXM_SERVER_BASE_ENV) {
        Ok(base) if !base.is_empty() => Ok(base),
        _ => anyhow::bail!(
            "no apxm-server address: pass --server <URL> or set {APXM_SERVER_BASE_ENV}"
        ),
    }
}

/// Build a generated [`Client`] suitable for long-lived SSE.
///
/// Reqwest's default is no read timeout. Do not express that as
/// `read_timeout(Duration::from_secs(0))`: zero is an immediate deadline, so
/// it can abort an otherwise correctly framed HTTP response before the body is
/// read.
#[cfg(feature = "driver")]
pub fn client_for_sse(base: &str) -> Client {
    let http = reqwest::ClientBuilder::new()
        .build()
        .expect("reqwest client");
    Client::new_with_client(base.trim_end_matches('/'), http)
}

/// Re-export reqwest so CLI consumers share one version with the generated client.
pub use reqwest;

#[cfg(test)]
mod tests {
    use super::Client;

    /// Sample consumer: compile against generated `Client` only.
    #[test]
    fn sample_consumer_compiles() {
        let _client = Client::new("http://127.0.0.1:18800");
    }
}
