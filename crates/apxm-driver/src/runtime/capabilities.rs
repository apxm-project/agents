//! Capability registry configuration for the runtime.

use crate::{config::ApXmConfig, error::DriverError};
use apxm_core::types::AISOperationType;
use apxm_runtime::CapabilitySystem;
use apxm_runtime::sandbox::{policy::SandboxPolicy, process::ProcessSandbox};

pub fn configure_capability_registry(
    capability_system: std::sync::Arc<CapabilitySystem>,
    config: &ApXmConfig,
) -> Result<(), DriverError> {
    let tools_config = config.tools_config();
    apxm_tools::register_standard_tools(&capability_system, &tools_config)
        .map_err(DriverError::Runtime)?;

    if let Err(e) = register_user_tools(&capability_system) {
        tracing::warn!("Failed to load user tools from ~/.apxm/tools.json: {}", e);
    }

    // NOTE: ACP agents are invoked via SPAWN_AGENT + COMMUNICATE, not the legacy tool-invocation path.
    // The ACP tool entry is intentionally not registered.

    Ok(())
}

fn register_user_tools(capability_system: &CapabilitySystem) -> Result<(), DriverError> {
    let home = dirs::home_dir()
        .ok_or_else(|| DriverError::Driver("Could not determine home directory".to_string()))?;
    let tools_path = home.join(".apxm").join("tools.json");
    if !tools_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&tools_path).map_err(|e| {
        DriverError::Driver(format!("Failed to read {}: {}", tools_path.display(), e))
    })?;

    let tools: Vec<UserToolEntry> = serde_json::from_str(&content).map_err(|e| {
        DriverError::Driver(format!("Failed to parse {}: {}", tools_path.display(), e))
    })?;

    for tool in tools {
        if capability_system.has_capability(&tool.name) {
            continue;
        }
        let metadata = apxm_runtime::capability::metadata::CapabilityMetadata::new(
            &tool.name,
            &tool.description,
            serde_json::json!({}),
        );
        let cap = UserToolCapability {
            metadata,
            command: tool.command,
            args: tool.args,
            timeout_ms: tool.timeout_ms,
        };
        capability_system
            .register(std::sync::Arc::new(cap))
            .map_err(DriverError::Runtime)?;
    }

    Ok(())
}

fn default_timeout_ms() -> u64 {
    30_000
}

#[derive(serde::Deserialize)]
struct UserToolEntry {
    name: String,
    description: String,
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
}

struct UserToolCapability {
    metadata: apxm_runtime::capability::metadata::CapabilityMetadata,
    command: String,
    args: Vec<String>,
    timeout_ms: u64,
}

impl UserToolCapability {
    fn cap_err(&self, message: String) -> apxm_core::error::RuntimeError {
        apxm_core::error::RuntimeError::Capability {
            capability: self.metadata.name.clone(),
            message,
        }
    }
}

#[async_trait::async_trait]
impl apxm_runtime::capability::executor::CapabilityExecutor for UserToolCapability {
    async fn execute(
        &self,
        args: std::collections::HashMap<String, apxm_core::types::values::Value>,
    ) -> apxm_runtime::capability::executor::CapabilityResult<apxm_core::types::values::Value> {
        use apxm_core::types::values::Value;

        let json_input = serde_json::to_string(&args)
            .map_err(|e| self.cap_err(format!("Failed to serialize arguments: {e}")))?;

        let policy = SandboxPolicy {
            timeout: std::time::Duration::from_millis(self.timeout_ms),
            ..SandboxPolicy::default()
        };
        let sandbox = ProcessSandbox::new(policy);

        let args_refs: Vec<&str> = self.args.iter().map(|s| s.as_str()).collect();
        let result = sandbox
            .execute(&self.command, &args_refs, Some(&json_input))
            .await
            .map_err(|e| self.cap_err(format!("Failed to execute subprocess: {e}")))?;

        if result.timed_out {
            return Err(self.cap_err(format!("Tool timed out after {}ms", self.timeout_ms)));
        }

        if result.exit_code != 0 {
            return Err(self.cap_err(format!(
                "Tool exited with code {}: {}",
                result.exit_code,
                result.stderr.trim()
            )));
        }

        // Try to parse stdout as JSON, fall back to plain string
        match serde_json::from_str::<serde_json::Value>(&result.stdout) {
            Ok(json_val) => Value::try_from(json_val)
                .map_err(|e| self.cap_err(format!("Failed to convert JSON output to Value: {e}"))),
            Err(_) => Ok(Value::String(result.stdout)),
        }
    }

