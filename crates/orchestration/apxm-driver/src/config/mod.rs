//! Configuration primitives for APxM tooling and runtimes.
//!
//! This module parses the TOML-based `~/.apxm/config.toml` (and project-specific variants)
//! so that the driver, runtime, and future tooling can load provider definitions,
//! tool guards, and system prompts from a single schema.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use apxm_backends::llm::BackendConfig;
use apxm_core::constants::env::APXM_CONFIG as APXM_CONFIG_ENV_VAR;
use apxm_core::env::APXM_HOME;
use dirs::home_dir;
use serde::{Deserialize, Serialize};
use std::env;
use thiserror::Error;

pub(crate) type Result<T> = std::result::Result<T, ConfigError>;

/// Model governance configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelsConfig {
    /// Optional allowlist of approved model names.
    /// If set, the compiler will reject graphs using models not in this list.
    #[serde(default)]
    pub allowlist: Option<Vec<String>>,
}

/// Configurable execution hook event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    GraphStart,
    GraphEnd,
    NodeStart,
    NodeComplete,
    NodeError,
    ToolStart,
    ToolEnd,
}

impl HookEvent {
    pub const ALL: [Self; 7] = [
        Self::GraphStart,
        Self::GraphEnd,
        Self::NodeStart,
        Self::NodeComplete,
        Self::NodeError,
        Self::ToolStart,
        Self::ToolEnd,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::GraphStart => "graph_start",
            Self::GraphEnd => "graph_end",
            Self::NodeStart => "node_start",
            Self::NodeComplete => "node_complete",
            Self::NodeError => "node_error",
            Self::ToolStart => "tool_start",
            Self::ToolEnd => "tool_end",
        }
    }
}

/// Subprocess hook configuration declared in `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookConfig {
    pub event: HookEvent,
    pub command: String,
    #[serde(default)]
    pub shell: Option<String>,
}

/// Stable TOML keys for execution hook configuration.
pub mod hook_toml_keys {
    pub const TABLE: &str = "hooks";
    pub const EVENT: &str = "event";
    pub const COMMAND: &str = "command";
    pub const SHELL: &str = "shell";
}

/// Built-in runtime middleware configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MiddlewareConfig {
    Timeout {
        #[serde(default)]
        default_timeout_ms: Option<u64>,
    },
    LoopGuard {
        #[serde(default = "default_loop_guard_repeats")]
        max_repeats: usize,
    },
}

fn default_loop_guard_repeats() -> usize {
    1
}

/// Application configuration loaded from TOML files.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ApXmConfig {
    /// Chat/runtime specific flags.
    pub chat: ChatConfig,

    /// HTTP server and streaming operational controls.
    #[serde(default)]
    pub server: ServerConfig,

    /// Unified backend definitions.
    #[serde(default)]
    pub backends: Vec<BackendConfig>,

    /// Tool-specific behavior overrides.
    pub tools: HashMap<String, ToolConfig>,

    /// System prompts for LLM operations (ask, think, reason, plan, reflect).
    #[serde(default)]
    pub instruction: InstructionConfig,

    /// Model governance configuration.
    #[serde(default)]
    pub models: ModelsConfig,

    /// Observer-only execution hooks. Failures are logged and swallowed.
    #[serde(default)]
    pub hooks: Vec<HookConfig>,

    /// Built-in dispatcher middlewares for local runtime execution.
    #[serde(default)]
    pub middlewares: Vec<MiddlewareConfig>,
}

/// Configuration for the chat/runtime surface.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatConfig {
    /// Explicit LLM providers to load.
    pub providers: Vec<String>,

    /// Default backend to use when a request does not select one explicitly.
    pub default_backend: Option<String>,

    /// Default exec policy (e.g., `project:execpolicy.toml`).
    pub default_exec_policy: Option<String>,

    /// Optional default model identifier.
    pub default_model: Option<String>,

    /// Path to session storage directory.
    #[serde(default)]
    pub session_storage: Option<PathBuf>,

    /// Maximum context tokens for chat sessions.
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,

    /// Model to use for planning (defaults to default_model if not specified).
    pub planning_model: Option<String>,

    /// Typed routing policy layered on top of backend registration.
    #[serde(default)]
    pub routing: LlmRoutingConfig,

    /// System prompt for chat sessions.
    pub system_prompt: Option<String>,
}

