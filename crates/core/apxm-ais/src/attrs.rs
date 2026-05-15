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
pub const STRUCTURED: &str = "structured";

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
pub const TOOL_GROUPS: &str = "tool_groups";
pub const CODE: &str = "code";
pub const INTERPRETER: &str = "interpreter";
pub const CAPABILITY_NAME: &str = "capability_name";
pub const DESCRIPTION: &str = "description";
pub const PARAMETERS_SCHEMA: &str = "parameters_schema";
/// When set, this capability is backed by a Python handler instead of a Rust
/// builtin. The runtime dispatches to the python tool worker using this stable
/// id. Must match `^sha256:[0-9a-f]{64}$`.
pub const PYTHON_HANDLER_ID: &str = "python_handler_id";

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
pub const CONDITION: &str = "condition";
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
pub const ERROR_MESSAGE: &str = "error_message";

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
pub const ARGS: &str = "args";
pub const SESSION_ROOT: &str = "session_root";
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
/// Legacy alias produced by the Python frontend when the user passes
/// `g.ask(reuse_group=...)` directly. The MLIR PromptCanonicalization
/// pass emits the canonical `REUSE_GROUP` ("shared_prefix_group") name;
/// explicit user-authored kwargs land under this literal key. The
/// runtime accepts both to preserve backward compatibility with
/// workloads that explicitly tag `reuse_group=` (e.g.
/// `examples/python/benchmarks/workloads/pin_demo.py`). The long-term
/// resolution is to define a single canonical attribute enum that
/// every layer (Python kwarg, MLIR pass, Rust runtime) shares — see
/// the followup tracked in `docs/claims/dispatch-phase1-honest-null.md`.
pub const REUSE_GROUP_LEGACY: &str = "reuse_group";
pub const EST_TEMPLATE_TOKENS: &str = "est_template_tokens";

// -- Graph-aware backend hints --
// Mirrors of MLIR Constants.h::attrs::* (with ais. prefix). Written by the
// MLIR PromptCanonicalization + AssignPriority passes; ArtifactEmitter strips
// the prefix into the bare graph attrs consumed by the runtime.
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
    PROPOSAL,
    GOAL,
    CONDITION,
    CLAIM_TEXT,
    EVIDENCE,
    TRACE_ID,
    DISCRIMINANT,
    RECOVERY_TEMPLATE,
];

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
    TOOL_GROUPS,
    CODE,
    INTERPRETER,
    CAPABILITY_NAME,
    DESCRIPTION,
    PARAMETERS_SCHEMA,
    PYTHON_HANDLER_ID,
    MESSAGE,
    RECIPIENT,
    TARGET,
    TARGET_KIND,
    PROTOCOL,
    LLM_OPERATION,
    GOAL,
    GOAL_ID,
    PRIORITY,
    CONDITION,
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
    ERROR_MESSAGE,
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
    ARGS,
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
    "count_token",
    "cases",
    "default",
    "ordering",
    "payload",
    "error_handler",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Drift detector: every AIS_*-prefixed Rust constant must equal the
    /// matching `constexpr llvm::StringLiteral X = "..."` in MLIR Constants.h.
    /// Editing one side without the other breaks payload contracts silently
    /// in production; this test fails the build instead.
    #[test]
    fn ais_attrs_match_mlir_constants_h() {
        let constants_h =
            include_str!("../../../compiler/apxm-compiler/mlir/include/ais/Common/Constants.h");
        let pairs = [
            ("SHARED_PREFIX_GROUP", AIS_SHARED_PREFIX_GROUP),
            ("SHARED_PREFIX_EST_TOKENS", AIS_SHARED_PREFIX_EST_TOKENS),
            ("WARMUP_CANDIDATE", AIS_WARMUP_CANDIDATE),
            ("DOWNSTREAM_NODES", AIS_DOWNSTREAM_NODES),
            ("FANOUT_COUNT", AIS_FANOUT_COUNT),
            ("REMAINING_PATH_LEN", AIS_REMAINING_PATH_LEN),
            ("LATENCY_CLASS", AIS_LATENCY_CLASS),
            ("BATCH_GROUP", AIS_BATCH_GROUP),
            ("STAGE_INDEX", AIS_STAGE_INDEX),
            ("ESTIMATED_DYNAMIC_TOKENS", AIS_ESTIMATED_DYNAMIC_TOKENS),
        ];
        for (cpp_name, rust_value) in pairs {
            let needle = format!("{cpp_name} = \"{rust_value}\"");
            assert!(
                constants_h.contains(&needle),
                "MLIR Constants.h drift: expected `{}` (Rust constant differs from C++ literal)",
                needle
            );
        }
    }

    #[test]
    fn all_attr_names_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        let mut duplicates = Vec::new();
        for name in ALL_ATTR_NAMES {
            if !seen.insert(*name) {
                duplicates.push(*name);
            }
        }

        assert!(duplicates.is_empty(), "duplicate attrs: {duplicates:?}");
    }
}
