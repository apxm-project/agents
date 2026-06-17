//! Typed HTTP client for [`apxm-server`](https://github.com/apxm-project/apxm).
//!
//! Bindings are generated from the checked-in OpenAPI contract at
//! `openapi/openapi-session-v1.yaml` (spec 0002 US6 / T081).

include!(concat!(env!("OUT_DIR"), "/codegen.rs"));

#[cfg(test)]
mod tests {
    use super::Client;

    /// Sample consumer: compile against generated `Client` only (US6 checkpoint).
    #[test]
    fn sample_consumer_compiles() {
        let _client = Client::new("http://127.0.0.1:18800");
    }
}