/// APXM server operational configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
#[derive(Default)]
pub struct ServerConfig {
    /// Optional bind address, overridden by `APXM_SERVER_ADDR` and CLI `--port`.
    pub bind_addr: Option<String>,

    /// Optional public URL advertised by clients and discovery endpoints.
    pub public_url: Option<String>,

    /// Process-level defaults consumed before the async server starts.
    pub process: ServerProcessConfig,

    /// Runtime scheduler limits for the server process.
    pub runtime: ServerRuntimeConfig,

    /// Process-local limiter for expensive inference work.
    pub inference: ServerInferenceConfig,

    /// Opt-in, fail-closed bearer auth for mutating routes. Default off.
    pub auth: ServerAuthConfig,

    /// `/v1/generate-stream` transport controls.
    pub generate_stream: GenerateStreamConfig,

    /// Runtime and skill execution SSE transport controls.
    pub execution_stream: ExecutionStreamConfig,

    /// Server-owned execution record/index controls.
    pub executions: ServerExecutionsConfig,

    /// `/v1/runs/{id}/events/stream` replay/live transport controls.
    pub run_events: RunEventsConfig,

    /// Outbound lifecycle webhook transport controls.
    pub webhook: ServerWebhookConfig,

    /// Durable rollout writer controls.
    pub rollout: ServerRolloutConfig,

    /// MCP tool behavior and bounded request controls.
    pub mcp: ServerMcpConfig,

    /// Server observability exporter controls.
    pub observability: ServerObservabilityConfig,

    /// HTTP safety controls (rate limit, body cap).
    pub safety: ServerSafetyConfig,

    /// Graceful shutdown drain controls.
    pub shutdown: ServerShutdownConfig,
}


/// APXM server process configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerProcessConfig {
    pub tokio_worker_threads: Option<usize>,
    pub log_filter: String,
}

impl Default for ServerProcessConfig {
    fn default() -> Self {
        Self {
            tokio_worker_threads: None,
            log_filter: default_server_log_filter(),
        }
    }
}

fn default_server_log_filter() -> String {
    "info,apxm_server=debug".to_string()
}

/// Server runtime scheduler limits.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerRuntimeConfig {
    pub max_concurrency: Option<usize>,
    pub max_inflight: Option<usize>,
    pub llm_inflight: usize,
    pub max_parallel_tool_calls: usize,
}

impl Default for ServerRuntimeConfig {
    fn default() -> Self {
        Self {
            max_concurrency: None,
            max_inflight: None,
            llm_inflight: 4,
            max_parallel_tool_calls:
                apxm_runtime::LlmToolDispatchConfig::DEFAULT_MAX_PARALLEL_TOOL_CALLS,
        }
    }
}

/// Server-wide inference limiter configuration.
///
/// `max_concurrent` caps how many skill/AIR executions may hold an inference slot
/// at once (process-wide). Override via `[server.inference]` in config or
/// `APXM_SERVER_MAX_INFERENCE`. Per-conversation ordering is **not** governed
/// here — the runtime's [`SessionLaneGuard`] serializes same-`session_id` work
/// while different sessions run in parallel up to this limit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerInferenceConfig {
    pub max_concurrent: usize,
    pub acquire_timeout_ms: u64,
}

impl Default for ServerInferenceConfig {
    fn default() -> Self {
        Self {
            // Raised from 2 so many concurrent cross-conversation webhook deliveries
            // are not artificially starved; same-chat ordering stays in SessionLaneGuard.
            max_concurrent: 16,
            acquire_timeout_ms: 250,
        }
    }
}

/// Opt-in, fail-closed bearer auth configuration for the server's mutating
/// routes (`/v1/execute`, `/v1/execute/stream`, skill execute/execute-stream).
///
/// Default is **off** so existing tests and local development are unaffected.
/// When `require_auth` is enabled, every request to a mutating route must
/// present `Authorization: Bearer <token>` matching the bearer token resolved
/// at request time (the token is re-read per request so apxm-auth rotation
/// does not strand callers). Resolution order:
///   1. `bearer` (explicit override, or `APXM_SERVER_BEARER`)
///   2. `bearer_file` if set, else `$XDG_STATE_HOME/apxm/auth/auth.bearer`,
///      falling back to `$HOME/.local/state/apxm/auth/auth.bearer`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ServerAuthConfig {
    /// When true, mutating routes require a valid bearer token (fail-closed).
    pub require_auth: bool,
    /// Explicit bearer token override. Highest precedence when set.
    pub bearer: Option<String>,
    /// Override path to the apxm-auth per-run bearer file.
    pub bearer_file: Option<String>,
}

