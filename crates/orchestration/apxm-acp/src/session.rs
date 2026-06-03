use std::process::Stdio;

use tokio::process::{Child, Command};
use tokio::time::Duration;

use apxm_core::apxm_acp;

use crate::AcpError;
use crate::auth;
use crate::constants::{
    args as cap_args, client, client_capabilities as client_caps, fields, methods, protocol,
    stop_reasons, timeouts, wire,
};
use crate::content::ContentBlock;
use crate::protocol::StdioTransport;
use crate::registry::AcpAgentProfile;
use crate::reverse::{CapabilityReverseHandler, ReverseHandler};

/// Result of a prompt round-trip.
#[derive(Debug, Clone)]
pub struct PromptResult {
    pub text: String,
    pub model: Option<String>,
    pub stop_reason: String,
    pub token_usage: Option<TokenUsage>,
}

/// Token usage from a prompt response.
#[derive(Debug, Clone)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// A live ACP session with a spawned agent process.
pub struct AcpSession {
    session_id: String,
    agent_session_id: Option<String>,
    profile_name: String,
    transport: Option<StdioTransport>,
    child: Child,
    close_grace_ms: u64,
    turn_count: u32,
    closed: bool,
}

impl AcpSession {
    /// Spawn an agent, initialize the ACP protocol, and create a session.
    ///
    /// `aam_context` is the projected AAM state to inject as a system preamble.
    pub async fn spawn(
        profile_name: &str,
        profile: &AcpAgentProfile,
        cwd: &std::path::Path,
        aam_context: &apxm_core::types::aam::AamContext,
        sandbox: Option<std::sync::Arc<dyn apxm_runtime::sandbox::SandboxBackend>>,
    ) -> Result<Self, AcpError> {
        let parts = shell_words::split(&profile.command).map_err(|e| AcpError::Spawn {
            agent: profile_name.to_string(),
            reason: format!("bad command: {e}"),
        })?;
        let (program, args) = parts.split_first().ok_or_else(|| AcpError::Spawn {
            agent: profile_name.to_string(),
            reason: "empty command".to_string(),
        })?;

        // Confine the long-running agent when the driver supplied a capable
        // backend. Coding agents reach the model gateway, so network stays on.
        let (program, args) = match &sandbox {
            Some(backend) => backend.wrap_command(program, args, cwd, true),
            None => (program.clone(), args.to_vec()),
        };

        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(cwd);
        for (k, v) in &profile.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| AcpError::Spawn {
            agent: profile_name.to_string(),
            reason: e.to_string(),
        })?;

