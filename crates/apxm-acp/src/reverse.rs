use async_trait::async_trait;
use std::sync::Arc;

use apxm_core::apxm_acp;

use crate::AcpError;
use crate::constants::{fields, methods, option_kinds, outcomes, tool_kinds, update_types};
use crate::registry::PermissionMode;
use crate::terminal::TerminalManager;

/// Trait for handling reverse requests and notifications from an ACP agent.
#[async_trait]
pub trait ReverseHandler: Send + Sync {
    /// Handle a reverse JSON-RPC request from the agent.
    async fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AcpError>;

    /// Handle a notification (no response expected). Default: log and ignore.
    async fn on_notification(&self, _method: &str, _params: Option<&serde_json::Value>) {}
}

/// Reverse handler that delegates file I/O and terminal ops to APXM's capability system.
pub struct CapabilityReverseHandler {
    capability_system: Arc<apxm_runtime::CapabilitySystem>,
    permission_mode: PermissionMode,
    terminals: Arc<TerminalManager>,
    /// Accumulated text chunks from `session/update` notifications.
    response_text: std::sync::Mutex<String>,
    /// Token usage from `usage_update` notifications.
    token_usage: std::sync::Mutex<(Option<u64>, Option<u64>)>,
}

impl CapabilityReverseHandler {
    pub fn new(
        capability_system: Arc<apxm_runtime::CapabilitySystem>,
        permission_mode: PermissionMode,
    ) -> Self {
        Self {
            capability_system,
            permission_mode,
            terminals: Arc::new(TerminalManager::new()),
            response_text: std::sync::Mutex::new(String::new()),
            token_usage: std::sync::Mutex::new((None, None)),
        }
    }

    /// Take the accumulated response text (drains the buffer).
    pub fn take_response_text(&self) -> String {
        std::mem::take(&mut *self.response_text.lock().unwrap())
    }

    /// Take the accumulated token usage.
    pub fn take_token_usage(&self) -> (Option<u64>, Option<u64>) {
        let mut guard = self.token_usage.lock().unwrap();
        std::mem::replace(&mut *guard, (None, None))
    }

    fn check_read_permission(&self) -> Result<(), AcpError> {
        match self.permission_mode {
            PermissionMode::DenyAll => Err(AcpError::PermissionDenied(
                "file read denied by policy".to_string(),
            )),
            _ => Ok(()),
        }
    }

    fn check_write_permission(&self) -> Result<(), AcpError> {
        match self.permission_mode {
            PermissionMode::ApproveAll => Ok(()),
            _ => Err(AcpError::PermissionDenied(
                "file write denied by policy".to_string(),
            )),
        }
    }

    fn check_exec_permission(&self) -> Result<(), AcpError> {
        match self.permission_mode {
            PermissionMode::ApproveAll => Ok(()),
            _ => Err(AcpError::PermissionDenied(
                "terminal execution denied by policy".to_string(),
            )),
        }
    }

    async fn handle_read_file(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        self.check_read_permission()?;
        let path = params["path"]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("missing path".to_string()))?;

        let mut args = std::collections::HashMap::new();
        args.insert(
            "path".to_string(),
            apxm_core::types::values::Value::String(path.to_string()),
        );
        if let Some(line) = params["line"].as_u64() {
            args.insert(
                "offset".to_string(),
                apxm_core::types::values::Value::Number(apxm_core::types::values::Number::Integer(
                    line as i64,
                )),
            );
        }

        let result = self
            .capability_system
            .invoke("read", args)
            .await
            .map_err(|e| AcpError::Protocol(format!("read failed: {e}")))?;

