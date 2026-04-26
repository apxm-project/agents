//! Shared string constants for cross-crate protocol fields.
//!
//! Keep graph contract keys and common node attribute keys centralized here
//! so APXM and AgentMate frontends/backends stay consistent.

pub mod diagnostics {
    /// Compile diagnostics mode for canonical graph input.
    pub const MODE_GRAPH: &str = "graph";
    /// Compile diagnostics mode for `.air` (raw MLIR text) input.
    pub const MODE_AIR: &str = "air";
}

pub mod env {
    pub const APXM_BACKEND: &str = "APXM_BACKEND";
    /// Makes Python graph files emit AIR to stdout for the Rust compiler driver.
    pub const APXM_EMIT_AIR: &str = "APXM_EMIT_AIR";
    /// Enables the in-process mock backend used by tests and offline benchmarks.
    pub const APXM_MOCK_BACKEND: &str = "APXM_MOCK_BACKEND";
    /// Configures mock backend latency in milliseconds.
    pub const APXM_MOCK_LATENCY_MS: &str = "APXM_MOCK_LATENCY_MS";
    pub const LLVM_DIR: &str = "LLVM_DIR";
    pub const MLIR_DIR: &str = "MLIR_DIR";
    pub const PYTHONPATH: &str = "PYTHONPATH";

    pub mod flag_values {
        pub const ENABLED: &str = "1";
    }
}

pub mod inner_plan {
    /// Payload key for graph JSON in structured inner-plan outputs.
    pub const GRAPH_PAYLOAD: &str = "graph";
    /// Payload key for structured task DAG in inner-plan outputs.
    pub const TASK_DAG: &str = "task_dag";
}

pub mod graph {
    pub mod metadata {
        /// Graph metadata flag indicating entry flow.
        pub const IS_ENTRY: &str = "is_entry";
        /// Prefix for generated graph ids when the input graph has no stable name.
        pub const GENERATED_GRAPH_ID_PREFIX: &str = "dag-";
        /// Prefix for generated node display names when a node has no metadata name.
        pub const GENERATED_NODE_NAME_PREFIX: &str = "node_";
        /// Priority value at or above which graph-aware backends should treat a
        /// node as critical-path work.
        pub const CRITICAL_PATH_PRIORITY_THRESHOLD: i64 = 70;
    }

    pub mod backend_kind {
        pub const VLLM: &str = "vllm";
        pub const GENERIC: &str = "generic";
    }

    pub mod attrs {
        include!(concat!(env!("OUT_DIR"), "/apxm_graph_attrs.rs"));
    }
}

pub mod runtime {
    pub mod metadata {
        pub const PARENT_EXECUTION_ID: &str = "parent_execution_id";
        pub const SCOPE_ID: &str = "scope_id";
        pub const PARENT_SCOPE_ID: &str = "parent_scope_id";
        pub const SESSION_DIR: &str = "session_dir";
        pub const SESSION_ROOT: &str = "session_root";
        pub const DELEGATE_TASK_SPEC: &str = "delegate_task_spec";
        pub const DELEGATE_TARGET: &str = "delegate_target";
        pub const NEGOTIATE_PROPOSAL: &str = "negotiate_proposal";
        pub const NEGOTIATE_ROUND: &str = "negotiate_round";
        pub const NEGOTIATE_PARTY: &str = "negotiate_party";
        pub const COMMUNICATE_SENDER: &str = "communicate_sender";
        pub const COMMUNICATE_RECIPIENT: &str = "communicate_recipient";
        pub const COMMUNICATE_MODE: &str = "communicate_mode";
        pub const FLOW_CALL_DEPTH: &str = "flow_call_depth";
        pub const TARGET_AGENT: &str = "target_agent";
        pub const TARGET_FLOW: &str = "target_flow";
    }

    pub mod context_stack {
        pub const DEFAULT_PROFILE: &str = "default";
        pub const PROFILE_REVIEWER: &str = "reviewer";

        pub const DEFAULT_PROMPT_BUDGET_TOKENS: usize = 16_000;
        pub const DEFAULT_UPSTREAM_DEPTH: usize = 1;
        pub const DEFAULT_UPSTREAM_FRAME_BUDGET_TOKENS: usize = 2_000;
        pub const DEFAULT_SESSION_FRAME_BUDGET_TOKENS: usize = 200;

        pub const REVIEWER_UPSTREAM_DEPTH: usize = usize::MAX;
        pub const REVIEWER_UPSTREAM_FRAME_BUDGET_TOKENS: usize = 2_000;
    }

