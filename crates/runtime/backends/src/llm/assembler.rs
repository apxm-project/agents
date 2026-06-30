//! StreamAssembler: assembles fragmented StreamChunk sequences into complete events.
//!
//! Tool-call arguments arrive as a sequence of `ToolCallStart` + `ToolCallDelta`
//! chunks.  The assembler accumulates those fragments and emits a single
//! `AssembledEvent::ToolCall` when the stream completes (via `Done`) or when an
//! accumulator times out.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use apxm_core::types::TokenUsage;

use crate::llm::backends::{LLMResponse, StreamChunk};

/// Fragment assembly state for an in-progress tool call.
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
    started_at: Instant,
}

/// Assembles fragmented [`StreamChunk`] sequences into complete events.
///
/// # Algorithm
///
/// 1. `Token`         -- emit immediately.
/// 2. `Thought`       -- emit immediately.
/// 3. `ToolCallStart` -- create accumulator (duplicate start resets it).
/// 4. `ToolCallDelta` -- append to accumulator (unknown id creates implicit one).
/// 5. `Done`          -- flush all pending accumulators, then emit `Done`.
/// 6. `Error`         -- flush all pending accumulators with `partial = true`, then emit `Error`.
///
/// **Timeout:** 30 s of no activity on an accumulator flushes it as partial.
pub struct StreamAssembler {
    accumulators: HashMap<String, ToolCallAccumulator>,
    timeout: Duration,
}

impl StreamAssembler {
    /// Create a new assembler with the default 30-second timeout.
    pub fn new() -> Self {
        Self {
            accumulators: HashMap::new(),
            timeout: Duration::from_secs(30),
        }
    }

    /// Override the inactivity timeout for tool-call accumulators.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Process a single chunk and return any complete events.
    pub fn process(&mut self, chunk: StreamChunk) -> Vec<AssembledEvent> {
        // Check for timed-out accumulators first (skip if none pending).
        let mut events = if self.accumulators.is_empty() {
            Vec::new()
        } else {
            self.flush_timed_out()
        };

        match chunk {
            StreamChunk::Token(text) => {
                events.push(AssembledEvent::Token(text));
            }
            StreamChunk::Thought(text) => {
                events.push(AssembledEvent::Thought(text));
            }
            StreamChunk::ToolCallStart { id, name } => {
                // Duplicate start resets accumulator.
                self.accumulators.insert(
                    id.clone(),
                    ToolCallAccumulator {
                        id,
                        name,
                        arguments: String::new(),
                        started_at: Instant::now(),
                    },
                );
            }
            StreamChunk::ToolCallDelta {
                id,
                arguments_delta,
            } => {
                if let Some(acc) = self.accumulators.get_mut(&id) {
                    acc.arguments.push_str(&arguments_delta);
                    acc.started_at = Instant::now(); // Reset timeout.
                } else {
                    // Unknown delta -- create implicit accumulator.
                    self.accumulators.insert(
                        id.clone(),
                        ToolCallAccumulator {
                            id,
                            name: String::new(),
                            arguments: arguments_delta,
                            started_at: Instant::now(),
                        },
                    );
                }
            }
            StreamChunk::Done(response) => {
                // Flush all accumulators (complete).
                events.extend(self.flush_all(false));
                events.push(AssembledEvent::Done(response));
            }
            StreamChunk::Usage(usage) => {
                events.push(AssembledEvent::Usage(usage));
            }
            StreamChunk::Error(msg) => {
                // Flush all accumulators as partial.
                events.extend(self.flush_all(true));
                events.push(AssembledEvent::Error(msg));
            }
        }

        events
    }

    /// Returns `true` when there are no pending tool-call accumulators.
    pub fn is_empty(&self) -> bool {
        self.accumulators.is_empty()
    }

    // -- private helpers --

    fn flush_timed_out(&mut self) -> Vec<AssembledEvent> {
        let now = Instant::now();
        let timed_out: Vec<String> = self
            .accumulators
            .iter()
            .filter(|(_, acc)| now.duration_since(acc.started_at) > self.timeout)
            .map(|(id, _)| id.clone())
            .collect();

        timed_out
            .iter()
            .filter_map(|id| {
                self.accumulators.remove(id).map(|acc| {
                    AssembledEvent::ToolCall(AssembledToolCall {
                        id: acc.id,
                        name: acc.name,
                        arguments: Self::parse_arguments(&acc.arguments),
                        partial: true,
                    })
                })
            })
            .collect()
    }

    fn flush_all(&mut self, partial: bool) -> Vec<AssembledEvent> {
        self.accumulators
            .drain()
            .map(|(_, acc)| {
                AssembledEvent::ToolCall(AssembledToolCall {
                    id: acc.id,
                    name: acc.name,
                    arguments: Self::parse_arguments(&acc.arguments),
                    partial,
                })
            })
            .collect()
    }

    fn parse_arguments(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
    }
}

impl Default for StreamAssembler {
    fn default() -> Self {
        Self::new()
    }
}

/// A fully assembled event produced by [`StreamAssembler::process`].
#[derive(Debug, Clone)]
pub enum AssembledEvent {
    /// A text token.
    Token(String),
    /// An extended-thinking / reasoning token.
    Thought(String),
    /// A complete (or partial) tool call.
    ToolCall(AssembledToolCall),
    /// Incremental usage data.
    Usage(TokenUsage),
    /// Final response.
    Done(LLMResponse),
    /// Stream error.
    Error(String),
}

/// A tool call assembled from `ToolCallStart` + `ToolCallDelta` fragments.
#[derive(Debug, Clone)]
pub struct AssembledToolCall {
    /// Tool-call ID assigned by the provider.
    pub id: String,
    /// Name of the tool to invoke.
    pub name: String,
    /// Parsed arguments (JSON object or raw string fallback).
    pub arguments: serde_json::Value,
    /// `true` when the tool call was flushed due to timeout or error
    /// before a `Done` chunk arrived.
    pub partial: bool,
}
