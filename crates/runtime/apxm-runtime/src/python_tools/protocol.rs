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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_request_serializes() {
        let req = WorkerRequest::Call(CallRequest {
            v: PROTOCOL_VERSION,
            req_id: "u-1".into(),
            tool_id: "sha256:abc".into(),
            args: serde_json::json!({"a": 1, "b": 2}),
            deadline_ms: 9500,
        });
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"call\""));
        assert!(json.contains("\"req_id\":\"u-1\""));
        assert!(json.contains("\"tool_id\":\"sha256:abc\""));
    }

    #[test]
    fn test_cancel_request_serializes() {
        let req = WorkerRequest::Cancel(CancelRequest {
            v: PROTOCOL_VERSION,
            req_id: "u-2".into(),
        });
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"cancel\""));
        assert!(json.contains("\"req_id\":\"u-2\""));
    }

    #[test]
    fn test_success_response_deserializes() {
        let json = r#"{"v":1,"type":"result","req_id":"u-1","ok":true,"value":42}"#;
        let resp: WorkerResponse = serde_json::from_str(json).unwrap();
        match resp {
            WorkerResponse::Result(r) => {
                assert!(r.ok);
                assert_eq!(r.req_id, "u-1");
                assert_eq!(r.value, Some(serde_json::json!(42)));
                assert!(r.error.is_none());
            }
        }
    }

    #[test]
    fn test_error_response_deserializes() {
        let json = r#"{
            "v":1,"type":"result","req_id":"u-3","ok":false,
            "error":{"kind":"validation","message":"bad input","traceback":"  File ..."}
        }"#;
        let resp: WorkerResponse = serde_json::from_str(json).unwrap();
        match resp {
            WorkerResponse::Result(r) => {
                assert!(!r.ok);
                let err = r.error.unwrap();
                assert_eq!(err.kind, "validation");
                assert_eq!(err.message, "bad input");
                assert!(err.traceback.is_some());
            }
        }
    }
}