    pub mod belief_keys {
        pub const STAGED_PREFIX: &str = "_stage:";
        pub const DELEGATE_PREFIX: &str = "_delegate:";
        pub const NEGOTIATE_ACTIVE: &str = "_negotiate_active";
        pub const NEGOTIATE_PROPOSAL: &str = "_negotiate_proposal";
        pub const AUTONOMOUS_NODE_PREFIX: &str = "_autonomous_node:";
        pub const IDENTITY_NODE_PREFIX: &str = "_identity_node:";
        pub const SPAWNED_AGENT_PREFIX: &str = "_spawned_agent:";
        pub const AGENT_INFO_PREFIX: &str = "_agent_info:";
        pub const REGISTERED_CAPABILITY_PREFIX: &str = "_registered_capability:";
        pub const CAPABILITY_REGISTERED_PREFIX: &str = "_capability_registered:";
        pub const DELEGATE_TASK_SPEC: &str = "_delegate_task_spec";
        pub const DELEGATE_INPUT: &str = "_delegate_input";
        pub const PENDING_COMMUNICATE_PREFIX: &str = "_pending_communicate:";
        pub const COMMUNICATE_MESSAGE: &str = "_communicate_message";
        pub const PENDING_FLOW_CALL_PREFIX: &str = "_pending_flow_call:";
        pub const FLOW_ARG_PREFIX: &str = "_flow_arg_";
        pub const BRANCH_PREFIX: &str = "_branch:";
        pub const GUARD_PREFIX: &str = "_guard:";
        pub const SWITCH_PREFIX: &str = "_switch:";
        pub const INV_PREFIX: &str = "_inv:";
        pub const LLM_RESULT_PREFIX: &str = "_llm_result:";
        pub const REFLECT_PREFIX: &str = "_reflect:";
        pub const VERIFY_PREFIX: &str = "_verify:";
        pub const PRINT_PREFIX: &str = "_print:";
        pub const ERR_PREFIX: &str = "_err:";
        pub const PAUSE_PREFIX: &str = "_pause:";
        pub const RESUME_PREFIX: &str = "_resume:";
        pub const CLAIM_PREFIX: &str = "_claim:";
        pub const CLAIM_TOKEN_PREFIX: &str = "_claim_token:";
        pub const EXC_PREFIX: &str = "exc:";
        pub const CHECKPOINT_SNAPSHOT_PREFIX: &str = "_checkpoint_snapshot:";
        pub const LOOP_START_PREFIX: &str = "_loop_start:";
        pub const LOOP_END_PREFIX: &str = "_loop_end:";
        pub const EXECUTION_ID: &str = "_execution_id";
        pub const PLAN_PREFIX: &str = "plan:";
        pub const GOAL_PREFIX: &str = "goal:";
        pub const TOOL_RESULTS_PREFIX: &str = "tool_results:";
        pub const GOALS_PREFIX: &str = "goals:";
        pub const INNER_PLAN_SPLICED_PREFIX: &str = "inner_plan_spliced:";
        pub const INSIGHT_PREFIX: &str = "insight:";
        /// Prefix for internal beliefs that should not be transmitted to external agents.
        pub const INTERNAL_PREFIX: &str = "_";
    }

    pub mod transition_labels {
        pub const IDENTITY: &str = "identity";
        pub const NEGOTIATE_START: &str = "negotiate_start";
        pub const NEGOTIATE_COMPLETE: &str = "negotiate_complete";
        pub const SCOPE_INHERIT_BELIEF: &str = "scope:inherit_belief";
        pub const SCOPE_FILTER_BELIEF: &str = "scope:filter_belief";
        pub const SCOPE_INHERIT_CAPABILITY: &str = "scope:inherit_capability";
        pub const SCOPE_FILTER_CAPABILITY: &str = "scope:filter_capability";
        pub const SCOPE_INHERIT_GOAL: &str = "scope:inherit_goal";
        pub const SCOPE_FILTER_GOAL: &str = "scope:filter_goal";
    }

    pub mod response_keys {
        pub const TASK_HANDLE: &str = "task_handle";
        pub const RESULT: &str = "result";
        pub const CONSENSUS: &str = "consensus";
        pub const PROPOSAL: &str = "proposal";
        pub const RESPONSES: &str = "responses";
        pub const CAPABILITY_NAME: &str = "capability_name";
        pub const DESCRIPTION: &str = "description";
        pub const REGISTERED: &str = "registered";
        pub const NAME: &str = "name";
        pub const TEAM_NAME: &str = "team_name";
        pub const MEMBERS: &str = "members";
        pub const SPAWNED_BY: &str = "spawned_by";
        pub const CAPABILITIES: &str = "capabilities";
        pub const GOALS: &str = "goals";
        pub const PROCESS_ID: &str = "process_id";
        pub const PROFILE: &str = "profile";
        pub const SYSTEM_PROMPT: &str = "system_prompt";
        pub const BACKEND: &str = "backend";
        pub const MODEL: &str = "model";
    }

    /// Runtime-owned structured response contract for spawned-agent prompt turns.
    ///
    /// Adapters such as ACP populate these fields, but the contract belongs to
    /// the APXM runtime so core execution and metrics stay frontend/backend
    /// agnostic.
    pub mod agent_result_keys {
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
}

pub mod memory {
    pub const STM: &str = "stm";
    pub const LTM: &str = "ltm";
    pub const EPISODIC: &str = "episodic";
}

pub mod sandbox {
    pub mod executables {
        pub const BASH: &str = "bash";
        pub const BUBBLEWRAP: &str = "bwrap";
        pub const SHELL: &str = "sh";
    }

    pub mod backend_names {
        pub const PROCESS: &str = "apxm-process";
        pub const BUBBLEWRAP: &str = "apxm-bwrap";
    }

    pub mod session_prefixes {
        pub const PROCESS: &str = "process";
        pub const BUBBLEWRAP: &str = "bwrap";
        pub const SCRATCH: &str = "scratch";
        pub const WORKDIR: &str = "workdir";
        pub const SCRIPT: &str = "script";
    }

