//! Relay-backed ACP transport for T2 LINK-RUNTIME hosts.
//!
//! In the T2 tier, the host-side ACP child process is spawned on the host
//! machine and its JSON-RPC 2.0 stdio is tunnelled through the Link WSS relay.
//! `RelayTransport` implements [`AcpTransport`] by driving a pair of async
//! channels that the relay connection handler owns:
//!
//! - `tx`: frames going *down* to the host (APXM → host ACP child)
//! - `rx`: frames coming *up* from the host (host ACP child → APXM)
//!
//! The relay connection handler is responsible for framing, sequencing,
//! heartbeat, and reconnect; `RelayTransport` only sees plain `serde_json::Value`.

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;

use apxm_core::apxm_acp;
use apxm_core::constants::jsonrpc;

use crate::AcpError;
use crate::constants::json_rpc_errors;
use crate::protocol::{AcpTransport, JsonRpcError, JsonRpcMessage, JsonRpcRequest, JsonRpcResponse};
use crate::reverse::ReverseHandler;

/// Relay-backed JSON-RPC 2.0 transport for T2 LINK-RUNTIME sessions.
///
/// Owned by a `LinkAcpSession`. Send/receive use channel halves; the WSS relay
/// loop drives the underlying connection and exposes only plain JSON values here.
pub struct RelayTransport {
    tx: mpsc::Sender<serde_json::Value>,
    rx: mpsc::Receiver<serde_json::Value>,
    next_id: AtomicU64,
}

impl RelayTransport {
    /// Create a relay transport from an established frame channel pair.
    ///
    /// `tx` carries JSON-RPC frames from APXM down to the host ACP child.
    /// `rx` carries JSON-RPC frames from the host ACP child up to APXM.
    pub fn new(
        tx: mpsc::Sender<serde_json::Value>,
        rx: mpsc::Receiver<serde_json::Value>,
    ) -> Self {
        Self {
            tx,
            rx,
            next_id: AtomicU64::new(1),
        }
    }

    async fn write_value(&self, value: serde_json::Value) -> Result<(), AcpError> {
        self.tx.send(value).await.map_err(|_| {
            AcpError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "relay channel closed",
            ))
        })
    }

    async fn read_message(&mut self) -> Result<JsonRpcMessage, AcpError> {
        let raw = self.rx.recv().await.ok_or(AcpError::SessionClosed)?;
        classify_relay(raw)
    }
}

#[async_trait::async_trait]
impl AcpTransport for RelayTransport {
    async fn send_request(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<u64, AcpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = JsonRpcRequest {
            jsonrpc: jsonrpc::VERSION.to_string(),
            id: serde_json::Value::Number(id.into()),
            method: method.to_string(),
            params,
        };
        apxm_acp!(debug, method = %req.method, id = id, "[relay] -> request");
        let value = serde_json::to_value(&req)
            .map_err(|e| AcpError::Protocol(format!("serialize: {e}")))?;
        self.write_value(value).await?;
        Ok(id)
    }

    async fn send_response(
        &mut self,
        id: serde_json::Value,
        result: serde_json::Value,
    ) -> Result<(), AcpError> {
        let resp = JsonRpcResponse {
            jsonrpc: jsonrpc::VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        };
        let value = serde_json::to_value(&resp)
            .map_err(|e| AcpError::Protocol(format!("serialize: {e}")))?;
        self.write_value(value).await
    }

    async fn send_error(
        &mut self,
        id: serde_json::Value,
        code: i64,
        message: &str,
    ) -> Result<(), AcpError> {
        let resp = JsonRpcResponse {
            jsonrpc: jsonrpc::VERSION.to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
                data: None,
            }),
        };
        let value = serde_json::to_value(&resp)
            .map_err(|e| AcpError::Protocol(format!("serialize: {e}")))?;
        self.write_value(value).await
    }

    async fn read_response_raw(
        &mut self,
        expected_id: u64,
        handler: &(dyn ReverseHandler + Send + Sync),
    ) -> Result<serde_json::Value, AcpError> {
        loop {
            let msg = self.read_message().await?;
            match msg {
                JsonRpcMessage::Response(resp) => {
                    let expected_val = serde_json::Value::Number(expected_id.into());
                    if resp.id == expected_val {
                        if let Some(err) = resp.error {
                            apxm_acp!(warn, id = expected_id, code = err.code, msg = %err.message, "[relay] <- error");
                            return Err(AcpError::AgentError {
                                code: err.code,
                                message: err.message,
                            });
                        }
                        apxm_acp!(debug, id = expected_id, "[relay] <- ok");
                        return Ok(resp.result.unwrap_or(serde_json::Value::Null));
                    }
                    apxm_acp!(warn, expected = expected_id, got = %resp.id, "[relay] unexpected response id");
                }
                JsonRpcMessage::Notification { method, params } => {
                    apxm_acp!(trace, method = %method, "[relay] <- notification");
                    handler.on_notification(&method, params.as_ref()).await;
                }
                JsonRpcMessage::ReverseRequest(req) => {
                    apxm_acp!(debug, method = %req.method, id = %req.id, "[relay] <- reverse request");
                    let req_id = req.id.clone();
                    match handler
                        .handle(&req.method, req.params.unwrap_or_default())
                        .await
                    {
                        Ok(result) => {
                            self.send_response(req_id, result).await?;
                        }
                        Err(e) => {
                            apxm_acp!(warn, method = %req.method, error = %e, "[relay] reverse request failed");
                            self.send_error(req_id, json_rpc_errors::INTERNAL_ERROR, &e.to_string())
                                .await?;
                        }
                    }
                }
            }
        }
    }
}

fn classify_relay(raw: serde_json::Value) -> Result<JsonRpcMessage, AcpError> {
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
        let method = raw
            .get(jsonrpc::METHOD)
            .and_then(|m| m.as_str())
            .ok_or_else(|| AcpError::Protocol("method field must be a string".to_string()))?
            .to_string();
        let params = raw.get(jsonrpc::PARAMS).cloned();
        Ok(JsonRpcMessage::Notification { method, params })
    } else {
        Err(AcpError::Protocol(format!(
            "unclassifiable relay message: {raw}"
        )))
    }
}
