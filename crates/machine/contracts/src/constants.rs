//! Shared string constants for cross-crate protocol fields.

pub mod diagnostics {
    /// Compile diagnostics mode for canonical graph input.
    pub const MODE_GRAPH: &str = "graph";
    /// Compile diagnostics mode for `.air` (raw MLIR text) input.
    pub const MODE_AIR: &str = "air";
}

pub mod env {
    pub const APXM_BACKEND: &str = "APXM_BACKEND";
    /// Canonical integration catalog root (`<workspace>/integrations` or bundled source).
    /// Used by Studio, Server, OS, and Auth to discover provider integration folders.
    pub const APXM_INTEGRATIONS_ROOT: &str = "APXM_INTEGRATIONS_ROOT";
    /// Workspace root containing `integrations/` and `libs/` subdirectories.
    pub const APXM_WORKSPACE_ROOT: &str = "APXM_WORKSPACE_ROOT";
    /// Path to the APXM project/run configuration file.
    pub const APXM_CONFIG: &str = "APXM_CONFIG";
    /// Explicitly trusts author-supplied Python and TypeScript artifact handlers.
    pub const APXM_TRUST_SCRIPT_ARTIFACTS: &str = "APXM_TRUST_SCRIPT_ARTIFACTS";
    /// Requires Python and TypeScript artifact handlers to run in an OS sandbox.
    pub const APXM_SANDBOX_SCRIPTS: &str = "APXM_SANDBOX_SCRIPTS";
    /// Disables dispatch IR hints sent to vLLM-compatible backends.
    pub const APXM_DISABLE_HINTS: &str = "APXM_DISABLE_HINTS";
    /// Makes Python graph files emit AIR to stdout for the Rust compiler driver.
    pub const APXM_EMIT_AIR: &str = "APXM_EMIT_AIR";
    /// Path to the `apxm` binary a child process uses for explicit compiler
    /// bridge commands. The compile pipeline sets this to its own executable so
    /// callers route to the same Rust printer.
    pub const APXM_BIN: &str = "APXM_BIN";
    /// Root of the installed TypeScript frontend package used for source
    /// execution and handler workers.
    pub const APXM_TYPESCRIPT_FRONTEND_PACKAGE: &str = "APXM_TYPESCRIPT_FRONTEND_PACKAGE";
    /// Output path supplied to Python frontend processes for their handler manifest.
    pub const APXM_PYTHON_TOOLS_OUT: &str = "APXM_PYTHON_TOOLS_OUT";
    /// Enables the in-process mock backend for tests and offline benchmarks.
    pub const APXM_MOCK_BACKEND: &str = "APXM_MOCK_BACKEND";
    /// Configures mock backend latency in milliseconds.
    pub const APXM_MOCK_LATENCY_MS: &str = "APXM_MOCK_LATENCY_MS";
    /// Path to a JSON script of ordered `{"contains": ..., "response": ...}`
    /// pattern rules (plus an optional top-level `"default"` string) that
    /// `configure_llm_registry` loads into the mock backend via
    /// `MockLLMBackend::when_prompt_contains`, so a multi-message scripted
    /// transcript (context injection -> tool use -> compaction) gets
    /// distinct deterministic answers per prompt instead of one static
    /// response. Only read when
    /// `APXM_MOCK_BACKEND` is also set; malformed/missing files are a hard
    /// error, never a silent fall-back to the single default response.
    pub const APXM_MOCK_SCRIPT_PATH: &str = "APXM_MOCK_SCRIPT_PATH";
    /// Disables LLM response caching for this execution.
    pub const APXM_NO_CACHE: &str = "APXM_NO_CACHE";
    /// Public base URL the APXM server advertises (e.g. http://host:18800).
    pub const APXM_PUBLIC_URL: &str = "APXM_PUBLIC_URL";
    /// Stream channel capacity for `/v1/generate-stream`.
    pub const APXM_GENERATE_STREAM_CHANNEL_CAPACITY: &str = "APXM_GENERATE_STREAM_CHANNEL_CAPACITY";
    /// Inactivity timeout, in seconds, for `/v1/generate-stream`.
    pub const APXM_GENERATE_STREAM_TIMEOUT_SECS: &str = "APXM_GENERATE_STREAM_TIMEOUT_SECS";
    /// SSE keep-alive interval, in seconds, for `/v1/generate-stream`.
    pub const APXM_GENERATE_STREAM_KEEP_ALIVE_SECS: &str = "APXM_GENERATE_STREAM_KEEP_ALIVE_SECS";
    /// Stream channel capacity for runtime and skill execution SSE endpoints.
    pub const APXM_EXECUTION_STREAM_CHANNEL_CAPACITY: &str =
        "APXM_EXECUTION_STREAM_CHANNEL_CAPACITY";
    /// SSE keep-alive interval, in seconds, for runtime and skill execution streams.
    pub const APXM_EXECUTION_STREAM_KEEP_ALIVE_SECS: &str = "APXM_EXECUTION_STREAM_KEEP_ALIVE_SECS";
    /// Maximum hot entries retained by the server execution index.
    pub const APXM_EXECUTION_INDEX_MAX_ENTRIES: &str = "APXM_EXECUTION_INDEX_MAX_ENTRIES";
    /// Broadcast buffer size for per-run event subscriptions.
    pub const APXM_RUN_EVENT_STREAM_BUFFER: &str = "APXM_RUN_EVENT_STREAM_BUFFER";
    /// Number of per-run events retained in memory for replay.
    pub const APXM_RUN_EVENT_RETAINED_EVENTS: &str = "APXM_RUN_EVENT_RETAINED_EVENTS";
    /// SSE keep-alive interval, in seconds, for run event subscriptions.
    pub const APXM_RUN_EVENT_KEEP_ALIVE_SECS: &str = "APXM_RUN_EVENT_KEEP_ALIVE_SECS";
    /// Default page size for `/v1/runs`.
    pub const APXM_RUN_LIST_DEFAULT_LIMIT: &str = "APXM_RUN_LIST_DEFAULT_LIMIT";
    /// Maximum page size for `/v1/runs`.
    pub const APXM_RUN_LIST_MAX_LIMIT: &str = "APXM_RUN_LIST_MAX_LIMIT";
    /// Default page size for `/v1/runs/{id}/events`.
    pub const APXM_RUN_EVENT_DEFAULT_LIMIT: &str = "APXM_RUN_EVENT_DEFAULT_LIMIT";
    /// Maximum page size for `/v1/runs/{id}/events`.
    pub const APXM_RUN_EVENT_MAX_LIMIT: &str = "APXM_RUN_EVENT_MAX_LIMIT";
    /// Server rollout writer event queue capacity.
    pub const APXM_ROLLOUT_EVENT_BUFFER: &str = "APXM_ROLLOUT_EVENT_BUFFER";
    /// Server rollout payload spill threshold, in bytes.
    pub const APXM_ROLLOUT_SPILL_THRESHOLD_BYTES: &str = "APXM_ROLLOUT_SPILL_THRESHOLD_BYTES";
    /// Retention: rollout max age (days) before compaction archives its content.
    /// See state-layout.v1 `sessions/rollouts` (class=durable). .
    pub const APXM_RETENTION_ROLLOUT_MAX_AGE_DAYS: &str = "APXM_RETENTION_ROLLOUT_MAX_AGE_DAYS";
    /// Retention: grace period (hours) an unreferenced blob must sit idle
    /// before GC deletes it, to avoid racing an in-flight spill write.
    pub const APXM_RETENTION_BLOB_GC_GRACE_HOURS: &str = "APXM_RETENTION_BLOB_GC_GRACE_HOURS";
    /// Bind address for the APXM server (e.g. 127.0.0.1:18800).
    pub const APXM_SERVER_ADDR: &str = "APXM_SERVER_ADDR";
    /// Opt-in: require a bearer token on mutating routes (fail-closed when set).
    pub const APXM_SERVER_REQUIRE_AUTH: &str = "APXM_SERVER_REQUIRE_AUTH";
    /// Explicit server bearer token (overrides the apxm-auth bearer file).
    pub const APXM_SERVER_BEARER: &str = "APXM_SERVER_BEARER";
    /// Maximum concurrent expensive inference work admitted by the server.
    pub const APXM_SERVER_MAX_INFERENCE: &str = "APXM_SERVER_MAX_INFERENCE";
    /// Maximum inference limiter wait before returning 429, in milliseconds.
    pub const APXM_SERVER_INFERENCE_WAIT_MS: &str = "APXM_SERVER_INFERENCE_WAIT_MS";
    /// Per-principal HTTP requests-per-second cap (`None` disables when unset/0).
    pub const APXM_SERVER_RATE_LIMIT_RPS: &str = "APXM_SERVER_RATE_LIMIT_RPS";
    /// HTTP request body size cap in bytes.
    pub const APXM_SERVER_MAX_BODY_BYTES: &str = "APXM_SERVER_MAX_BODY_BYTES";
    /// Graceful shutdown rollout flush timeout in seconds.
    pub const APXM_SERVER_DRAIN_TIMEOUT_SECS: &str = "APXM_SERVER_DRAIN_TIMEOUT_SECS";