/// Streaming LLM endpoint transport configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GenerateStreamConfig {
    pub channel_capacity: usize,
    pub inactivity_timeout_secs: u64,
    pub keep_alive_secs: u64,
}

impl Default for GenerateStreamConfig {
    fn default() -> Self {
        Self {
            channel_capacity: 128,
            inactivity_timeout_secs: 60,
            keep_alive_secs: 15,
        }
    }
}

/// Runtime and skill execution SSE endpoint transport configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExecutionStreamConfig {
    pub channel_capacity: usize,
    pub keep_alive_secs: u64,
}

impl Default for ExecutionStreamConfig {
    fn default() -> Self {
        Self {
            channel_capacity: 128,
            keep_alive_secs: 15,
        }
    }
}

/// Server execution record/index configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerExecutionsConfig {
    pub index_max_entries: usize,
}

impl Default for ServerExecutionsConfig {
    fn default() -> Self {
        Self {
            index_max_entries: 10_000,
        }
    }
}

/// Run event bus and SSE transport configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RunEventsConfig {
    pub stream_buffer: usize,
    pub retained_events: usize,
    pub keep_alive_secs: u64,
    pub default_list_limit: usize,
    pub max_list_limit: usize,
    pub default_events_limit: usize,
    pub max_events_limit: usize,
}

impl Default for RunEventsConfig {
    fn default() -> Self {
        Self {
            stream_buffer: 1024,
            retained_events: 4096,
            keep_alive_secs: 15,
            default_list_limit: 200,
            max_list_limit: 500,
            default_events_limit: 500,
            max_events_limit: 2_000,
        }
    }
}

/// Outbound run lifecycle webhook configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerWebhookConfig {
    pub url: Option<String>,
    pub timeout_secs: u64,
}

impl Default for ServerWebhookConfig {
    fn default() -> Self {
        Self {
            url: None,
            timeout_secs: 5,
        }
    }
}

/// Server-side rollout persistence configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerRolloutConfig {
    pub event_buffer: usize,
    pub spill_threshold_bytes: Option<u64>,
}

impl Default for ServerRolloutConfig {
    fn default() -> Self {
        Self {
            event_buffer: 2048,
            spill_threshold_bytes: None,
        }
    }
}

/// MCP tool configuration for server-owned agent tools.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ServerMcpConfig {
    pub workflow_max_tokens: usize,
    pub workflow_temperature: f64,
    pub workflow_emit_timeout_ms: u64,
    pub workflow_repair_attempts: usize,
    pub workflow_capability_guidance_limit: usize,
    pub default_top_k: usize,
    pub max_top_k: usize,
    pub default_evidence_limit: usize,
    pub max_evidence_limit: usize,
    pub default_trace_event_limit: usize,
    pub trace_max_scan_files: usize,
    pub evidence_max_scan_files: usize,
    pub evidence_max_file_bytes: u64,
}

impl Default for ServerMcpConfig {
    fn default() -> Self {
        Self {
            workflow_max_tokens: 8192,
            workflow_temperature: 0.0,
            workflow_emit_timeout_ms: 120_000,
            workflow_repair_attempts: 3,
            workflow_capability_guidance_limit: 32,
            default_top_k: 10,
            max_top_k: 100,
            default_evidence_limit: 10,
            max_evidence_limit: 100,
            default_trace_event_limit: 64,
            trace_max_scan_files: 4_096,
            evidence_max_scan_files: 4_096,
            evidence_max_file_bytes: 128 * 1024,
        }
    }
}

/// Server observability exporter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerObservabilityConfig {
    pub otlp_endpoint: Option<String>,
    /// When true, expose unversioned `GET /metrics` for Prometheus scrape.
    pub metrics_enabled: bool,
}

impl Default for ServerObservabilityConfig {
    fn default() -> Self {
        Self {
            otlp_endpoint: None,
            metrics_enabled: true,
        }
    }
}

