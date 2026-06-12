//! Workflow-authoring built-ins: `compose_workflow` and `run_workflow`.
//!
//! These let the conversational agent CREATE and RUN workflows first-class
//! (Goal 1) without the operator shelling commands. They are write-class (not
//! read-only) and live in the `authoring` group, which the server's ASK-exposure
//! check admits as a deliberate, admit-gated, confined exception (workflow-scoped
//! admission): the agent may *propose* them, but *executing* them still requires
//! an explicit capability grant (the write boundary), and `compose_workflow` only
//! ever writes under a confined staging directory.

use super::require_string_arg;
use crate::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::CapabilityMetadata,
};
use apxm_core::{error::RuntimeError, types::Value};
use async_trait::async_trait;
use reqwest::Client;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

const AUTHORING_GROUP: &str = "authoring";

/// The confined staging directory for authored workflows: the `workflows/`
/// subdirectory of the apxm state home (where other durable agent state lives).
fn staging_dir() -> PathBuf {
    apxm_core::env::state_home().join("workflows")
}

/// Validate a workflow name to a single safe filename and return `<name>.air`.
/// Rejects path separators and traversal so authored files can never escape the
/// staging directory.
fn safe_workflow_filename(cap: &str, name: &str) -> CapabilityResult<String> {
    let trimmed = name.trim();
    let stem = trimmed.strip_suffix(".air").unwrap_or(trimmed);
    let valid = !stem.is_empty()
        && stem != "."
        && stem != ".."
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !stem.contains("..");
    if !valid {
        return Err(RuntimeError::Capability {
            capability: cap.to_string(),
            message: format!(
                "invalid workflow name '{name}': use only letters, digits, '.', '_', '-'"
            ),
        });
    }
    Ok(format!("{stem}.air"))
}

/// Default apxm-server base for `run_workflow`'s internal compile call. Operator
/// configured via `$APXM_SERVER_BASE`; never taken from model args (so this is
/// not an SSRF surface).
fn server_base() -> String {
    std::env::var("APXM_SERVER_BASE")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:18800".to_string())
}

fn shared_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default()
    })
}

/// `compose_workflow` — validate AIR text and stage it as a runnable workflow
/// file under the confined staging directory. Returns the staged path.
pub struct ComposeWorkflowCapability {
    metadata: CapabilityMetadata,
}

impl ComposeWorkflowCapability {
    pub fn new() -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                "compose_workflow",
                "Create a workflow: validate APXM AIR text and save it as a named, \
                 runnable workflow file. Returns the staged path. Pair with \
                 run_workflow to execute it.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "workflow name (filename-safe)" },
                        "air": { "type": "string", "description": "canonical APXM AIR module text" }
                    },
                    "required": ["name", "air"]
                }),
            )
            .with_returns("string (staged workflow path)")
            .with_groups(vec![AUTHORING_GROUP.to_string(), "workflow".to_string()]),
        }
    }
}

impl Default for ComposeWorkflowCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for ComposeWorkflowCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let cap = "compose_workflow";
        let name = require_string_arg(&args, "name", cap)?;
        let air = require_string_arg(&args, "air", cap)?;

        // Light structural validation; full validation happens at run time when
        // the server compiles the AIR.
        let looks_like_air = air.contains("module") && air.contains("func.func");
        if !looks_like_air {
            return Err(RuntimeError::Capability {
                capability: cap.to_string(),
                message:
                    "air does not look like an APXM module (expected `module { func.func … }`)"
                        .to_string(),
            });
        }

        let filename = safe_workflow_filename(cap, name)?;
        let dir = staging_dir();
        std::fs::create_dir_all(&dir).map_err(|e| RuntimeError::Capability {
            capability: cap.to_string(),
            message: format!("could not create staging dir {}: {e}", dir.display()),
        })?;
        let path = dir.join(&filename);
        std::fs::write(&path, air).map_err(|e| RuntimeError::Capability {
            capability: cap.to_string(),
            message: format!("could not write {}: {e}", path.display()),
        })?;
        Ok(Value::String(path.to_string_lossy().to_string()))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

/// `run_workflow` — run a previously staged workflow by name through the
/// apxm-server's compile endpoint, returning the result text.
pub struct RunWorkflowCapability {
    metadata: CapabilityMetadata,
}

impl RunWorkflowCapability {
    pub fn new() -> Self {
        Self {
            metadata: CapabilityMetadata::new(
                "run_workflow",
                "Run a workflow previously created with compose_workflow, by name. \
                 Returns the workflow's result text.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "name of a staged workflow" }
                    },
                    "required": ["name"]
                }),
            )
            .with_returns("string (workflow result)")
            .with_groups(vec![AUTHORING_GROUP.to_string(), "workflow".to_string()]),
        }
    }
}

impl Default for RunWorkflowCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for RunWorkflowCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let cap = "run_workflow";
        let name = require_string_arg(&args, "name", cap)?;
        let filename = safe_workflow_filename(cap, name)?;
        let path = staging_dir().join(&filename);
        let air = std::fs::read_to_string(&path).map_err(|e| RuntimeError::Capability {
            capability: cap.to_string(),
            message: format!("workflow '{name}' not found at {}: {e}", path.display()),
        })?;

        let url = format!("{}/v1/compile", server_base().trim_end_matches('/'));
        let resp = shared_client()
            .post(&url)
            .json(&serde_json::json!({ "air": air }))
            .send()
            .await
            .map_err(|e| RuntimeError::Capability {
                capability: cap.to_string(),
                message: format!("workflow run request failed: {e}"),
            })?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(RuntimeError::Capability {
                capability: cap.to_string(),
                message: format!("workflow run returned {status}: {body}"),
            });
        }
        // Surface the workflow's final content if present, else the raw body.
        let content = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| {
                v.get("content")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or(body);
        Ok(Value::String(content))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

