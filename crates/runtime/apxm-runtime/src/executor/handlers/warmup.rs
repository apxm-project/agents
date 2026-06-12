//! Warmup request dispatch for shared-prefix optimization.
//!
//! Before dispatching a fan-out group of LLM calls that share a prefix, this module
//! sends a lightweight "warmup" request that prefills the shared prefix and generates
//! zero or one token. Prefix-cache-aware backends can retain that prefix before
//! the actual requests arrive, reducing downstream recomputation.
//!
//! ## Warmup Strategy
//!
//! For large shared contexts, APXM may send a warmup request that:
//! - Prefills the shared prefix
//! - Generates zero or one cheap token (max_tokens=1)
//! - Marks the request as warmup in metadata
//!
//! Warmup is gated by thresholds:
//! - Estimated shared prefix >= X tokens (configurable, default: 512)
//! - Fan-out >= Y downstream consumers (configurable, default: 2)
//! - Request target is explicitly latency-oriented

use super::{ExecutionContext, Result};
use apxm_backends::LLMRequest;
use apxm_core::constants::{
    graph::attrs as graph_attrs, runtime::llm_request_metadata as request_metadata,
};
use apxm_core::types::OptimizationTarget;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

const WARMUP_MAX_TOKENS: usize = 1;

/// Warmup configuration for shared-prefix optimization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WarmupConfig {
    /// Enable warmup requests.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Minimum shared prefix tokens to trigger warmup.
    #[serde(default = "default_min_prefix_tokens")]
    pub min_prefix_tokens: u32,
    /// Minimum fan-out (downstream consumers) to trigger warmup.
    #[serde(default = "default_min_fanout")]
    pub min_fanout: u32,
}

fn default_enabled() -> bool {
    true
}

fn default_min_prefix_tokens() -> u32 {
    512
}

fn default_min_fanout() -> u32 {
    2
}

impl Default for WarmupConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            min_prefix_tokens: default_min_prefix_tokens(),
            min_fanout: default_min_fanout(),
        }
    }
}

/// Warmup metrics tracking.
#[derive(Debug, Default)]
pub struct WarmupMetrics {
    /// Number of warmup requests sent.
    pub warmup_requests_sent: AtomicU64,
    /// Number of warmup requests that were later reused by downstream nodes.
    pub warmup_requests_reused: AtomicU64,
    /// Estimated tokens saved via warmup (shared prefix tokens * reuse count).
    pub warmup_tokens_saved: AtomicU64,
}

