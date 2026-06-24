use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GoalId(Uuid);

impl GoalId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for GoalId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for GoalId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Lifecycle state for a goal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalStatus {
    Pending,
    Active,
    Completed,
    Failed,
    Cancelled,
}

/// Goal descriptor shared across compiler-adjacent and runtime consumers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    pub id: GoalId,
    pub description: String,
    pub priority: u32,
    pub status: GoalStatus,
    pub parent_id: Option<GoalId>,
}

/// What a gate/eval node reports about whether a goal is met after one bounded
/// goal pass.
///
/// This is the typed counterpart to the free-text summary a gate node used to
/// pass through. Making it a typed value is what lets the runtime *decide*
/// convergence in code (see [`decide`]) instead of trusting a prompt to honor a
/// stop instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    /// Goal is satisfied; no further passes are needed.
    Done,
    /// Goal is not yet met; another bounded pass is warranted.
    NeedsMore,
    /// Goal cannot proceed without external input (missing dependency or host,
    /// ambiguous requirement). Halt and surface for a human/parent decision.
    Blocked,
    /// Continuing would be unsafe or violate policy. Halt for review.
    Unsafe,
}

impl GateStatus {
    /// Wire spelling (matches the serde `snake_case` representation).
    pub const fn as_str(self) -> &'static str {
        match self {
            GateStatus::Done => "done",
            GateStatus::NeedsMore => "needs_more",
            GateStatus::Blocked => "blocked",
            GateStatus::Unsafe => "unsafe",
        }
    }

    /// Whether this status means the goal is finished and converged.
    pub const fn is_done(self) -> bool {
        matches!(self, GateStatus::Done)
    }

    /// Whether this status means the goal must stop without converging
    /// (independently of how many passes remain).
    pub const fn is_halting(self) -> bool {
        matches!(self, GateStatus::Blocked | GateStatus::Unsafe)
    }

    /// Parse a status from loose text. Accepts the canonical wire spellings plus
    /// common synonyms a model may emit, case-insensitively. Returns `None` if
    /// nothing recognizable is present.
    pub fn parse_loose(text: &str) -> Option<Self> {
        let t = text.trim().to_ascii_lowercase();
        match t.as_str() {
            "done" | "complete" | "completed" | "success" | "succeeded" | "satisfied" => {
                Some(GateStatus::Done)
            }
            "needs_more" | "needs-more" | "needs more" | "more" | "continue" | "incomplete"
            | "partial" => Some(GateStatus::NeedsMore),
            "blocked" | "block" | "stuck" | "needs_input" | "needs-input" => {
                Some(GateStatus::Blocked)
            }
            "unsafe" | "unsafe_stop" | "policy_violation" | "halt" => Some(GateStatus::Unsafe),
            _ => None,
        }
    }
}

impl std::str::FromStr for GateStatus {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        GateStatus::parse_loose(s).ok_or(())
    }
}

/// A typed verdict from a gate/eval node about one bounded pass.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GateVerdict {
    /// Whether the goal is met, needs another pass, or must halt.
    pub status: GateStatus,
    /// Human-readable justification for the status.
    #[serde(default)]
    pub reason: String,
    /// Concrete remaining work items when `status` is `needs_more`. Carried
    /// forward as context into the next admitted pass.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining: Vec<String>,
    /// Optional model/self-reported confidence in [0, 1].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

impl GateVerdict {
    /// A converged verdict.
    pub fn done(reason: impl Into<String>) -> Self {
        Self {
            status: GateStatus::Done,
            reason: reason.into(),
            remaining: Vec::new(),
            confidence: None,
        }
    }

    /// A verdict requesting another bounded pass, with the remaining work.
    pub fn needs_more(reason: impl Into<String>, remaining: Vec<String>) -> Self {
        Self {
            status: GateStatus::NeedsMore,
            reason: reason.into(),
            remaining,
            confidence: None,
        }
    }

    /// A halting verdict (blocked or unsafe).
    pub fn halt(status: GateStatus, reason: impl Into<String>) -> Self {
        Self {
            status,
            reason: reason.into(),
            remaining: Vec::new(),
            confidence: None,
        }
    }

    /// Derive a *structural* verdict from a deterministic pass outcome, used when
    /// no LLM gate is present (deterministic transport). The goal is considered
    /// done when every node executed without error; any failed node means the
    /// pass left work undone and another bounded pass is warranted.
    ///
    /// This is deliberately conservative: it can only observe operational
    /// success, not semantic completion. A semantic verdict requires an LLM gate
    /// (see [`Self::parse`]).
    pub fn from_pass_outcome(failed_nodes: usize, remaining: Vec<String>) -> Self {
        if failed_nodes == 0 {
            Self::done("all nodes in the pass completed without error")
        } else {
            Self::needs_more(
                format!("{failed_nodes} node(s) in the pass failed; rerun is warranted"),
                remaining,
            )
        }
    }

    /// Parse a verdict from explicit JSON only: a direct JSON object, or the
    /// first balanced JSON object embedded in surrounding text (e.g. a fenced
    /// block). Returns `None` when there is no conforming JSON.
    ///
    /// Strict runtime path for auto-detecting an LLM gate's verdict, where a
    /// loose text match could false-positive on worker prose.
    pub fn parse_json(text: &str) -> Option<Self> {
        if let Ok(verdict) = serde_json::from_str::<GateVerdict>(text.trim()) {
            return Some(verdict);
        }
        if let Some(obj) = extract_json_object(text)
            && let Ok(verdict) = serde_json::from_str::<GateVerdict>(&obj) {
                return Some(verdict);
            }
        None
    }