    pub mod env {
        pub const PATH: &str = "PATH";
        pub const HOME: &str = "HOME";
        pub const LANG: &str = "LANG";
        pub const LC_ALL: &str = "LC_ALL";
        pub const TERM: &str = "TERM";
        pub const TMPDIR: &str = "TMPDIR";
        pub const TEMP: &str = "TEMP";
        pub const TMP: &str = "TMP";

        pub const SAFE_PASSTHROUGH: &[&str] = &[PATH, HOME, LANG, LC_ALL, TERM];

        pub const AWS_SECRET_ACCESS_KEY: &str = "AWS_SECRET_ACCESS_KEY";
        pub const AWS_ACCESS_KEY_ID: &str = "AWS_ACCESS_KEY_ID";
        pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
        pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
        pub const DATABASE_URL: &str = "DATABASE_URL";
        pub const SECRET_KEY: &str = "SECRET_KEY";
        pub const PRIVATE_KEY: &str = "PRIVATE_KEY";

        pub const BLOCKED_DEFAULTS: &[&str] = &[
            AWS_SECRET_ACCESS_KEY,
            AWS_ACCESS_KEY_ID,
            OPENAI_API_KEY,
            ANTHROPIC_API_KEY,
            DATABASE_URL,
            SECRET_KEY,
            PRIVATE_KEY,
        ];
    }

    pub mod bubblewrap {
        pub const VERSION_ARG: &str = "--version";
        pub const FLAG_NEW_SESSION: &str = "--new-session";
        pub const FLAG_DIE_WITH_PARENT: &str = "--die-with-parent";
        pub const FLAG_RO_BIND: &str = "--ro-bind";
        pub const FLAG_BIND: &str = "--bind";
        pub const FLAG_DEV: &str = "--dev";
        pub const FLAG_PROC: &str = "--proc";
        pub const FLAG_DIR: &str = "--dir";
        pub const FLAG_CHDIR: &str = "--chdir";
        pub const FLAG_UNSHARE_USER: &str = "--unshare-user";
        pub const FLAG_UNSHARE_PID: &str = "--unshare-pid";
        pub const FLAG_UNSHARE_NET: &str = "--unshare-net";
        pub const FLAG_SEPARATOR: &str = "--";
        pub const FILESYSTEM_ROOT: &str = "/";
        pub const FILESYSTEM_DEV: &str = "/dev";
        pub const FILESYSTEM_PROC: &str = "/proc";
        pub const TMP_DIR: &str = "/apxm-tmp";
        pub const WORKDIR: &str = "/apxm-workdir";
        pub const WARN_READ_ALLOWLISTS: &str = "bubblewrap backend currently enforces read-only root plus writable carve-outs, not per-path read allowlists";
        pub const ERR_NOT_AVAILABLE: &str = "bubblewrap is not available on this host";
        pub const ERR_ONLY_LINUX: &str = "bubblewrap backend is only supported on Linux";
        pub const ERR_SESSION_STATE: &str = "invalid bubblewrap session state";
        pub const ERR_WORKDIR_NOT_DIRECTORY: &str = "sandbox working directory must be a directory";
        pub const ERR_TIMED_OUT: &str = "command timed out and was killed";
        pub const ERR_EXECUTION_PREFIX: &str = "bubblewrap sandbox";
    }

    pub mod shell_args {
        pub const COMMAND: &str = "-c";
        pub const LOGIN_COMMAND: &str = "-lc";
    }

    pub mod messages {
        pub const COMMAND_BLOCKED_BY_POLICY: &str = "command blocked by sandbox policy";
        pub const PROCESS_TIMED_OUT_AND_KILLED: &str = "Process timed out and was killed";
    }
}

pub mod protocols {
    /// MCP (Model Context Protocol) version string.
    pub const MCP_VERSION: &str = "2025-11-05";
    /// A2A (Agent-to-Agent) protocol version.
    pub const A2A_VERSION: &str = "0.3";
}

pub mod acp {
    pub mod session_params {
        pub const MCP_SERVERS: &str = "mcpServers";
        pub const CWD: &str = "cwd";
    }

    pub mod client_capabilities {
        pub const FS: &str = "fs";
        pub const READ_TEXT_FILE: &str = "readTextFile";
        pub const WRITE_TEXT_FILE: &str = "writeTextFile";
        pub const TERMINAL: &str = "terminal";
        pub const NAME: &str = "name";
        pub const VERSION: &str = "version";
    }

    pub mod reverse_params {
        pub const PATH: &str = "path";
        pub const LINE: &str = "line";
        pub const CONTENT: &str = "content";
        pub const COMMAND: &str = "command";
        pub const ARGS: &str = "args";
        pub const CWD: &str = "cwd";
        pub const ENV: &str = "env";
        pub const NAME: &str = "name";
        pub const VALUE: &str = "value";
        pub const OPTIONS: &str = "options";
        pub const KIND: &str = "kind";
        pub const OFFSET: &str = "offset";
    }

    pub mod reverse_response {
        pub const CONTENT: &str = "content";
        pub const OUTPUT: &str = "output";
        pub const TRUNCATED: &str = "truncated";
        pub const SIGNAL: &str = "signal";
        pub const OUTCOME: &str = "outcome";
    }

