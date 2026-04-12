//! Lightweight ACP client for communicating with agent subprocesses.
//!
//! Spawns a command (e.g. `openclaw acp`) and communicates via NDJson on
//! stdin/stdout, implementing the Agent Client Protocol handshake and
//! prompt/response cycle.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use apxm_acp::constants::{
    fields as acp_fields, methods as acp_methods, stop_reasons, update_keys, update_types,
};
use apxm_core::constants::jsonrpc;
use apxm_core::events::payload::EventPayload;

use crate::events::{
    AgentDonePayload, AgentErrorPayload, AgentTokenPayload, AgentToolCallPayload,
    AgentToolResultPayload, AgentUsagePayload,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Errors from ACP client operations.
#[derive(Debug)]
pub struct AgentError(pub String);

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AgentError {}

impl From<std::io::Error> for AgentError {
    fn from(e: std::io::Error) -> Self {
        AgentError(format!("IO error: {e}"))
    }
}

impl From<serde_json::Error> for AgentError {
    fn from(e: serde_json::Error) -> Self {
        AgentError(format!("JSON error: {e}"))
    }
}

// ---------------------------------------------------------------------------
// AgentSession
// ---------------------------------------------------------------------------

/// An active ACP session with a spawned agent subprocess.
pub struct AgentSession {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    session_id: String,
    next_id: AtomicU64,
}

impl AgentSession {
    /// Spawn an agent subprocess and complete the ACP handshake.
    ///
    /// `command` is the executable (e.g. `"openclaw"`), which is invoked with
    /// `acp` as its first argument. The subprocess's cwd is set to `cwd`.
    pub async fn spawn(command: &str, cwd: &Path) -> Result<Self, AgentError> {
        let parts: Vec<&str> = command.split_whitespace().collect();
        let (bin, args) = parts
            .split_first()
            .ok_or_else(|| AgentError("empty command".to_string()))?;

        let mut child = tokio::process::Command::new(bin)
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| AgentError(format!("failed to spawn {command}: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AgentError("failed to capture subprocess stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AgentError("failed to capture subprocess stdout".to_string()))?;

        let reader = BufReader::new(stdout);
        let next_id = AtomicU64::new(1);

        let mut session = Self {
            child,
            stdin,
            reader,
            session_id: String::new(),
            next_id,
        };

        // ACP handshake: initialize
        let init_id = session.next_id();
        let init_req = serde_json::json!({
            jsonrpc::JSONRPC: jsonrpc::VERSION,
            jsonrpc::ID: init_id,
            jsonrpc::METHOD: acp_methods::INITIALIZE,
            jsonrpc::PARAMS: {
                acp_fields::PROTOCOL_VERSION: apxm_acp::constants::protocol::ACP_PROTOCOL_VERSION,
                acp_fields::CLIENT_INFO: {
                    "name": "apxm-gui",
                    "version": env!("CARGO_PKG_VERSION")
                },
                acp_fields::CLIENT_CAPABILITIES: {
                    "roots": true
                }
            }
        });
        session.send_message(&init_req).await?;
        let _init_resp = session.read_response(init_id).await?;
        debug!("ACP initialize complete");

        // ACP handshake: session/new
        let new_id = session.next_id();
        let new_req = serde_json::json!({
            jsonrpc::JSONRPC: jsonrpc::VERSION,
            jsonrpc::ID: new_id,
            jsonrpc::METHOD: acp_methods::SESSION_NEW,
            jsonrpc::PARAMS: {
                apxm_core::constants::acp::session_params::CWD: cwd.to_string_lossy(),
                apxm_core::constants::acp::session_params::MCP_SERVERS: []
            }
        });
        session.send_message(&new_req).await?;
        let new_resp = session.read_response(new_id).await?;

        session.session_id = new_resp
            .get(jsonrpc::RESULT)
            .and_then(|r| r.get(acp_fields::SESSION_ID))
            .and_then(|s| s.as_str())
            .unwrap_or("default")
            .to_string();

        debug!(session_id = %session.session_id, "ACP session established");
        Ok(session)
    }

    /// Return the ACP session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Send a prompt and stream agent events to `tx`.
    ///
    /// Blocks until the agent completes its response (the JSON-RPC response to
    /// our prompt request arrives). Reverse requests from the agent (file I/O,
    /// terminal, permissions) are handled inline.
    pub async fn prompt(
        &mut self,
        text: &str,
        tx: mpsc::Sender<Arc<dyn EventPayload>>,
    ) -> Result<(), AgentError> {
        let prompt_id = self.next_id();
        let req = serde_json::json!({
            jsonrpc::JSONRPC: jsonrpc::VERSION,
            jsonrpc::ID: prompt_id,
            jsonrpc::METHOD: acp_methods::SESSION_PROMPT,
            jsonrpc::PARAMS: {
                acp_fields::SESSION_ID: self.session_id,
                "prompt": [{"type": "text", "text": text}]
            }
        });
        self.send_message(&req).await?;

        // Read lines until we see the response to our prompt_id.
        let mut line_buf = String::new();
        loop {
            line_buf.clear();
            let n = self.reader.read_line(&mut line_buf).await?;
            if n == 0 {
                send_payload(
                    &tx,
                    AgentErrorPayload {
                        error: "agent process closed stdout".into(),
                    },
                )
                .await;
                return Err(AgentError("agent process closed stdout".into()));
            }

            let trimmed = line_buf.trim();
            if trimmed.is_empty() {
                continue;
            }

            let msg: Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(e) => {
                    warn!(line = trimmed, error = %e, "ignoring unparseable line from agent");
                    continue;
                }
            };

            // Check if this is a response to our prompt request.
            if let Some(id) = msg.get(jsonrpc::ID) {
                if id.as_u64() == Some(prompt_id) {
                    // This is the final response — extract usage and stop reason.
                    let result = msg.get(jsonrpc::RESULT);
                    let stop_reason = result
                        .and_then(|r| r.get(acp_fields::STOP_REASON))
                        .and_then(|s| s.as_str())
                        .unwrap_or(stop_reasons::END_TURN)
                        .to_string();

                    // Send token usage from the final response
                    if let Some(usage) = result.and_then(|r| r.get("usage")) {
                        let input = usage
                            .get(acp_fields::INPUT_TOKENS)
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let output = usage
                            .get(acp_fields::OUTPUT_TOKENS)
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        send_payload(
                            &tx,
                            AgentUsagePayload {
                                input_tokens: input,
                                output_tokens: output,
                            },
                        )
                        .await;
                    }

                    send_payload(
                        &tx,
                        AgentDonePayload {
                            stop_reason: stop_reason.clone(),
                            session_id: self.session_id.clone(),
                        },
                    )
                    .await;
                    return Ok(());
                }

                // Has `method` + `id` — this is a reverse request from the agent.
                if msg.get(jsonrpc::METHOD).is_some() {
                    self.handle_reverse_request(&msg, &tx).await;
                    continue;
                }
            }

            // Notification (no `id`, has `method`)
            if let Some(method) = msg.get(jsonrpc::METHOD).and_then(|m| m.as_str()) {
                let params = msg.get(jsonrpc::PARAMS).cloned().unwrap_or(Value::Null);
                self.handle_notification(method, &params, &tx).await;
            }
        }
    }

    /// Graceful shutdown: close stdin, wait briefly, then SIGTERM/SIGKILL.
    pub async fn close(mut self) {
        // Drop stdin to signal EOF.
        drop(self.stdin);

        // Wait up to 3 seconds for the process to exit.
        let timeout = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        match timeout {
            Ok(Ok(status)) => {
                debug!(status = %status, "agent process exited");
            }
            _ => {
                warn!("agent process did not exit in time, sending kill");
                let _ = self.child.kill().await;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send_message(&mut self, msg: &Value) -> Result<(), AgentError> {
        let mut line = serde_json::to_string(msg)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;
        Ok(())
    }

    /// Read lines until we get a JSON-RPC response matching `expected_id`.
    /// Any notifications encountered along the way are silently discarded
    /// (used only during handshake).
    async fn read_response(&mut self, expected_id: u64) -> Result<Value, AgentError> {
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self.reader.read_line(&mut buf).await?;
            if n == 0 {
                return Err(AgentError("agent closed stdout during handshake".into()));
            }
            let trimmed = buf.trim();
            if trimmed.is_empty() {
                continue;
            }
            let msg: Value = serde_json::from_str(trimmed)?;
            if let Some(id) = msg.get(jsonrpc::ID) {
                if id.as_u64() == Some(expected_id) {
                    if let Some(err) = msg.get(jsonrpc::ERROR) {
                        return Err(AgentError(format!("agent error: {err}")));
                    }
                    return Ok(msg);
                }
            }
            // Not our response — skip (handshake notification or unrelated).
        }
    }

    /// Handle a notification from the agent (streaming content, usage, etc.).
    ///
    /// Wire format (ACP v0.24+):
    /// ```json
    /// { "params": { "update": { "sessionUpdate": "<type>", ... } } }
    /// ```
    async fn handle_notification(
        &self,
        method: &str,
        params: &Value,
        tx: &mpsc::Sender<Arc<dyn EventPayload>>,
    ) {
        if method != acp_methods::SESSION_UPDATE {
            debug!(method, "unhandled notification method");
            return;
        }

        let update = match params.get(update_keys::UPDATE) {
            Some(u) => u,
            None => return,
        };

        let update_type = update
            .get(update_keys::SESSION_UPDATE)
            .and_then(|t| t.as_str())
            .unwrap_or("");

        match update_type {
            update_types::AGENT_MESSAGE_CHUNK => {
                if let Some(text) = update
                    .get(update_keys::CONTENT)
                    .and_then(|c| c.get(update_keys::TEXT))
                    .and_then(|t| t.as_str())
                {
                    if !text.is_empty() {
                        send_payload(
                            tx,
                            AgentTokenPayload {
                                token: text.to_string(),
                            },
                        )
                        .await;
                    }
                }
            }
            update_types::AGENT_THOUGHT_CHUNK => {
                // Thinking tokens — skip for now (could emit as a separate event)
            }
            update_types::USAGE_UPDATE => {
                // Usage in session/update is context-level (used/size).
                // Token-level usage comes in the final response.
                // Extract what we can from the update.
                let used = update
                    .get(update_keys::USED)
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if used > 0 {
                    send_payload(
                        tx,
                        AgentUsagePayload {
                            input_tokens: used,
                            output_tokens: 0,
                        },
                    )
                    .await;
                }
            }
            update_types::TOOL_USE => {
                let id = update
                    .get(jsonrpc::ID)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = update
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let args = update.get("arguments").cloned().unwrap_or(Value::Null);
                send_payload(
                    tx,
                    AgentToolCallPayload {
                        id,
                        name,
                        arguments: args,
                    },
                )
                .await;
            }
            update_types::TOOL_RESULT => {
                let id = update
                    .get(jsonrpc::ID)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let success = update
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let output = update
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                send_payload(
                    tx,
                    AgentToolResultPayload {
                        id,
                        success,
                        output,
                    },
                )
                .await;
            }
            update_types::AVAILABLE_COMMANDS => {
                // Commands list — informational, skip
            }
            _ => {
                debug!(update_type, "unhandled session/update type");
            }
        }
    }

    /// Handle a reverse request from the agent (file I/O, terminal, permissions).
    async fn handle_reverse_request(
        &mut self,
        msg: &Value,
        tx: &mpsc::Sender<Arc<dyn EventPayload>>,
    ) {
        use apxm_core::constants::acp::reverse_params;
        use apxm_core::constants::acp::reverse_response;

        let request_id = msg.get(jsonrpc::ID).cloned().unwrap_or(Value::Null);
        let method = msg
            .get(jsonrpc::METHOD)
            .and_then(|m| m.as_str())
            .unwrap_or("");
        let params = msg.get(jsonrpc::PARAMS).cloned().unwrap_or(Value::Null);

        let result = match method {
            acp_methods::FS_READ_TEXT_FILE => {
                let path_str = params
                    .get(reverse_params::PATH)
                    .and_then(|p| p.as_str())
                    .unwrap_or("");
                match tokio::fs::read_to_string(path_str).await {
                    Ok(content) => serde_json::json!({ reverse_response::CONTENT: content }),
                    Err(e) => {
                        let _ = self
                            .send_error_response(&request_id, &format!("read failed: {e}"))
                            .await;
                        return;
                    }
                }
            }
            acp_methods::FS_WRITE_TEXT_FILE => {
                let path_str = params
                    .get(reverse_params::PATH)
                    .and_then(|p| p.as_str())
                    .unwrap_or("");
                let content = params
                    .get(reverse_params::CONTENT)
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                match tokio::fs::write(path_str, content).await {
                    Ok(()) => serde_json::json!({ "success": true }),
                    Err(e) => {
                        let _ = self
                            .send_error_response(&request_id, &format!("write failed: {e}"))
                            .await;
                        return;
                    }
                }
            }
            acp_methods::TERMINAL_CREATE => {
                let command = params
                    .get(reverse_params::COMMAND)
                    .and_then(|c| c.as_str())
                    .unwrap_or("echo ok");
                let output = tokio::process::Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .output()
                    .await;
                match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                        serde_json::json!({
                            acp_fields::TERMINAL_ID: "term-1",
                            "stdout": stdout,
                            "stderr": stderr,
                            acp_fields::EXIT_CODE: out.status.code().unwrap_or(-1)
                        })
                    }
                    Err(e) => {
                        let _ = self
                            .send_error_response(
                                &request_id,
                                &format!("terminal create failed: {e}"),
                            )
                            .await;
                        return;
                    }
                }
            }
            acp_methods::TERMINAL_OUTPUT => {
                // Simplified: return empty output since we run commands
                // synchronously via terminal/create.
                serde_json::json!({ reverse_response::OUTPUT: "" })
            }
            acp_methods::REQUEST_PERMISSION => {
                // Auto-approve read operations; approve writes in GUI context.
                send_payload(
                    tx,
                    AgentToolCallPayload {
                        id: request_id.to_string(),
                        name: acp_methods::REQUEST_PERMISSION.to_string(),
                        arguments: params.clone(),
                    },
                )
                .await;
                serde_json::json!({ "granted": true })
            }
            _ => {
                warn!(method, "unknown reverse request method");
                let _ = self
                    .send_error_response(&request_id, &format!("unsupported method: {method}"))
                    .await;
                return;
            }
        };

        // Send success response.
        let resp = serde_json::json!({
            jsonrpc::JSONRPC: jsonrpc::VERSION,
            jsonrpc::ID: request_id,
            jsonrpc::RESULT: result
        });
        if let Err(e) = self.send_message(&resp).await {
            error!(error = %e, "failed to send reverse request response");
        }
    }

    /// Send a JSON-RPC error response back to the agent.
    async fn send_error_response(&mut self, id: &Value, message: &str) -> Result<(), AgentError> {
        let resp = serde_json::json!({
            jsonrpc::JSONRPC: jsonrpc::VERSION,
            jsonrpc::ID: id,
            jsonrpc::ERROR: {
                "code": jsonrpc::error_codes::INTERNAL_ERROR,
                "message": message
            }
        });
        self.send_message(&resp).await
    }
}

async fn send_payload<P>(tx: &mpsc::Sender<Arc<dyn EventPayload>>, payload: P)
where
    P: EventPayload,
{
    let _ = tx.send(Arc::new(payload) as Arc<dyn EventPayload>).await;
}
