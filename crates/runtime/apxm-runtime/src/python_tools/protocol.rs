//! Wire protocol types for the Python tool worker bridge.
//!
//! NDJSON over stdin/stdout, multiplexed via `req_id`.
//! Protocol version is `1`.

use serde::{Deserialize, Serialize};

/// Protocol version stamped on every frame.
pub const PROTOCOL_VERSION: u32 = 1;

/// Request sent to the Python worker over stdin.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum WorkerRequest {
    /// Invoke a tool handler.
    #[serde(rename = "call")]
    Call(CallRequest),

    /// Cancel an in-flight call.
    #[serde(rename = "cancel")]
    Cancel(CancelRequest),
}

/// Invoke a registered tool handler.
#[derive(Debug, Clone, Serialize)]
pub struct CallRequest {
    /// Protocol version.
    pub v: u32,
    /// Unique request identifier for demuxing.
    pub req_id: String,
    /// Handler identifier (`sha256:...`).
    pub tool_id: String,
    /// Tool arguments as a JSON object.
    pub args: serde_json::Value,
    /// Deadline in milliseconds from now.
    pub deadline_ms: u64,
}

/// Cancel an in-flight call.
#[derive(Debug, Clone, Serialize)]
pub struct CancelRequest {
    /// Protocol version.
    pub v: u32,
    /// Request identifier to cancel.
    pub req_id: String,
}

/// Response received from the Python worker over stdout.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum WorkerResponse {
    /// Successful or failed result.
    #[serde(rename = "result")]
    Result(CallResponse),
}

/// Result frame from the worker.
#[derive(Debug, Clone, Deserialize)]
pub struct CallResponse {
    /// Protocol version.
    #[allow(dead_code)]
    pub v: u32,
    /// Request identifier this response correlates to.
    pub req_id: String,
    /// Whether the call succeeded.
    pub ok: bool,
    /// Return value on success.
    pub value: Option<serde_json::Value>,
    /// Error envelope on failure.
    pub error: Option<ErrorEnvelope>,
}

/// Structured error returned by a tool handler.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ErrorEnvelope {
    /// Error category (e.g. "validation", "timeout", "internal").
    pub kind: String,
    /// Human-readable error message.
    pub message: String,
    /// Optional Python traceback.
    pub traceback: Option<String>,
}

