use apxm_core::constants::env as apxm_env;
use apxm_driver::{ApXmConfig, ServerConfig};

pub(crate) fn server_config_from_layers() -> anyhow::Result<ServerConfig> {
    let mut config = ApXmConfig::load_scoped()?.server;
    apply_server_env_overrides(&mut config);
    Ok(config)
}

pub(crate) fn apply_server_env_overrides(config: &mut ServerConfig) {
    if let Some(value) = env_usize(apxm_env::APXM_TOKIO_WORKERS) {
        config.process.tokio_worker_threads = Some(value);
    }
    if let Ok(value) = std::env::var(apxm_env::RUST_LOG) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            config.process.log_filter = trimmed.to_string();
        }
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUNTIME_MAX_CONCURRENCY) {
        config.runtime.max_concurrency = Some(value);
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUNTIME_MAX_INFLIGHT) {
        config.runtime.max_inflight = Some(value);
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUNTIME_LLM_INFLIGHT) {
        config.runtime.llm_inflight = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUNTIME_MAX_PARALLEL_TOOL_CALLS) {
        config.runtime.max_parallel_tool_calls = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_MCP_WORKFLOW_MAX_TOKENS) {
        config.mcp.workflow_max_tokens = value;
    }
    if let Some(value) = env_f64(apxm_env::APXM_MCP_WORKFLOW_TEMPERATURE) {
        config.mcp.workflow_temperature = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_MCP_WORKFLOW_EMIT_TIMEOUT_MS) {
        config.mcp.workflow_emit_timeout_ms = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_MCP_WORKFLOW_REPAIR_ATTEMPTS) {
        config.mcp.workflow_repair_attempts = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_MCP_WORKFLOW_CAPABILITY_GUIDANCE_LIMIT) {
        config.mcp.workflow_capability_guidance_limit = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_MCP_DEFAULT_TOP_K) {
        config.mcp.default_top_k = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_MCP_MAX_TOP_K) {
        config.mcp.max_top_k = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_MCP_DEFAULT_EVIDENCE_LIMIT) {
        config.mcp.default_evidence_limit = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_MCP_MAX_EVIDENCE_LIMIT) {
        config.mcp.max_evidence_limit = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_MCP_DEFAULT_TRACE_EVENT_LIMIT) {
        config.mcp.default_trace_event_limit = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_MCP_TRACE_MAX_SCAN_FILES) {
        config.mcp.trace_max_scan_files = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_MCP_EVIDENCE_MAX_SCAN_FILES) {
        config.mcp.evidence_max_scan_files = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_MCP_EVIDENCE_MAX_FILE_BYTES) {
        config.mcp.evidence_max_file_bytes = value;
    }
    if let Some(value) = env_bool(apxm_env::APXM_SERVER_REQUIRE_AUTH) {
        config.auth.require_auth = value;
    }
    if let Ok(value) = std::env::var(apxm_env::APXM_SERVER_BEARER) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            config.auth.bearer = Some(trimmed.to_string());
        }
    }
    if let Some(value) = env_usize(apxm_env::APXM_SERVER_MAX_INFERENCE) {
        config.inference.max_concurrent = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_SERVER_INFERENCE_WAIT_MS) {
        config.inference.acquire_timeout_ms = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_GENERATE_STREAM_CHANNEL_CAPACITY) {
        config.generate_stream.channel_capacity = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_GENERATE_STREAM_TIMEOUT_SECS) {
        config.generate_stream.inactivity_timeout_secs = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_GENERATE_STREAM_KEEP_ALIVE_SECS) {
        config.generate_stream.keep_alive_secs = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_EXECUTION_STREAM_CHANNEL_CAPACITY) {
        config.execution_stream.channel_capacity = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_EXECUTION_STREAM_KEEP_ALIVE_SECS) {
        config.execution_stream.keep_alive_secs = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_EXECUTION_INDEX_MAX_ENTRIES) {
        config.executions.index_max_entries = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUN_EVENT_STREAM_BUFFER) {
        config.run_events.stream_buffer = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUN_EVENT_RETAINED_EVENTS) {
        config.run_events.retained_events = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_RUN_EVENT_KEEP_ALIVE_SECS) {
        config.run_events.keep_alive_secs = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUN_LIST_DEFAULT_LIMIT) {
        config.run_events.default_list_limit = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUN_LIST_MAX_LIMIT) {
        config.run_events.max_list_limit = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUN_EVENT_DEFAULT_LIMIT) {
        config.run_events.default_events_limit = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_RUN_EVENT_MAX_LIMIT) {
        config.run_events.max_events_limit = value;
    }
    if let Ok(value) = std::env::var(apxm_env::APXM_RUN_WEBHOOK_URL) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            config.webhook.url = Some(trimmed.to_string());
        }
    }
    if let Some(value) = env_u64(apxm_env::APXM_RUN_WEBHOOK_TIMEOUT_SECS) {
        config.webhook.timeout_secs = value;
    }
    if let Some(value) = env_usize(apxm_env::APXM_ROLLOUT_EVENT_BUFFER) {
        config.rollout.event_buffer = value;
    }
    if let Some(value) = env_u64(apxm_env::APXM_ROLLOUT_SPILL_THRESHOLD_BYTES) {
        config.rollout.spill_threshold_bytes = Some(value);
    }
    if let Ok(value) = std::env::var(apxm_env::OTEL_EXPORTER_OTLP_ENDPOINT) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            config.observability.otlp_endpoint = Some(trimmed.to_string());
        }
    }
    if let Some(value) = env_u32(apxm_env::APXM_SERVER_RATE_LIMIT_RPS) {
        config.safety.rate_limit_rps = Some(value);
    }
    if let Some(value) = env_usize(apxm_env::APXM_SERVER_MAX_BODY_BYTES) {
        config.safety.max_body_bytes = Some(value);
    }
    if let Some(value) = env_u64(apxm_env::APXM_SERVER_DRAIN_TIMEOUT_SECS) {
        config.shutdown.drain_timeout_secs = value;
    }
    if let Ok(value) = std::env::var(apxm_env::APXM_PUBLIC_URL) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            config.public_url = Some(trimmed.to_string());
        }
    }
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
}

fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
}

fn env_bool(name: &str) -> Option<bool> {
    let value = std::env::var(name).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn env_f64(name: &str) -> Option<f64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
}