/// HTTP safety limits for ingress-exposed or shared deployments.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerSafetyConfig {
    /// Per-principal requests-per-second cap. `None` disables rate limiting.
    pub rate_limit_rps: Option<u32>,
    /// Burst allowance for the token bucket. Defaults to `rate_limit_rps` when unset.
    pub rate_limit_burst: Option<u32>,
    /// Maximum accepted request body size in bytes. `None` disables the cap.
    pub max_body_bytes: Option<usize>,
}

impl Default for ServerSafetyConfig {
    fn default() -> Self {
        Self {
            rate_limit_rps: None,
            rate_limit_burst: None,
            max_body_bytes: Some(default_max_body_bytes()),
        }
    }
}

fn default_max_body_bytes() -> usize {
    10 * 1024 * 1024
}

/// Graceful shutdown drain configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerShutdownConfig {
    /// Maximum seconds to wait for rollout writers to flush after SIGTERM.
    pub drain_timeout_secs: u64,
}

impl Default for ServerShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout_secs: 30,
        }
    }
}

/// Policy configuration layered over the dynamic registry.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmRoutingConfig {
    /// Operation-scoped routing rules.
    #[serde(default)]
    pub operation_routes: HashMap<String, OperationRouteConfig>,

    /// Named model aliases shared across runtimes.
    #[serde(default)]
    pub model_aliases: HashMap<String, ModelAliasConfig>,

    /// Backend fallback chains for availability-aware recovery.
    #[serde(default)]
    pub fallback_chains: Vec<BackendFallbackConfig>,
}

/// Routing rule for a logical operation like `plan` or `reason`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperationRouteConfig {
    /// Preferred backend for this operation.
    pub backend: Option<String>,
    /// Preferred model or model alias for this operation.
    pub model: Option<String>,
    /// Optional optimization target for this operation: `cost`, `latency`,
    /// `quality`, or `balanced` (default). Drives price/capability-aware
    /// model selection when no explicit backend/model is pinned.
    #[serde(default)]
    pub target: Option<String>,
}

/// Named model alias with an optional preferred backend binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelAliasConfig {
    /// Canonical model identifier to use at request time.
    pub model: String,
    /// Optional backend to bind this alias to.
    pub backend: Option<String>,
}

/// Fallback policy for a backend registered in the runtime.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BackendFallbackConfig {
    /// Primary backend name.
    pub backend: String,
    /// Ordered fallback backend names.
    #[serde(default)]
    pub fallbacks: Vec<String>,
}

/// Tool-specific configuration overrides.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolConfig {
    /// Whether the tool is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Trusted folders for this tool (used as fallback for allowed_paths/working_directory).
    #[serde(default)]
    pub trusted_folders: Vec<PathBuf>,

    /// Commands to block (bash).
    #[serde(default)]
    pub blocked_commands: Vec<String>,

    /// Commands to allow (bash whitelist mode).
    #[serde(default)]
    pub allowed_commands: Vec<String>,

    /// Paths to block (read/write).
    #[serde(default)]
    pub blocked_paths: Vec<PathBuf>,

    /// Paths to allow (read/write).
    #[serde(default)]
    pub allowed_paths: Vec<PathBuf>,

    /// File extensions to allow (read/write).
    #[serde(default)]
    pub allowed_extensions: Vec<String>,

    /// File extensions to block (write).
    #[serde(default)]
    pub blocked_extensions: Vec<String>,

    /// Query terms to block (search_web).
    #[serde(default)]
    pub blocked_queries: Vec<String>,

    /// Domains to allow (search_web).
    #[serde(default)]
    pub allowed_domains: Vec<String>,

    /// Domains to block (search_web).
    #[serde(default)]
    pub blocked_domains: Vec<String>,

    /// Working directory (bash) or base directory (read/write).
    pub working_directory: Option<PathBuf>,

    /// Timeout in seconds (bash).
    pub timeout_secs: Option<u64>,

    /// Maximum output bytes (bash).
    pub max_output_bytes: Option<usize>,

    /// Maximum file size bytes (read/write).
    pub max_file_size: Option<usize>,

    /// Maximum results (search_web).
    pub max_results: Option<usize>,

    /// Safe search toggle (search_web).
    pub safe_search: Option<bool>,

    /// Search depth mode (search_web).
    pub search_depth: Option<apxm_runtime::capability::builtins::SearchDepth>,

    /// Endpoint override (search_web).
    pub endpoint: Option<String>,

    /// Include answer summary (search_web).
    pub include_answer: Option<bool>,

    /// Create parent directories automatically (write).
    pub create_directories: Option<bool>,

    /// Overwrite existing files (write).
    pub overwrite_existing: Option<bool>,

    /// Maximum default lines for reads.
    pub max_default_lines: Option<usize>,
}

