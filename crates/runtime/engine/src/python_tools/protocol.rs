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

    /// Reply to a worker-initiated host call (e.g. an `llm.ask` from a hook).
    #[serde(rename = "host_result")]
    HostResult(HostResultResponse),
}

/// Runtime's reply to a `host_call` the worker raised mid-handler. Correlated
/// by the callback `req_id` the worker chose, NOT the parent call's req_id.
#[derive(Debug, Clone, Serialize)]
pub struct HostResultResponse {
    /// Protocol version.
    pub v: u32,
    /// Callback identifier echoed from the originating `host_call`.
    pub req_id: String,
    /// Whether the host serviced the call.
    pub ok: bool,
    /// Return value on success.
    pub value: Option<serde_json::Value>,
    /// Error envelope on failure.
    pub error: Option<ErrorEnvelope>,
}

/// Invoke a registered tool handler.
#[derive(Debug, Clone, Serialize)]
pub struct CallRequest {
    /// Protocol version.
    pub v: u32,
    /// Unique request identifier for demuxing.
    pub req_id: String,
    /// Runtime call identifier preserved from the originating invocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
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

    /// A worker-initiated call back into the runtime (e.g. a hook invoking
    /// `llm.ask` via `ctx.summarize`). The runtime services it with the same
    /// `ExecutionContext` that owns the parent call and replies with a
    /// `host_result` correlated by this frame's `req_id`.
    #[serde(rename = "host_call")]
    HostCall(HostCallRequest),
}

/// A host call raised by the worker while servicing a parent call.
#[derive(Debug, Clone, Deserialize)]
pub struct HostCallRequest {
    /// Protocol version.
    #[allow(dead_code)]
    pub v: u32,
    /// Callback identifier the worker assigned; echoed in the `host_result`.
    pub req_id: String,
    /// The parent call (tool/hook) on whose behalf this host call is raised.
    pub parent_req_id: String,
    /// Host method to invoke (currently `llm.ask`).
    pub method: String,
    /// Method parameters as a JSON object.
    pub params: serde_json::Value,
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
