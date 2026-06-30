use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};

use apxm_core::apxm_acp;

use crate::AcpError;
use crate::constants::json_rpc_errors;
use apxm_core::constants::jsonrpc;

/// A JSON-RPC 2.0 request (client→agent).
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC error object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Classified incoming message from the agent.
#[derive(Debug)]
pub enum JsonRpcMessage {
    /// Response to one of our requests (has id + result/error).
    Response(JsonRpcResponse),
    /// Unsolicited notification (has method, no id).
    Notification {
        method: String,
        params: Option<serde_json::Value>,
    },
    /// Reverse request from agent to us (has id + method, no result/error).
    ReverseRequest(JsonRpcRequest),
}

/// Abstraction over the JSON-RPC 2.0 transport layer used by ACP sessions.
///
/// `StdioTransport` covers the local subprocess case; `RelayTransport`
/// covers the T2 LINK-RUNTIME case where the ACP child runs on the host side
/// and frames are tunnelled over the Link WSS relay.
#[async_trait::async_trait]
pub trait AcpTransport: Send + Sync {
    async fn send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<u64, crate::AcpError>;

    async fn send_response(
        &mut self,
        id: serde_json::Value,
        result: serde_json::Value,
    ) -> Result<(), crate::AcpError>;

    async fn send_error(
        &mut self,
        id: serde_json::Value,
        code: i64,
        message: &str,
    ) -> Result<(), crate::AcpError>;

    async fn read_response_raw(
        &mut self,
        expected_id: u64,
        handler: &(dyn crate::reverse::ReverseHandler + Send + Sync),
    ) -> Result<serde_json::Value, crate::AcpError>;
}

/// NDJson transport over a child process's stdio.
pub struct StdioTransport {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: AtomicU64,
}

impl StdioTransport {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            stdin,
            reader: BufReader::new(stdout),
            next_id: AtomicU64::new(1),
        }
    }

    /// Serialize a value as NDJson and write it to stdin.
    async fn write_line(&mut self, value: &impl Serialize) -> Result<(), AcpError> {
        let mut line = serde_json::to_string(value)
            .map_err(|e| AcpError::Protocol(format!("serialize: {e}")))?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(AcpError::Io)?;
        self.stdin.flush().await.map_err(AcpError::Io)?;
        Ok(())
    }

    /// Send a JSON-RPC request and return the assigned message ID.
    pub async fn send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<u64, AcpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let request = JsonRpcRequest {
            jsonrpc: jsonrpc::VERSION.to_string(),
            id: serde_json::Value::Number(id.into()),
            method: method.to_string(),
            params,
        };
        apxm_acp!(debug, method = %request.method, id = id, "-> request");
        self.write_line(&request).await?;
        Ok(id)
    }

    /// Send a JSON-RPC response back to the agent (for reverse requests).
    pub async fn send_response(
        &mut self,
        id: serde_json::Value,
        result: serde_json::Value,
    ) -> Result<(), AcpError> {
        let response = JsonRpcResponse {
            jsonrpc: jsonrpc::VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        };
        self.write_line(&response).await
    }

    /// Send a JSON-RPC error response to the agent.
    pub async fn send_error(
        &mut self,
        id: serde_json::Value,
        code: i64,
        message: &str,
    ) -> Result<(), AcpError> {
        let response = JsonRpcResponse {
            jsonrpc: jsonrpc::VERSION.to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
                data: None,
            }),
        };
        self.write_line(&response).await
    }

    /// Read one NDJson line and classify it.
    pub async fn read_message(&mut self) -> Result<JsonRpcMessage, AcpError> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .await
            .map_err(AcpError::Io)?;
        if n == 0 {
            return Err(AcpError::SessionClosed);
        }
        let raw: serde_json::Value = serde_json::from_str(line.trim())
            .map_err(|e| AcpError::Protocol(format!("invalid JSON: {e}: {line}")))?;
        classify(raw)
    }

    /// Read messages until we get the response matching `expected_id`.
    ///
    /// Dispatches reverse requests and notifications along the way.
    pub async fn read_response<H: crate::reverse::ReverseHandler>(
        &mut self,
        expected_id: u64,
        handler: &H,
    ) -> Result<serde_json::Value, AcpError> {
        loop {
            let msg = self.read_message().await?;
            match msg {
                JsonRpcMessage::Response(resp) => {
                    let expected_val = serde_json::Value::Number(expected_id.into());
                    if resp.id == expected_val {
                        if let Some(err) = resp.error {
                            apxm_acp!(warn, id = expected_id, code = err.code, msg = %err.message, data = ?err.data, "<- error");
                            return Err(AcpError::AgentError {
                                code: err.code,
                                message: err.message,
                            });
                        }
                        apxm_acp!(debug, id = expected_id, "<- ok");
                        return Ok(resp.result.unwrap_or(serde_json::Value::Null));
                    }
                    apxm_acp!(warn, expected = expected_id, got = %resp.id, "unexpected response id");
                }
                JsonRpcMessage::Notification { method, params } => {
                    apxm_acp!(trace, method = %method, "<- notification");
                    handler.on_notification(&method, params.as_ref()).await;
                }
                JsonRpcMessage::ReverseRequest(req) => {
                    apxm_acp!(debug, method = %req.method, id = %req.id, "<- reverse request");
                    let req_id = req.id.clone();
                    match handler
                        .handle(&req.method, req.params.unwrap_or_default())
                        .await
                    {
                        Ok(result) => {
                            self.send_response(req_id, result).await?;
                        }
                        Err(e) => {
                            apxm_acp!(warn, method = %req.method, error = %e, "reverse request failed");
                            self.send_error(
                                req_id,
                                json_rpc_errors::INTERNAL_ERROR,
                                &e.to_string(),
                            )
                            .await?;
                        }
                    }
                }
            }
        }
    }

    /// Consume the transport, returning the stdin handle (for explicit close).
    pub fn into_stdin(self) -> ChildStdin {
        self.stdin
    }
}