pub use apxm_core::InstructionConfig;

fn default_true() -> bool {
    true
}

fn default_max_context_tokens() -> usize {
    8192
}

impl ApXmConfig {
    /// Loads configuration from the given path.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path).map_err(ConfigError::Io)?;
        toml::from_str::<ApXmConfig>(&contents).map_err(ConfigError::Parse)
    }

    /// Returns the global configuration path (`$APXM_HOME/config.toml` or
    /// `$HOME/.apxm/config.toml`).
    pub fn default_path() -> Result<PathBuf> {
        if let Ok(path) = env::var(APXM_HOME) {
            let trimmed = path.trim();
            if !trimmed.is_empty() {
                return Ok(PathBuf::from(trimmed).join("config.toml"));
            }
        }
        let home = home_dir().ok_or(ConfigError::HomeDirMissing)?;
        Ok(home.join(".apxm").join("config.toml"))
    }

    /// Load configuration from the default location.
    pub fn load_default() -> Result<Self> {
        let path = Self::default_path()?;
        Self::from_file(path)
    }

    /// Load configuration for the current working directory.
    ///
    /// Precedence is built-in defaults < global config < nearest project
    /// `.apxm/config.toml` < explicit `APXM_CONFIG`. TOML tables are merged
    /// before deserializing, so omitted project keys do not reset global keys
    /// to struct defaults.
    pub fn load_scoped() -> Result<Self> {
        Self::load_scoped_with_explicit(explicit_config_path())
    }

    /// Load scoped configuration with an explicit highest-precedence layer.
    pub fn load_scoped_with_explicit(explicit_path: Option<PathBuf>) -> Result<Self> {
        let mut paths = Vec::new();
        if let Some(path) = existing_global_config_path()? {
            paths.push(path);
        }
        if let Some(path) = project_config_path() {
            paths.push(path);
        }
        if let Some(path) = explicit_path {
            paths.push(path);
        }

        Self::from_layered_files(paths)
    }

    /// Load and merge config files in ascending precedence order.
    pub fn from_layered_files<I, P>(paths: I) -> Result<Self>
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut merged = toml::Value::Table(toml::value::Table::new());
        let mut loaded = false;

        for path in paths {
            let path = path.as_ref();
            let contents = fs::read_to_string(path).map_err(ConfigError::Io)?;
            let table: toml::value::Table =
                toml::from_str(&contents).map_err(ConfigError::Parse)?;
            merge_toml_value(&mut merged, toml::Value::Table(table));
            loaded = true;
        }

        if !loaded {
            return Ok(Self::default());
        }

        merged.try_into::<ApXmConfig>().map_err(ConfigError::Parse)
    }

    /// Load only the explicit config pointed to by `APXM_CONFIG`, when set.
    pub fn load_explicit() -> Result<Option<Self>> {
        if let Some(path) = explicit_config_path() {
            return Self::from_file(path).map(Some);
        }
        Ok(None)
    }

    /// Write this config to the given path.
    pub fn write_to_file(&self, path: impl AsRef<Path>) -> Result<()> {
        let content =
            toml::to_string_pretty(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent).map_err(ConfigError::Io)?;
        }
        fs::write(path, content).map_err(ConfigError::Io)?;
        Ok(())
    }

    /// Build APxM standard tool configuration from config file fields.
    pub fn tools_config(&self) -> apxm_runtime::capability::builtins::ToolsConfig {
        let mut tools_config = apxm_runtime::capability::builtins::ToolsConfig::default();

        for (tool_name, tool_config) in &self.tools {
            apply_tool_preset(tool_name, &mut tools_config);
            apply_enabled_override(tool_name, tool_config.enabled, &mut tools_config);
            apply_tool_overrides(tool_name, tool_config, &mut tools_config);
        }

        tools_config
    }
}

fn explicit_config_path() -> Option<PathBuf> {
    let path = env::var(APXM_CONFIG_ENV_VAR).ok()?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(PathBuf::from(trimmed))
}

