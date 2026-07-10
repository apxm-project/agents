//! Canonical graph attribute constants for AIS operations.
//!
//! Every attribute name used in OperationSpec fields, MLIR TableGen,
//! runtime handlers, and the Python frontend MUST be defined here.
//! No other crate may introduce raw string literals for attribute names.

// -- Agent / identity --
pub const AGENT_NAME: &str = "agent_name";
pub const FLOW_NAME: &str = "flow_name";
pub const PROFILE: &str = "profile";
pub const AGENT_ROUTE: &str = "agent_route";
pub const REQUIRED_CAPABILITIES: &str = "required_capabilities";
pub const PREFERRED_PROFILES: &str = "preferred_profiles";
pub const NODE_NAME: &str = "node_name";
pub const MODE: &str = "mode";
pub const CWD: &str = "cwd";

// -- LLM / model --
pub const MODEL: &str = "model";
/// Semantic model tier (e.g. "reasoning-tier", "fast-draft") resolved via
/// `ProfileRouter`/`ProfileRegistry` at dispatch time, before
/// `ModelRouter::select` runs. Distinct from [`PROFILE`] (agent/context-stack
/// profile). Node-level `MODEL`/`BACKEND` still win when set (explicit wins);
/// an unset node value falls back to the owning package's
/// `agent.toml [runtime].default_model_profile`, when threaded in by the host.
pub const MODEL_PROFILE: &str = "model_profile";
pub const PROVIDER: &str = "provider";
pub const API_KEY: &str = "api_key";
pub const BASE_URL: &str = "base_url";
pub const TEMPERATURE: &str = "temperature";
pub const SYSTEM_PROMPT: &str = "system_prompt";
pub const TOOLS_CONFIG: &str = "tools_config";
pub const TOKEN_BUDGET: &str = "token_budget";
pub const OUTPUT_SCHEMA: &str = "output_schema";
pub const MAX_SCHEMA_RETRIES: &str = "max_schema_retries";
pub const MAX_ITERATIONS: &str = "max_iterations";
pub const HANDOFF_TARGETS: &str = "handoff_targets";
pub const INNER_PLAN_SUPPORTED: &str = "inner_plan_supported";
pub const ENABLE_INNER_PLAN: &str = "enable_inner_plan";
pub const BIND_INNER_PLAN_OUTPUTS: &str = "bind_inner_plan_outputs";
pub const BACKEND: &str = "backend";
pub const MAX_TOOL_ITERATIONS: &str = "max_tool_iterations";
pub const BUDGET: &str = "budget";
pub const STRUCTURED: &str = "structured";
/// Thinking/reasoning effort for LLM ops: `off` | `low` | `medium` | `high`.
/// Lowered by the runtime into an extended-thinking token budget; an unset or
/// `off` value disables extended thinking.
pub const EFFORT: &str = "effort";

// -- Template / prompt --
pub const TEMPLATE_STR: &str = "template_str";
pub const PROMPT: &str = "prompt";
pub const TEMPLATE: &str = "template";
/// Parallel string array enumerating the human-readable name of each
/// incoming Data edge. Templates reference inputs by these names via
/// `{name}` placeholders; the runtime substitutes by index lookup.
pub const INPUT_NAMES: &str = "input_names";

// -- Memory --
pub const QUERY: &str = "query";
pub const MEMORY_TIER: &str = "memory_tier";
pub const KEY: &str = "key";
pub const VALUE: &str = "value";
pub const LIMIT: &str = "limit";

// -- Capability / tools --
pub const CAPABILITY: &str = "capability";
pub const PARAMS_JSON: &str = "params_json";
pub const TOOLS_ENABLED: &str = "tools_enabled";
pub const TOOLS: &str = "tools";
pub const CAPABILITY_GROUPS: &str = "capability_groups";
/// Self-declared per-tool call-count budget, a JSON object string
/// `{"capability": max_calls}`. A DECLARATION the trusted host reads and folds
/// into the enforced budget — never self-enforced by the program.
pub const TOOL_CALL_BUDGETS: &str = "tool_call_budgets";
/// Opt-in: a converse/autonomous coordinator that also gets the synthetic
/// `delegate` tool, letting it spawn focused specialist sub-agents at runtime.
pub const ENABLE_DELEGATE: &str = "enable_delegate";
pub const CODE: &str = "code";
pub const INTERPRETER: &str = "interpreter";
pub const CAPABILITY_NAME: &str = "capability_name";
pub const DESCRIPTION: &str = "description";
pub const PARAMETERS_SCHEMA: &str = "parameters_schema";
/// When set, this capability is backed by a Python handler instead of a Rust
/// builtin. The runtime dispatches to the python tool worker using this stable
/// id. Must match `^sha256:[0-9a-f]{64}$`.
pub const PYTHON_HANDLER_ID: &str = "python_handler_id";

