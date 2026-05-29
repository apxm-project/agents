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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memoizable_default_off_for_self_hosted_backends() {
        assert!(!default_memoizable_for_backend(Some("vllm")));
        assert!(!default_memoizable_for_backend(Some("ollama")));
    }

    #[test]
    fn memoizable_default_on_for_cloud_backends() {
        assert!(default_memoizable_for_backend(Some("openai")));
        assert!(default_memoizable_for_backend(Some("anthropic")));
        assert!(default_memoizable_for_backend(Some("google")));
        assert!(default_memoizable_for_backend(Some("hosted")));
    }

    #[test]
    fn memoizable_default_on_when_backend_unknown() {
        assert!(default_memoizable_for_backend(None));
    }
}