    pub mod notification {
        pub const UPDATE: &str = "update";
        pub const SESSION_UPDATE: &str = "sessionUpdate";
        pub const TYPE: &str = "type";
        pub const TEXT: &str = "text";
        pub const USED: &str = "used";
        pub const SIZE: &str = "size";
    }

    pub mod set_session {
        pub const MODEL: &str = "model";
        pub const VALUE: &str = "value";
    }
}

pub mod jsonrpc {
    pub const ID: &str = "id";
    pub const METHOD: &str = "method";
    pub const PARAMS: &str = "params";
    pub const RESULT: &str = "result";
    pub const ERROR: &str = "error";
    pub const JSONRPC: &str = "jsonrpc";
    pub const VERSION: &str = "2.0";

    pub mod error_codes {
        pub const PARSE_ERROR: i64 = -32700;
        pub const METHOD_NOT_FOUND: i64 = -32601;
        pub const INVALID_PARAMS: i64 = -32602;
        pub const INTERNAL_ERROR: i64 = -32000;
    }
}

/// COMMUNICATE operation protocol dispatch modes.
pub mod communicate_protocols {
    /// In-process sub-flow execution via FlowRegistry.
    pub const LOCAL: &str = "local";
    /// HTTP POST to an external APXM agent's `/v1/receive` endpoint.
    pub const HTTP: &str = "http";
    /// HTTPS variant of the HTTP protocol.
    pub const HTTPS: &str = "https";
    /// ACP JSON-RPC over stdio to a spawned agent subprocess.
    pub const ACP: &str = "acp";
    /// Fan-out to ALL registered agents in parallel.
    pub const BROADCAST: &str = "broadcast";
}

pub mod capabilities {
    pub const BASH: &str = "bash";
    pub const READ: &str = "read";
    pub const WRITE: &str = "write";
    pub const SEARCH_WEB: &str = "search_web";

    pub const BUILTINS: &[&str] = &[BASH, READ, WRITE, SEARCH_WEB];
}

pub mod http {
    pub mod headers {
        pub const CONTENT_TYPE: &str = "content-type";
        pub const CONTENT_TYPE_JSON: &str = "application/json";
        pub const AUTHORIZATION: &str = "authorization";
        pub const X_REQUEST_ID: &str = "x-request-id";
        pub const X_API_KEY: &str = "x-api-key";
        pub const ANTHROPIC_VERSION: &str = "anthropic-version";
        pub const X_GOOG_API_KEY: &str = "x-goog-api-key";
    }
}

pub mod llm {
    pub mod message_keys {
        pub const ROLE: &str = "role";
        pub const CONTENT: &str = "content";
        pub const TYPE: &str = "type";
        pub const TEXT: &str = "text";
        pub const IMAGE: &str = "image";
        pub const IMAGE_URL: &str = "image_url";
        pub const SOURCE: &str = "source";
        pub const URL: &str = "url";
        pub const MODEL: &str = "model";
        pub const MESSAGES: &str = "messages";
        pub const MAX_TOKENS: &str = "max_tokens";
        pub const TEMPERATURE: &str = "temperature";
        pub const STREAM: &str = "stream";
    }

    pub mod tool_keys {
        pub const TOOLS: &str = "tools";
        pub const TOOL_CHOICE: &str = "tool_choice";
        pub const TOOL_USE: &str = "tool_use";
        pub const TOOL_RESULT: &str = "tool_result";
        pub const TOOL_CALLS: &str = "tool_calls";
        pub const FUNCTION: &str = "function";
        pub const NAME: &str = "name";
        pub const ID: &str = "id";
        pub const INPUT: &str = "input";
        pub const INPUT_SCHEMA: &str = "input_schema";
        pub const ARGUMENTS: &str = "arguments";
    }

    pub mod roles {
        pub const USER: &str = "user";
        pub const ASSISTANT: &str = "assistant";
        pub const SYSTEM: &str = "system";
        pub const TOOL: &str = "tool";
    }

    pub mod finish_reasons {
        pub const STOP: &str = "stop";
        pub const TOOL_USE: &str = "tool_use";
        pub const LENGTH: &str = "length";
        pub const ERROR: &str = "error";
    }

    pub mod streaming {
        pub const DELTA: &str = "delta";
        pub const CHOICES: &str = "choices";
        pub const TEXT_DELTA: &str = "text_delta";
    }

    pub mod api_paths {
        pub const VERSION_PREFIX: &str = "/v1";
        pub const CHAT_COMPLETIONS: &str = "/chat/completions";
        pub const MODELS: &str = "/models";
        pub const MESSAGES: &str = "/messages";
        pub const API_CHAT: &str = "/api/chat";
        pub const API_TAGS: &str = "/api/tags";
        pub const APXM_GRAPHS: &str = "/apxm/graphs";
        pub const APXM_GRAPHS_REGISTER: &str = "/apxm/graphs/register";
    }