// -- Lifecycle hooks (REGISTER_HOOK) --
/// Lifecycle event a hook binds to (session_start, pre/post_turn,
/// pre/post_ask, pre/post_cap).
pub const HOOK_EVENT: &str = "hook_event";
/// Glob over tool/op name the hook applies to (default `*`).
pub const HOOK_MATCH: &str = "hook_match";
/// Hook mode: `observe` or `gate`.
pub const HOOK_MODE: &str = "hook_mode";
/// Stable content-addressed id (sha256:<hex64>) for the Python hook handler,
/// dispatched via the SAME tool bridge as `@tool` (one handler mechanism).
pub const PYTHON_HOOK_HANDLER_ID: &str = "python_hook_handler_id";

// -- Communication --
pub const MESSAGE: &str = "message";
pub const RECIPIENT: &str = "recipient";
pub const TARGET: &str = "target";
pub const TARGET_KIND: &str = "target_kind";
pub const PROTOCOL: &str = "protocol";
/// Semantic LLM operation represented by an agent communication turn.
///
/// `COMMUNICATE` remains the routing/session operation, but frontends can set
/// this to `ASK`, `THINK`, or `REASON` so analysis and backend telemetry keep
/// the corresponding LLM latency and mode semantics.
pub const LLM_OPERATION: &str = "llm_operation";

// -- Goals / reasoning --
pub const GOAL: &str = "goal";
pub const GOAL_ID: &str = "goal_id";
pub const PRIORITY: &str = "priority";
pub const EVIDENCE: &str = "evidence";
pub const CLAIM_TEXT: &str = "claim";
pub const GUARDRAIL_KIND: &str = "guardrail_kind";

// -- Control flow --
pub const DISCRIMINANT: &str = "discriminant";
pub const LABEL: &str = "label";
pub const TRUE_LABEL: &str = "true_label";
pub const FALSE_LABEL: &str = "false_label";
pub const CASE_LABELS: &str = "case_labels";
pub const TRY_LABEL: &str = "try_label";
pub const CATCH_LABEL: &str = "catch_label";
pub const AWAIT_RESULT: &str = "await_result";
pub const RECOVERY_TEMPLATE: &str = "recovery_template";
pub const CASE_REGIONS: &str = "case_regions";
pub const DEFAULT_REGION: &str = "default_region";
pub const REGION: &str = "region";

// -- Error handling --
pub const ON_FAIL: &str = "on_fail";
/// Declarative per-node retry/backoff + error-output primitive (additive,
/// back-compatible). The runtime honors these on ANY node generically:
///
/// - [`RETRY_MAX`]        : max retry attempts after the first try (0 = no retry).
/// - [`RETRY_BACKOFF_MS`] : base delay for exponential backoff between attempts
///   (`base * 2^(attempt-1)` ms); defaults to [`DEFAULT_RETRY_BACKOFF_MS`].
/// - [`CONTINUE_ON_ERROR`]: when `true`, a node that still fails after its retries
///   does NOT halt the run — it emits a structured error value
///   (see [`ERROR_OUTPUT_KEY`]) downstream so an error edge can consume it.
///
/// These are distinct from the op-specific `max_retries` some handlers already
/// read; the generic primitive lives on the dispatcher so every op gets it
/// without per-handler wiring. A node may set either or both.
pub const RETRY_MAX: &str = "retry_max";
pub const RETRY_BACKOFF_MS: &str = "retry_backoff_ms";
pub const CONTINUE_ON_ERROR: &str = "continue_on_error";
/// Default base backoff (ms) for the generic retry primitive when
/// [`RETRY_BACKOFF_MS`] is unset.
pub const DEFAULT_RETRY_BACKOFF_MS: u64 = 100;
/// Object key under which a continue-on-error node emits its structured error
/// value (so downstream "error edge" consumers can detect + branch on it).
pub const ERROR_OUTPUT_KEY: &str = "__apxm_error";

