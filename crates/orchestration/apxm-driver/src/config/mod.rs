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

/// Stable TOML keys used by execution hook configuration.
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerConfig {
    /// Optional bind address, overridden by `APXM_SERVER_ADDR` and CLI `--port`.
    pub bind_addr: Option<String>,

    /// Optional public URL advertised by clients and discovery endpoints.
    pub public_url: Option<String>,

    /// Process-level defaults consumed before the async server starts.
    pub process: ServerProcessConfig,

    /// Runtime scheduler limits used by the server process.
    pub runtime: ServerRuntimeConfig,

    /// Process-local limiter for expensive inference work.
    pub inference: ServerInferenceConfig,

    /// `/v1/generate-stream` transport controls.
    pub generate_stream: GenerateStreamConfig,

    /// Runtime and skill execution SSE transport controls.
    pub execution_stream: ExecutionStreamConfig,

    /// `/v1/runs/{id}/events/stream` replay/live transport controls.
    pub run_events: RunEventsConfig,

    /// Outbound lifecycle webhook transport controls.
    pub webhook: ServerWebhookConfig,

    /// Durable rollout writer controls.
    pub rollout: ServerRolloutConfig,

    /// Server observability exporter controls.
    pub observability: ServerObservabilityConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: None,
            public_url: None,
            process: ServerProcessConfig::default(),
            runtime: ServerRuntimeConfig::default(),
            inference: ServerInferenceConfig::default(),
            generate_stream: GenerateStreamConfig::default(),
            execution_stream: ExecutionStreamConfig::default(),
            run_events: RunEventsConfig::default(),
            webhook: ServerWebhookConfig::default(),
            rollout: ServerRolloutConfig::default(),
            observability: ServerObservabilityConfig::default(),
        }
    }
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
}

impl Default for ServerRuntimeConfig {
    fn default() -> Self {
        Self {
            max_concurrency: None,
            max_inflight: None,
            llm_inflight: 4,
        }
    }
}

/// Server-wide inference limiter configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ServerInferenceConfig {
    pub max_concurrent: usize,
    pub acquire_timeout_ms: u64,
}

impl Default for ServerInferenceConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 2,
            acquire_timeout_ms: 250,
        }
    }
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
}

impl Default for ServerRolloutConfig {
    fn default() -> Self {
        Self { event_buffer: 2048 }
    }
}

/// Server observability exporter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ServerObservabilityConfig {
    pub otlp_endpoint: Option<String>,
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

