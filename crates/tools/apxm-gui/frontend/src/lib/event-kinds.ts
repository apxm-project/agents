// Typed event kind constants — mirrors Rust EventKind from apxm-core/events/kind.rs.
// All event matching uses these constants, never raw string literals.

export type EventCategory =
  | "stream"
  | "lifecycle"
  | "error"
  | "observability"
  | "user_action";

export interface EventKind<N extends string = string> {
  readonly name: N;
  readonly category: EventCategory;
  readonly terminal: boolean;
}

function kind<N extends string>(
  name: N,
  category: EventCategory,
  terminal = false,
): EventKind<N> {
  return { name, category, terminal };
}

// -- LLM event kinds --------------------------------------------------------
export const TOKEN = kind("token", "stream");
export const THOUGHT = kind("thought", "stream");
export const TOOL_CALL = kind("tool_call", "lifecycle");
export const LLM_DONE = kind("llm_done", "lifecycle", true);
export const USAGE = kind("usage", "observability");
export const RETRY = kind("retry", "error");
export const WARNING = kind("warning", "error");
export const CITATION = kind("citation", "observability");
export const PROVIDER_EVENT = kind("provider_event", "observability");

// -- Runtime event kinds -----------------------------------------------------
export const OPERATION_START = kind("operation_start", "lifecycle");
export const OPERATION_END = kind("operation_end", "lifecycle");
export const NODE_OUTPUT = kind("node_output", "observability");
export const NODE_METRICS = kind("node_metrics", "observability");
export const TOOL_START = kind("tool_start", "lifecycle");
export const TOOL_END = kind("tool_end", "lifecycle");
export const PLAN_CREATED = kind("plan_created", "lifecycle");
export const PLAN_STEP_STARTED = kind("plan_step_started", "lifecycle");
export const PLAN_STEP_COMPLETED = kind("plan_step_completed", "lifecycle");
export const MEMORY_READ = kind("memory_read", "observability");
export const MEMORY_WRITE = kind("memory_write", "observability");
export const CHECKPOINT_SAVED = kind("checkpoint_saved", "lifecycle");
export const CHECKPOINT_RESTORED = kind("checkpoint_restored", "lifecycle");
export const SCHEDULER_DECISION = kind("scheduler_decision", "observability");
export const HEAD_OF_LINE_BLOCK = kind("head_of_line_block", "observability");
export const GPU_UTILIZATION = kind("gpu_utilization", "observability");
export const TOKEN_USAGE = kind("token_usage", "observability");
export const MEMOIZATION_HIT = kind("memoization_hit", "observability");
export const ERROR = kind("error", "error", true);

// -- Session event kinds -----------------------------------------------------
export const CONTEXT_COMPACTED = kind("context_compacted", "observability");
export const MODEL_REROUTED = kind("model_rerouted", "lifecycle");
export const CANCELLED = kind("cancelled", "error", true);
export const LOOP_DETECTED = kind("loop_detected", "error");
export const CONTEXT_WINDOW_WARNING = kind("context_window_warning", "error");
export const SESSION_START = kind("session_start", "lifecycle");
export const SESSION_END = kind("session_end", "lifecycle", true);
export const TURN_BOUNDARY = kind("turn_boundary", "lifecycle");

// -- ACP event kinds ---------------------------------------------------------
export const ACP_SESSION_SPAWNED = kind("acp_session_spawned", "lifecycle");
export const ACP_SESSION_PROMPT_START = kind(
  "acp_session_prompt_start",
  "lifecycle",
);
export const ACP_SESSION_CHUNK = kind("acp_session_chunk", "stream");
export const ACP_SESSION_PROMPT_END = kind(
  "acp_session_prompt_end",
  "lifecycle",
  true,
);
export const ACP_SESSION_CLOSED = kind(
  "acp_session_closed",
  "lifecycle",
  true,
);
export const ACP_REVERSE_REQUEST = kind("acp_reverse_request", "lifecycle");
export const ACP_SESSION_ERROR = kind("acp_session_error", "error", true);
export const ACP_TOOL_RESULT = kind("acp_tool_result", "lifecycle");

// -- GUI event kinds ---------------------------------------------------------
export const TOOL_RESULT = kind("tool_result", "lifecycle");
export const DONE = kind("done", "lifecycle", true);