        let content = result
            .as_string()
            .map(|s| s.to_string())
            .unwrap_or_default();
        Ok(serde_json::json!({"content": content}))
    }

    async fn handle_write_file(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        self.check_write_permission()?;
        let path = params["path"]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("missing path".to_string()))?;
        let content = params["content"]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("missing content".to_string()))?;

        let mut args = std::collections::HashMap::new();
        args.insert(
            "path".to_string(),
            apxm_core::types::values::Value::String(path.to_string()),
        );
        args.insert(
            "content".to_string(),
            apxm_core::types::values::Value::String(content.to_string()),
        );

        self.capability_system
            .invoke("write", args)
            .await
            .map_err(|e| AcpError::Protocol(format!("write failed: {e}")))?;

        Ok(serde_json::json!({}))
    }

    async fn handle_terminal_create(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        self.check_exec_permission()?;
        let command = params["command"]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("missing command".to_string()))?;
        let args: Vec<String> = params["args"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let cwd = params["cwd"].as_str();
        let env: Vec<(String, String)> = params["env"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| {
                        let name = v["name"].as_str()?;
                        let value = v["value"].as_str()?;
                        Some((name.to_string(), value.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        let terminal_id = self.terminals.create(command, &args, cwd, &env).await?;
        Ok(serde_json::json!({fields::TERMINAL_ID: terminal_id}))
    }

    async fn handle_terminal_output(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        let terminal_id = params[fields::TERMINAL_ID]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("missing terminalId".to_string()))?;
        let (output, truncated, exit_status) = self.terminals.output(terminal_id).await?;
        let mut result = serde_json::json!({"output": output, "truncated": truncated});
        if let Some(code) = exit_status {
            result[fields::EXIT_STATUS] = serde_json::json!(code);
        }
        Ok(result)
    }

    async fn handle_terminal_wait(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        let terminal_id = params[fields::TERMINAL_ID]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("missing terminalId".to_string()))?;
        let (exit_code, signal) = self.terminals.wait_for_exit(terminal_id).await?;
        let mut result = serde_json::json!({fields::EXIT_CODE: exit_code});
        if let Some(sig) = signal {
            result["signal"] = serde_json::json!(sig);
        }
        Ok(result)
    }

    async fn handle_terminal_kill(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        let terminal_id = params[fields::TERMINAL_ID]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("missing terminalId".to_string()))?;
        self.terminals.kill(terminal_id).await?;
        Ok(serde_json::json!({}))
    }

    async fn handle_terminal_release(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        let terminal_id = params[fields::TERMINAL_ID]
            .as_str()
            .ok_or_else(|| AcpError::Protocol("missing terminalId".to_string()))?;
        self.terminals.release(terminal_id);
        Ok(serde_json::json!({}))
    }

    fn handle_request_permission(
        &self,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        let options = params["options"]
            .as_array()
            .ok_or_else(|| AcpError::Protocol("missing options".to_string()))?;

        let selected = match self.permission_mode {
            PermissionMode::ApproveAll => find_option_id(options, option_kinds::ALLOW),
            PermissionMode::DenyAll => find_option_id(options, option_kinds::REJECT),
            PermissionMode::ApproveReads => {
                let tool_kind = params[fields::TOOL_CALL]["kind"].as_str().unwrap_or("");
                if tool_kind == tool_kinds::READ || tool_kind == tool_kinds::SEARCH {
                    find_option_id(options, option_kinds::ALLOW)
                } else {
                    find_option_id(options, option_kinds::REJECT)
                }
            }
        };

        match selected {
            Some(id) => Ok(serde_json::json!({
                "outcome": {"outcome": outcomes::SELECTED, fields::OPTION_ID: id}
            })),
            None => Ok(serde_json::json!({
                "outcome": {"outcome": outcomes::CANCELLED}
            })),
        }
    }
}

/// Find the first option whose "kind" starts with the given prefix and return its optionId.
fn find_option_id<'a>(options: &'a [serde_json::Value], kind_prefix: &str) -> Option<&'a str> {
    options
        .iter()
        .find(|o| {
            o["kind"]
                .as_str()
                .is_some_and(|k| k.starts_with(kind_prefix))
        })
        .and_then(|o| o[fields::OPTION_ID].as_str())
}

#[async_trait]
impl ReverseHandler for CapabilityReverseHandler {
    async fn handle(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        apxm_acp!(debug, method = method, "handling reverse request");
        match method {
            methods::FS_READ_TEXT_FILE => self.handle_read_file(&params).await,
            methods::FS_WRITE_TEXT_FILE => self.handle_write_file(&params).await,
            methods::TERMINAL_CREATE => self.handle_terminal_create(&params).await,
            methods::TERMINAL_OUTPUT => self.handle_terminal_output(&params).await,
            methods::TERMINAL_WAIT_FOR_EXIT => self.handle_terminal_wait(&params).await,
            methods::TERMINAL_KILL => self.handle_terminal_kill(&params).await,
            methods::TERMINAL_RELEASE => self.handle_terminal_release(&params).await,
            methods::REQUEST_PERMISSION => self.handle_request_permission(&params),
            _ => Err(AcpError::UnknownMethod(method.to_string())),
        }
    }

