//! Runtime accounting for the host-admitted invocation budget.
//!
//! The host serializes the generated `BudgetSet` from the canonical
//! `AgentInvocationEnvelope`; this module is deliberately the only runtime
//! projection of that transport. Model requests reserve both input and output
//! capacity before egress, and reconcile the reservation against provider
//! usage after the response. Cloned execution contexts share this ledger, so
//! a child cannot multiply its parent's admitted budget.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use apxm_core::{
    error::RuntimeError,
    types::{context_contracts::BudgetSet, models::TokenUsage},
};

/// Shared execution ledger derived directly from the host-admitted envelope.
#[derive(Debug)]
pub(crate) struct AdmittedInvocationBudget {
    budget: BudgetSet,
    consumed_input_tokens: AtomicU64,
    consumed_output_tokens: AtomicU64,
}

impl AdmittedInvocationBudget {
    pub(crate) fn new(budget: BudgetSet) -> Result<Self, RuntimeError> {
        if budget.concurrency == 0 {
            return Err(invalid_budget("concurrency must be at least one"));
        }
        if budget.wall_clock_ms == 0 {
            return Err(invalid_budget("wall_clock_ms must be at least one"));
        }
        Ok(Self {
            budget,
            consumed_input_tokens: AtomicU64::new(0),
            consumed_output_tokens: AtomicU64::new(0),
        })
    }

