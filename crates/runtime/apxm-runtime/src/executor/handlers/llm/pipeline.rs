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

/// Token pipeline for streaming producer → consumer.
///
/// Enables overlapping producer completion with downstream prefill by
/// streaming tokens as they're generated.
#[derive(Debug)]
#[allow(dead_code)]
pub struct TokenPipeline {
    /// Producer node ID
    pub producer_node_id: u64,
    /// Consumer node ID
    pub consumer_node_id: u64,
    /// Accumulated tokens from producer
    pub buffer: Vec<String>,
    /// Whether consumer has been started
    pub consumer_started: bool,
    /// Minimum tokens before starting consumer
    pub min_tokens_before_start: usize,
}

#[allow(dead_code)]
impl TokenPipeline {
    /// Create a new token pipeline
    pub fn new(producer_id: u64, consumer_id: u64, min_tokens: usize) -> Self {
        Self {
            producer_node_id: producer_id,
            consumer_node_id: consumer_id,
            buffer: Vec::new(),
            consumer_started: false,
            min_tokens_before_start: min_tokens,
        }
    }

    /// Add a token to the buffer
    pub fn push_token(&mut self, token: String) {
        self.buffer.push(token);
    }

    /// Check if we have enough tokens to start the consumer
    pub fn can_start_consumer(&self) -> bool {
        !self.consumer_started && self.buffer.len() >= self.min_tokens_before_start
    }

    /// Get the current buffered content
    pub fn get_content(&self) -> String {
        self.buffer.join("")
    }

    /// Mark consumer as started
    pub fn mark_consumer_started(&mut self) {
        self.consumer_started = true;
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

    #[test]
    fn token_pipeline_buffers_until_threshold() {
        let mut pipe = TokenPipeline::new(1, 2, 3);
        pipe.push_token("a".into());
        assert!(!pipe.can_start_consumer());
        pipe.push_token("b".into());
        pipe.push_token("c".into());
        assert!(pipe.can_start_consumer());
        pipe.mark_consumer_started();
        assert!(!pipe.can_start_consumer());
        assert_eq!(pipe.get_content(), "abc");
    }
}