    pub mod apxm {
        pub const OBJECT_GRAPH_REGISTRATION: &str = "apxm.graph.registration";
        pub const OBJECT_GRAPH_STATUS: &str = "apxm.graph.status";
        pub const OBJECT_GRAPH_RELEASE: &str = "apxm.graph.release";
        pub const OBJECT: &str = "object";
        pub const REQUEST_PRIORITY: &str = "priority";
        pub const VLLM_XARGS: &str = "vllm_xargs";
        pub const HINTS_FIELD: &str = "apxm";
        pub const SCHEMA_VERSION: &str = "schema_version";
        pub const GRAPH_ID: &str = "graph_id";
        pub const EXECUTION_ID: &str = "execution_id";
        pub const NODE_ID: &str = "node_id";
        pub const NODE_NAME: &str = "node_name";
        pub const PRIORITY_CLASS: &str = "priority_class";
        pub const DOWNSTREAM_NODES: &str = "downstream_nodes";
        pub const REUSE_GROUP: &str = "reuse_group";
        pub const PIN_POLICY: &str = "pin_policy";
        pub const PIN_POLICY_MODE: &str = "mode";
        pub const PIN_POLICY_TTL_MS: &str = "ttl_ms";
        pub const COMPILER_HINTS: &str = "compiler_hints";
        pub const SHARED_PREFIX_EST_TOKENS: &str = "shared_prefix_est_tokens";
        pub const WARMUP_CANDIDATE: &str = "warmup_candidate";
        pub const REGISTERED_NODES: &str = "registered_nodes";
        pub const CRITICAL_PATH_LENGTH: &str = "critical_path_length";
        pub const MAX_PARALLELISM: &str = "max_parallelism";
        pub const DEFAULT_PIN_TTL_MS: &str = "default_pin_ttl_ms";
        pub const RELEASED_HANDLES: &str = "released_handles";
        pub const RELEASED_BLOCKS: &str = "released_blocks";
        pub const REMAINING_HANDLES: &str = "remaining_handles";
        pub const REMAINING_BLOCKS: &str = "remaining_blocks";

        pub const PRIORITY_CRITICAL_PATH: &str = "critical_path";
        pub const PRIORITY_PARALLEL: &str = "parallel";
        pub const PRIORITY_NORMAL_LEGACY: &str = "normal";
        pub const PRIORITY_SPECULATIVE_LEGACY: &str = "speculative";

        pub const PIN_MODE_PREFIX: &str = "prefix";
        pub const PIN_MODE_NONE: &str = "none";
        pub const PIN_MODE_PREFIX_LEGACY: &str = "pin_strong";
        pub const PIN_MODE_NONE_LEGACY: &str = "pin_weak";
    }

    pub mod vllm {
        pub const APXM_PROBE_GRAPH_ID: &str = "__apxm_probe__";
    }

    pub mod anthropic_events {
        pub const MESSAGE_START: &str = "message_start";
        pub const CONTENT_BLOCK_START: &str = "content_block_start";
        pub const CONTENT_BLOCK_DELTA: &str = "content_block_delta";
        pub const CONTENT_BLOCK_STOP: &str = "content_block_stop";
        pub const MESSAGE_DELTA: &str = "message_delta";
        pub const MESSAGE_STOP: &str = "message_stop";
        pub const ERROR: &str = "error";
    }

    pub mod sse {
        pub const DATA_PREFIX: &str = "data: ";
        pub const EVENT_PREFIX: &str = "event: ";
        pub const DONE_MARKER: &str = "[DONE]";
    }

    pub mod config_keys {
        pub const EXTRA_HEADERS: &str = "extra_headers";
        pub const ENV_PREFIX: &str = "env:";
        /// Whether the backend accepts `tool_choice="auto"` on chat-completion
        /// requests. Stock vLLM rejects it unless launched with
        /// `--enable-auto-tool-choice`. Plumbed from `BackendConfig.auto_tool_choice`.
        pub const AUTO_TOOL_CHOICE: &str = "auto_tool_choice";
        /// vLLM-only: when present and `false`, the graph-aware vLLM backend
        /// allows a stock (non-fork) server. Default behavior (key absent or
        /// `true`) is to hard-fail at `health_check` if `/v1/apxm/*` is missing,
        /// because stock vLLM silently drops `vllm_xargs.apxm` hints.
        /// Plumbed from `BackendConfig.require_apxm_endpoints`.
        pub const REQUIRE_APXM_ENDPOINTS: &str = "require_apxm_endpoints";
        /// Per-model array forwarded to the backend so it can apply
        /// model-specific request shaping (e.g. disabling thinking-mode for
        /// Qwen3). Each entry carries at least `id` and `supports_thinking`.
        pub const MODELS: &str = "models";
        /// Per-model flag: when `false`, the backend must instruct the server
        /// to suppress chain-of-thought output. For vLLM/Qwen3 this maps to
        /// `chat_template_kwargs.enable_thinking = false`.
        pub const SUPPORTS_THINKING: &str = "supports_thinking";
        /// Per-model flag: when `false`, the backend must omit the explicit
        /// `temperature` field and let the provider default apply.
        pub const SUPPORTS_CUSTOM_TEMPERATURE: &str = "supports_custom_temperature";
        /// Backend-level flag: when `false`, the OpenAI-compatible adapter
        /// keeps APXM `output_schema` as runtime validation only and does not
        /// send provider-specific structured-output request fields.
        pub const SUPPORTS_STRUCTURED_OUTPUTS: &str = "supports_structured_outputs";
        /// Top-level body key recognised by vLLM's OpenAI-compatible endpoint
        /// to forward kwargs into the model's chat template (e.g.
        /// `{"enable_thinking": false}` for Qwen3).
        pub const CHAT_TEMPLATE_KWARGS: &str = "chat_template_kwargs";
        /// Chat-template kwarg consumed by Qwen3 (and compatible) templates
        /// to gate `<think>...</think>` emission.
        pub const ENABLE_THINKING: &str = "enable_thinking";
    }