    async fn on_notification(&self, method: &str, params: Option<&serde_json::Value>) {
        if method == methods::SESSION_UPDATE {
            if let Some(params) = params {
                let update = &params["update"];
                // ACP uses "sessionUpdate" as the discriminator key (not "type")
                // and nests text under "content.text"
                let update_kind = update["sessionUpdate"].as_str()
                    .or_else(|| update["type"].as_str()); // fallback for older protocol
                match update_kind {
                    Some(t) if t == update_types::AGENT_MESSAGE_CHUNK => {
                        // Text is under content.text in the current ACP protocol
                        let text = update["content"]["text"].as_str()
                            .or_else(|| update["text"].as_str()); // fallback
                        if let Some(text) = text {
                            self.response_text.lock().unwrap().push_str(text);
                        }
                    }
                    Some(t) if t == update_types::USAGE_UPDATE => {
                        let mut usage = self.token_usage.lock().unwrap();
                        // Try standard field names first, then ACP-specific names
                        if let Some(input) = update[fields::INPUT_TOKENS].as_u64()
                            .or_else(|| update["used"].as_u64()) {
                            usage.0 = Some(input);
                        }
                        if let Some(output) = update[fields::OUTPUT_TOKENS].as_u64()
                            .or_else(|| update["size"].as_u64()) {
                            usage.1 = Some(output);
                        }
                    }
                    Some(other) => {
                        apxm_acp!(trace, update_type = other, "ignoring session/update");
                    }
                    None => {}
                }
            }
        } else {
            apxm_acp!(trace, method = method, "ignoring notification");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_approve_all_selects_allow() {
        let handler = CapabilityReverseHandler::new(
            Arc::new(apxm_runtime::CapabilitySystem::new()),
            PermissionMode::ApproveAll,
        );
        let params = serde_json::json!({
            "toolCall": {"kind": "write"},
            "options": [
                {"optionId": "reject_1", "kind": "reject_once"},
                {"optionId": "allow_1", "kind": "allow_once"},
            ]
        });
        let result = handler.handle_request_permission(&params).unwrap();
        assert_eq!(result["outcome"]["optionId"], "allow_1");
    }

    #[test]
    fn permission_deny_all_selects_reject() {
        let handler = CapabilityReverseHandler::new(
            Arc::new(apxm_runtime::CapabilitySystem::new()),
            PermissionMode::DenyAll,
        );
        let params = serde_json::json!({
            "toolCall": {"kind": "write"},
            "options": [
                {"optionId": "allow_1", "kind": "allow_once"},
                {"optionId": "reject_1", "kind": "reject_once"},
            ]
        });
        let result = handler.handle_request_permission(&params).unwrap();
        assert_eq!(result["outcome"]["optionId"], "reject_1");
    }

    #[test]
    fn permission_approve_reads_allows_read() {
        let handler = CapabilityReverseHandler::new(
            Arc::new(apxm_runtime::CapabilitySystem::new()),
            PermissionMode::ApproveReads,
        );
        let params = serde_json::json!({
            "toolCall": {"kind": "read"},
            "options": [
                {"optionId": "allow_1", "kind": "allow_once"},
                {"optionId": "reject_1", "kind": "reject_once"},
            ]
        });
        let result = handler.handle_request_permission(&params).unwrap();
        assert_eq!(result["outcome"]["optionId"], "allow_1");
    }

    #[test]
    fn permission_approve_reads_rejects_write() {
        let handler = CapabilityReverseHandler::new(
            Arc::new(apxm_runtime::CapabilitySystem::new()),
            PermissionMode::ApproveReads,
        );
        let params = serde_json::json!({
            "toolCall": {"kind": "write"},
            "options": [
                {"optionId": "allow_1", "kind": "allow_once"},
                {"optionId": "reject_1", "kind": "reject_once"},
            ]
        });
        let result = handler.handle_request_permission(&params).unwrap();
        assert_eq!(result["outcome"]["optionId"], "reject_1");
    }
}
