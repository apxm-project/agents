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
    /// Override working directory (uses temp dir if None).
    pub working_dir: Option<PathBuf>,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_output_bytes: 1024 * 1024, // 1 MB
            allowed_commands: None,
            blocked_env_vars: vec![
                "AWS_SECRET_ACCESS_KEY".into(),
                "AWS_ACCESS_KEY_ID".into(),
                "OPENAI_API_KEY".into(),
                "ANTHROPIC_API_KEY".into(),
                "DATABASE_URL".into(),
                "SECRET_KEY".into(),
                "PRIVATE_KEY".into(),
            ],
            working_dir: None,
        }
    }
}