    pub mod tags {
        pub const DEAD_ENDPOINT: &str = "dead-endpoint";
    }

    pub mod openai {
        pub const MODEL: &str = "model";
        pub const MESSAGES: &str = "messages";
        pub const ROLE: &str = "role";
        pub const CONTENT: &str = "content";
        pub const TEMPERATURE: &str = "temperature";
        pub const EXTRA_BODY: &str = "extra_body";
        pub const TOP_P: &str = "top_p";
        pub const FREQUENCY_PENALTY: &str = "frequency_penalty";
        pub const PRESENCE_PENALTY: &str = "presence_penalty";
        pub const STOP: &str = "stop";
        pub const RESPONSE_FORMAT: &str = "response_format";
        pub const RESPONSE_FORMAT_TYPE: &str = "type";
        pub const RESPONSE_FORMAT_JSON_SCHEMA: &str = "json_schema";
        pub const JSON_SCHEMA: &str = "json_schema";
        pub const JSON_SCHEMA_NAME: &str = "name";
        pub const JSON_SCHEMA_SCHEMA: &str = "schema";
        pub const JSON_SCHEMA_STRICT: &str = "strict";
        pub const APXM_OUTPUT_SCHEMA_NAME: &str = "apxm_output";
    }

    pub mod vllm_request {
        pub const THINKING_TOKEN_BUDGET: &str = "thinking_token_budget";
    }

    pub mod backend_metadata {
        pub const BACKEND_TYPE: &str = "backend_type";
        pub const VLLM_GRAPH_AWARE: &str = "vllm-graph-aware";
    }

    pub mod google {
        pub const USAGE_METADATA: &str = "usageMetadata";
        pub const PROMPT_TOKEN_COUNT: &str = "promptTokenCount";
        pub const CANDIDATES_TOKEN_COUNT: &str = "candidatesTokenCount";
    }

    pub mod ollama {
        pub const NUM_PREDICT: &str = "num_predict";
    }
}

pub mod parameters {
    /// Valid parameter types for graph parameters.
    pub const VALID_TYPES: &[&str] = &["str", "int", "float", "bool", "json"];
}

pub mod extensions {
    /// Agent IR text format — canonical intermediate representation (like LLVM .ll).
    /// This is the primary authoring format for graph IR.
    pub const AIR: &str = "air";
    /// Compiled artifact extension.
    pub const ARTIFACT: &str = "apxmobj";
    /// JSON graph source file extension.
    pub const JSON: &str = "json";
}

pub mod cache {
    /// SQLite cache database filename.
    pub const DB_FILE: &str = "cache.db";
    /// DSPy optimization cache subdirectory.
    pub const DSPY_DIR: &str = "dspy";
}

pub mod dspy {
    /// Graph metadata key for DSPy configuration.
    pub const METADATA_KEY: &str = "dspy";
    /// Training data path field in DSPy metadata.
    pub const TRAINING_DATA: &str = "training_data";
    /// Optimizer name field (mipro, bootstrap, copro).
    pub const OPTIMIZER: &str = "optimizer";
    /// Auto-tuning level (light, medium, heavy).
    pub const AUTO: &str = "auto";
    /// Metric function name (token_overlap, exact_match, contains, llm_judge).
    pub const METRIC: &str = "metric";
    /// Backend name for DSPy LLM calls.
    pub const BACKEND: &str = "backend";
    /// MLIR module attribute: training data path.
    pub const ATTR_TRAINING_DATA_PATH: &str = "ais.dspy_training_data_path";
    /// MLIR module attribute: backend config JSON.
    pub const ATTR_BACKEND_JSON: &str = "ais.dspy_backend_json";
    /// MLIR module attribute: optimizer name.
    pub const ATTR_OPTIMIZER: &str = "ais.dspy_optimizer";
    /// MLIR module attribute: auto-tuning level.
    pub const ATTR_AUTO: &str = "ais.dspy_auto";
    /// MLIR module attribute: metric function.
    pub const ATTR_METRIC: &str = "ais.dspy_metric";
    /// MLIR module attribute: count of optimized templates.
    pub const ATTR_OPTIMIZED: &str = "ais.dspy_optimized";
}

pub mod session {
    pub mod files {
        pub const MANIFEST: &str = "manifest.json";
        pub const INPUT_GRAPH: &str = "input.air";
        pub const RESULTS: &str = "results.json";
        pub const METRICS: &str = "metrics.json";
        pub const NODE_STATUSES: &str = "node_statuses.json";
        pub const TRACE: &str = "trace.ndjson";
        pub const LIVE: &str = "live.json";
        pub const NODES_DIR: &str = "nodes";
    }

    /// JSON keys serialized into `results.json`. Mirrored on the Python side
    /// by `tools/quality_eval/_keys.py::ResultsKeys`. Drift breaks tier-3.
    pub mod results_keys {
        pub const NODE_OUTPUTS: &str = "node_outputs";
        pub const TOKEN_VALUES: &str = "token_values";
        pub const EXIT_VALUES: &str = "exit_values";
        pub const FINAL_NODE_ID: &str = "final_node_id";
        pub const FINAL_OUTPUT: &str = "final_output";
    }