fn existing_global_config_path() -> Result<Option<PathBuf>> {
    match ApXmConfig::default_path() {
        Ok(path) => Ok(path.exists().then_some(path)),
        Err(ConfigError::HomeDirMissing) => Ok(None),
        Err(error) => Err(error),
    }
}

fn project_config_path() -> Option<PathBuf> {
    let cwd = env::current_dir().ok()?;
    for ancestor in cwd.ancestors() {
        let candidate = ancestor.join(".apxm").join("config.toml");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn merge_toml_value(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, overlay_value) in overlay_table {
                match base_table.get_mut(&key) {
                    Some(base_value) => merge_toml_value(base_value, overlay_value),
                    None => {
                        base_table.insert(key, overlay_value);
                    }
                }
            }
        }
        (base_value, overlay_value) => {
            *base_value = overlay_value;
        }
    }
}

fn apply_enabled_override(
    name: &str,
    enabled: bool,
    config: &mut apxm_runtime::capability::builtins::ToolsConfig,
) {
    match normalize_tool_name(name) {
        Some("bash") => config.bash.enabled = enabled,
        Some("read") => config.read.enabled = enabled,
        Some("write") => config.write.enabled = enabled,
        Some("search_web") => config.search_web.enabled = enabled,
        _ => {}
    }
}

