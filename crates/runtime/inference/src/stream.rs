//! Streaming model inference with exact content references and cancellation.
//!
//! A stream yields ordered events followed by exactly one terminal outcome.
//! Content-bearing events preserve their references instead of projecting
//! transport payloads into inline text. A cancelled stream stops requesting
//! steps and commits [`ModelOutcome::Cancelled`]; it never fabricates success
//! from a truncated stream.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

use crate::effect::{ErrorCategory, ModelCallRequest, ModelOutcome, TypedError};

/// A cooperative cancellation token shared with a streaming backend.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Request cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// A reference to content retained at the exact streaming boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelContentRef(pub String);

/// One ordered event from a model stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelStreamEvent {
    /// A delta for model-authored content stored behind `content_ref`.
    ContentDelta {
        sequence: u64,
        content_ref: ModelContentRef,
    },
    /// A delta for a model-authored Tool-call proposal stored behind
    /// `content_ref`. It does not invoke an APXM Capability.
    ToolCallDelta {
        sequence: u64,
        content_ref: ModelContentRef,
    },
    /// A non-content liveness/progress signal.
    Heartbeat { sequence: u64 },
}

impl ModelStreamEvent {
    fn sequence(&self) -> u64 {
        match self {
            Self::ContentDelta { sequence, .. }
            | Self::ToolCallDelta { sequence, .. }
            | Self::Heartbeat { sequence } => *sequence,
        }
    }
}

/// One unambiguous step at the streaming transport boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelStreamStep {
    /// The next ordered stream event.
    Event { event: ModelStreamEvent },
    /// The stream's single terminal outcome.
    Terminal { outcome: ModelOutcome },
}

/// A completed stream: its ordered events and single terminal outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamResult {
    pub events: Vec<ModelStreamEvent>,
    pub terminal: ModelOutcome,
}

/// A streaming inference backend.
pub trait ModelStreamPort {
    /// Produce the next event or the stream's single terminal outcome. The
    /// transport observes the same cancellation token as the runtime driver so
    /// it can interrupt an in-flight receive when its dependency supports it.
    fn next_step(
        &self,
        request: &ModelCallRequest,
        expected_sequence: u64,
        cancel: &CancelToken,
    ) -> ModelStreamStep;
}

/// Drive a streaming effect to completion while preserving exact events and
/// honoring cancellation before and during every transport step.
#[must_use]
pub fn stream<P: ModelStreamPort + ?Sized>(
    port: &P,
    request: &ModelCallRequest,
    cancel: &CancelToken,
) -> StreamResult {
    let mut events = Vec::new();
    let mut expected_sequence = 0u64;
    loop {
        if cancel.is_cancelled() {
            return StreamResult {
                events,
                terminal: ModelOutcome::Cancelled,
            };
        }

        let step = port.next_step(request, expected_sequence, cancel);

        if cancel.is_cancelled() {
            return match step {
                ModelStreamStep::Terminal { outcome } => StreamResult {
                    events,
                    terminal: outcome,
                },
                ModelStreamStep::Event { .. } => StreamResult {
                    events,
                    terminal: ModelOutcome::ModelOutcomeUnknown {
                        uncertain_usage: None,
                    },
                },
            };
        }

        match step {
            ModelStreamStep::Event { event } => {
                let actual_sequence = event.sequence();
                if actual_sequence != expected_sequence {
                    return StreamResult {
                        events,
                        terminal: ModelOutcome::TypedFailure {
                            error: TypedError {
                                category: ErrorCategory::Internal,
                                code: "stream_sequence_mismatch".to_string(),
                                message: format!(
                                    "stream event sequence {actual_sequence} did not match expected {expected_sequence}"
                                ),
                            },
                        },
                    };
                }
                events.push(event);
                expected_sequence += 1;
            }
            ModelStreamStep::Terminal { outcome } => {
                return StreamResult {
                    events,
                    terminal: outcome,
                };
            }
        }
    }
}
