//! Shared constants for the ACP protocol client.
//!
//! All magic strings, timeouts, and defaults are centralized here
//! following the pattern in `apxm-core/src/constants.rs`.

pub mod protocol {
    /// JSON-RPC 2.0 version string.
    pub const JSONRPC_VERSION: &str = "2.0";
    /// ACP protocol version negotiated during initialize.
    pub const ACP_PROTOCOL_VERSION: &str = "2025-11-05";
}

pub mod client {
    /// Client name reported in initialize.
    pub const CLIENT_NAME: &str = "apxm";
    /// Client version reported in initialize.
    pub const CLIENT_VERSION: &str = "0.1.0";
}

pub mod methods {
    pub const INITIALIZE: &str = "initialize";
    pub const AUTHENTICATE: &str = "authenticate";
    pub const SESSION_NEW: &str = "session/new";
    pub const SESSION_PROMPT: &str = "session/prompt";
    pub const SESSION_UPDATE: &str = "session/update";
    pub const SESSION_SET_MODE: &str = "session/set_mode";
    pub const SESSION_SET_CONFIG_OPTION: &str = "session/set_config_option";
    pub const SESSION_CANCEL: &str = "session/cancel";
    pub const UNSTABLE_SET_SESSION_MODEL: &str = "unstable_setSessionModel";
    pub const FS_READ_TEXT_FILE: &str = "fs/readTextFile";
    pub const FS_WRITE_TEXT_FILE: &str = "fs/writeTextFile";
    pub const TERMINAL_CREATE: &str = "terminal/create";
    pub const TERMINAL_OUTPUT: &str = "terminal/output";
    pub const TERMINAL_WAIT_FOR_EXIT: &str = "terminal/waitForTerminalExit";
    pub const TERMINAL_KILL: &str = "terminal/kill";
    pub const TERMINAL_RELEASE: &str = "terminal/release";
    pub const REQUEST_PERMISSION: &str = "requestPermission";
}

pub mod update_types {
    pub const AGENT_MESSAGE_CHUNK: &str = "agent_message_chunk";
    pub const USAGE_UPDATE: &str = "usage_update";
}

/// ACP protocol wire-format field names (camelCase as on the wire).
pub mod fields {
    pub const SESSION_ID: &str = "sessionId";
    pub const TERMINAL_ID: &str = "terminalId";
    pub const STOP_REASON: &str = "stopReason";
    pub const PROTOCOL_VERSION: &str = "protocolVersion";
    pub const CLIENT_CAPABILITIES: &str = "clientCapabilities";
    pub const CLIENT_INFO: &str = "clientInfo";
    pub const AUTH_METHODS: &str = "authMethods";
    pub const METHOD_ID: &str = "methodId";
    pub const AGENT_CAPABILITIES: &str = "agentCapabilities";
    pub const INPUT_TOKENS: &str = "inputTokens";
    pub const OUTPUT_TOKENS: &str = "outputTokens";
    pub const EXIT_STATUS: &str = "exitStatus";
    pub const EXIT_CODE: &str = "exitCode";
    pub const TOOL_CALL: &str = "toolCall";
    pub const OPTION_ID: &str = "optionId";
    pub const MODE_ID: &str = "modeId";
    pub const CONFIG_ID: &str = "configId";
    pub const CREDENTIAL: &str = "credential";
}

pub mod tool_kinds {
    pub const READ: &str = "read";
    pub const WRITE: &str = "write";
    pub const SEARCH: &str = "search";
}

/// Prefixes for permission option kinds in requestPermission.
pub mod option_kinds {
    pub const ALLOW: &str = "allow";
    pub const REJECT: &str = "reject";
}

/// Outcome values for requestPermission responses.
pub mod outcomes {
    pub const SELECTED: &str = "selected";
    pub const CANCELLED: &str = "cancelled";
}

pub mod stop_reasons {
    pub const END_TURN: &str = "end_turn";
}