// -- Synchronization --
pub const SCOPE: &str = "scope";
pub const STORAGE: &str = "storage";
pub const TTL_SECONDS: &str = "ttl_seconds";
pub const STRATEGY: &str = "strategy";
pub const SEPARATOR: &str = "separator";
pub const ACTION: &str = "action";
/// MERGE op: ordered list of incoming `{{node_<id>}}` token references
/// that the runtime concatenates into the merged output.
pub const TOKENS: &str = "tokens";

// -- Tracing --
pub const TRACE_ID: &str = "trace_id";
pub const TRACE: &str = "trace";
pub const TRACE_QUERY: &str = "trace_query";

// -- Server / queue --
pub const CHECKPOINT: &str = "checkpoint";
pub const CHECKPOINT_ID: &str = "checkpoint_id";
pub const SERVER_URL: &str = "server_url";
pub const STAGING_ID: &str = "staging_id";
pub const CONTEXT_KEY: &str = "context_key";
pub const NOTIFICATION_URL: &str = "notification_url";
pub const POLL_INTERVAL_MS: &str = "poll_interval_ms";
pub const POLL_MAX_ATTEMPTS: &str = "poll_max_attempts";
pub const TIMEOUT_MS: &str = "timeout_ms";
pub const TIMEOUT: &str = "timeout";
pub const MAX_RETRIES: &str = "max_retries";
pub const HISTORY_LIMIT: &str = "history_limit";
pub const COUNT: &str = "count";

// -- Coordination --
pub const TASK_SPEC: &str = "task_spec";
pub const TARGET_AGENT: &str = "target_agent";
pub const ARGS: &str = "args";
pub const SESSION_ROOT: &str = "session_root";
/// Flow name on the structural flow-definition op.
pub const NAME: &str = "name";
/// Optional session id scoping a memory query (QMEM).
pub const SID: &str = "sid";

// -- Skill linkage (CALL_SKILL) --
/// Canonical skill identifier on a `CALL_SKILL` op (`"id"` or `"id@version"`).
pub const SKILL_ID: &str = "skill_id";
pub const HANDOFF: &str = "handoff";
pub const HANDOFF_FROM: &str = "handoff_from";
pub const HANDOFF_TO: &str = "handoff_to";
pub const TRANSFER_STATE: &str = "transfer_state";

// -- Optimization hints --
pub const CACHED_SYSTEM_PROMPT: &str = "cached_system_prompt";
pub const MEMOIZABLE: &str = "memoizable";
pub const RETRY_COUNT: &str = "retry_count";
pub const PROFILE_LATENCY_MS: &str = "__profile_latency_ms";
pub const PROFILE_P99_LATENCY_MS: &str = "__profile_p99_latency_ms";
pub const PROFILE_ERROR_RATE: &str = "__profile_error_rate";
pub const PROFILE_AVG_TOKENS: &str = "__profile_avg_tokens";
pub const PROFILE_TOKEN_WARNING: &str = "__profile_token_warning";
pub const WARMUP_CANDIDATE: &str = "warmup_candidate";
pub const SHARED_PREFIX_EST_TOKENS: &str = "shared_prefix_est_tokens";
pub const SHARED_PREFIX_GROUP_SIZE: &str = "shared_prefix_group_size";
pub const DOWNSTREAM_NODES: &str = "downstream_nodes";
pub const FANOUT_COUNT: &str = "fanout_count";
pub const REMAINING_PATH_LEN: &str = "remaining_path_len";
pub const LATENCY_CLASS: &str = "latency_class";
pub const BATCH_GROUP: &str = "batch_group";
pub const STAGE_INDEX: &str = "stage_index";
pub const ESTIMATED_DYNAMIC_TOKENS: &str = "estimated_dynamic_tokens";
pub const REUSE_GROUP: &str = "shared_prefix_group";
pub const EST_TEMPLATE_TOKENS: &str = "est_template_tokens";