    /// Max seconds the runtime waits for host consent on `requires_approval` capabilities.
    pub const APXM_PERMISSION_TIMEOUT_SECS: &str = "APXM_PERMISSION_TIMEOUT_SECS";
    /// Maximum scheduler concurrency for server-owned runtime work.
    pub const APXM_RUNTIME_MAX_CONCURRENCY: &str = "APXM_RUNTIME_MAX_CONCURRENCY";
    /// Maximum scheduler in-flight work for server-owned runtime work.
    pub const APXM_RUNTIME_MAX_INFLIGHT: &str = "APXM_RUNTIME_MAX_INFLIGHT";
    /// Maximum scheduler in-flight LLM work for server-owned runtime work.
    pub const APXM_RUNTIME_LLM_INFLIGHT: &str = "APXM_RUNTIME_LLM_INFLIGHT";
    /// Maximum parallel tool calls admitted within one LLM tool-call batch.
    pub const APXM_RUNTIME_MAX_PARALLEL_TOOL_CALLS: &str = "APXM_RUNTIME_MAX_PARALLEL_TOOL_CALLS";
    /// Maximum tokens requested for MCP workflow emission.
    pub const APXM_MCP_WORKFLOW_MAX_TOKENS: &str = "APXM_MCP_WORKFLOW_MAX_TOKENS";
    /// Temperature used for MCP workflow emission.
    pub const APXM_MCP_WORKFLOW_TEMPERATURE: &str = "APXM_MCP_WORKFLOW_TEMPERATURE";
    /// Maximum wall-clock time, in milliseconds, for one MCP workflow emission request.
    pub const APXM_MCP_WORKFLOW_EMIT_TIMEOUT_MS: &str = "APXM_MCP_WORKFLOW_EMIT_TIMEOUT_MS";
    /// Repair attempts for invalid MCP workflow emissions.
    pub const APXM_MCP_WORKFLOW_REPAIR_ATTEMPTS: &str = "APXM_MCP_WORKFLOW_REPAIR_ATTEMPTS";
    /// Capability entries included in MCP workflow prompt guidance.
    pub const APXM_MCP_WORKFLOW_CAPABILITY_GUIDANCE_LIMIT: &str =
        "APXM_MCP_WORKFLOW_CAPABILITY_GUIDANCE_LIMIT";
    /// Default top-k for MCP recall style tools.
    pub const APXM_MCP_DEFAULT_TOP_K: &str = "APXM_MCP_DEFAULT_TOP_K";
    /// Maximum top-k accepted by MCP recall style tools.
    pub const APXM_MCP_MAX_TOP_K: &str = "APXM_MCP_MAX_TOP_K";
    /// Default result limit for MCP evidence lookup.
    pub const APXM_MCP_DEFAULT_EVIDENCE_LIMIT: &str = "APXM_MCP_DEFAULT_EVIDENCE_LIMIT";
    /// Maximum result limit accepted by MCP evidence lookup.
    pub const APXM_MCP_MAX_EVIDENCE_LIMIT: &str = "APXM_MCP_MAX_EVIDENCE_LIMIT";
    /// Default trace event count returned by MCP trace fetch.
    pub const APXM_MCP_DEFAULT_TRACE_EVENT_LIMIT: &str = "APXM_MCP_DEFAULT_TRACE_EVENT_LIMIT";
    /// Maximum files scanned by MCP trace fetch fallback lookup.
    pub const APXM_MCP_TRACE_MAX_SCAN_FILES: &str = "APXM_MCP_TRACE_MAX_SCAN_FILES";
    /// Maximum files scanned by MCP evidence lookup.
    pub const APXM_MCP_EVIDENCE_MAX_SCAN_FILES: &str = "APXM_MCP_EVIDENCE_MAX_SCAN_FILES";
    /// Maximum evidence file bytes read by MCP evidence lookup.
    pub const APXM_MCP_EVIDENCE_MAX_FILE_BYTES: &str = "APXM_MCP_EVIDENCE_MAX_FILE_BYTES";
    /// Tokio worker threads for the APXM server process.
    pub const APXM_TOKIO_WORKERS: &str = "APXM_TOKIO_WORKERS";
    pub const LLVM_DIR: &str = "LLVM_DIR";
    pub const MLIR_DIR: &str = "MLIR_DIR";
    pub const OTEL_EXPORTER_OTLP_ENDPOINT: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";
    pub const PYTHONPATH: &str = "PYTHONPATH";
    pub const RUST_LOG: &str = "RUST_LOG";