// Re-export InstructionConfig from apxm-core for consistency
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

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::events::kind;
    use dirs::home_dir;
    use std::env;

    const MOCK_PROVIDER_NAME: &str = "mock-provider";
    const MOCK_PROVIDER_NAME_ALT: &str = "mock-provider-alt";
    const MOCK_MODEL_NAME: &str = "mock-model";
    const MOCK_MODEL_NAME_ALT: &str = "mock-model-alt";
    const NOOP_COMMAND: &str = "true";
    // Core ApxmEvent kinds are trace/event-bus names, not subprocess hook
    // configuration events.
    const APXM_EVENT_KIND_NAMES: [&str; 4] = [
        kind::TOKEN.name(),
        kind::TOOL_CALL.name(),
        kind::OPERATION_START.name(),
        kind::OPERATION_END.name(),
    ];

    #[test]
    fn hook_event_all_contains_every_typed_event_kind() {
        use HookEvent::*;

        assert_eq!(
            HookEvent::ALL,
            [
                GraphStart,
                GraphEnd,
                NodeStart,
                NodeComplete,
                NodeError,
                ToolStart,
                ToolEnd,
            ]
        );
    }

    #[test]
    fn hook_event_wire_names_roundtrip() {
        for event in HookEvent::ALL {
            let hook = HookConfig {
                event,
                command: NOOP_COMMAND.to_string(),
                shell: None,
            };
            let encoded = toml::to_string(&hook).expect("hook config toml");
            assert!(
                encoded.contains(&format!(
                    "{} = \"{}\"",
                    hook_toml_keys::EVENT,
                    event.as_str()
                )),
                "encoded hook should use stable snake_case event name: {encoded}"
            );

            let decoded: HookConfig = toml::from_str(&encoded).expect("hook config decode");
            assert_eq!(decoded.event, event);
            assert_eq!(decoded.command, NOOP_COMMAND);
        }
    }

    #[test]
    fn apxm_event_kind_names_are_rejected_as_hook_events() {
        for event in APXM_EVENT_KIND_NAMES {
            let toml = format!(
                r#"
                {event_key} = "{event}"
                {command_key} = "{NOOP_COMMAND}"
                "#,
                event_key = hook_toml_keys::EVENT,
                command_key = hook_toml_keys::COMMAND,
            );
            let err = toml::from_str::<HookConfig>(&toml).expect_err("invalid hook event");
            assert!(
                err.to_string().contains(event),
                "parse error should identify unsupported hook event {event}: {err}"
            );
        }
    }

    #[test]
    fn deserialize_basic_config() {
        let toml = format!(
            r#"
            [chat]
            providers = ["{MOCK_PROVIDER_NAME}", "{MOCK_PROVIDER_NAME_ALT}"]
            default_backend = "{MOCK_PROVIDER_NAME}"
            default_exec_policy = "project:policy.toml"
            default_model = "{MOCK_MODEL_NAME}"
            planning_model = "{MOCK_MODEL_NAME_ALT}"

            [server]
            bind_addr = "127.0.0.1:18801"

            [server.process]
            tokio_worker_threads = 6
            log_filter = "warn,apxm_server=info"

            [server.runtime]
            max_concurrency = 8
            max_inflight = 16
            llm_inflight = 3

            [server.inference]
            max_concurrent = 4
            acquire_timeout_ms = 500

            [server.generate_stream]
            channel_capacity = 256
            inactivity_timeout_secs = 90
            keep_alive_secs = 10

            [server.execution_stream]
            channel_capacity = 384
            keep_alive_secs = 12

            [server.run_events]
            stream_buffer = 2048
            retained_events = 8192
            keep_alive_secs = 20
            default_list_limit = 100
            max_list_limit = 300
            default_events_limit = 250
            max_events_limit = 750

            [server.webhook]
            url = "http://127.0.0.1:18802/hook"
            timeout_secs = 7

            [server.rollout]
            event_buffer = 4096

            [server.observability]
            otlp_endpoint = "http://127.0.0.1:4317"

            [chat.routing.operation_routes.plan]
            backend = "{MOCK_PROVIDER_NAME}"
            model = "fast"

            [chat.routing.model_aliases.fast]
            model = "{MOCK_MODEL_NAME_ALT}"
            backend = "{MOCK_PROVIDER_NAME}"

            [[chat.routing.fallback_chains]]
            backend = "{MOCK_PROVIDER_NAME}"
            fallbacks = ["{MOCK_PROVIDER_NAME_ALT}"]

            [[{hook_table}]]
            {event_key} = "{node_complete_event}"
            {command_key} = "echo {{node_id}}"

            [[{hook_table}]]
            {event_key} = "{tool_end_event}"
            {command_key} = "echo {{tool_name}}"

            [[middlewares]]
            kind = "timeout"
            default_timeout_ms = 7500

            [[middlewares]]
            kind = "loop_guard"
            max_repeats = 2

            [tools.shell]
            enabled = true
            trusted_folders = ["/home/work"]
        "#,
            hook_table = hook_toml_keys::TABLE,
            event_key = hook_toml_keys::EVENT,
            command_key = hook_toml_keys::COMMAND,
            node_complete_event = HookEvent::NodeComplete.as_str(),
            tool_end_event = HookEvent::ToolEnd.as_str(),
        );

        let config: ApXmConfig = toml::from_str(&toml).unwrap();
        assert_eq!(config.chat.providers.len(), 2);
        assert_eq!(
            config.chat.default_backend.as_deref(),
            Some(MOCK_PROVIDER_NAME)
        );
        assert_eq!(
            config.chat.default_exec_policy.as_deref(),
            Some("project:policy.toml")
        );
        assert_eq!(config.chat.default_model.as_deref(), Some(MOCK_MODEL_NAME));
        assert_eq!(
            config.chat.planning_model.as_deref(),
            Some(MOCK_MODEL_NAME_ALT)
        );
        assert_eq!(config.server.bind_addr.as_deref(), Some("127.0.0.1:18801"));
        assert_eq!(config.server.process.tokio_worker_threads, Some(6));
        assert_eq!(config.server.process.log_filter, "warn,apxm_server=info");
        assert_eq!(config.server.runtime.max_concurrency, Some(8));
        assert_eq!(config.server.runtime.max_inflight, Some(16));
        assert_eq!(config.server.runtime.llm_inflight, 3);
        assert_eq!(config.server.inference.max_concurrent, 4);
        assert_eq!(config.server.inference.acquire_timeout_ms, 500);
        assert_eq!(config.server.generate_stream.channel_capacity, 256);
        assert_eq!(config.server.generate_stream.inactivity_timeout_secs, 90);
        assert_eq!(config.server.generate_stream.keep_alive_secs, 10);
        assert_eq!(config.server.execution_stream.channel_capacity, 384);
        assert_eq!(config.server.execution_stream.keep_alive_secs, 12);
        assert_eq!(config.server.run_events.stream_buffer, 2048);
        assert_eq!(config.server.run_events.retained_events, 8192);
        assert_eq!(config.server.run_events.keep_alive_secs, 20);
        assert_eq!(config.server.run_events.default_list_limit, 100);
        assert_eq!(config.server.run_events.max_list_limit, 300);
        assert_eq!(config.server.run_events.default_events_limit, 250);
        assert_eq!(config.server.run_events.max_events_limit, 750);
        assert_eq!(
            config.server.webhook.url.as_deref(),
            Some("http://127.0.0.1:18802/hook")
        );
        assert_eq!(config.server.webhook.timeout_secs, 7);
        assert_eq!(config.server.rollout.event_buffer, 4096);
        assert_eq!(
            config.server.observability.otlp_endpoint.as_deref(),
            Some("http://127.0.0.1:4317")
        );
        assert_eq!(
            config
                .chat
                .routing
                .operation_routes
                .get("plan")
                .and_then(|route| route.backend.as_deref()),
            Some(MOCK_PROVIDER_NAME)
        );
        assert_eq!(
            config
                .chat
                .routing
                .operation_routes
                .get("plan")
                .and_then(|route| route.model.as_deref()),
            Some("fast")
        );
        assert_eq!(
            config
                .chat
                .routing
                .model_aliases
                .get("fast")
                .map(|alias| alias.model.as_str()),
            Some(MOCK_MODEL_NAME_ALT)
        );
        assert_eq!(
            config
                .chat
                .routing
                .model_aliases
                .get("fast")
                .and_then(|alias| alias.backend.as_deref()),
            Some(MOCK_PROVIDER_NAME)
        );
        assert_eq!(
            config
                .chat
                .routing
                .fallback_chains
                .first()
                .map(|chain| chain.backend.as_str()),
            Some(MOCK_PROVIDER_NAME)
        );
        assert_eq!(
            config
                .chat
                .routing
                .fallback_chains
                .first()
                .and_then(|chain| chain.fallbacks.first())
                .map(String::as_str),
            Some(MOCK_PROVIDER_NAME_ALT)
        );
        assert_eq!(config.hooks.len(), 2);
        assert_eq!(config.hooks[0].event, HookEvent::NodeComplete);
        assert_eq!(config.hooks[1].event, HookEvent::ToolEnd);
        assert_eq!(config.middlewares.len(), 2);
        assert_eq!(
            config.middlewares[0],
            MiddlewareConfig::Timeout {
                default_timeout_ms: Some(7500)
            }
        );
        assert_eq!(
            config.middlewares[1],
            MiddlewareConfig::LoopGuard { max_repeats: 2 }
        );
        assert!(config.tools.contains_key("shell"));
    }

    #[test]
    fn tools_config_applies_presets_and_overrides() {
        let toml = r#"
            [tools.bash_safe]
            enabled = true
            timeout_secs = 42

            [tools.read_source]
            enabled = true
            allowed_paths = ["/repo"]
            max_default_lines = 120

            [tools.search_docs]
            enabled = true
            blocked_queries = ["secrets"]
            max_results = 7
        "#;

        let config: ApXmConfig = toml::from_str(toml).unwrap();
        let tools = config.tools_config();

        assert!(tools.bash.enabled);
        assert_eq!(tools.bash.timeout_secs, 42);
        assert!(
            tools
                .bash
                .blocked_commands
                .iter()
                .any(|command| command == "rm recursive")
        );
        assert!(
            tools
                .bash
                .blocked_commands
                .iter()
                .any(|command| command == "sudo")
        );

        assert!(tools.read.enabled);
        assert_eq!(
            tools
                .read
                .allowed_paths
                .unwrap_or_default()
                .first()
                .cloned(),
            Some(PathBuf::from("/repo"))
        );
        assert_eq!(tools.read.max_default_lines, 120);
        assert!(
            tools
                .read
                .allowed_extensions
                .unwrap_or_default()
                .iter()
                .any(|ext| ext == "rs")
        );

        assert!(tools.search_web.enabled);
        assert_eq!(tools.search_web.max_results, 7);
        assert!(
            tools
                .search_web
                .blocked_queries
                .iter()
                .any(|term| term == "secrets")
        );
        assert!(tools.search_web.safe_search);
    }

    #[test]
    fn bash_build_preset_uses_recursive_force_rm_policy() {
        let toml = r#"
            [tools.bash_build]
            enabled = true
        "#;

        let config: ApXmConfig = toml::from_str(toml).unwrap();
        let tools = config.tools_config();

        assert!(tools.bash.enabled);
        assert_eq!(tools.bash.timeout_secs, 600);
        assert!(
            tools
                .bash
                .blocked_commands
                .iter()
                .any(|command| command == "rm recursive force"),
            "bash_build should block recursive force rm"
        );
        assert!(
            !tools
                .bash
                .blocked_commands
                .iter()
                .any(|command| command == "rm recursive"),
            "bash_build should not use the broader recursive rm policy"
        );
    }

    #[test]
    fn default_tools_config_uses_safe_bash_policy() {
        let config = ApXmConfig::default();
        let tools = config.tools_config();

        assert!(tools.bash.enabled);
        assert!(
            tools
                .bash
                .blocked_commands
                .iter()
                .any(|command| command == "rm recursive"),
            "default bash policy should block recursive rm"
        );
        assert!(
            tools
                .bash
                .blocked_commands
                .iter()
                .any(|command| command == "sudo"),
            "default bash policy should block sudo"
        );
    }

    #[test]
    fn deserialize_instruction_config() {
        let toml = r#"
            [instruction]
            ask = "You are a helpful AI assistant."
            think = "Think step by step."
            reason = "Provide structured reasoning."
            plan = "Create actionable plans."
            reflect = "Analyze execution patterns."
        "#;

        let config: ApXmConfig = toml::from_str(toml).unwrap();
        assert_eq!(
            config.instruction.ask.as_deref(),
            Some("You are a helpful AI assistant.")
        );
        assert_eq!(
            config.instruction.think.as_deref(),
            Some("Think step by step.")
        );
        assert_eq!(
            config.instruction.reason.as_deref(),
            Some("Provide structured reasoning.")
        );
        assert_eq!(
            config.instruction.plan.as_deref(),
            Some("Create actionable plans.")
        );
        assert_eq!(
            config.instruction.reflect.as_deref(),
            Some("Analyze execution patterns.")
        );
    }

    #[test]
    fn instruction_config_defaults_to_none() {
        let toml = format!(
            r#"
            [chat]
            providers = ["{MOCK_PROVIDER_NAME}"]
        "#
        );

        let config: ApXmConfig = toml::from_str(&toml).unwrap();
        assert!(config.instruction.ask.is_none());
        assert!(config.instruction.think.is_none());
        assert!(config.instruction.reason.is_none());
        assert!(config.instruction.plan.is_none());
        assert!(config.instruction.reflect.is_none());
    }

    #[test]
    #[allow(unsafe_code)]
    fn default_path_respects_home() {
        let original_apxm_home = env::var(APXM_HOME).ok();
        unsafe {
            env::remove_var(APXM_HOME);
        }
        // Use HOME if set, otherwise skip gracefully (CI without home dir)
        let home = match env::var("HOME") {
            Ok(h) => h,
            Err(_) => {
                // Try to get from system home_dir as fallback
                if let Some(h) = home_dir() {
                    h.to_string_lossy().into_owned()
                } else {
                    if let Some(value) = original_apxm_home {
                        unsafe {
                            env::set_var(APXM_HOME, value);
                        }
                    }
                    return; // skip test if no home dir available
                }
            }
        };
        let expected = PathBuf::from(home).join(".apxm").join("config.toml");
        assert_eq!(ApXmConfig::default_path().unwrap(), expected);
        if let Some(value) = original_apxm_home {
            unsafe {
                env::set_var(APXM_HOME, value);
            }
        } else {
            unsafe {
                env::remove_var(APXM_HOME);
            }
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn default_path_respects_apxm_home() {
        let dir = tempfile::tempdir().expect("temp APXM_HOME");
        let original = env::var(APXM_HOME).ok();
        unsafe {
            env::set_var(APXM_HOME, dir.path());
        }

        assert_eq!(
            ApXmConfig::default_path().unwrap(),
            dir.path().join("config.toml")
        );

        match original {
            Some(value) => unsafe {
                env::set_var(APXM_HOME, value);
            },
            None => unsafe {
                env::remove_var(APXM_HOME);
            },
        }
    }

    #[test]
    fn layered_config_merges_global_project_and_explicit_layers() {
        let dir = tempfile::tempdir().expect("temp config layers");
        let global = dir.path().join("global.toml");
        let project = dir.path().join("project.toml");
        let explicit = dir.path().join("explicit.toml");

        std::fs::write(
            &global,
            format!(
                r#"
                [chat]
                providers = ["{MOCK_PROVIDER_NAME}"]
                default_backend = "{MOCK_PROVIDER_NAME}"
                default_model = "{MOCK_MODEL_NAME}"

                [chat.routing.operation_routes.plan]
                backend = "{MOCK_PROVIDER_NAME}"
                model = "{MOCK_MODEL_NAME}"

                [tools.bash]
                enabled = false
                timeout_secs = 10
                "#
            ),
        )
        .expect("write global config");
        std::fs::write(
            &project,
            format!(
                r#"
                [chat]
                default_model = "{MOCK_MODEL_NAME_ALT}"
                planning_model = "project-planner"

                [tools.bash]
                timeout_secs = 20
                "#
            ),
        )
        .expect("write project config");
        std::fs::write(
            &explicit,
            format!(
                r#"
                [chat]
                default_backend = "{MOCK_PROVIDER_NAME_ALT}"
                "#
            ),
        )
        .expect("write explicit config");

        let config = ApXmConfig::from_layered_files([
            global.as_path(),
            project.as_path(),
            explicit.as_path(),
        ])
        .expect("layered config");

        assert_eq!(config.chat.providers, vec![MOCK_PROVIDER_NAME.to_string()]);
        assert_eq!(
            config.chat.default_backend.as_deref(),
            Some(MOCK_PROVIDER_NAME_ALT)
        );
        assert_eq!(
            config.chat.default_model.as_deref(),
            Some(MOCK_MODEL_NAME_ALT)
        );
        assert_eq!(
            config.chat.planning_model.as_deref(),
            Some("project-planner")
        );
        assert_eq!(
            config
                .chat
                .routing
                .operation_routes
                .get("plan")
                .and_then(|route| route.backend.as_deref()),
            Some(MOCK_PROVIDER_NAME)
        );
        let bash = config.tools.get("bash").expect("merged bash config");
        assert!(
            !bash.enabled,
            "omitted project key should not reset enabled"
        );
        assert_eq!(bash.timeout_secs, Some(20));
    }

    #[test]
    #[allow(unsafe_code)]
    fn load_scoped_prefers_explicit_apxm_config_env() {
        let dir = tempfile::tempdir().expect("temp config dir");
        let config_path = dir.path().join("config.toml");
        std::fs::write(
            &config_path,
            format!(
                r#"
                [chat]
                providers = ["{MOCK_PROVIDER_NAME}"]
                default_backend = "{MOCK_PROVIDER_NAME}"
                default_model = "{MOCK_MODEL_NAME}"
                "#
            ),
        )
        .expect("write explicit config");

        let original = env::var(APXM_CONFIG_ENV_VAR).ok();
        unsafe {
            env::set_var(APXM_CONFIG_ENV_VAR, &config_path);
        }

        let config = ApXmConfig::load_scoped().expect("load explicit config");
        assert_eq!(
            config.chat.default_backend.as_deref(),
            Some(MOCK_PROVIDER_NAME)
        );
        assert_eq!(config.chat.default_model.as_deref(), Some(MOCK_MODEL_NAME));

        match original {
            Some(value) => unsafe {
                env::set_var(APXM_CONFIG_ENV_VAR, value);
            },
            None => unsafe {
                env::remove_var(APXM_CONFIG_ENV_VAR);
            },
        }
    }

    #[test]
    #[allow(unsafe_code)]
    fn error_when_home_missing() {
        let original = env::var("HOME").ok();
        // SAFETY: Test runs in isolation; HOME is restored at the end.
        unsafe {
            env::remove_var("HOME");
        }
        if home_dir().is_some() {
            if let Some(value) = original {
                unsafe {
                    env::set_var("HOME", value);
                }
            }
            return;
        }
        let res = ApXmConfig::default_path();
        assert!(matches!(res, Err(ConfigError::HomeDirMissing)));
        if let Some(value) = original {
            unsafe {
                env::set_var("HOME", value);
            }
        }
    }
}
