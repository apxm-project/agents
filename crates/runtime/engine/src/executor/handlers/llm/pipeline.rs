//! Token bookkeeping, budget enforcement, and producer→consumer streaming pipeline.

use super::ExecutionContext;
use apxm_backends::LLMRequest;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::types::execution::Node;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use super::super::{Result, get_optional_u64_attribute};

const DEFAULT_OUTPUT_TOKEN_RESERVATION: usize = 1_024;

/// A pre-dispatch token-budget reservation for one backend request.
///
/// The shared counter includes committed usage and in-flight reservations. A
/// reservation is released on dispatch failure and reconciled to provider
/// usage after a response arrives, so parallel calls cannot over-admit a
/// bounded execution budget.
#[derive(Debug)]
pub(crate) struct ModelCallReservation {
    counter: Arc<AtomicU64>,
    reserved_tokens: u64,
    budget: Option<u64>,
    settled: bool,
}

impl ModelCallReservation {
    /// Reconcile the pre-dispatch estimate to provider-reported total usage.
    pub(crate) fn reconcile(mut self, actual_tokens: usize) -> Result<()> {
        if self.reserved_tokens == 0 {
            self.settled = true;
            return Ok(());
        }

        let actual_tokens = u64::try_from(actual_tokens).unwrap_or(u64::MAX);
        loop {
            let current = self.counter.load(Ordering::SeqCst);
            let committed = current.saturating_sub(self.reserved_tokens);
            let next = committed.saturating_add(actual_tokens);
            if self
                .counter
                .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                self.settled = true;
                if self.budget.is_some_and(|limit| next > limit) {
                    return Err(RuntimeError::LLM {
                        message: format!(
                            "provider-reported token usage {actual_tokens} exceeded the reserved budget; {next} tokens are committed"
                        ),
                        backend: None,
                    });
                }
                return Ok(());
            }
        }
    }
}

impl Drop for ModelCallReservation {
    fn drop(&mut self) {
        if !self.settled && self.reserved_tokens > 0 {
            self.counter
                .fetch_sub(self.reserved_tokens, Ordering::SeqCst);
        }
    }
}

/// Reserve estimated prompt, tool-schema, and output tokens before dispatch.
pub(crate) fn reserve_model_call(
    ctx: &ExecutionContext,
    request: &LLMRequest,
) -> Result<ModelCallReservation> {
    let estimated = estimate_request_tokens(request);
    reserve_tokens(ctx.consumed_tokens.clone(), ctx.token_budget, estimated)
}

fn reserve_tokens(
    counter: Arc<AtomicU64>,
    budget: Option<u64>,
    estimate: usize,
) -> Result<ModelCallReservation> {
    let reserved_tokens = u64::try_from(estimate).unwrap_or(u64::MAX);
    let Some(limit) = budget else {
        return Ok(ModelCallReservation {
            counter,
            reserved_tokens: 0,
            budget,
            settled: true,
        });
    };

    loop {
        let current = counter.load(Ordering::SeqCst);
        let next = current.saturating_add(reserved_tokens);
        if next > limit {
            return Err(RuntimeError::LLM {
                message: format!(
                    "token budget reservation denied: {current} committed or reserved + {reserved_tokens} requested exceeds {limit}"
                ),
                backend: None,
            });
        }
        if counter
            .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Ok(ModelCallReservation {
                counter,
                reserved_tokens,
                budget,
                settled: false,
            });
        }
    }
}

fn estimate_request_tokens(request: &LLMRequest) -> usize {
    let prompt_tokens = if request.has_messages() {
        request
            .messages
            .iter()
            .map(|message| crate::context_stack::estimate_tokens(&message.text_content()))
            .sum()
    } else {
        crate::context_stack::estimate_tokens(&request.prompt)
    };
    let system_tokens = request
        .system_prompt
        .as_deref()
        .map(crate::context_stack::estimate_tokens)
        .unwrap_or(0);
    let tool_tokens = request
        .tools
        .as_ref()
        .map(|tools| {
            tools
                .iter()
                .map(|tool| {
                    crate::context_stack::estimate_tokens(&format!(
                        "{}\n{}\n{}",
                        tool.name, tool.description, tool.parameters
                    ))
                })
                .sum()
        })
        .unwrap_or(0);
    let output_tokens = request
        .max_tokens
        .unwrap_or(DEFAULT_OUTPUT_TOKEN_RESERVATION)
        .max(
            request
                .thinking_token_budget
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(0),
        );
    prompt_tokens
        .saturating_add(system_tokens)
        .saturating_add(tool_tokens)
        .saturating_add(output_tokens)
}

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
/// `memoizable=true` so the in-process memo remains the primary
/// application-level cache for them.
pub(super) fn default_memoizable_for_backend(backend: Option<&str>) -> bool {
    !matches!(backend, Some("vllm" | "ollama"))
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
    loop {
        let current = ctx.consumed_tokens.load(Ordering::SeqCst);
        let next = current.saturating_add(added);
        if next > limit {
            return Err(RuntimeError::LLM {
                message: format!("token budget exceeded: {current} + {added} > {limit}"),
                backend: None,
            });
        }
        if ctx
            .consumed_tokens
            .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Ok(());
        }
    }
}

pub(super) fn resolve_global_token_budget(ctx: &ExecutionContext) -> Option<u64> {
    ctx.token_budget
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_releases_unused_output_capacity_after_reconciliation() {
        let counter = Arc::new(AtomicU64::new(10));
        let reservation = reserve_tokens(counter.clone(), Some(100), 60).expect("reserve");
        assert_eq!(counter.load(Ordering::SeqCst), 70);

        reservation.reconcile(25).expect("reconcile");
        assert_eq!(counter.load(Ordering::SeqCst), 35);
    }

    #[test]
    fn reservation_rejects_an_overlapping_budget_claim_before_dispatch() {
        let counter = Arc::new(AtomicU64::new(70));
        let error = reserve_tokens(counter.clone(), Some(100), 31).expect_err("deny overspend");

        assert!(error.to_string().contains("reservation denied"));
        assert_eq!(counter.load(Ordering::SeqCst), 70);
    }
}