/// `StdioTransport` implements the `AcpTransport` trait so callers can hold a
/// `Box<dyn AcpTransport>` and swap in `RelayTransport` for T2 sessions.
#[async_trait::async_trait]
impl AcpTransport for StdioTransport {
    async fn send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<u64, AcpError> {
        self.send_request(method, params).await
    }

    async fn send_response(
        &mut self,
        id: serde_json::Value,
        result: serde_json::Value,
    ) -> Result<(), AcpError> {
        self.send_response(id, result).await
    }

    async fn send_error(
        &mut self,
        id: serde_json::Value,
        code: i64,
        message: &str,
    ) -> Result<(), AcpError> {
        self.send_error(id, code, message).await
    }

    async fn read_response_raw(
        &mut self,
        expected_id: u64,
        handler: &(dyn crate::reverse::ReverseHandler + Send + Sync),
    ) -> Result<serde_json::Value, AcpError> {
        self.read_response(expected_id, handler).await
    }
}

/// Classify a raw JSON-RPC message by field presence.
/// Takes ownership to avoid cloning the Value tree during deserialization.
fn classify(raw: serde_json::Value) -> Result<JsonRpcMessage, AcpError> {
    let has_id = raw.get(jsonrpc::ID).is_some();
    let has_method = raw
        .get(jsonrpc::METHOD)
        .and_then(|m| m.as_str())
        .is_some_and(|s| !s.is_empty());
    let has_result = raw.get(jsonrpc::RESULT).is_some();
    let has_error = raw.get(jsonrpc::ERROR).is_some();

    if has_id && (has_result || has_error) {
        let resp: JsonRpcResponse = serde_json::from_value(raw)
            .map_err(|e| AcpError::Protocol(format!("bad response: {e}")))?;
        Ok(JsonRpcMessage::Response(resp))
    } else if has_id && has_method {
        let req: JsonRpcRequest = serde_json::from_value(raw)
            .map_err(|e| AcpError::Protocol(format!("bad reverse request: {e}")))?;
        Ok(JsonRpcMessage::ReverseRequest(req))
    } else if has_method && !has_id {
        // Safe: `has_method` guard guarantees `method` is a non-empty string.
        let method = raw
            .get(jsonrpc::METHOD)
            .and_then(|m| m.as_str())
            .ok_or_else(|| AcpError::Protocol("method field must be a string".to_string()))?
            .to_string();
        let params = raw.get(jsonrpc::PARAMS).cloned();
        Ok(JsonRpcMessage::Notification { method, params })
    } else {
        Err(AcpError::Protocol(format!("unclassifiable message: {raw}")))
    }
}