    pub mod flag_values {
        pub const ENABLED: &str = "1";
    }
}

/// JSON keys used inside OpenAI-compatible `extra_body` payloads carrying APXM hints.
pub mod extra_body {
    pub const CACHE_SALT_KEY: &str = "cache_salt";
    pub const REUSE_GROUP_KEY: &str = "reuse_group";
}

pub mod inner_plan {
    /// Payload key for AIR text in structured inner-plan outputs.
    pub const AIR_PAYLOAD: &str = "air";
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
        pub const GRAPH_AWARE: &str = "graph_aware";
        pub const GENERIC: &str = "generic";
    }

    pub mod attrs {
        include!(concat!(env!("OUT_DIR"), "/apxm_graph_attrs.rs"));
    }
}

pub mod runtime {
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

    /// Context-window compaction defaults.
    /// Sibling of [`context_stack`]'s prompt-budget family: that module
    /// bounds ONE assembled prompt; this one bounds the ACCUMULATED
    /// accumulated context window before older frames can fold into a rolling
    /// summary.
    pub mod conversation_compaction {
        /// Messages kept verbatim once compaction folds older messages into
        /// the rolling summary.
        pub const DEFAULT_KEEP_RECENT_MESSAGES: usize = 4;
        /// Accumulated-window token budget above which compaction triggers.
        pub const DEFAULT_COMPACT_AT_TOKENS: usize = 20_000;
        /// Utilization percentage (of `compact_at_tokens`) at or above which
        /// `context_window_warning` fires, ahead of the hard compaction
        /// trigger at 100%.
        pub const DEFAULT_WARNING_UTILIZATION_PCT: f64 = 80.0;
    }