    pub(crate) fn reserve(
        self: &Arc<Self>,
        input_tokens: usize,
        output_tokens: usize,
    ) -> Result<AdmittedInvocationReservation, RuntimeError> {
        let reserved_input_tokens = reserve_tokens(
            &self.consumed_input_tokens,
            self.budget.input_tokens,
            input_tokens,
            "input",
        )?;
        let reserved_output_tokens = match reserve_tokens(
            &self.consumed_output_tokens,
            self.budget.output_tokens,
            output_tokens,
            "output",
        ) {
            Ok(reserved) => reserved,
            Err(error) => {
                self.consumed_input_tokens
                    .fetch_sub(reserved_input_tokens, Ordering::SeqCst);
                return Err(error);
            }
        };
        Ok(AdmittedInvocationReservation {
            budget: Arc::clone(self),
            reserved_input_tokens,
            reserved_output_tokens,
            settled: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn consumed(&self) -> (u64, u64) {
        (
            self.consumed_input_tokens.load(Ordering::SeqCst),
            self.consumed_output_tokens.load(Ordering::SeqCst),
        )
    }
}

/// One in-flight reservation against an [`AdmittedInvocationBudget`].
#[derive(Debug)]
pub(crate) struct AdmittedInvocationReservation {
    budget: Arc<AdmittedInvocationBudget>,
    reserved_input_tokens: u64,
    reserved_output_tokens: u64,
    settled: bool,
}

impl AdmittedInvocationReservation {
    /// Replace the pre-dispatch estimate with authoritative provider usage.
    ///
    /// A provider which omits usage reports zeroes. In that case we retain the
    /// safe reservation rather than silently freeing an unknown amount of the
    /// host-admitted budget.
    pub(crate) fn reconcile(mut self, usage: &TokenUsage) -> Result<(), RuntimeError> {
        let (actual_input_tokens, actual_output_tokens) = if usage.total_tokens == 0 {
            (self.reserved_input_tokens, self.reserved_output_tokens)
        } else {
            (
                u64::try_from(usage.input_tokens).unwrap_or(u64::MAX),
                u64::try_from(usage.output_tokens).unwrap_or(u64::MAX),
            )
        };
        let input_within_budget = reconcile_tokens(
            &self.budget.consumed_input_tokens,
            self.reserved_input_tokens,
            actual_input_tokens,
            self.budget.budget.input_tokens,
        );
        let output_within_budget = reconcile_tokens(
            &self.budget.consumed_output_tokens,
            self.reserved_output_tokens,
            actual_output_tokens,
            self.budget.budget.output_tokens,
        );
        self.settled = true;

        match (input_within_budget, output_within_budget) {
            (true, true) => Ok(()),
            (false, true) => Err(RuntimeError::LLM {
                message: "provider-reported input usage exceeded the admitted invocation budget"
                    .to_string(),
                backend: None,
            }),
            (true, false) => Err(RuntimeError::LLM {
                message: "provider-reported output usage exceeded the admitted invocation budget"
                    .to_string(),
                backend: None,
            }),
            (false, false) => Err(RuntimeError::LLM {
                message: "provider-reported usage exceeded the admitted invocation budget"
                    .to_string(),
                backend: None,
            }),
        }
    }
}

impl Drop for AdmittedInvocationReservation {
    fn drop(&mut self) {
        if !self.settled {
            self.budget
                .consumed_input_tokens
                .fetch_sub(self.reserved_input_tokens, Ordering::SeqCst);
            self.budget
                .consumed_output_tokens
                .fetch_sub(self.reserved_output_tokens, Ordering::SeqCst);
        }
    }
}

fn reserve_tokens(
    counter: &AtomicU64,
    limit: u64,
    requested: usize,
    kind: &str,
) -> Result<u64, RuntimeError> {
    let requested = u64::try_from(requested).unwrap_or(u64::MAX);
    loop {
        let current = counter.load(Ordering::SeqCst);
        let next = current.saturating_add(requested);
        if next > limit {
            return Err(RuntimeError::LLM {
                message: format!(
                    "admitted invocation {kind} budget reservation denied: {current} committed or reserved + {requested} requested exceeds {limit}"
                ),
                backend: None,
            });
        }
        if counter
            .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Ok(requested);
        }
    }
}

fn reconcile_tokens(counter: &AtomicU64, reserved: u64, actual: u64, limit: u64) -> bool {
    loop {
        let current = counter.load(Ordering::SeqCst);
        let committed = current.saturating_sub(reserved);
        let next = committed.saturating_add(actual);
        if counter
            .compare_exchange(current, next, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return next <= limit;
        }
    }
}

fn invalid_budget(message: &str) -> RuntimeError {
    RuntimeError::LLM {
        message: format!("admitted invocation budget is invalid: {message}"),
        backend: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget(input_tokens: u64, output_tokens: u64) -> BudgetSet {
        BudgetSet {
            input_tokens,
            output_tokens,
            tool_calls: 0,
            memory_bytes: 0,
            concurrency: 1,
            effects: 0,
            wall_clock_ms: 1,
        }
    }

    #[test]
    fn rejects_exhausted_input_before_a_model_request_is_dispatched() {
        let ledger = Arc::new(AdmittedInvocationBudget::new(budget(3, 32)).expect("budget"));
        let error = ledger.reserve(4, 1).expect_err("input must be denied");

        assert!(
            error
                .to_string()
                .contains("input budget reservation denied")
        );
        assert_eq!(ledger.consumed(), (0, 0));
    }

    #[test]
    fn rejects_exhausted_output_before_a_model_request_is_dispatched() {
        let ledger = Arc::new(AdmittedInvocationBudget::new(budget(32, 3)).expect("budget"));
        let error = ledger.reserve(1, 4).expect_err("output must be denied");

        assert!(
            error
                .to_string()
                .contains("output budget reservation denied")
        );
        assert_eq!(ledger.consumed(), (0, 0));
    }

    #[test]
    fn child_reservations_share_one_authoritative_budget() {
        let ledger = Arc::new(AdmittedInvocationBudget::new(budget(8, 8)).expect("budget"));
        let reservation = ledger.reserve(4, 4).expect("parent reservation");
        assert!(
            ledger.reserve(5, 1).is_err(),
            "child cannot widen input budget"
        );
        drop(reservation);
        assert_eq!(ledger.consumed(), (0, 0));
    }
}