impl WarmupMetrics {
    /// Create new warmup metrics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment warmup requests sent counter.
    pub fn inc_requests_sent(&self) {
        self.warmup_requests_sent.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment warmup requests reused counter.
    pub fn inc_requests_reused(&self) {
        self.warmup_requests_reused.fetch_add(1, Ordering::Relaxed);
    }

    /// Add tokens saved via warmup.
    pub fn add_tokens_saved(&self, tokens: u64) {
        self.warmup_tokens_saved
            .fetch_add(tokens, Ordering::Relaxed);
    }

    /// Get current warmup requests sent count.
    pub fn requests_sent(&self) -> u64 {
        self.warmup_requests_sent.load(Ordering::Relaxed)
    }

    /// Get current warmup requests reused count.
    pub fn requests_reused(&self) -> u64 {
        self.warmup_requests_reused.load(Ordering::Relaxed)
    }

    /// Get current warmup tokens saved count.
    pub fn tokens_saved(&self) -> u64 {
        self.warmup_tokens_saved.load(Ordering::Relaxed)
    }
}

/// Check if compiler metadata makes a node eligible for runtime warmup.
///
/// This does not dispatch anything and does not inspect the selected backend.
/// It only interprets backend-agnostic node hints plus runtime thresholds and
/// the caller-selected optimization target.
pub fn should_warmup(
    node: &apxm_core::types::execution::Node,
    config: &WarmupConfig,
    target: OptimizationTarget,
) -> Option<u32> {
    if !config.enabled {
        return None;
    }

    // Check if node has warmup_candidate attribute
    let warmup_candidate = node
        .attributes
        .get(graph_attrs::WARMUP_CANDIDATE)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !warmup_candidate {
        return None;
    }

    // Check estimated shared prefix tokens
    let estimated_prefix_tokens = node
        .attributes
        .get(graph_attrs::SHARED_PREFIX_EST_TOKENS)
        .and_then(|v| v.as_u64())
        .map(|u| u as u32)
        .unwrap_or(0);

    if estimated_prefix_tokens < config.min_prefix_tokens {
        return None;
    }

    // Check shared-prefix fan-out. Prefer the compiler's group-size annotation:
    // per-node graph fan-out is often one even when several sibling requests
    // reuse the same prefix.
    let fanout = node
        .attributes
        .get(graph_attrs::SHARED_PREFIX_GROUP_SIZE)
        .and_then(|v| v.as_u64())
        .map(|u| u as u32)
        .or_else(|| {
            node.attributes
                .get(graph_attrs::DOWNSTREAM_NODES)
                .and_then(|v| match v {
                    apxm_core::types::Value::Array(items) => Some(items.len() as u32),
                    _ => None,
                })
        })
        .unwrap_or(0);

    if fanout < config.min_fanout {
        return None;
    }

    // Warmup is a cost-bearing runtime side effect, so analysis metadata alone
    // is not enough. The caller must explicitly select a latency-oriented
    // optimization target for this execution.
    if !matches!(
        target,
        OptimizationTarget::Latency | OptimizationTarget::Parallelism
    ) {
        return None;
    }

    Some(estimated_prefix_tokens)
}

/// Decide whether this node should dispatch a synthetic runtime warmup request.
///
/// Compiler hints are declarative eligibility metadata. The runtime owns the
/// side effect: it only dispatches warmup when policy thresholds pass and the
/// selected backend advertises graph extensions.
pub fn should_dispatch_warmup(
    ctx: &ExecutionContext,
    node: &apxm_core::types::execution::Node,
    request: &LLMRequest,
) -> Option<u32> {
    let estimated_prefix_tokens = should_warmup(node, &ctx.warmup_config, ctx.optimization_target)?;
    if !target_backend_supports_graph_extensions(ctx, request) {
        return None;
    }
    Some(estimated_prefix_tokens)
}

fn target_backend_supports_graph_extensions(ctx: &ExecutionContext, request: &LLMRequest) -> bool {
    let backend_name = if let Some(router) = &ctx.model_router {
        router.select(request).ok().map(|decision| decision.backend)
    } else {
        let prepared = ctx.llm_registry.prepare_request(request);
        ctx.llm_registry.resolve_backend_name(&prepared).ok()
    };

    backend_name
        .as_deref()
        .and_then(|name| ctx.llm_registry.get_backend(name))
        .is_some_and(|backend| backend.supports_graph_extensions())
}

/// Create a warmup request from a regular LLM request.
///
/// The warmup request:
/// - Uses the same prompt/messages as the original
/// - Sets max_tokens=1 to generate minimal output
/// - Marks the request as warmup in metadata
pub fn create_warmup_request(original: &LLMRequest, node_id: u64) -> LLMRequest {
    let mut warmup_req = original.clone();

    // Generate minimal output (0 or 1 token)
    warmup_req.max_tokens = Some(WARMUP_MAX_TOKENS);

    // Mark as warmup in metadata
    warmup_req.metadata.insert(
        request_metadata::WARMUP.to_string(),
        serde_json::json!(true),
    );
    warmup_req.metadata.insert(
        request_metadata::WARMUP_NODE_ID.to_string(),
        serde_json::json!(node_id),
    );

    // If the request has APXM hints, preserve the declarative warmup marker
    // for graph-aware backends that inspect compiler hints.
    if let Some(ref mut hints) = warmup_req.apxm_hints {
        hints.compiler_hints.warmup_candidate = Some(true);
    }

    warmup_req
}

/// Dispatch a warmup request before the main request.
///
/// This is a non-blocking fire-and-forget operation - we don't wait for the warmup
/// to complete before dispatching the real request.
pub async fn dispatch_warmup(
    ctx: &ExecutionContext,
    node_id: u64,
    phase: &str,
    request: &LLMRequest,
    estimated_prefix_tokens: u32,
) -> Result<()> {
    let warmup_req = create_warmup_request(request, node_id);
    ctx.warmup_metrics.inc_requests_sent();

    // Fire-and-forget warmup request
    let ctx_clone = ctx.clone();
    let phase_clone = phase.to_string();
    let estimated_tokens = estimated_prefix_tokens as u64;

    tokio::spawn(async move {
        // Execute warmup request (ignore result - this is best-effort)
        let result = if let Some(router) = &ctx_clone.model_router {
            router.generate(warmup_req).await
        } else {
            ctx_clone.llm_registry.generate(warmup_req).await
        };

        if let Err(e) = result {
            // Log warmup failure but don't propagate - warmup is best-effort
            tracing::debug!(
                "Warmup request for node {} phase {} failed (non-fatal): {}",
                node_id,
                phase_clone,
                e
            );
        } else {
            ctx_clone.warmup_metrics.inc_requests_reused();
            ctx_clone.warmup_metrics.add_tokens_saved(estimated_tokens);
            tracing::debug!(
                "Warmup request for node {} phase {} succeeded, estimated {} tokens saved",
                node_id,
                phase_clone,
                estimated_tokens
            );
        }
    });

    Ok(())
}

