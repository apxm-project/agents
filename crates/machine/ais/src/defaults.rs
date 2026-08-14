//! Canonical default values shared across the stack and authoring front-ends.

/// Default per-node output token budget for reasoning/LLM nodes when the author
/// hasn't set one. Generous so visible output (and any model-internal thinking)
/// isn't truncated; lowered to the `token_budget` attribute (runtime
/// `max_tokens`).
pub const DEFAULT_OUTPUT_TOKEN_BUDGET: u64 = 8192;
