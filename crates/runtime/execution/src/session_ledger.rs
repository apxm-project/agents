//! Pure per-session ledger semantics: turn caps and per-tool budgets.
//!
//! This is the canonical, plane-correct replacement for the legacy engine's
//! process-global `session_ledger`. It owns only the semantics — cap and budget
//! enforcement over in-memory counters — and holds no global registry and no
//! durable store. Durability is a separate concern behind [`SessionLedgerStore`],
//! which a lifecycle/persistence owner (Server) implements; the runtime plane
//! keeps only the fail-closed charging rules.

use std::collections::HashMap;
use std::sync::Mutex;

/// Typed session-ledger failures. Charging fails closed: a turn or tool call
/// that would exceed its bound is refused rather than served.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLedgerError {
    /// The next turn would exceed the configured session cap.
    TurnCapReached { cap: usize },
    /// The next tool call would exceed the configured per-session budget.
    ToolBudgetExhausted { tool: String, budget: usize },
    /// A durable store could not read a session's counters.
    DurableRead { session_id: String, message: String },
    /// A durable store could not persist a session's counters.
    DurableWrite { session_id: String, message: String },
    /// A durable row exists but is invalid for the current typed schema.
    CorruptRecord { session_id: String, message: String },
}

impl std::fmt::Display for SessionLedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TurnCapReached { cap } => write!(f, "session turn cap reached ({cap})"),
            Self::ToolBudgetExhausted { tool, budget } => {
                write!(f, "session tool budget exhausted for '{tool}' ({budget})")
            }
            Self::DurableRead { session_id, message } => {
                write!(f, "failed to read durable session ledger for '{session_id}': {message}")
            }
            Self::DurableWrite { session_id, message } => {
                write!(f, "failed to write durable session ledger for '{session_id}': {message}")
            }
            Self::CorruptRecord { session_id, message } => {
                write!(f, "invalid durable session ledger for '{session_id}': {message}")
            }
        }
    }
}

impl std::error::Error for SessionLedgerError {}

/// The persistable counter state for one session. A durable store round-trips
/// exactly this; it carries no caps/budgets (those are fixed configuration
/// supplied when the ledger is constructed).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LedgerState {
    pub turns_used: usize,
    pub tool_consumed: HashMap<String, usize>,
}

/// A durable backing for session counters, implemented by the lifecycle owner.
/// The runtime plane never assumes a store exists; it enforces caps purely and
/// lets a caller persist/rehydrate [`LedgerState`] through this seam.
pub trait SessionLedgerStore: Send + Sync {
    /// Load persisted counters for `session_id`, if any.
    ///
    /// # Errors
    ///
    /// Returns [`SessionLedgerError::DurableRead`] or
    /// [`SessionLedgerError::CorruptRecord`] when a stored row cannot be read
    /// or decoded.
    fn load(&self, session_id: &str) -> Result<Option<LedgerState>, SessionLedgerError>;

    /// Persist `state` for `session_id`.
    ///
    /// # Errors
    ///
    /// Returns [`SessionLedgerError::DurableWrite`] when the row cannot be
    /// written.
    fn persist(&self, session_id: &str, state: &LedgerState) -> Result<(), SessionLedgerError>;
}

/// Per-session caps and budgets over in-memory counters. Caps/budgets are fixed
/// at construction; counters advance only through the fail-closed charge
/// methods. This type is pure: it does not persist itself. A durable owner reads
/// [`SessionLedger::snapshot`] after a successful charge and writes it through a
/// [`SessionLedgerStore`].
#[derive(Debug)]
pub struct SessionLedger {
    turn_cap: Option<usize>,
    tool_budgets: HashMap<String, usize>,
    state: Mutex<LedgerState>,
}

impl SessionLedger {
    /// A fresh ledger with zeroed counters.
    #[must_use]
    pub fn new(turn_cap: Option<usize>, tool_budgets: HashMap<String, usize>) -> Self {
        Self {
            turn_cap,
            tool_budgets,
            state: Mutex::new(LedgerState::default()),
        }
    }

    /// A ledger rehydrated from persisted counters (e.g. after a restart).
    #[must_use]
    pub fn with_state(
        turn_cap: Option<usize>,
        tool_budgets: HashMap<String, usize>,
        state: LedgerState,
    ) -> Self {
        Self {
            turn_cap,
            tool_budgets,
            state: Mutex::new(state),
        }
    }

    /// Charge one substantive turn, returning the new running count. Fails
    /// closed when the turn would exceed the cap.
    ///
    /// # Errors
    ///
    /// Returns [`SessionLedgerError::TurnCapReached`] when the next turn would
    /// exceed the configured cap.
    pub fn charge_turn(&self) -> Result<usize, SessionLedgerError> {
        let mut state = self.state.lock().expect("session ledger poisoned");
        let next = state.turns_used + 1;
        if let Some(cap) = self.turn_cap
            && next > cap
        {
            return Err(SessionLedgerError::TurnCapReached { cap });
        }
        state.turns_used = next;
        Ok(next)
    }

    /// Charge one call against a tool's session budget, returning its new count.
    /// Fails closed when the call would exceed the budget.
    ///
    /// # Errors
    ///
    /// Returns [`SessionLedgerError::ToolBudgetExhausted`] when the next call
    /// would exceed the tool's configured budget.
    pub fn charge_tool(&self, tool: &str) -> Result<usize, SessionLedgerError> {
        let mut state = self.state.lock().expect("session ledger poisoned");
        let used = state.tool_consumed.get(tool).copied().unwrap_or(0);
        if let Some(&budget) = self.tool_budgets.get(tool)
            && used >= budget
        {
            return Err(SessionLedgerError::ToolBudgetExhausted {
                tool: tool.to_string(),
                budget,
            });
        }
        let next = used + 1;
        state.tool_consumed.insert(tool.to_string(), next);
        Ok(next)
    }

    #[must_use]
    pub fn turns_used(&self) -> usize {
        self.state.lock().expect("session ledger poisoned").turns_used
    }

    /// Remaining per-tool call budget for the session (unbounded tools omitted).
    #[must_use]
    pub fn tool_budgets_remaining(&self) -> HashMap<String, usize> {
        let state = self.state.lock().expect("session ledger poisoned");
        self.tool_budgets
            .iter()
            .map(|(tool, budget)| {
                let used = state.tool_consumed.get(tool).copied().unwrap_or(0);
                (tool.clone(), budget.saturating_sub(used))
            })
            .collect()
    }

    /// Whether the next substantive turn would exceed the cap.
    #[must_use]
    pub fn would_exceed_turn_cap(&self) -> bool {
        self.turn_cap.is_some_and(|cap| self.turns_used() >= cap)
    }

    #[must_use]
    pub fn turn_cap(&self) -> Option<usize> {
        self.turn_cap
    }

    /// Remaining substantive turns before the cap, if bounded.
    #[must_use]
    pub fn turns_remaining(&self) -> Option<usize> {
        self.turn_cap.map(|cap| cap.saturating_sub(self.turns_used()))
    }

    /// The current persistable counter snapshot.
    #[must_use]
    pub fn snapshot(&self) -> LedgerState {
        self.state.lock().expect("session ledger poisoned").clone()
    }
}