    /// Deterministic tool-result trimming: the
    /// budget an oversized `INV_CAP` result is trimmed against before it can
    /// inflate the conversation's token accounting, via the SAME
    /// `truncate_to_budget` primitive the subagent prompt-budget mechanism
    /// already ships (`context_stack::frame`) — never a chars/4 re-derivation.
    pub mod tool_result_trim {
        /// Default max tokens a single tool result is trimmed to. Generous
        /// relative to `conversation_compaction::DEFAULT_COMPACT_AT_TOKENS`
        /// (one tool call should not, by itself, exhaust a whole
        /// conversation's budget) while still bounding pathological
        /// oversized results (e.g. a full raw web page body).
        pub const DEFAULT_MAX_TOKENS: usize = 8_000;
    }

    pub mod belief_keys {
        pub const STAGED_PREFIX: &str = "_stage:";
        pub const DELEGATE_PREFIX: &str = "_delegate:";
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
        pub const FLOW_CALL_OUTPUT_PREFIX: &str = "_flow_call_outputs:";
        pub const FLOW_ARG_PREFIX: &str = "_flow_arg_";
        pub const BRANCH_PREFIX: &str = "_branch:";
        pub const SWITCH_PREFIX: &str = "_switch:";
        pub const INV_PREFIX: &str = "_inv:";
        pub const LLM_RESULT_PREFIX: &str = "_llm_result:";
        pub const REFLECT_PREFIX: &str = "_reflect:";
        pub const VERIFY_PREFIX: &str = "_verify:";
        pub const PRINT_PREFIX: &str = "_print:";
        pub const ERR_PREFIX: &str = "_err:";
        pub const PAUSE_PREFIX: &str = "_pause:";
        pub const RESUME_PREFIX: &str = "_resume:";
        pub const EXC_PREFIX: &str = "exc:";
        pub const CHECKPOINT_SNAPSHOT_PREFIX: &str = "_checkpoint_snapshot:";
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