fn apply_tool_preset(name: &str, config: &mut apxm_runtime::capability::builtins::ToolsConfig) {
    match name {
        "bash_safe" => {
            config.bash = apxm_runtime::capability::builtins::BashConfig::safe_preset();
        }
        "bash_build" => {
            config.bash = apxm_runtime::capability::builtins::BashConfig::build_preset();
        }
        "bash_git" => {
            config.bash = apxm_runtime::capability::builtins::BashConfig::git_preset();
        }
        "read_source" => {
            config.read.allowed_extensions = Some(
                vec![
                    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "c", "cpp", "h", "hpp",
                    "toml", "yaml", "yml", "json", "md", "txt", "sh", "css", "scss", "html", "sql",
                    "graphql", "proto", "xml", "lock", "mod", "sum",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            );
            config.read.blocked_paths = vec![
                PathBuf::from(".env"),
                PathBuf::from(".secret"),
                PathBuf::from("credentials"),
                PathBuf::from(".git/config"),
            ];
            config.read.enabled = true;
        }
        "write_safe" => {
            config.write.blocked_extensions = vec![
                "exe", "com", "msi", "app", "dmg", "sh", "bat", "ps1", "cmd", "vbs", "dll", "so",
                "dylib", "bin",
            ]
            .into_iter()
            .map(str::to_string)
            .collect();
            config.write.enabled = true;
        }
        "search_docs" => {
            config.search_web.allowed_domains = Some(
                vec![
                    "docs.rs",
                    "doc.rust-lang.org",
                    "crates.io",
                    "docs.python.org",
                    "pypi.org",
                    "developer.mozilla.org",
                    "nodejs.org",
                    "pkg.go.dev",
                    "learn.microsoft.com",
                    "docs.github.com",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
            );
            config.search_web.max_results = 10;
            config.search_web.safe_search = true;
            config.search_web.search_depth = apxm_runtime::capability::builtins::SearchDepth::Basic;
            config.search_web.enabled = true;
        }
        "search_research" => {
            config.search_web.max_results = 15;
            config.search_web.safe_search = true;
            config.search_web.search_depth =
                apxm_runtime::capability::builtins::SearchDepth::Advanced;
            config.search_web.enabled = true;
        }
        _ => {}
    }
}

fn apply_tool_overrides(
    tool_name: &str,
    tool_config: &ToolConfig,
    config: &mut apxm_runtime::capability::builtins::ToolsConfig,
) {
    match normalize_tool_name(tool_name) {
        Some("bash") => {
            if !tool_config.blocked_commands.is_empty() {
                config.bash.blocked_commands = tool_config.blocked_commands.clone();
            }
            if !tool_config.allowed_commands.is_empty() {
                config.bash.allowed_commands = Some(tool_config.allowed_commands.clone());
            }
            if let Some(working_directory) = &tool_config.working_directory {
                config.bash.working_directory = Some(working_directory.clone());
            } else if !tool_config.trusted_folders.is_empty()
                && config.bash.working_directory.is_none()
            {
                config.bash.working_directory = tool_config.trusted_folders.first().cloned();
            }
            if let Some(timeout_secs) = tool_config.timeout_secs {
                config.bash.timeout_secs = timeout_secs;
            }
            if let Some(max_output_bytes) = tool_config.max_output_bytes {
                config.bash.max_output_bytes = max_output_bytes;
            }
        }
        Some("read") => {
            if !tool_config.blocked_paths.is_empty() {
                config.read.blocked_paths = tool_config.blocked_paths.clone();
            }
            if !tool_config.allowed_paths.is_empty() {
                config.read.allowed_paths = Some(tool_config.allowed_paths.clone());
            } else if !tool_config.trusted_folders.is_empty() {
                config.read.allowed_paths = Some(tool_config.trusted_folders.clone());
            }
            if !tool_config.allowed_extensions.is_empty() {
                config.read.allowed_extensions = Some(tool_config.allowed_extensions.clone());
            }
            if let Some(max_file_size) = tool_config.max_file_size {
                config.read.max_file_size = max_file_size;
            }
            if let Some(base_directory) = &tool_config.working_directory {
                config.read.base_directory = Some(base_directory.clone());
            }
            if let Some(max_default_lines) = tool_config.max_default_lines {
                config.read.max_default_lines = max_default_lines;
            }
        }
        Some("write") => {
            if !tool_config.blocked_paths.is_empty() {
                config.write.blocked_paths = tool_config.blocked_paths.clone();
            }
            if !tool_config.allowed_paths.is_empty() {
                config.write.allowed_paths = Some(tool_config.allowed_paths.clone());
            } else if !tool_config.trusted_folders.is_empty() {
                config.write.allowed_paths = Some(tool_config.trusted_folders.clone());
            }
            if !tool_config.allowed_extensions.is_empty() {
                config.write.allowed_extensions = Some(tool_config.allowed_extensions.clone());
            }
            if !tool_config.blocked_extensions.is_empty() {
                config.write.blocked_extensions = tool_config.blocked_extensions.clone();
            }
            if let Some(max_file_size) = tool_config.max_file_size {
                config.write.max_file_size = Some(max_file_size);
            }
            if let Some(base_directory) = &tool_config.working_directory {
                config.write.base_directory = Some(base_directory.clone());
            }
            if let Some(create_directories) = tool_config.create_directories {
                config.write.create_directories = create_directories;
            }
            if let Some(overwrite_existing) = tool_config.overwrite_existing {
                config.write.overwrite_existing = overwrite_existing;
            }
        }
        Some("search_web") => {
            if !tool_config.allowed_domains.is_empty() {
                config.search_web.allowed_domains = Some(tool_config.allowed_domains.clone());
            }
            if !tool_config.blocked_domains.is_empty() {
                config.search_web.blocked_domains = tool_config.blocked_domains.clone();
            }
            if !tool_config.blocked_queries.is_empty() {
                config.search_web.blocked_queries = tool_config.blocked_queries.clone();
            }
            if let Some(max_results) = tool_config.max_results {
                config.search_web.max_results = max_results;
            }
            if let Some(safe_search) = tool_config.safe_search {
                config.search_web.safe_search = safe_search;
            }
            if let Some(search_depth) = tool_config.search_depth {
                config.search_web.search_depth = search_depth;
            }
            if let Some(endpoint) = &tool_config.endpoint {
                config.search_web.endpoint = endpoint.clone();
            }
            if let Some(include_answer) = tool_config.include_answer {
                config.search_web.include_answer = include_answer;
            }
        }
        _ => {}
    }
}

fn normalize_tool_name(name: &str) -> Option<&'static str> {
    match name {
        "bash" | "shell" | "terminal" | "bash_safe" | "bash_build" | "bash_git" => Some("bash"),
        "read" | "read_file" | "read_source" => Some("read"),
        "write" | "write_file" | "write_safe" => Some("write"),
        "search_web" | "web_search" | "search" | "search_docs" | "search_research" => {
            Some("search_web")
        }
        _ => None,
    }
}

/// Errors that can occur while parsing APxM configuration files.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("IO failure when reading config: {0}")]
    Io(#[from] std::io::Error),

    #[error("Failed to parse TOML config: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("Unable to determine home directory for default config path")]
    HomeDirMissing,

    #[error("Failed to serialize config: {0}")]
    Serialize(String),
}