// -- Graph-aware backend hints --
// Graph-aware backend hints written with the `ais.` prefix by the MLIR
// PromptCanonicalization + AssignPriority passes; ArtifactEmitter strips the
// prefix into the bare graph attrs the runtime accepts.
pub const AIS_SHARED_PREFIX_GROUP: &str = "ais.shared_prefix_group";
pub const AIS_SHARED_PREFIX_EST_TOKENS: &str = "ais.shared_prefix_est_tokens";
pub const AIS_SHARED_PREFIX_GROUP_SIZE: &str = "ais.shared_prefix_group_size";
pub const AIS_WARMUP_CANDIDATE: &str = "ais.warmup_candidate";
pub const AIS_DOWNSTREAM_NODES: &str = "ais.downstream_nodes";
pub const AIS_FANOUT_COUNT: &str = "ais.fanout_count";
pub const AIS_REMAINING_PATH_LEN: &str = "ais.remaining_path_len";
pub const AIS_LATENCY_CLASS: &str = "ais.latency_class";
pub const AIS_BATCH_GROUP: &str = "ais.batch_group";
pub const AIS_STAGE_INDEX: &str = "ais.stage_index";
pub const AIS_ESTIMATED_DYNAMIC_TOKENS: &str = "ais.estimated_dynamic_tokens";

/// Bare-name forms of MLIR-derived attributes (the result of
/// `ArtifactEmitter.cpp` stripping the `ais.` prefix when serializing).
/// These are re-derived by the MLIR PromptCanonicalization +
/// AssignPriority passes on every compile, so emitting them back into
/// AIR text would produce both the bare and prefixed forms on the same
/// op after the next pass run, breaking compile-decompile-recompile
/// idempotency. The AIR emitter filters this set when generating MLIR
/// text from a (possibly roundtripped) `AirModule`.
pub const MLIR_DERIVED_BARE_ATTRS: &[&str] = &[
    REUSE_GROUP,              // "shared_prefix_group"
    SHARED_PREFIX_EST_TOKENS, // "shared_prefix_est_tokens"
    SHARED_PREFIX_GROUP_SIZE, // "shared_prefix_group_size"
    WARMUP_CANDIDATE,         // "warmup_candidate"
    DOWNSTREAM_NODES,         // "downstream_nodes"
    FANOUT_COUNT,             // "fanout_count"
    REMAINING_PATH_LEN,       // "remaining_path_len"
    LATENCY_CLASS,            // "latency_class"
    BATCH_GROUP,              // "batch_group"
    STAGE_INDEX,              // "stage_index"
    ESTIMATED_DYNAMIC_TOKENS, // "estimated_dynamic_tokens"
];

/// AIS dialect prefix for MLIR-level attribute names.
pub const MLIR_ATTR_PREFIX: &str = "ais.";

/// Attribute keys whose string values are templates carrying `{name}`
/// placeholders. The compiler validator resolves every placeholder against
/// the node's [`INPUT_NAMES`] parallel array (matching incoming Data edges)
/// or the module's declared parameters. Adding a new template-bearing
/// attribute only requires extending this list.
pub const TEMPLATE_BEARING_ATTRS: &[&str] = &[
    TEMPLATE_STR,
    PROMPT,
    TEMPLATE,
    MESSAGE,
    PARAMS_JSON,
    TASK_SPEC,
    GOAL,
    CLAIM_TEXT,
    EVIDENCE,
    TRACE_ID,
    DISCRIMINANT,
    RECOVERY_TEMPLATE,
];

