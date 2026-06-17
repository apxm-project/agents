//! Typed HTTP client for [`apxm-server`](https://github.com/apxm-project/apxm).
//!
//! Session/permission bindings are generated from `openapi/openapi-session-v1.yaml`
//! (spec 0002 US6 / T081). Execute-stream helpers live in [`execute`] until the
//! OpenAPI contract grows those paths.

include!(concat!(env!("OUT_DIR"), "/codegen.rs"));

pub mod events;
pub mod execute;

/// Default apxm-server bind address (`apxm-server` `DEFAULT_PORT` = 18800).
pub const DEFAULT_SERVER_BASE: &str = "http://127.0.0.1:18800";

/// Build a generated [`Client`] suitable for long-lived SSE (no read timeout).
pub fn client_for_sse(base: &str) -> Client {
    let http = reqwest::ClientBuilder::new()
        .read_timeout(std::time::Duration::from_secs(0))
        .build()
        .expect("reqwest client");
    Client::new_with_client(base.trim_end_matches('/'), http)
}

/// Re-export reqwest so CLI consumers share one version with generated client.
pub use reqwest;

#[cfg(test)]
mod tests {
    use super::Client;

    /// Sample consumer: compile against generated `Client` only (US6 checkpoint).
    #[test]
    fn sample_consumer_compiles() {
        let _client = Client::new("http://127.0.0.1:18800");
    }
}
