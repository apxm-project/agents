use serde::Serialize;

/// ACP-specific event payloads for integration with APXM's event bus.
///
/// These are emitted via the `ExecutionEventEmitter` trait during ACP
/// session lifecycle, flowing through the same channels as LLM events,
/// memory events, and scheduler events.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcpEventPayload {
    SessionSpawned {
        agent: String,
        session_id: String,
        cwd: String,
    },
    SessionPromptStart {
        session_id: String,
        prompt_len: usize,
    },
    SessionChunk {
        session_id: String,
        chunk_len: usize,
        total_len: usize,
    },
    SessionPromptEnd {
        session_id: String,
        stop_reason: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    SessionClosed {
        session_id: String,
        total_turns: u32,
        close_reason: String,
    },
    ReverseRequest {
        session_id: String,
        method: String,
        approved: bool,
    },
    SessionError {
        session_id: String,
        error: String,
    },
}