        let stdin = child.stdin.take().ok_or_else(|| AcpError::Spawn {
            agent: profile_name.to_string(),
            reason: "no stdin".to_string(),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| AcpError::Spawn {
            agent: profile_name.to_string(),
            reason: "no stdout".to_string(),
        })?;

        let mut transport = StdioTransport::new(stdin, stdout);
        let session_id = uuid::Uuid::new_v4().to_string();
        let timeout = Duration::from_millis(profile.session_create_timeout_ms);

        // Step 1: protocol handshake
        let init_params = serde_json::json!({
            fields::PROTOCOL_VERSION: protocol::ACP_PROTOCOL_VERSION,
            fields::CLIENT_CAPABILITIES: {
                client_caps::FS: {client_caps::READ_TEXT_FILE: true, client_caps::WRITE_TEXT_FILE: true},
                client_caps::TERMINAL: true,
            },
            fields::CLIENT_INFO: {
                client_caps::NAME: client::CLIENT_NAME,
                client_caps::VERSION: client::CLIENT_VERSION,
            },
        });

        let no_op = NoOpReverseHandler;
        let init_id = transport
            .send_request(methods::INITIALIZE, Some(init_params))
            .await?;
        let init_result = tokio::time::timeout(timeout, transport.read_response(init_id, &no_op))
            .await
            .map_err(|_| AcpError::Timeout(format!("{} timed out", methods::INITIALIZE)))??;

        // Step 2: optional auth
        if let Some(auth_methods) = init_result
            .get(fields::AUTH_METHODS)
            .and_then(|v| v.as_array())
        {
            for method in auth_methods {
                if let Some(method_id) = method.get(fields::METHOD_ID).and_then(|v| v.as_str()) {
                    if let Some(credential) = auth::resolve_auth_credential(method_id) {
                        let auth_params = serde_json::json!({
                            fields::METHOD_ID: method_id,
                            fields::CREDENTIAL: credential,
                        });
                        let auth_id = transport
                            .send_request(methods::AUTHENTICATE, Some(auth_params))
                            .await?;
                        let _ = tokio::time::timeout(
                            Duration::from_secs(timeouts::AUTH_TIMEOUT_SECS),
                            transport.read_response(auth_id, &no_op),
                        )
                        .await
                        .map_err(|_| {
                            AcpError::Timeout(format!("{} timed out", methods::AUTHENTICATE))
                        })??;
                        break;
                    }
                }
            }
        }

        // Step 3: open session
        let new_params = crate::aam_bridge::render_session_params(cwd, &profile.capabilities);
        let new_id = transport
            .send_request(methods::SESSION_NEW, Some(new_params))
            .await?;
        let new_result = tokio::time::timeout(timeout, transport.read_response(new_id, &no_op))
            .await
            .map_err(|_| AcpError::Timeout(format!("{} timed out", methods::SESSION_NEW)))??;

        let agent_session_id = new_result
            .get(fields::SESSION_ID)
            .and_then(|v| v.as_str())
            .map(String::from);

        apxm_acp!(info,
            agent = profile_name,
            session_id = %session_id,
            agent_session_id = ?agent_session_id,
            "session established"
        );

        let mut session = Self {
            session_id,
            agent_session_id,
            profile_name: profile_name.to_string(),
            transport: Some(transport),
            child,
            close_grace_ms: profile.close_grace_ms,
            turn_count: 0,
            closed: false,
        };

        // Inject AAM context as a system preamble (turn 0, doesn't increment turn_count)
        // Skip preamble for agents that read context from their cwd (e.g. AGENTS.md).
        if !profile.skip_preamble {
            if let Some(preamble) = crate::aam_bridge::render_system_prompt(aam_context) {
                let system_text = match &profile.system_prompt {
                    Some(sp) => format!("{sp}\n\n{preamble}"),
                    None => preamble,
                };
                session.send_system_preamble(&system_text).await?;
            } else if let Some(sp) = &profile.system_prompt {
                session.send_system_preamble(sp).await?;
            }
        }

        Ok(session)
    }

    fn transport(&mut self) -> Result<&mut StdioTransport, AcpError> {
        self.transport.as_mut().ok_or(AcpError::SessionClosed)
    }

    /// Send a system preamble as an inaugural prompt (turn 0).
    ///
    /// This sets context for the agent without incrementing `turn_count`.
    /// The response is discarded — this is a context-setting turn, not a user turn.
    async fn send_system_preamble(&mut self, text: &str) -> Result<(), AcpError> {
        let blocks = vec![ContentBlock::text(text)];
        let params = serde_json::json!({
            fields::SESSION_ID: self.agent_session_id,
            cap_args::PROMPT: blocks,
        });

        let no_op = NoOpReverseHandler;
        let id = self
            .transport()?
            .send_request(methods::SESSION_PROMPT, Some(params))
            .await?;
        let timeout = Duration::from_secs(timeouts::PREAMBLE_TIMEOUT_SECS);
        let _ = tokio::time::timeout(timeout, self.transport()?.read_response(id, &no_op))
            .await
            .map_err(|_| AcpError::Timeout("system preamble timed out".to_string()))??;

        apxm_acp!(info,
            agent = %self.profile_name,
            preamble_len = text.len(),
            "system preamble injected"
        );
        Ok(())
    }