    /// Runtime-owned structured response contract for spawned-agent prompts.
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
        pub const MODEL: &str = "model";
        pub const INPUT_TOKENS: &str = "input_tokens";
        pub const OUTPUT_TOKENS: &str = "output_tokens";
    }

    /// Runtime-owned metadata stamped on synthetic LLM requests.
    pub mod llm_request_metadata {
        pub const WARMUP: &str = "warmup";
        pub const WARMUP_NODE_ID: &str = "warmup_node_id";
    }
}

pub mod memory {
    pub const STM: &str = "stm";
    pub const LTM: &str = "ltm";
    pub const EPISODIC: &str = "episodic";
}

pub mod protocols {
    /// Managed inbound MCP version implemented by Auth's public resource edge
    /// and Server's private transport. The outbound MCP client bridge is a
    /// separate ADR-0018 trust boundary with its own independently pinned
    /// protocol version.
    pub const MCP_VERSION: &str = "2026-07-28";
    /// A2A (Agent-to-Agent) protocol version.
    pub const A2A_VERSION: &str = "0.3";
}

pub mod mcp {
    /// HTTP route for APXM's JSON-RPC MCP endpoint.
    pub const ROUTE: &str = "/v1/mcp";

    pub mod methods {
        pub const INITIALIZE: &str = "initialize";
        pub const TOOLS_LIST: &str = "tools/list";
        pub const TOOLS_CALL: &str = "tools/call";
        pub const RESOURCES_LIST: &str = "resources/list";
        pub const RESOURCES_READ: &str = "resources/read";
        pub const PING: &str = "ping";
    }

    pub mod fields {
        pub const ARGUMENTS: &str = "arguments";
        pub const CONTENT: &str = "content";
        pub const ERROR: &str = "error";
        pub const IS_ERROR: &str = "isError";
        pub const NAME: &str = "name";
        pub const RESULT: &str = "result";
        pub const TEXT: &str = "text";
    }

