use std::time::Duration;

const CONNECT_TIMEOUT_ENV: &str = "APXM_LLM_CONNECT_TIMEOUT_MS";
const READ_TIMEOUT_ENV: &str = "APXM_LLM_READ_TIMEOUT_MS";
const POOL_IDLE_TIMEOUT_ENV: &str = "APXM_LLM_POOL_IDLE_TIMEOUT_MS";
const TCP_KEEPALIVE_ENV: &str = "APXM_LLM_TCP_KEEPALIVE_MS";

const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_READ_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_POOL_IDLE_TIMEOUT_MS: u64 = 90_000;
const DEFAULT_TCP_KEEPALIVE_MS: u64 = 60_000;

pub(crate) fn llm_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(env_duration_ms(
            CONNECT_TIMEOUT_ENV,
            DEFAULT_CONNECT_TIMEOUT_MS,
        ))
        .read_timeout(env_duration_ms(READ_TIMEOUT_ENV, DEFAULT_READ_TIMEOUT_MS))
        .pool_idle_timeout(env_duration_ms(
            POOL_IDLE_TIMEOUT_ENV,
            DEFAULT_POOL_IDLE_TIMEOUT_MS,
        ))
        .tcp_keepalive(Some(env_duration_ms(
            TCP_KEEPALIVE_ENV,
            DEFAULT_TCP_KEEPALIVE_MS,
        )))
        .build()
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to build tuned LLM HTTP client");
            reqwest::Client::new()
        })
}

fn env_duration_ms(name: &str, default_ms: u64) -> Duration {
    duration_ms(std::env::var(name).ok().as_deref(), default_ms)
}

fn duration_ms(value: Option<&str>, default_ms: u64) -> Duration {
    value
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(default_ms))
}