    /// JSON keys serialized into `metrics.json` by the runtime's
    /// `TokenAccountingSnapshot::to_json`. Mirrored by `MetricsKeys` on the
    /// Python side; tier-3 budget enforcement relies on these.
    pub mod metrics_keys {
        pub const TOKEN_ACCOUNTING: &str = "token_accounting";
        pub const TOTAL: &str = "total";
        pub const PER_NODE: &str = "per_node";
        pub const PER_FLOW: &str = "per_flow";
        pub const PER_AGENT: &str = "per_agent";
        pub const INPUT_TOKENS: &str = "input_tokens";
        pub const OUTPUT_TOKENS: &str = "output_tokens";
        pub const TOTAL_TOKENS: &str = "total_tokens";
        pub const CALL_COUNT: &str = "call_count";

        pub const SCHEMA_VERSION: &str = "schema_version";
        pub const SECTION_COMPILER: &str = "compiler";
        pub const SECTION_RUNTIME: &str = "runtime";
        pub const SECTION_BACKENDS: &str = "backends";
        pub const RUNTIME_EXECUTION: &str = "execution";
        pub const RUNTIME_SCHEDULER: &str = "scheduler";
        pub const RUNTIME_LLM: &str = "llm";
        pub const RUNTIME_GRAPH_METRICS: &str = "graph_metrics";
        pub const BACKENDS_AGGREGATE: &str = "aggregate";
        pub const BACKENDS_PER_BACKEND: &str = "per_backend";
        pub const BACKENDS_GRAPHS: &str = "graphs";
        pub const SCHEMA_VERSION_VALUE: u32 = 2;

        // Compiler diagnostics section keys
        pub const COMPILER_PASSES: &str = "passes";
        pub const COMPILER_SUMMARY: &str = "summary";
        pub const SUMMARY_TOTAL_PASSES: &str = "total_passes";
        pub const SUMMARY_INITIAL_OPS: &str = "initial_ops";
        pub const SUMMARY_FINAL_OPS: &str = "final_ops";
        pub const SUMMARY_TOTAL_OPS_ELIMINATED: &str = "total_ops_eliminated";
        pub const SUMMARY_TOTAL_TOKENS_SAVED: &str = "total_tokens_saved";
        pub const SUMMARY_FIRED_PASSES: &str = "fired_passes";
        pub const SUMMARY_ACTIVE_PASSES: &str = "active_passes";

        /// Wire keys nested under `runtime.execution`.
        pub mod execution_keys {
            pub const NODES_EXECUTED: &str = "nodes_executed";
            pub const NODES_FAILED: &str = "nodes_failed";
            pub const DURATION_MS: &str = "duration_ms";
            pub const STATUS: &str = "status";
            pub const STATUS_SUCCESS: &str = "success";
            pub const STATUS_PARTIAL_FAILURE: &str = "partial_failure";
        }

        /// Wire keys nested under `runtime.llm`.
        pub mod llm_keys {
            pub const TOTAL_REQUESTS: &str = "total_requests";
            pub const SUCCESSFUL_REQUESTS: &str = "successful_requests";
            pub const FAILED_REQUESTS: &str = "failed_requests";
            pub const TOTAL_INPUT_TOKENS: &str = "total_input_tokens";
            pub const TOTAL_OUTPUT_TOKENS: &str = "total_output_tokens";
            pub const INPUT_TOKENS: &str = "input_tokens";
            pub const OUTPUT_TOKENS: &str = "output_tokens";
            pub const AVG_LATENCY_MS: &str = "avg_latency_ms";
            pub const P50_LATENCY_MS: &str = "p50_latency_ms";
            pub const P99_LATENCY_MS: &str = "p99_latency_ms";
        }

        /// Wire keys nested under `runtime.graph_metrics`.
        pub mod graph_metric_keys {
            pub const GRAPH: &str = "graph";
            pub const NODES: &str = "nodes";
            pub const AGGREGATES: &str = "aggregates";
            pub const BY_AGENT: &str = "by_agent";
            pub const PROCESS_SPAWNS: &str = "process_spawns";
            pub const PROMPT_TURNS: &str = "prompt_turns";
        }

        /// Wire keys nested under `runtime.link_phases` (compile vs runtime split).
        pub mod link_phase_keys {
            pub const LINK_PHASES: &str = "link_phases";
            pub const COMPILE_MS: &str = "compile_ms";
            pub const RUNTIME_MS: &str = "runtime_ms";
        }

        /// Wire keys for serialized graph-status snapshots in backend metrics.
        pub mod graph_status_keys {
            pub const OBJECT: &str = "object";
            pub const BACKEND_KIND: &str = "backend_kind";
            pub const BACKEND_NAME: &str = "backend_name";
            pub const GRAPH_ID: &str = "graph_id";
            pub const REGISTERED: &str = "registered";
            pub const PINNED_BLOCKS: &str = "pinned_blocks";
            pub const PINNED_HANDLES: &str = "pinned_handles";
            pub const CRITICAL_PATH_LENGTH: &str = "critical_path_length";
            pub const NODE_COUNT: &str = "node_count";
            /// Object-tag value emitted by the fork.
            pub const OBJECT_TAG_GRAPH_STATUS: &str =
                crate::constants::llm::apxm::OBJECT_GRAPH_STATUS;
        }

