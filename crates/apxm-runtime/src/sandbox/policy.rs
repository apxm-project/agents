use apxm_core::constants::sandbox::env as sandbox_env;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Security policy for sandboxed code execution.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    /// Maximum execution time before killing the process.
    pub timeout: Duration,
    /// Maximum bytes to capture from stdout+stderr.
    pub max_output_bytes: usize,
    /// If set, only these commands/interpreters are allowed.
    pub allowed_commands: Option<Vec<String>>,
    /// Environment variables to remove before execution.
    pub blocked_env_vars: Vec<String>,
    /// Additional environment variables to inject after the safe baseline.
    pub env_overrides: HashMap<String, String>,
    /// Override working directory (uses temp dir if None).
    pub working_dir: Option<PathBuf>,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_output_bytes: 1024 * 1024, // 1 MB
            allowed_commands: None,
            blocked_env_vars: sandbox_env::BLOCKED_DEFAULTS
                .iter()
                .map(|key| (*key).to_string())
                .collect(),
            env_overrides: HashMap::new(),
            working_dir: None,
        }
    }
}
