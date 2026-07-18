//! Token bookkeeping, budget enforcement, and producer→consumer streaming pipeline.

use super::ExecutionContext;
use crate::context_stack::{
    ContextPermissionScope, ContextPlan, ContextPlanningPolicy, ContextScope, ContextSegmentRole,
    ContextSegmentSpec, ContextSensitivity,
};
use apxm_backends::LLMRequest;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::types::execution::Node;
use apxm_core::types::models::TokenUsage;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use super::super::{Result, get_optional_u64_attribute};

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
    admitted_budget: Option<crate::executor::admitted_budget::AdmittedInvocationReservation>,
}

/// A request annotated with the exact input capacity admitted by the shared
/// context planner and its matching pre-dispatch token reservation.
pub(crate) struct ModelCallAdmission {
    pub(crate) request: LLMRequest,
    pub(crate) reservation: ModelCallReservation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RequestTokenEstimate {
    input_tokens: usize,
    output_tokens: usize,
}

impl RequestTokenEstimate {
    fn total_tokens(self) -> usize {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

impl ModelCallReservation {
    /// Reconcile the pre-dispatch estimate to provider-reported total usage.
    pub(crate) fn reconcile(mut self, usage: &TokenUsage) -> Result<()> {
        let total_result = self.reconcile_total(usage.total_tokens);
        let admitted_result = self
            .admitted_budget
            .take()
            .map(|reservation| reservation.reconcile(usage))
            .transpose()
            .map(|_| ());

        total_result.and(admitted_result)
    }

    fn reconcile_total(&mut self, actual_tokens: usize) -> Result<()> {
        if self.reserved_tokens == 0 {
            self.settled = true;
            return Ok(());
        }

        if actual_tokens == 0 {
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

/// Compute one context-backed request admission used by every model egress path.
pub(crate) fn admit_model_call(
    ctx: &ExecutionContext,
    request: &LLMRequest,
) -> Result<ModelCallAdmission> {
    let planning = context_planning_policy(ctx)?;
    let estimated = estimate_request_tokens(planning, request)?;
    let mut reservation = reserve_tokens(
        ctx.consumed_tokens.clone(),
        ctx.token_budget,
        estimated.total_tokens(),
    )?;
    let admitted_budget = ctx
        .admitted_invocation_budget
        .as_ref()
        .map(|budget| budget.reserve(estimated.input_tokens, estimated.output_tokens))
        .transpose()?;
    reservation.admitted_budget = admitted_budget;
    Ok(ModelCallAdmission {
        request: request
            .clone()
            .with_context_input_tokens(estimated.input_tokens),
        reservation,
    })
}

/// Resolve the policy that proves tokenizer and packing capacity for a model
/// request. Requests without this evidence are not safe to estimate.
pub(super) fn context_planning_policy(ctx: &ExecutionContext) -> Result<&ContextPlanningPolicy> {
    if let Some(stack) = ctx.context_stack.as_deref() {
        return stack.planning_policy().map_err(|error| RuntimeError::LLM {
            message: format!(
                "context planning policy is required for model token reservation: {error}"
            ),
            backend: None,
        });
    }
    ctx.context_planning
        .as_ref()
        .ok_or_else(|| RuntimeError::LLM {
            message: "context planning policy is required for model token reservation".to_string(),
            backend: None,
        })
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
            admitted_budget: None,
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
                admitted_budget: None,
            });
        }
    }
}

fn estimate_request_tokens(
    planning: &ContextPlanningPolicy,
    request: &LLMRequest,
) -> Result<RequestTokenEstimate> {
    let mut plan = ContextPlan::from_policy(planning, None).map_err(|error| RuntimeError::LLM {
        message: format!("request context planning failed: {error}"),
        backend: request.backend.clone(),
    })?;
    for (index, message) in request.resolved_messages().iter().enumerate() {
        let role = match message.role {
            apxm_backends::Role::System => ContextSegmentRole::System,
            apxm_backends::Role::Tool => ContextSegmentRole::Tool,
            apxm_backends::Role::User | apxm_backends::Role::Assistant => ContextSegmentRole::User,
        };
        let text = if message.text_content().is_empty() {
            serde_json::to_string(&message.content).map_err(|error| RuntimeError::LLM {
                message: format!("request message context serialization failed: {error}"),
                backend: request.backend.clone(),
            })?
        } else {
            message.text_content()
        };
        let max_tokens = plan.remaining_tokens();
        plan.admit(
            ContextSegmentSpec {
                scope: ContextScope::Local,
                role,
                provenance: format!("request:message:{index}"),
                permission: ContextPermissionScope::Local,
                sensitivity: ContextSensitivity::Private,
                protected: true,
                max_tokens,
            },
            &text,
        )
        .map_err(|error| RuntimeError::LLM {
            message: format!("request context planning failed: {error}"),
            backend: request.backend.clone(),
        })?;
    }
    if let Some(tools) = request.tools.as_ref() {
        for (index, tool) in tools.iter().enumerate() {
            let text = format!("{}\n{}\n{}", tool.name, tool.description, tool.parameters);
            let max_tokens = plan.remaining_tokens();
            plan.admit(
                ContextSegmentSpec {
                    scope: ContextScope::Local,
                    role: ContextSegmentRole::Tool,
                    provenance: format!("request:tool_schema:{index}"),
                    permission: ContextPermissionScope::Local,
                    sensitivity: ContextSensitivity::Internal,
                    protected: true,
                    max_tokens,
                },
                &text,
            )
            .map_err(|error| RuntimeError::LLM {
                message: format!("tool schema context planning failed: {error}"),
                backend: request.backend.clone(),
            })?;
        }
    }
    let output_tokens = request.max_tokens.ok_or_else(|| RuntimeError::LLM {
        message: "model token reservation requires an explicit resolved max_tokens value"
            .to_string(),
        backend: request.backend.clone(),
    })?;
    let thinking_tokens = match request.thinking_token_budget {
        Some(value) => usize::try_from(value).map_err(|_| RuntimeError::LLM {
            message: "thinking token budget exceeds the platform maximum".to_string(),
            backend: request.backend.clone(),
        })?,
        None => 0,
    };
    Ok(RequestTokenEstimate {
        input_tokens: plan.admitted_tokens(),
        output_tokens: output_tokens.max(thinking_tokens),
    })
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
    use crate::context_stack::{ContextTokenizer, ScopeRules};
    use std::collections::BTreeMap;

    fn planning(tokenizer: ContextTokenizer) -> ContextPlanningPolicy {
        ContextPlanningPolicy {
            tokenizer,
            token_budget: 1_024,
            profiles: BTreeMap::from([(
                "fixture".to_string(),
                ScopeRules {
                    upstream_depth: 0,
                    upstream_frame_budget: 0,
                    session_frame_budget: 0,
                    include_upstream_prompts: false,
                },
            )]),
        }
    }

    #[test]
    fn reservation_releases_unused_output_capacity_after_reconciliation() {
        let counter = Arc::new(AtomicU64::new(10));
        let reservation = reserve_tokens(counter.clone(), Some(100), 60).expect("reserve");
        assert_eq!(counter.load(Ordering::SeqCst), 70);

        reservation
            .reconcile(&TokenUsage::new(10, 15))
            .expect("reconcile");
        assert_eq!(counter.load(Ordering::SeqCst), 35);
    }

    #[test]
    fn reservation_rejects_an_overlapping_budget_claim_before_dispatch() {
        let counter = Arc::new(AtomicU64::new(70));
        let error = reserve_tokens(counter.clone(), Some(100), 31).expect_err("deny overspend");

        assert!(error.to_string().contains("reservation denied"));
        assert_eq!(counter.load(Ordering::SeqCst), 70);
    }

    #[test]
    fn request_estimate_uses_the_configured_policy_tokenizer_and_request_limit() {
        let policy = planning(ContextTokenizer::Cl100kBase);
        let request = LLMRequest::new("policy-selected token accounting")
            .with_system_prompt("system evidence")
            .with_max_tokens(17);

        let expected =
            crate::context_stack::estimate_tokens_with(policy.tokenizer, &request.prompt)
                + crate::context_stack::estimate_tokens_with(
                    policy.tokenizer,
                    request.system_prompt.as_deref().expect("system prompt"),
                )
                + 17;
        assert_eq!(
            estimate_request_tokens(&policy, &request)
                .unwrap()
                .total_tokens(),
            expected
        );
    }

    #[test]
    fn request_estimate_rejects_missing_output_reservation_evidence() {
        let error = estimate_request_tokens(
            &planning(ContextTokenizer::O200kBase),
            &LLMRequest::new("prompt"),
        )
        .expect_err("missing max_tokens must fail closed");

        assert!(
            error
                .to_string()
                .contains("requires an explicit resolved max_tokens")
        );
    }

    #[test]
    fn zero_usage_reconciliation_retains_the_safe_reservation_estimate() {
        let counter = Arc::new(AtomicU64::new(10));
        let reservation = reserve_tokens(counter.clone(), Some(100), 60).expect("reserve");

        reservation
            .reconcile(&TokenUsage::default())
            .expect("retain estimate");

        assert_eq!(counter.load(Ordering::SeqCst), 70);
    }
}