    pub mod tools {
        pub const APXM_WORKFLOW_START: &str = "workflow_start";
        pub const APXM_WORKFLOW_STATUS: &str = "workflow_status";
        pub const APXM_WORKFLOW_EVENTS: &str = "workflow_events";
        pub const APXM_WORKFLOW_CANCEL: &str = "workflow_cancel";
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

pub mod capabilities {
    //! Re-exported from `apxm-ais` (single source of truth). Do not redefine
    //! these here; add new capability ids in `apxm_ais::capabilities`.
    pub use apxm_ais::capabilities::groups;
    pub use apxm_ais::capabilities::{
        AGENT_MANAGEMENT_BUILTINS, BASH, BUILTINS, CAPABILITY_DISCOVERY, HTTP_GET, HTTP_POST,
        LIST_LOCAL_SKILLS, MANAGE_TASK, READ, READ_LOCAL_SKILL, SCHEDULE, SEARCH_SKILLS,
        SEARCH_WEB, STANDARD_BUILTINS, WRITE,
    };
}

pub mod agent_tools {
    /// SQLite database file shared by durable agent-management capabilities.
    pub const STORE_FILENAME: &str = "agent_tools.sqlite";
    /// Payload field for `schedule` fires to select a server task queue.
    pub const PAYLOAD_QUEUE: &str = "queue";
    /// Default queue used when a fired scheduled prompt does not specify one.
    pub const SCHEDULED_PROMPT_QUEUE: &str = "scheduled_prompts";
}

pub mod orchestration {
    pub mod admission {
        pub const SPAWN_AGENT: &str = "SPAWN_AGENT";
    }

    pub mod execution_status {
        pub const RUNNING: &str = "running";
        pub const SUCCEEDED: &str = "succeeded";
        pub const FAILED: &str = "failed";
    }

    pub mod workflow_status {
        pub const SUCCESS: &str = "success";
        pub const FAILED: &str = "failed";
        pub const SKIPPED: &str = "skipped";
        pub const PARTIAL_FAILURE: &str = "partial_failure";
    }
}

pub mod llm {
    pub mod apxm {
        pub const OBJECT_GRAPH_REGISTRATION: &str = "apxm.graph.registration";
        pub const OBJECT_GRAPH_STATUS: &str = "apxm.graph.status";
        pub const OBJECT_GRAPH_RELEASE: &str = "apxm.graph.release";
        pub const OBJECT: &str = "object";
        pub const REQUEST_PRIORITY: &str = "priority";
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

        pub const PIN_MODE_PREFIX: &str = "prefix";
        pub const PIN_MODE_NONE: &str = "none";

        /// Wire names for the dispatch-IR hint fields a backend can
        /// honor or drop. These are the single source of truth for
        /// the runtime to collect `fields_sent` (apxm-runtime
        /// `dispatch::v1::mod`) and by `BackendGraphCapabilities` to
        /// classify each into `dispatch_fields_honored` /
        /// `unsupported_dispatch_fields`. Adding a new dispatch hint
        /// means adding a constant here AND a `supports_*` flag on
        /// `BackendGraphCapabilities` — keep the two in lockstep.
        pub mod dispatch_fields {
            pub const GRAPH_REGISTRATION: &str = "graph_registration";
            pub const REQUEST_HINTS: &str = "request_hints";
            pub const PRIORITY: &str = "priority";
            pub const PREFIX_COHORTS: &str = "prefix_cohorts";
            pub const PIN_RELEASE: &str = "pin_release";
            pub const STRUCTURED_OUTPUTS: &str = "structured_outputs";
            pub const BACKEND_QUEUE_STATE: &str = "backend_queue_state";
            pub const BACKEND_CACHE_STATE: &str = "backend_cache_state";
            pub const CANCEL_GROUPS: &str = "cancel_groups";
            pub const DISPATCH_IR_V1_INTERNAL: &str = "dispatch_ir_v1_internal";
            pub const ADMIN_RESET_PREFIX_CACHE: &str = "admin_reset_prefix_cache";
        }

        /// Wire names for the metrics + telemetry surface APXM declares
        /// in `BackendCapabilityRequirements.optional` /
        /// `TelemetryContract.requested_metrics`. Adding a metric the
        /// runtime asks the backend to expose means adding a constant
        /// here so producer + consumer share a single source of truth.
        pub mod telemetry_metrics {
            pub const SCHEDULER_POLICY: &str = "scheduler_policy";
            pub const GRAPH_STATUS: &str = "graph_status";
            pub const PINNED_BLOCKS_PEAK: &str =
                crate::types::metrics::GraphStatusKey::PinnedBlocksPeak.as_str();
        }

