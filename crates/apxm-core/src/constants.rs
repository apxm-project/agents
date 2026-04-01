//! Shared string constants for cross-crate protocol fields.
//!
//! Keep graph contract keys and common node attribute keys centralized here
//! so APXM and AgentMate frontends/backends stay consistent.

pub mod diagnostics {
    /// Compile diagnostics mode for canonical graph input.
    pub const MODE_GRAPH: &str = "graph";
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
    }

    pub mod attrs {
        pub const AGENT_NAME: &str = "agent_name";
        pub const FLOW_NAME: &str = "flow_name";
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
        pub const TEMPLATE_STR: &str = "template_str";
        pub const PROMPT: &str = "prompt";
        pub const TEMPLATE: &str = "template";
        pub const QUERY: &str = "query";
        pub const MEMORY_TIER: &str = "memory_tier";
        pub const SPACE: &str = "space";
        pub const CAPABILITY: &str = "capability";
        pub const PARAMS_JSON: &str = "params_json";
        pub const GOAL: &str = "goal";
        pub const TRACE_ID: &str = "trace_id";
        pub const TRACE: &str = "trace";
        pub const TRUE_LABEL: &str = "true_label";
        pub const FALSE_LABEL: &str = "false_label";
        pub const CASE_LABELS: &str = "case_labels";
        pub const LABEL: &str = "label";
        pub const TRY_LABEL: &str = "try_label";
        pub const CATCH_LABEL: &str = "catch_label";
        pub const RECOVERY_TEMPLATE: &str = "recovery_template";
        pub const TOOLS_ENABLED: &str = "tools_enabled";
        pub const TOOLS: &str = "tools";
        pub const MESSAGE: &str = "message";
        pub const RECIPIENT: &str = "recipient";
        pub const TARGET: &str = "target";
        pub const PROTOCOL: &str = "protocol";
        pub const CONDITION: &str = "condition";
        pub const VALUE: &str = "value";
        pub const KEY: &str = "key";
        pub const QUEUE: &str = "queue";
        pub const CHECKPOINT: &str = "checkpoint";
        pub const CHECKPOINT_ID: &str = "checkpoint_id";
        pub const SERVER_URL: &str = "server_url";
        pub const ACTION: &str = "action";
        pub const GOAL_ID: &str = "goal_id";
        pub const PRIORITY: &str = "priority";
        pub const ON_FAIL: &str = "on_fail";
        pub const ERROR_MESSAGE: &str = "error_message";
        pub const STRATEGY: &str = "strategy";
        pub const TIMEOUT_MS: &str = "timeout_ms";
        pub const BUDGET: &str = "budget";
        pub const MAX_RETRIES: &str = "max_retries";
        pub const HISTORY_LIMIT: &str = "history_limit";
        pub const LIMIT: &str = "limit";
        pub const STAGING_ID: &str = "staging_id";
        pub const CONTEXT_KEY: &str = "context_key";
        pub const LEASE_MS: &str = "lease_ms";
        pub const MAX_WAIT_MS: &str = "max_wait_ms";
        pub const NOTIFICATION_URL: &str = "notification_url";
        pub const POLL_INTERVAL_MS: &str = "poll_interval_ms";
        pub const POLL_MAX_ATTEMPTS: &str = "poll_max_attempts";
        pub const CASE_REGIONS: &str = "case_regions";
        pub const DEFAULT_REGION: &str = "default_region";
        pub const BACKEND: &str = "backend";
        pub const CLAIM_TEXT: &str = "claim";
        pub const CODE: &str = "code";
        pub const COUNT: &str = "count";
        pub const INTERPRETER: &str = "interpreter";
        pub const HANDOFF_FROM: &str = "handoff_from";
        pub const HANDOFF_TO: &str = "handoff_to";
        pub const MAX_TOOL_ITERATIONS: &str = "max_tool_iterations";
        // Phase 2 coordination/identity/self-organization attributes
        pub const TASK_SPEC: &str = "task_spec";
        pub const TARGET_AGENT: &str = "target_agent";
        pub const PARTIES: &str = "parties";
        pub const PROPOSAL: &str = "proposal";
        pub const TIMEOUT: &str = "timeout";
        pub const MAX_ROUNDS: &str = "max_rounds";
        pub const CAPABILITY_NAME: &str = "capability_name";
        pub const DESCRIPTION: &str = "description";
        pub const REGION: &str = "region";
        pub const PARAMETERS_SCHEMA: &str = "parameters_schema";

        // Optimization hint attributes (set by graph-level passes)
        /// Prompt caching hint: system prompt already sent by an earlier node.
        pub const CACHED_SYSTEM_PROMPT: &str = "cached_system_prompt";
        /// Memoization hint: this pure operation duplicates an earlier one.
        pub const MEMOIZABLE: &str = "memoizable";
    }
}

pub mod runtime {
    pub mod metadata {
        pub const PARENT_EXECUTION_ID: &str = "parent_execution_id";
        pub const SCOPE_ID: &str = "scope_id";
        pub const PARENT_SCOPE_ID: &str = "parent_scope_id";
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
        pub const SPAWNED_BY: &str = "spawned_by";
        pub const CAPABILITIES: &str = "capabilities";
        pub const GOALS: &str = "goals";
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
}