        /// Top-level meta fields attached to the runtime section by the CLI
        /// (e.g. command-line invocation context).
        pub mod runtime_meta_keys {
            pub const INPUT: &str = "input";
            pub const OPTIMIZATION_LEVEL: &str = "optimization_level";
        }

        /// CLI execution-response wire keys (the JSON returned to the caller of
        /// `apxm execute`/`apxm run`). Distinct from `execution_keys` because
        /// the response uses `executed_nodes`/`failed_nodes` while the metrics
        /// report uses `nodes_executed`/`nodes_failed`.
        pub mod cli_response_keys {
            pub const CONTENT: &str = "content";
            pub const EXECUTION_ID: &str = "execution_id";
            pub const SESSION_DIR: &str = "session_dir";
            pub const METRICS_PATH: &str = "metrics_path";
            pub const PROFILE_PATH: &str = "profile_path";
            pub const RESULTS: &str = "results";
            pub const STATS: &str = "stats";
            pub const STATS_EXECUTED_NODES: &str = "executed_nodes";
            pub const STATS_FAILED_NODES: &str = "failed_nodes";
            pub const LLM_USAGE: &str = "llm_usage";
        }
    }

    pub mod node {
        pub const NODE_JSON: &str = "node.json";
        pub const LIVE_JSON: &str = "live.json";
        pub const OUTPUT_JSON: &str = "output.json";
        pub const STATUS_JSON: &str = "status.json";
        pub const METRICS_JSON: &str = "metrics.json";
        pub const TRACE_NDJSON: &str = "trace.ndjson";
        pub const PROMPT_TXT: &str = "prompt.txt";
        pub const RESPONSE_TXT: &str = "response.txt";
        pub const SKILLS_DIR: &str = "skills";
    }
}

pub mod ui {
    /// Status icons for terminal output.
    pub mod icons {
        /// Operation started / in progress.
        pub const STARTED: &str = "\u{25b6}";
        /// Operation completed successfully.
        pub const SUCCESS: &str = "\u{2713}";
        /// Operation failed.
        pub const FAILED: &str = "\u{2717}";
        /// Information hint.
        pub const INFO: &str = "\u{2139}";
        /// Warning sign.
        pub const WARNING: &str = "!";
        /// Caution / alert.
        pub const CAUTION: &str = "\u{26a0}";
        /// Lightning bolt / performance.
        pub const LIGHTNING: &str = "\u{26a1}";
        /// Rocket / speedup.
        pub const ROCKET: &str = "\u{1f680}";
        /// Bullet point.
        pub const BULLET: &str = "\u{2022}";
        /// Horizontal rule (thin).
        pub const HRULE: &str = "\u{2500}";
        /// Horizontal rule (double).
        pub const HRULE_DOUBLE: &str = "\u{2550}";
        /// Em dash.
        pub const EM_DASH: &str = "\u{2014}";
        /// Left arrow.
        pub const ARROW_LEFT: &str = "\u{2190}";
        /// Right arrow.
        pub const ARROW_RIGHT: &str = "\u{2192}";
    }

    pub mod labels {
        pub const STARTED: &str = "started";
        pub const OK: &str = "OK";
        pub const WARN: &str = "WARN";
        pub const MISSING: &str = "MISSING";
    }
}

pub mod mlir {
    /// MLIR type strings used in AIS dialect lowering.
    pub mod types {
        /// Token type (!ais.token).
        pub const TOKEN: &str = "!ais.token";
        /// Handle type (!ais.handle).
        pub const HANDLE: &str = "!ais.handle";
        /// Goal type (!ais.goal<0>).
        pub const GOAL: &str = "!ais.goal<0>";
    }
}

pub mod defaults {
    pub const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:18800";
    pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
    pub const DEFAULT_LEASE_MS: u64 = 60_000;
    pub const DEFAULT_MAX_WAIT_MS: u64 = 5_000;
    pub const DEFAULT_MEMORY_LIMIT: u64 = 10;
    pub const DEFAULT_MAX_RETRIES: u32 = 3;
    pub const DEFAULT_MAX_CONTEXT_TOKENS: usize = 8192;
    pub const DEFAULT_MAX_NEGOTIATE_ROUNDS: usize = 3;
    pub const DEFAULT_DESCRIPTION: &str = "Dynamically registered capability";

    // Resource governance limits
    /// Maximum number of concurrent agent processes in the process table.
    pub const DEFAULT_MAX_PROCESSES: usize = 32;
    /// Maximum spawn depth for recursive SPAWN_AGENT chains.
    pub const DEFAULT_MAX_SPAWN_DEPTH: usize = 4;
    /// Maximum number of concurrent ACP sessions in the session pool.
    pub const DEFAULT_MAX_SESSIONS: usize = 16;

    // LLM backend defaults
    pub const DEFAULT_ANTHROPIC_MAX_TOKENS: usize = 4096;
    pub const DEFAULT_GOOGLE_MAX_OUTPUT_TOKENS: usize = 2048;
    pub const DEFAULT_GOOGLE_TOP_P: f64 = 0.95;
}