        /// HTTP response-header contract for per-request runtime
        /// evidence from the vLLM fork. The fork-side emitter is
        /// covered by the fork-side header emitter; the APXM-side ingestion path
        /// (apxm-backends `vllm::backend` response handler) reads this
        /// header and populates `fields_honored` in the per-node
        /// dispatch record. Distinct from
        /// `BackendGraphCapabilities::dispatch_fields_capability_supported`
        /// which is a static capability table, not runtime evidence.
        pub const APXM_FIELDS_HONORED_HEADER: &str = "x-apxm-fields-honored";

        /// Per-request honor record key surfaced in
        /// `dispatch_ir_metrics.fields_honored`. Per-request union of
        /// all `x-apxm-fields-honored` header values observed during
        /// the graph execution.
        pub const FIELDS_HONORED_RECORD_KEY: &str = "fields_honored";
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
    /// Python frontend source that emits canonical AIR.
    pub const PYTHON: &str = "py";
    /// TypeScript frontend source that emits canonical AIR.
    pub const TYPESCRIPT: &str = "ts";
    /// Compiled artifact extension.
    pub const ARTIFACT: &str = "apxmobj";
    /// Structured JSON data extension for metrics, sessions, manifests,
    /// diagnostics, caches, and API envelopes. JSON is not a graph source.
    pub const JSON_DATA: &str = "json";
}

pub mod cache {
    /// SQLite cache database filename.
    pub const DB_FILE: &str = "cache.db";
    /// Compiler cache subdirectory.
    pub const COMPILER_DIR: &str = "compiler";
    /// DSPy optimization cache subdirectory.
    pub const DSPY_DIR: &str = "dspy";
    /// DSPy training data cache subdirectory.
    pub const DSPY_TRAINING_DIR: &str = "training";
}

pub mod dspy {
    /// APXM config table containing compiler-owned configuration.
    pub const CONFIG_TABLE_COMPILER: &str = "compiler";
    /// APXM config subsection containing DSPy optimizer configuration.
    pub const CONFIG_TABLE_DSPY: &str = "dspy";
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
    /// MLIR module attribute: compiler-owned DSPy result cache directory.
    pub const ATTR_CACHE_DIR: &str = "ais.dspy_cache_dir";
    /// MLIR module attribute: optimizer name.
    pub const ATTR_OPTIMIZER: &str = "ais.dspy_optimizer";
    /// MLIR module attribute: auto-tuning level.
    pub const ATTR_AUTO: &str = "ais.dspy_auto";
    /// MLIR module attribute: metric function.
    pub const ATTR_METRIC: &str = "ais.dspy_metric";
    /// MLIR module attribute: cache bypass flag.
    pub const ATTR_NO_CACHE: &str = "ais.dspy_no_cache";
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

    /// JSON keys serialized into `results.json`.
    pub mod results_keys {
        pub const NODE_OUTPUTS: &str = "node_outputs";
        pub const TOKEN_VALUES: &str = "token_values";
        pub const EXIT_VALUES: &str = "exit_values";
        pub const FINAL_NODE_ID: &str = "final_node_id";
        pub const FINAL_OUTPUT: &str = "final_output";
    }

    /// JSON keys serialized into `metrics.json` by the runtime's
    /// `TokenAccountingSnapshot::to_json`. Python-side `MetricsKeys` uses the
    /// same contract; tier-3 budget enforcement relies on these.
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
        pub const RUNTIME_OBSERVED_GRAPH: &str = "observed_graph";
        pub const RUNTIME_NODE_STATUSES: &str = "node_statuses";
        pub const RUNTIME_DISPATCH_IR_V1: &str = "dispatch_ir_v1";
        pub const BACKENDS_AGGREGATE: &str = "aggregate";
        pub const BACKENDS_PER_BACKEND: &str = "per_backend";
        pub const BACKENDS_GRAPHS: &str = "graphs";
        pub const BACKENDS_GRAPH_CAPABILITIES: &str = "graph_capabilities";
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
            pub const PROMPTS: &str = "prompts";
        }