    /// Parse a verdict leniently: JSON first (see [`Self::parse_json`]), then a
    /// loose `status: <word>` line or a bare recognized status word. Returns
    /// `None` if no status can be recovered.
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(verdict) = Self::parse_json(text) {
            return Some(verdict);
        }
        // Loose fallback: find a "status: <word>" or a bare recognized word.
        for line in text.lines() {
            let line = line.trim();
            let candidate = line
                .split_once(':')
                .map_or(line, |(k, v)| {
                    if k.trim().eq_ignore_ascii_case("status") {
                        v
                    } else {
                        line
                    }
                });
            let candidate = candidate
                .trim()
                .trim_matches(|c| matches!(c, '"' | '`' | '*' | '.' | ' '));
            if let Some(status) = GateStatus::parse_loose(candidate) {
                return Some(Self {
                    status,
                    reason: text.trim().chars().take(280).collect(),
                    remaining: Vec::new(),
                    confidence: None,
                });
            }
        }
        None
    }

    /// JSON Schema for this verdict, suitable as an `output_schema` attribute on
    /// an LLM gate op so the model is forced to return a conforming object.
    pub fn output_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["status", "reason"],
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["done", "needs_more", "blocked", "unsafe"],
                    "description": "Whether the goal is met (done), needs another pass (needs_more), is blocked on external input (blocked), or must stop for safety/policy (unsafe)"
                },
                "reason": { "type": "string", "description": "Justification for the status" },
                "remaining": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Concrete remaining work items when status is needs_more"
                },
                "confidence": { "type": "number", "minimum": 0, "maximum": 1 }
            }
        })
    }
}

/// The runtime decision about a goal after a bounded pass produced a verdict.
///
/// This is the output of the "goal brain" ([`decide`]). It is the runtime fact
/// that drives whether another admitted pass starts — the analogue of a
/// completion gate that keeps working until the goal is met, bounded so it
/// always terminates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum GoalDecision {
    /// The goal is met. Stop; success.
    Converged { reason: String },
    /// The goal is not met but passes remain. Start another admitted pass.
    Iterate {
        reason: String,
        /// Zero-based index of the next pass to run.
        next_iteration: usize,
    },
    /// Stop without converging: blocked, unsafe, or the pass budget is spent.
    Halted {
        reason: String,
        /// True when halting solely because the pass budget was exhausted (as
        /// opposed to a blocked/unsafe verdict).
        exhausted: bool,
    },
}

impl GoalDecision {
    /// Wire name of the event kind that should be emitted for this decision
    /// (`kind::GOAL_CONVERGED` / `GOAL_NEEDS_ANOTHER_PASS` / `GOAL_HALTED`).
    pub const fn event_kind_name(&self) -> &'static str {
        match self {
            GoalDecision::Converged { .. } => "goal_converged",
            GoalDecision::Iterate { .. } => "goal_needs_another_pass",
            GoalDecision::Halted { .. } => "goal_halted",
        }
    }

    /// Whether the goal run should stop after this decision.
    pub const fn is_terminal(&self) -> bool {
        !matches!(self, GoalDecision::Iterate { .. })
    }
}

/// Decide, from a typed gate verdict and the bounded-pass budget, whether the
/// goal has converged, needs another admitted pass, or must halt.
///
/// `iteration` is the zero-based index of the pass that produced `verdict`.
/// `max_iterations` is the hard ceiling on passes; it is clamped to at least 1
/// so a goal always runs at least one pass and always terminates.
///
/// This function is the single place completion is decided. It is pure and
/// exhaustively tested, which is what moves the convergence guarantee from a
/// prompt instruction into a runtime feature.
pub fn decide(verdict: &GateVerdict, iteration: usize, max_iterations: usize) -> GoalDecision {
    let max_iterations = max_iterations.max(1);
    match verdict.status {
        GateStatus::Done => GoalDecision::Converged {
            reason: non_empty_reason(&verdict.reason, "gate reported the goal is met"),
        },
        GateStatus::Blocked | GateStatus::Unsafe => GoalDecision::Halted {
            reason: non_empty_reason(
                &verdict.reason,
                match verdict.status {
                    GateStatus::Blocked => "gate reported the goal is blocked",
                    _ => "gate reported continuing is unsafe",
                },
            ),
            exhausted: false,
        },
        GateStatus::NeedsMore => {
            let next_iteration = iteration + 1;
            if next_iteration < max_iterations {
                GoalDecision::Iterate {
                    reason: non_empty_reason(&verdict.reason, "gate requested another pass"),
                    next_iteration,
                }
            } else {
                GoalDecision::Halted {
                    reason: format!(
                        "pass budget exhausted after {} pass(es); last gate: {}",
                        max_iterations,
                        non_empty_reason(&verdict.reason, "needs more work"),
                    ),
                    exhausted: true,
                }
            }
        }
    }
}

fn non_empty_reason(reason: &str, fallback: &str) -> String {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Extract the first balanced top-level JSON object substring from `text`.
/// Brace-aware and string-aware (ignores braces inside JSON strings). Returns
/// the substring including the outer braces, or `None`.
fn extract_json_object(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, &b) in bytes[start..].iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(text[start..=start + offset].to_string());
                }
            }
            _ => {}
        }
    }
    None
}
