//! Streaming model inference with explicit cancellation.
//!
//! A stream yields ordered chunks followed by exactly one terminal outcome. A
//! cancelled stream stops requesting chunks and commits [`ModelOutcome::Cancelled`];
//! it never fabricates a success or silently adapts a truncated stream into one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::effect::{ModelCallRequest, ModelOutcome};

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

/// One ordered stream chunk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamChunk {
    pub sequence: u64,
    pub text: String,
}

/// A completed stream: its ordered chunks and the single terminal outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamResult {
    pub chunks: Vec<StreamChunk>,
    pub terminal: ModelOutcome,
}

/// A streaming inference backend.
pub trait ModelStreamPort {
    /// Produce the next chunk, or `None` when the backend has no more chunks and
    /// the stream should terminate with `terminal_outcome`.
    fn next_chunk(&self, request: &ModelCallRequest, sequence: u64) -> Option<StreamChunk>;

    /// The terminal outcome for a stream that ran to completion.
    fn terminal_outcome(&self, request: &ModelCallRequest) -> ModelOutcome;
}

/// Drive a streaming effect to completion, honoring cancellation between chunks.
#[must_use]
pub fn stream<P: ModelStreamPort>(
    port: &P,
    request: &ModelCallRequest,
    cancel: &CancelToken,
) -> StreamResult {
    let mut chunks = Vec::new();
    let mut sequence = 0u64;
    loop {
        if cancel.is_cancelled() {
            return StreamResult {
                chunks,
                terminal: ModelOutcome::Cancelled,
            };
        }
        match port.next_chunk(request, sequence) {
            Some(chunk) => {
                chunks.push(chunk);
                sequence += 1;
            }
            None => {
                return StreamResult {
                    chunks,
                    terminal: port.terminal_outcome(request),
                };
            }
        }
    }
}