    /// Send a prompt to the agent and collect the response.
    pub async fn prompt(
        &mut self,
        text: &str,
        handler: &CapabilityReverseHandler,
    ) -> Result<PromptResult, AcpError> {
        self.turn_count += 1;
        let blocks = vec![ContentBlock::text(text)];

        let params = serde_json::json!({
            fields::SESSION_ID: self.agent_session_id,
            cap_args::PROMPT: blocks,
        });

        let id = self
            .transport()?
            .send_request(methods::SESSION_PROMPT, Some(params))
            .await?;

        // Read response — reverse requests, notifications, and final response
        // are all handled by the transport + handler.
        let timeout = Duration::from_secs(timeouts::PROMPT_TIMEOUT_SECS);
        let result = tokio::time::timeout(timeout, self.transport()?.read_response(id, handler))
            .await
            .map_err(|_| AcpError::Timeout("prompt turn timed out".to_string()))??;

        // Collect accumulated text from streaming notifications
        let text = handler.take_response_text();
        let (input_tokens, output_tokens) = handler.take_token_usage();

        let stop_reason = result
            .get(fields::STOP_REASON)
            .and_then(|v| v.as_str())
            .unwrap_or(stop_reasons::END_TURN)
            .to_string();
        let model = result
            .get(wire::MODEL)
            .and_then(|v| v.as_str())
            .map(String::from);

        let token_usage = if input_tokens.is_some() || output_tokens.is_some() {
            Some(TokenUsage {
                input_tokens,
                output_tokens,
            })
        } else {
            None
        };

        Ok(PromptResult {
            text,
            model,
            stop_reason,
            token_usage,
        })
    }

    /// Send a request and read the response directly (no reverse handler).
    /// Used for session control commands (set_mode, set_model, cancel).
    pub(crate) async fn send_request_no_reverse(
        &mut self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, AcpError> {
        let no_op = NoOpReverseHandler;
        let id = self.transport()?.send_request(method, params).await?;
        let result = tokio::time::timeout(
            Duration::from_secs(timeouts::CONTROL_TIMEOUT_SECS),
            self.transport()?.read_response(id, &no_op),
        )
        .await
        .map_err(|_| AcpError::Timeout(format!("{method} timed out")))??;
        Ok(result)
    }

    /// Gracefully close the session.
    pub async fn close(mut self) {
        self.closed = true;

        // Drop transport (closes stdin pipe → signals EOF to agent)
        self.transport.take();

        tokio::time::sleep(Duration::from_millis(self.close_grace_ms)).await;

        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }

        // SIGTERM
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            send_sigterm(pid);
        }
        #[cfg(not(unix))]
        {
            let _ = self.child.start_kill();
        }

        tokio::time::sleep(Duration::from_millis(timeouts::SIGTERM_GRACE_MS)).await;

        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }

        // SIGKILL
        let _ = self.child.kill().await;
        tokio::time::sleep(Duration::from_millis(timeouts::SIGKILL_GRACE_MS)).await;
        let _ = self.child.wait().await;
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn agent_session_id(&self) -> Option<&str> {
        self.agent_session_id.as_deref()
    }

    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }

    pub fn turn_count(&self) -> u32 {
        self.turn_count
    }
}

impl Drop for AcpSession {
    fn drop(&mut self) {
        // Skip if close() was already called
        if self.closed {
            return;
        }
        // Best-effort SIGKILL to prevent zombie processes
        #[cfg(unix)]
        if let Some(pid) = self.child.id() {
            // SAFETY: libc::kill is safe to call with a valid pid.
            #[allow(unsafe_code)]
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL)
            };
        }
        #[cfg(not(unix))]
        if let Some(_pid) = self.child.id() {
            let _ = self.child.start_kill();
        }
    }
}

/// Send SIGTERM to a process via libc (Unix only).
#[cfg(unix)]
#[allow(unsafe_code)]
fn send_sigterm(pid: u32) {
    // SAFETY: libc::kill is safe to call with a valid pid.
    unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
}

/// No-op reverse handler used during initialize/authenticate/session-new.
struct NoOpReverseHandler;

#[async_trait::async_trait]
impl ReverseHandler for NoOpReverseHandler {
    async fn handle(
        &self,
        method: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        apxm_acp!(
            warn,
            method = method,
            "reverse request during init phase — rejecting"
        );
        Err(AcpError::Protocol(format!(
            "reverse request {method} not supported during initialization"
        )))
    }
}
