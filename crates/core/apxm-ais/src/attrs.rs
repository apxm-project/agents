//! Canonical graph attribute constants for AIS operations.
//!
//! Every attribute name used in OperationSpec fields, MLIR TableGen,
//! runtime handlers, and the Python frontend MUST be defined here.
//! No other crate may introduce raw string literals for attribute names.

// -- Agent / identity --
pub const AGENT_NAME: &str = "agent_name";
pub const TEAM_NAME: &str = "team_name";
pub const FLOW_NAME: &str = "flow_name";
pub const PROFILE: &str = "profile";
pub const NODE_NAME: &str = "node_name";
pub const MODE: &str = "mode";
pub const CWD: &str = "cwd";

// -- LLM / model --
pub const MODEL: &str = "model";
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

// -- Template / prompt --
pub const TEMPLATE_STR: &str = "template_str";
pub const PROMPT: &str = "prompt";
pub const TEMPLATE: &str = "template";

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
pub const CODE: &str = "code";
pub const INTERPRETER: &str = "interpreter";
pub const CAPABILITY_NAME: &str = "capability_name";
pub const DESCRIPTION: &str = "description";
pub const PARAMETERS_SCHEMA: &str = "parameters_schema";

// -- Communication --
pub const MESSAGE: &str = "message";
pub const RECIPIENT: &str = "recipient";
pub const TARGET: &str = "target";
pub const PROTOCOL: &str = "protocol";

// -- Goals / reasoning --
pub const GOAL: &str = "goal";
pub const GOAL_ID: &str = "goal_id";
pub const PRIORITY: &str = "priority";
pub const CONDITION: &str = "condition";
pub const EVIDENCE: &str = "evidence";
pub const CLAIM_TEXT: &str = "claim";
pub const GUARDRAIL_KIND: &str = "guardrail_kind";

// -- Control flow --
pub const LABEL: &str = "label";
pub const TRUE_LABEL: &str = "true_label";
pub const FALSE_LABEL: &str = "false_label";
pub const CASE_LABELS: &str = "case_labels";
pub const TRY_LABEL: &str = "try_label";
pub const CATCH_LABEL: &str = "catch_label";
pub const RECOVERY_TEMPLATE: &str = "recovery_template";
pub const CASE_REGIONS: &str = "case_regions";
pub const DEFAULT_REGION: &str = "default_region";
pub const REGION: &str = "region";

// -- Error handling --
pub const ON_FAIL: &str = "on_fail";
pub const ERROR_MESSAGE: &str = "error_message";

// -- Synchronization --
pub const STRATEGY: &str = "strategy";
pub const SEPARATOR: &str = "separator";
pub const ACTION: &str = "action";

// -- Tracing --
pub const TRACE_ID: &str = "trace_id";
pub const TRACE: &str = "trace";
pub const TRACE_QUERY: &str = "trace_query";

// -- Server / queue --
pub const QUEUE: &str = "queue";
pub const CHECKPOINT: &str = "checkpoint";
pub const CHECKPOINT_ID: &str = "checkpoint_id";
pub const SERVER_URL: &str = "server_url";
pub const STAGING_ID: &str = "staging_id";
pub const CONTEXT_KEY: &str = "context_key";
pub const LEASE_MS: &str = "lease_ms";
pub const MAX_WAIT_MS: &str = "max_wait_ms";
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
pub const PARTIES: &str = "parties";
pub const PROPOSAL: &str = "proposal";
pub const MAX_ROUNDS: &str = "max_rounds";
pub const HANDOFF: &str = "handoff";
pub const HANDOFF_FROM: &str = "handoff_from";
pub const HANDOFF_TO: &str = "handoff_to";

// -- Optimization hints --
pub const CACHED_SYSTEM_PROMPT: &str = "cached_system_prompt";
pub const MEMOIZABLE: &str = "memoizable";
pub const WARMUP_CANDIDATE: &str = "warmup_candidate";
pub const SHARED_PREFIX_EST_TOKENS: &str = "shared_prefix_est_tokens";
pub const DOWNSTREAM_NODES: &str = "downstream_nodes";
pub const REUSE_GROUP: &str = "shared_prefix_group";
pub const EST_TEMPLATE_TOKENS: &str = "est_template_tokens";

/// AIS dialect prefix for MLIR-level attribute names.
pub const MLIR_ATTR_PREFIX: &str = "ais.";

/// All attribute name values defined in this module.
///
/// Used by the consistency test to verify that every OperationSpec field
/// references a constant defined here.
pub const ALL_ATTR_NAMES: &[&str] = &[
    AGENT_NAME,
    TEAM_NAME,
    FLOW_NAME,
    PROFILE,
    NODE_NAME,
    MODE,
    CWD,
    MODEL,
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
    TEMPLATE_STR,
    PROMPT,
    TEMPLATE,
    QUERY,
    MEMORY_TIER,
    KEY,
    VALUE,
    LIMIT,
    CAPABILITY,
    PARAMS_JSON,
    TOOLS_ENABLED,
    TOOLS,
    CODE,
    INTERPRETER,
    CAPABILITY_NAME,
    DESCRIPTION,
    PARAMETERS_SCHEMA,
    MESSAGE,
    RECIPIENT,
    TARGET,
    PROTOCOL,
    GOAL,
    GOAL_ID,
    PRIORITY,
    CONDITION,
    EVIDENCE,
    CLAIM_TEXT,
    GUARDRAIL_KIND,
    LABEL,
    TRUE_LABEL,
    FALSE_LABEL,
    CASE_LABELS,
    TRY_LABEL,
    CATCH_LABEL,
    RECOVERY_TEMPLATE,
    CASE_REGIONS,
    DEFAULT_REGION,
    REGION,
    ON_FAIL,
    ERROR_MESSAGE,
    STRATEGY,
    SEPARATOR,
    ACTION,
    TRACE_ID,
    TRACE,
    TRACE_QUERY,
    QUEUE,
    CHECKPOINT,
    CHECKPOINT_ID,
    SERVER_URL,
    STAGING_ID,
    CONTEXT_KEY,
    LEASE_MS,
    MAX_WAIT_MS,
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
    PARTIES,
    PROPOSAL,
    MAX_ROUNDS,
    HANDOFF,
    HANDOFF_FROM,
    HANDOFF_TO,
    CACHED_SYSTEM_PROMPT,
    MEMOIZABLE,
    WARMUP_CANDIDATE,
    SHARED_PREFIX_EST_TOKENS,
    DOWNSTREAM_NODES,
    REUSE_GROUP,
    EST_TEMPLATE_TOKENS,
    // OperationSpec-only fields (not graph attrs, but used in field names)
    "memory",
    "beliefs",
    "goals",
    "capabilities",
    "sandbox_config",
    "constraints",
    "reflection_prompt",
    "parameters",
    "structured",
    "token",
    "tokens",
    "count_token",
    "discriminant",
    "cases",
    "default",
    "args",
    "ordering",
    "error_handler",
    "scope",
    "storage",
    "ttl_seconds",
];
