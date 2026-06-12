//! Token bookkeeping, budget enforcement, and producer→consumer streaming pipeline.

use super::ExecutionContext;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::types::execution::Node;
use std::sync::atomic::Ordering;

use super::super::{Result, get_optional_u64_attribute};

/// Backends that maintain their own request-level prefix / KV cache get
/// `memoizable=false` by default. Reasoning:
///
/// 1. The APXM in-process memo cache only hits on bit-identical full prompts,
///    which is rare in real agent workflows where each call interpolates
///    different context. The downstream prefix cache is the better cache layer.
/// 2. A memo hit short-circuits before the backend HTTP call, which hides the
///    backend's own cache telemetry (e.g. vLLM's `cached_input_tokens`,
///    `pinned_blocks`) from the runtime metrics report.
///
/// Cloud backends without an application-visible cache control surface keep
/// the historical default of `memoizable=true` so the in-process memo remains
/// the primary application-level cache for them.
pub(super) fn default_memoizable_for_backend(backend: Option<&str>) -> bool {
    match backend {
        Some("vllm") | Some("ollama") => false,
        _ => true,
    }
}

/// Map an `effort` attribute (`off`/`low`/`medium`/`high`) to an extended-
/// thinking token budget. `off` (or unset) returns `None` — no thinking. The
/// numbers are the single tunable mapping for thinking effort across backends;
/// each budget is ≥ the Anthropic minimum (1024). An unrecognized value is a
/// build/author error surfaced at execution.
pub(super) fn effort_token_budget(node: &Node) -> Result<Option<u64>> {
    let Some(effort) = node
        .attributes
        .get(graph_attrs::EFFORT)
        .and_then(|v| v.as_str())
    else {
        return Ok(None);
    };
    let budget = match effort.trim().to_ascii_lowercase().as_str() {
        "" | "off" | "none" => return Ok(None),
        "low" => 2_048,
        "medium" | "med" => 8_192,
        "high" => 24_576,
        other => {
            return Err(RuntimeError::LLM {
                message: format!(
                    "unknown {} '{other}'; expected off, low, medium, or high",
                    graph_attrs::EFFORT
                ),
                backend: None,
            });
        }
    };
    Ok(Some(budget))
}

pub(super) fn resolve_node_output_token_limit(node: &Node) -> Result<Option<usize>> {
    let Some(tokens) = get_optional_u64_attribute(node, graph_attrs::TOKEN_BUDGET)? else {
        return Ok(None);
    };
    usize::try_from(tokens)
        .map(Some)
        .map_err(|_| RuntimeError::LLM {
            message: format!(
                "{} exceeds the platform maximum for request max_tokens",
                graph_attrs::TOKEN_BUDGET
            ),
            backend: None,
        })
}

pub(super) fn charge_tokens(
    ctx: &ExecutionContext,
    budget: Option<u64>,
    delta: usize,
) -> Result<()> {
    let Some(limit) = budget else {
        return Ok(());
    };
    let added = u64::try_from(delta).unwrap_or(u64::MAX);
    let total = ctx
        .consumed_tokens
        .fetch_add(added, Ordering::SeqCst)
        .saturating_add(added);
    if total > limit {
        return Err(RuntimeError::LLM {
            message: format!("token budget exceeded: used {} > budget {}", total, limit),
            backend: None,
        });
    }
    Ok(())
}

pub(super) fn resolve_global_token_budget(ctx: &ExecutionContext) -> Option<u64> {
    ctx.token_budget
}

