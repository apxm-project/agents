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

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::{FinishReason, LLMResponse, TokenUsage};

    fn make_done_response() -> LLMResponse {
        LLMResponse::new(
            "done",
            "test-model",
            TokenUsage::new(10, 20),
            FinishReason::Stop,
        )
    }

    #[test]
    fn test_token_passthrough() {
        let mut asm = StreamAssembler::new();
        let events = asm.process(StreamChunk::Token("hello".into()));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AssembledEvent::Token(t) if t == "hello"));
    }

    #[test]
    fn test_thought_passthrough() {
        let mut asm = StreamAssembler::new();
        let events = asm.process(StreamChunk::Thought("thinking...".into()));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AssembledEvent::Thought(t) if t == "thinking..."));
    }

    #[test]
    fn test_usage_passthrough() {
        let mut asm = StreamAssembler::new();
        let usage = TokenUsage::new(5, 10);
        let events = asm.process(StreamChunk::Usage(usage));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AssembledEvent::Usage(u) if u.input_tokens == 5));
    }

    #[test]
    fn test_tool_call_assembly() {
        let mut asm = StreamAssembler::new();

        // Start
        let events = asm.process(StreamChunk::ToolCallStart {
            id: "call_1".into(),
            name: "bash".into(),
        });
        assert!(events.is_empty());

        // Delta 1
        let events = asm.process(StreamChunk::ToolCallDelta {
            id: "call_1".into(),
            arguments_delta: r#"{"comm"#.into(),
        });
        assert!(events.is_empty());

        // Delta 2
        let events = asm.process(StreamChunk::ToolCallDelta {
            id: "call_1".into(),
            arguments_delta: r#"and": "ls"}"#.into(),
        });
        assert!(events.is_empty());

        // Done flushes
        let events = asm.process(StreamChunk::Done(make_done_response()));
        assert_eq!(events.len(), 2); // ToolCall + Done
        match &events[0] {
            AssembledEvent::ToolCall(tc) => {
                assert_eq!(tc.id, "call_1");
                assert_eq!(tc.name, "bash");
                assert_eq!(tc.arguments["command"], "ls");
                assert!(!tc.partial);
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
        assert!(matches!(&events[1], AssembledEvent::Done(_)));
    }

    #[test]
    fn test_duplicate_start_resets_accumulator() {
        let mut asm = StreamAssembler::new();

        asm.process(StreamChunk::ToolCallStart {
            id: "call_1".into(),
            name: "bash".into(),
        });
        asm.process(StreamChunk::ToolCallDelta {
            id: "call_1".into(),
            arguments_delta: "old_data".into(),
        });

        // Duplicate start resets
        asm.process(StreamChunk::ToolCallStart {
            id: "call_1".into(),
            name: "read".into(),
        });
        asm.process(StreamChunk::ToolCallDelta {
            id: "call_1".into(),
            arguments_delta: r#"{"path": "foo.rs"}"#.into(),
        });

        let events = asm.process(StreamChunk::Done(make_done_response()));
        match &events[0] {
            AssembledEvent::ToolCall(tc) => {
                assert_eq!(tc.name, "read");
                assert_eq!(tc.arguments["path"], "foo.rs");
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn test_error_flushes_as_partial() {
        let mut asm = StreamAssembler::new();

        asm.process(StreamChunk::ToolCallStart {
            id: "call_1".into(),
            name: "bash".into(),
        });
        asm.process(StreamChunk::ToolCallDelta {
            id: "call_1".into(),
            arguments_delta: r#"{"partial": true"#.into(),
        });

        let events = asm.process(StreamChunk::Error("connection lost".into()));
        // Should have ToolCall(partial=true) + Error
        assert_eq!(events.len(), 2);
        match &events[0] {
            AssembledEvent::ToolCall(tc) => {
                assert!(tc.partial);
            }
            other => panic!("Expected partial ToolCall, got {:?}", other),
        }
        assert!(matches!(&events[1], AssembledEvent::Error(msg) if msg == "connection lost"));
    }

    #[test]
    fn test_implicit_accumulator_on_unknown_delta() {
        let mut asm = StreamAssembler::new();

        // Delta without prior Start
        asm.process(StreamChunk::ToolCallDelta {
            id: "call_x".into(),
            arguments_delta: r#"{"key": "val"}"#.into(),
        });

        let events = asm.process(StreamChunk::Done(make_done_response()));
        assert_eq!(events.len(), 2);
        match &events[0] {
            AssembledEvent::ToolCall(tc) => {
                assert_eq!(tc.id, "call_x");
                assert!(tc.name.is_empty()); // Implicit -- no name known.
                assert_eq!(tc.arguments["key"], "val");
                assert!(!tc.partial);
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
    }

    #[test]
    fn test_timeout_flush() {
        let mut asm = StreamAssembler::with_timeout(
            StreamAssembler::new(),
            Duration::from_millis(0), // Instant timeout for test
        );

        asm.process(StreamChunk::ToolCallStart {
            id: "call_t".into(),
            name: "slow_tool".into(),
        });
        asm.process(StreamChunk::ToolCallDelta {
            id: "call_t".into(),
            arguments_delta: "partial_args".into(),
        });

        // Sleep is not needed: timeout is 0ms so any subsequent process call
        // will detect it as timed out.
        let events = asm.process(StreamChunk::Token("next".into()));
        // Should flush timed-out accumulator, then emit the token.
        assert!(events.len() >= 2);
        assert!(matches!(&events[0], AssembledEvent::ToolCall(tc) if tc.partial));
        assert!(matches!(&events[1], AssembledEvent::Token(t) if t == "next"));
    }

    #[test]
    fn test_done_with_no_accumulators() {
        let mut asm = StreamAssembler::new();
        let events = asm.process(StreamChunk::Done(make_done_response()));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AssembledEvent::Done(_)));
    }

    #[test]
    fn test_unparseable_arguments_become_string() {
        let mut asm = StreamAssembler::new();
        asm.process(StreamChunk::ToolCallStart {
            id: "call_bad".into(),
            name: "tool".into(),
        });
        asm.process(StreamChunk::ToolCallDelta {
            id: "call_bad".into(),
            arguments_delta: "not valid json{{{".into(),
        });
        let events = asm.process(StreamChunk::Done(make_done_response()));
        match &events[0] {
            AssembledEvent::ToolCall(tc) => {
                // Unparseable JSON falls back to a JSON string.
                assert!(tc.arguments.is_string());
                assert_eq!(tc.arguments.as_str().unwrap(), "not valid json{{{");
            }
            other => panic!("Expected ToolCall, got {:?}", other),
        }
    }
}