    fn metadata(&self) -> &apxm_runtime::capability::metadata::CapabilityMetadata {
        &self.metadata
    }

    fn to_exec_request(
        &self,
        args: &std::collections::HashMap<String, apxm_core::types::values::Value>,
    ) -> Option<apxm_sandbox::ExecRequest> {
        let json_input = serde_json::to_string(&args).ok()?;

        Some(apxm_sandbox::ExecRequest {
            min_isolation: apxm_sandbox::IsolationLevel::OsLevel,
            program: self.command.clone(),
            args: self.args.clone(),
            stdin_data: Some(json_input),
            timeout: std::time::Duration::from_millis(self.timeout_ms),
            needs_network: true,
            origin_op: Some(AISOperationType::InvTool.to_string()),
            ..apxm_sandbox::ExecRequest::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::values::Value;
    use apxm_runtime::capability::executor::CapabilityExecutor;
    use std::collections::HashMap;

    fn make_cap(command: &str, args: &[&str], timeout_ms: u64) -> UserToolCapability {
        let metadata = apxm_runtime::capability::metadata::CapabilityMetadata::new(
            "test-tool",
            "test tool",
            serde_json::json!({}),
        );
        UserToolCapability {
            metadata,
            command: command.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            timeout_ms,
        }
    }

    #[tokio::test]
    async fn test_user_tool_echoes_stdin() {
        let cap = make_cap("cat", &[], 5_000);
        let mut args = HashMap::new();
        args.insert("key".to_string(), Value::String("hello".to_string()));

        let result = cap.execute(args).await.unwrap();
        // cat echoes stdin back; JSON is {"key":"hello"}
        match result {
            Value::Object(map) => {
                assert_eq!(map.get("key"), Some(&Value::String("hello".to_string())));
            }
            _ => panic!("Expected JSON object output, got: {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_user_tool_timeout() {
        let cap = make_cap("sleep", &["10"], 500);
        let result = cap.execute(HashMap::new()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("timed out"),
            "Expected timeout error, got: {msg}"
        );
    }

    #[tokio::test]
    async fn test_user_tool_nonzero_exit() {
        let cap = make_cap("bash", &["-c", "echo oops >&2; exit 42"], 5_000);
        let result = cap.execute(HashMap::new()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("42"),
            "Expected exit code 42 in error, got: {msg}"
        );
        assert!(msg.contains("oops"), "Expected stderr in error, got: {msg}");
    }

    #[tokio::test]
    async fn test_user_tool_plain_text_output() {
        // echo outputs non-JSON text, should return as Value::String
        let cap = make_cap("echo", &["plain text"], 5_000);
        let result = cap.execute(HashMap::new()).await.unwrap();
        match result {
            Value::String(s) => assert!(s.contains("plain text")),
            _ => panic!("Expected string output, got: {:?}", result),
        }
    }

    #[test]
    fn test_user_tool_entry_deserialization() {
        let json = r#"[
            {
                "name": "my-tool",
                "description": "does stuff",
                "command": "/usr/bin/my-tool"
            },
            {
                "name": "other",
                "description": "other tool",
                "command": "other-cmd",
                "args": ["--flag"],
                "timeout_ms": 5000
            }
        ]"#;
        let tools: Vec<UserToolEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "my-tool");
        assert_eq!(tools[0].timeout_ms, 30_000); // default
        assert!(tools[0].args.is_empty());
        assert_eq!(tools[1].args, vec!["--flag"]);
        assert_eq!(tools[1].timeout_ms, 5_000);
    }
}
