//! BashCapability — INV(capability="bash") implementation

use super::{CapabilityExecutor, CapabilityResult};
use crate::capability::metadata::CapabilityMetadata;
use apxm_sandbox::ExecRequest;
use crate::sandbox::policy::SandboxPolicy;
use crate::sandbox::process::ProcessSandbox;
use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 3600;

/// Execute bash scripts via INV(capability="bash").
///
/// Args:
/// - `code` (string, required): bash script body
/// - `timeout_secs` (integer, optional, default 30): max runtime seconds
/// - `cwd` (string, optional): working directory (~ is expanded)
pub struct BashCapability {
    metadata: CapabilityMetadata,
}

impl BashCapability {
    pub fn new() -> Self {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "code":        { "type": "string",  "description": "Bash script to execute" },
                "timeout_secs":{ "type": "integer", "description": "Timeout in seconds (default 30, max 3600)", "minimum": 1, "maximum": 3600 },
                "cwd":         { "type": "string",  "description": "Working directory (optional, ~ expanded)" }
            },
            "required": ["code"]
        });
        Self {
            metadata: CapabilityMetadata::new(
                "bash",
                "Execute a bash script; returns stdout. Appends stderr and exit code on failure.",
                schema,
            )
            .with_returns("string")
            .with_latency(100),
        }
    }
}

impl Default for BashCapability {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl CapabilityExecutor for BashCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let code = args
            .get("code")
            .and_then(|v| v.as_string())
            .ok_or_else(|| RuntimeError::Capability {
                capability: "bash".to_string(),
                message: "Missing required argument 'code'".to_string(),
            })?
            .to_string();

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| if let Value::Number(n) = v { n.as_i64().map(|i| i.max(1) as u64) } else { None })
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);

        let working_dir = args.get("cwd").and_then(|v| v.as_string()).map(|s| {
            if s.starts_with("~/") {
                std::env::var("HOME")
                    .map(|h| PathBuf::from(format!("{}/{}", h, &s[2..])))
                    .unwrap_or_else(|_| PathBuf::from(s))
            } else {
                PathBuf::from(s)
            }
        });

        let policy = SandboxPolicy {
            timeout: Duration::from_secs(timeout_secs),
            working_dir,
            ..SandboxPolicy::default()
        };

        let result = ProcessSandbox::new(policy)
            .execute_script("bash", &code)
            .await
            .map_err(|e| RuntimeError::Capability {
                capability: "bash".to_string(),
                message: format!("Script spawn failed: {e}"),
            })?;

        if result.timed_out {
            return Ok(Value::String(format!(
                "[TIMEOUT after {timeout_secs}s]\nstdout:\n{}\nstderr:\n{}",
                result.stdout, result.stderr
            )));
        }

        let mut out = result.stdout.clone();
        if !result.stderr.is_empty() {
            if !out.is_empty() { out.push('\n'); }
            out.push_str("[stderr]\n");
            out.push_str(&result.stderr);
        }
        if result.exit_code != 0 {
            out.push_str(&format!("\n[exit code: {}]", result.exit_code));
        }
        Ok(Value::String(out))
    }

    fn metadata(&self) -> &CapabilityMetadata { &self.metadata }

    fn to_exec_request(&self, args: &HashMap<String, Value>) -> Option<ExecRequest> {
        let code = args.get("code").and_then(|v| v.as_string())?.to_string();

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| if let Value::Number(n) = v { n.as_i64().map(|i| i.max(1) as u64) } else { None })
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .min(MAX_TIMEOUT_SECS);

        let working_dir = args.get("cwd").and_then(|v| v.as_string()).map(|s| {
            if s.starts_with("~/") {
                std::env::var("HOME")
                    .map(|h| PathBuf::from(format!("{}/{}", h, &s[2..])))
                    .unwrap_or_else(|_| PathBuf::from(s))
            } else {
                PathBuf::from(s)
            }
        });

        Some(ExecRequest {
            min_isolation: apxm_sandbox::IsolationLevel::PolicyOnly,
            program: "bash".to_string(),
            args: vec!["-c".to_string(), code],
            working_dir,
            env: Default::default(),
            stdin_data: None,
            timeout: std::time::Duration::from_secs(timeout_secs),
            max_output_bytes: 1024 * 1024, // 1MB
            read_paths: vec![],
            write_paths: vec![],
            needs_network: false,
            needs_process_spawn: false,
            origin_op: Some("bash".to_string()),
            origin_node_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::values::Number;

    #[tokio::test]
    async fn bash_echo_works() {
        let cap = BashCapability::new();
        let mut a = HashMap::new();
        a.insert("code".to_string(), Value::String("echo hello_apxm".to_string()));
        let out = cap.execute(a).await.unwrap();
        assert!(out.as_string().unwrap().contains("hello_apxm"));
    }

    #[tokio::test]
    async fn bash_nonzero_exit_in_output() {
        let cap = BashCapability::new();
        let mut a = HashMap::new();
        a.insert("code".to_string(), Value::String("exit 42".to_string()));
        let out = cap.execute(a).await.unwrap();
        assert!(out.as_string().unwrap().contains("exit code: 42"));
    }

    #[tokio::test]
    async fn bash_stderr_captured() {
        let cap = BashCapability::new();
        let mut a = HashMap::new();
        a.insert("code".to_string(), Value::String("echo STDOUT && echo STDERR >&2".to_string()));
        let out = cap.execute(a).await.unwrap().as_string().unwrap().to_string();
        assert!(out.contains("STDOUT"), "missing stdout: {out}");
        assert!(out.contains("STDERR"), "missing stderr: {out}");
    }

    #[tokio::test]
    async fn bash_timeout_enforced() {
        let cap = BashCapability::new();
        let mut a = HashMap::new();
        a.insert("code".to_string(), Value::String("sleep 10".to_string()));
        a.insert("timeout_secs".to_string(), Value::Number(Number::Integer(1)));
        let out = cap.execute(a).await.unwrap();
        assert!(out.as_string().unwrap().contains("TIMEOUT"));
    }

    #[tokio::test]
    async fn bash_missing_code_is_err() {
        assert!(BashCapability::new().execute(HashMap::new()).await.is_err());
    }
}