/// All attribute name values defined in this module.
///
/// Verifies that every OperationSpec field references a constant defined here.
pub const ALL_ATTR_NAMES: &[&str] = &[
    AGENT_NAME,
    FLOW_NAME,
    PROFILE,
    AGENT_ROUTE,
    REQUIRED_CAPABILITIES,
    PREFERRED_PROFILES,
    NODE_NAME,
    MODE,
    CWD,
    MODEL,
    MODEL_PROFILE,
    PROVIDER,
    API_KEY,
    BASE_URL,
    TEMPERATURE,
    SYSTEM_PROMPT,
    TOOLS_CONFIG,
    TOKEN_BUDGET,
    OUTPUT_SCHEMA,
    MAX_SCHEMA_RETRIES,
    MAX_ITERATIONS,
    HANDOFF_TARGETS,
    INNER_PLAN_SUPPORTED,
    ENABLE_INNER_PLAN,
    BIND_INNER_PLAN_OUTPUTS,
    BACKEND,
    MAX_TOOL_ITERATIONS,
    BUDGET,
    EFFORT,
    TEMPLATE_STR,
    PROMPT,
    TEMPLATE,
    INPUT_NAMES,
    QUERY,
    MEMORY_TIER,
    KEY,
    VALUE,
    LIMIT,
    CAPABILITY,
    PARAMS_JSON,
    TOOLS_ENABLED,
    TOOLS,
    CAPABILITY_GROUPS,
    TOOL_CALL_BUDGETS,
    CODE,
    INTERPRETER,
    CAPABILITY_NAME,
    DESCRIPTION,
    PARAMETERS_SCHEMA,
    PYTHON_HANDLER_ID,
    HOOK_EVENT,
    HOOK_MATCH,
    HOOK_MODE,
    PYTHON_HOOK_HANDLER_ID,
    MESSAGE,
    RECIPIENT,
    TARGET,
    TARGET_KIND,
    PROTOCOL,
    LLM_OPERATION,
    GOAL,
    GOAL_ID,
    PRIORITY,
    EVIDENCE,
    CLAIM_TEXT,
    GUARDRAIL_KIND,
    DISCRIMINANT,
    LABEL,
    TRUE_LABEL,
    FALSE_LABEL,
    CASE_LABELS,
    TRY_LABEL,
    CATCH_LABEL,
    AWAIT_RESULT,
    RECOVERY_TEMPLATE,
    CASE_REGIONS,
    DEFAULT_REGION,
    REGION,
    ON_FAIL,
    RETRY_MAX,
    RETRY_BACKOFF_MS,
    CONTINUE_ON_ERROR,
    SCOPE,
    STORAGE,
    TTL_SECONDS,
    STRATEGY,
    SEPARATOR,
    ACTION,
    TOKENS,
    TRACE_ID,
    TRACE,
    TRACE_QUERY,
    CHECKPOINT,
    CHECKPOINT_ID,
    SERVER_URL,
    STAGING_ID,
    CONTEXT_KEY,
    NOTIFICATION_URL,
    POLL_INTERVAL_MS,
    POLL_MAX_ATTEMPTS,
    TIMEOUT_MS,
    TIMEOUT,
    MAX_RETRIES,
    HISTORY_LIMIT,
    COUNT,
    TASK_SPEC,
    TARGET_AGENT,
    ARGS,
    SKILL_ID,
    SESSION_ROOT,
    HANDOFF,
    HANDOFF_FROM,
    HANDOFF_TO,
    TRANSFER_STATE,
    CACHED_SYSTEM_PROMPT,
    MEMOIZABLE,
    RETRY_COUNT,
    PROFILE_LATENCY_MS,
    PROFILE_P99_LATENCY_MS,
    PROFILE_ERROR_RATE,
    PROFILE_AVG_TOKENS,
    PROFILE_TOKEN_WARNING,
    WARMUP_CANDIDATE,
    SHARED_PREFIX_EST_TOKENS,
    SHARED_PREFIX_GROUP_SIZE,
    DOWNSTREAM_NODES,
    FANOUT_COUNT,
    REMAINING_PATH_LEN,
    LATENCY_CLASS,
    BATCH_GROUP,
    STAGE_INDEX,
    ESTIMATED_DYNAMIC_TOKENS,
    REUSE_GROUP,
    EST_TEMPLATE_TOKENS,
    AIS_SHARED_PREFIX_GROUP,
    AIS_SHARED_PREFIX_EST_TOKENS,
    AIS_SHARED_PREFIX_GROUP_SIZE,
    AIS_WARMUP_CANDIDATE,
    AIS_DOWNSTREAM_NODES,
    AIS_FANOUT_COUNT,
    AIS_REMAINING_PATH_LEN,
    AIS_LATENCY_CLASS,
    AIS_BATCH_GROUP,
    AIS_STAGE_INDEX,
    AIS_ESTIMATED_DYNAMIC_TOKENS,
    // OperationSpec-only fields (not graph attrs, but used in field names)
    "memory",
    "beliefs",
    "goals",
    "capabilities",
    "sandbox_config",
    "constraints",
    "reflection_prompt",
    "parameters",
    STRUCTURED,
    "token",
    "cases",
    "default",
    "ordering",
    "payload",
    "error_handler",
    NAME,
    SID,
];