/// Capability argument names used in schema and extraction.
pub mod args {
    pub const AGENT: &str = "agent";
    pub const PROMPT: &str = "prompt";
    pub const CWD: &str = "cwd";
    pub const SESSION_HANDLE: &str = "session_handle";
    pub const MODE: &str = "mode";
    pub const MODEL: &str = "model";
    // Fallback arg names for positional arguments
    pub const ARG_AGENT: &str = "arg_agent";
    pub const ARG_PROMPT: &str = "arg_prompt";
    pub const ARG0: &str = "arg0";
    pub const ARG1: &str = "arg1";
}

/// Keys for the result Value map returned from AcpCapability::execute().
pub mod result_keys {
    pub const TEXT: &str = "text";
    pub const AGENT: &str = "agent";
    pub const STOP_REASON: &str = "stop_reason";
    pub const SESSION_ID: &str = "session_id";
    pub const AGENT_SESSION_ID: &str = "agent_session_id";
    pub const TURN: &str = "turn";
    pub const MODEL: &str = "model";
    pub const INPUT_TOKENS: &str = "input_tokens";
    pub const OUTPUT_TOKENS: &str = "output_tokens";
}

pub mod auth {
    /// Environment variable prefix for ACP authentication.
    pub const ENV_PREFIX: &str = "ACPX_AUTH_";
}

pub mod capability {
    /// Capability name for ACP agent invocation via INV nodes.
    pub const ACP_CAPABILITY_NAME: &str = "acp";
    /// Default session handle when none is specified.
    pub const DEFAULT_SESSION_HANDLE: &str = "main";
    /// Default estimated latency for ACP invocations (ms).
    pub const DEFAULT_LATENCY_MS: u64 = 30_000;
}

pub mod timeouts {
    /// Grace period after closing stdin before SIGTERM (ms).
    pub const DEFAULT_CLOSE_GRACE_MS: u64 = 100;
    /// Extended grace for agents that need longer (e.g. qoder) (ms).
    pub const QODER_CLOSE_GRACE_MS: u64 = 750;
    /// Default session create timeout (ms).
    pub const DEFAULT_SESSION_TIMEOUT_MS: u64 = 30_000;
    /// Claude-specific session create timeout (ms).
    pub const CLAUDE_SESSION_TIMEOUT_MS: u64 = 60_000;
    /// Gemini-specific session create timeout (ms).
    pub const GEMINI_SESSION_TIMEOUT_MS: u64 = 15_000;
    /// Time to wait after SIGTERM before SIGKILL (ms).
    pub const SIGTERM_GRACE_MS: u64 = 1500;
    /// Time to wait after SIGKILL before giving up (ms).
    pub const SIGKILL_GRACE_MS: u64 = 1000;
    /// Timeout for authentication requests (secs).
    pub const AUTH_TIMEOUT_SECS: u64 = 10;
    /// Timeout for session control requests (secs).
    pub const CONTROL_TIMEOUT_SECS: u64 = 10;
}

pub mod terminal {
    /// Maximum output buffer size per terminal (bytes).
    pub const DEFAULT_OUTPUT_LIMIT: usize = 65536;
    /// Timeout for non-blocking read polling (ms).
    pub const READ_POLL_TIMEOUT_MS: u64 = 50;
}

pub mod registry {
    /// TOML file header for user agent profiles.
    pub const FILE_HEADER: &str =
        "# APXM ACP Agent Profiles\n# User overrides — managed by `apxm agent`\n\n";
    /// File name for user agent config.
    pub const AGENTS_FILENAME: &str = "agents.toml";
}

pub mod permission_modes {
    pub const APPROVE_ALL: &str = "approve-all";
    pub const APPROVE_READS: &str = "approve-reads";
    pub const DENY_ALL: &str = "deny-all";
}

pub mod json_rpc_errors {
    /// Standard JSON-RPC error code for internal errors.
    pub const INTERNAL_ERROR: i64 = -32000;
    /// Standard JSON-RPC error code for method not found.
    pub const METHOD_NOT_FOUND: i64 = -32601;
}
