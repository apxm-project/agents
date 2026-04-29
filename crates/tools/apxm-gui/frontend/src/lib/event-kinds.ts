// Typed event kind constants. Core constants are generated from Rust
// CORE_EVENT_KINDS; extension constants live in this facade.

export * from "./generated/core-event-kinds";
import type { EventCategory, EventKind } from "./generated/core-event-kinds";

function kind<N extends string>(
  name: N,
  category: EventCategory,
  terminal = false,
): EventKind<N> {
  return { name, category, terminal };
}

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