        /// Wire keys nested under `runtime.link_phases` (compile vs runtime split).
        pub mod link_phase_keys {
            pub const LINK_PHASES: &str = "link_phases";
            pub const COMPILE_MS: &str = "compile_ms";
            pub const RUNTIME_MS: &str = "runtime_ms";
        }

        /// Wire keys for serialized graph-status snapshots in backend metrics.
        ///
        /// Each constant is derived from `apxm_core::types::GraphStatusKey`,
        /// which is the typed source of truth. Prefer the enum at new call
        /// sites.
        pub mod graph_status_keys {
            use crate::types::metrics::GraphStatusKey;

            pub const OBJECT: &str = GraphStatusKey::Object.as_str();
            pub const BACKEND_KIND: &str = GraphStatusKey::BackendKind.as_str();
            pub const BACKEND_NAME: &str = GraphStatusKey::BackendName.as_str();
            pub const GRAPH_ID: &str = GraphStatusKey::GraphId.as_str();
            pub const REGISTERED: &str = GraphStatusKey::Registered.as_str();
            pub const PINNED_BLOCKS: &str = GraphStatusKey::PinnedBlocks.as_str();
            pub const PINNED_HANDLES: &str = GraphStatusKey::PinnedHandles.as_str();
            pub const PINNED_BLOCKS_PEAK: &str = GraphStatusKey::PinnedBlocksPeak.as_str();
            pub const PINNED_HANDLES_PEAK: &str = GraphStatusKey::PinnedHandlesPeak.as_str();
            pub const CRITICAL_PATH_LENGTH: &str = GraphStatusKey::CriticalPathLength.as_str();
            pub const NODE_COUNT: &str = GraphStatusKey::NodeCount.as_str();
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
        pub const SCOPE_ID: &str = "scope_id";
        pub const PARENT_EXECUTION_ID: &str = "parent_execution_id";
        pub const PARENT_SESSION_DIR: &str = "parent_session_dir";
        pub const PARENT_SCOPE_ID: &str = "parent_scope_id";
        pub const SPAWN_NODE_ID: &str = "spawn_node_id";
        pub const PROMPT_TXT: &str = "prompt.txt";
        pub const RESPONSE_TXT: &str = "response.txt";
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
    /// MLIR textual syntax markers for frontends and driver routing.
    pub mod syntax {
        /// Top-level MLIR module keyword.
        pub const MODULE_KEYWORD: &str = "module";
        /// Function operation prefix accepted for standalone function AIR.
        pub const FUNC_FUNC_PREFIX: &str = "func.func";
        /// MLIR line-comment prefix.
        pub const LINE_COMMENT_PREFIX: &str = ";";
    }

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
    // URLs / output budget re-exported from `apxm-ais` (single source of truth).
    pub use apxm_ais::defaults::{DEFAULT_OS_URL, DEFAULT_OUTPUT_TOKEN_BUDGET, DEFAULT_SERVER_URL};
    pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;
    pub const DEFAULT_MEMORY_LIMIT: u64 = 10;
    pub const DEFAULT_MAX_RETRIES: u32 = 3;
    pub const DEFAULT_MAX_CONTEXT_TOKENS: usize = 8192;
    pub const DEFAULT_DESCRIPTION: &str = "Dynamically registered capability";

    // Resource governance limits
    /// Maximum number of concurrent agent processes in the process table.
    pub const DEFAULT_MAX_PROCESSES: usize = 32;
    /// Maximum spawn depth for recursive SPAWN_AGENT chains.
    pub const DEFAULT_MAX_SPAWN_DEPTH: usize = 4;
    /// Maximum number of concurrent ACP sessions in the session pool.
    pub const DEFAULT_MAX_SESSIONS: usize = 16;
}
