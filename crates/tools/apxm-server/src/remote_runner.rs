//! Remote agent spawner seam.
//!
//! When `APXM_RUNNER_URL` is set, `RemoteAgentRunner` delegates ACP agent
//! launch, prompt delivery, status queries, and cancellation to the apxm-runner
//! service via its HTTP API.
//!
//! ## Runner API (spec 0011)
//!
//! | Method   | Path                    | Purpose                         |
//! |----------|-------------------------|---------------------------------|
//! | `POST`   | `/v1/agents/spawn`      | Create one isolated ACP job     |
//! | `POST`   | `/v1/agents/{id}/prompt`| Deliver a prompt to the job     |
//! | `GET`    | `/v1/agents/{id}`       | Read status and resource metadata|
//! | `GET`    | `/v1/agents/{id}/logs`  | Fetch runner logs               |
//! | `DELETE` | `/v1/agents/{id}`       | Cancel and clean up             |

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, info};

use apxm_core::error::RuntimeError;
use apxm_core::types::operations::AISOperationType;
use apxm_runtime::agent_router::AgentRouteCandidate;
use apxm_runtime::process::{AgentProcess, ProcessKind};
use apxm_runtime::process_table::{
    AgentPromptResponse, AgentPrompter, AgentSpawnContext, AgentSpawner,
};

/// Payload sent to `POST /v1/agents/spawn`.
///
/// Field names follow the runner-api.md contract verbatim so they serialise
/// correctly over the wire without extra mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnRequest {
    /// Caller-assigned execution identifier; used as the runner's
    /// `execution_id` so logs and artifacts can be correlated.
    pub run_id: String,
    /// Registered runner profile name. Available names come from
    /// `/v1/agent-profiles`, not from server built-ins.
    pub agent_type: String,
    /// Optional parent workflow identifier for correlation.
    pub workflow_id: Option<String>,
    /// Opaque credential references forwarded to the runner so it can
    /// materialise short-lived secrets before launching the container.
    pub credential_ids: Vec<String>,
    pub session_id: Option<String>,
    pub node_id: Option<u64>,
    pub node_name: Option<String>,
    pub run_root: Option<String>,
    pub node_artifact_dir: Option<String>,
    pub runner_artifact_dir: Option<String>,
    pub context_ref: Option<String>,
    pub workdir_ref: Option<String>,
    pub mode: Option<String>,
    pub model: Option<String>,
}

impl SpawnRequest {
    /// Wire format expected by the runner.
    fn to_wire(&self) -> RunnerSpawnBody {
        RunnerSpawnBody {
            execution_id: self.run_id.clone(),
            workflow_id: self.workflow_id.clone(),
            session_id: self.session_id.clone(),
            agent_profile: self.agent_type.clone(),
            agent_name: None,
            workspace_ref: self.workdir_ref.clone().unwrap_or_default(),
            run_root: self.run_root.clone(),
            node_id: self.node_id,
            node_name: self.node_name.clone(),
            node_artifact_dir: self.node_artifact_dir.clone(),
            runner_artifact_dir: self.runner_artifact_dir.clone(),
            context_ref: self.context_ref.clone(),
            workdir_ref: self.workdir_ref.clone(),
            credential_refs: self.credential_ids.clone(),
            mode: self.mode.clone(),
            model: self.model.clone(),
            resource_limits: None,
        }
    }
}

/// Runner-API spawn body (matches runner-api.md § Spawn request exactly).
#[derive(Debug, Serialize)]
struct RunnerSpawnBody {
    execution_id: String,
    workflow_id: Option<String>,
    session_id: Option<String>,
    agent_name: Option<String>,
    agent_profile: String,
    workspace_ref: String,
    run_root: Option<String>,
    node_id: Option<u64>,
    node_name: Option<String>,
    node_artifact_dir: Option<String>,
    runner_artifact_dir: Option<String>,
    context_ref: Option<String>,
    workdir_ref: Option<String>,
    credential_refs: Vec<String>,
    mode: Option<String>,
    model: Option<String>,
    resource_limits: Option<ResourceLimits>,
}

/// Optional resource constraints the caller may attach to a spawn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub cpu: Option<String>,
    pub memory_mb: Option<u64>,
    pub timeout_seconds: Option<u64>,
}

/// Job lifecycle states as described in runner-api.md.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Starting,
    Running,
    Succeeded,
    Failed,
    Canceled,
    Expired,
}

/// Status response from `GET /v1/agents/{id}`.
#[derive(Debug, Clone, Deserialize)]
pub struct RunStatusResponse {
    #[serde(alias = "execution_id")]
    pub id: String,
    pub status: RunStatus,
    pub agent_profile: Option<String>,
    pub workflow_id: Option<String>,
}

/// Runner spawn response.
#[derive(Debug, Deserialize)]
struct SpawnResponse {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    execution_id: Option<String>,
    workflow_id: Option<String>,
    agent_profile: Option<String>,
    session_id: Option<String>,
    agent_session_id: Option<String>,
    runner_artifact_dir: Option<String>,
}

impl SpawnResponse {
    fn remote_id(&self) -> Option<&str> {
        self.id
            .as_deref()
            .or_else(|| self.execution_id.as_deref())
            .filter(|value| !value.trim().is_empty())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RunnerProfile {
    pub name: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub default_mode: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProfilesResponse {
    #[serde(default)]
    profiles: Vec<RunnerProfile>,
}

#[derive(Debug, Serialize)]
struct PromptBody<'a> {
    prompt: &'a str,
}

#[derive(Debug, Deserialize)]
struct PromptResponse {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    output_text: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    agent_session_id: Option<String>,
    #[serde(default)]
    turn: Option<u64>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    input_tokens: Option<usize>,
    #[serde(default)]
    output_tokens: Option<usize>,
    #[serde(default)]
    usage: Option<PromptUsage>,
}

#[derive(Debug, Deserialize)]
struct PromptUsage {
    #[serde(default)]
    input_tokens: Option<usize>,
    #[serde(default)]
    output_tokens: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct RemoteAgentSession {
    pub remote_id: String,
    pub execution_id: String,
    pub workflow_id: Option<String>,
    pub profile_name: String,
    pub session_id: Option<String>,
    pub agent_session_id: Option<String>,
    pub runner_artifact_dir: Option<String>,
}

// ─── RemoteAgentSpawner ──────────────────────────────────────────────────────

/// HTTP client wrapper for the apxm-runner agent isolation service.
///
/// Constructed once during server startup when `APXM_RUNNER_URL` is set and
/// shared via `Arc` across request handlers.
///
/// All three operations (`spawn`, `status`, `cancel`) are thin HTTP delegates;
/// policy decisions (admission, budget enforcement) remain in the server.
#[derive(Debug, Clone)]
pub struct RemoteAgentSpawner {
    runner_url: String,
    http: Client,
    route_candidates: Vec<AgentRouteCandidate>,
}

impl RemoteAgentSpawner {
    /// Build a spawner that talks to `runner_url`.
    ///
    /// Panics if `reqwest::Client::new()` fails (only possible when TLS
    /// initialisation cannot find the system cert store — fatal at startup).
    pub fn new(runner_url: impl Into<String>) -> Self {
        Self {
            runner_url: runner_url.into().trim_end_matches('/').to_owned(),
            http: Client::new(),
            route_candidates: Vec::new(),
        }
    }

    pub fn with_route_candidates(mut self, route_candidates: Vec<AgentRouteCandidate>) -> Self {
        self.route_candidates = route_candidates;
        self
    }

    /// Try to construct a spawner from the `APXM_RUNNER_URL` environment
    /// variable. Returns `None` when the variable is absent or empty; callers
    /// must leave external ACP profiles unregistered so profile-backed
    /// `spawn_agent` requests fail closed.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("APXM_RUNNER_URL")
            .ok()
            .filter(|v| !v.is_empty())?;
        info!(runner_url = %url, "remote agent runner configured");
        Some(Self::new(url))
    }

    pub async fn discover_route_candidates(
        &self,
    ) -> Result<Vec<AgentRouteCandidate>, RemoteRunnerError> {
        let url = format!("{}/v1/agent-profiles", self.runner_url);
        let response = self
            .runner_auth(self.http.get(&url))
            .send()
            .await
            .map_err(|e| RemoteRunnerError::Transport(e.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(RemoteRunnerError::Http {
                status: status.as_u16(),
                body: text,
            });
        }
        let body: ProfilesResponse = response
            .json()
            .await
            .map_err(|e| RemoteRunnerError::Decode(e.to_string()))?;
        Ok(body
            .profiles
            .into_iter()
            .filter(|profile| {
                profile
                    .status
                    .as_deref()
                    .map(|status| status == "available")
                    .unwrap_or(true)
            })
            .map(|profile| AgentRouteCandidate {
                profile: profile.name,
                description: None,
                source: Some(profile.source.unwrap_or_else(|| "runner".to_string())),
                executable: profile
                    .executable
                    .unwrap_or_else(|| "apxm-runner".to_string()),
                capabilities: profile.capabilities,
                default_mode: profile.default_mode,
                default_model: profile.default_model,
            })
            .collect())
    }

    /// `POST /v1/agents/spawn` — create an isolated ACP job on the runner.
    ///
    /// Returns the opaque run id assigned by the runner on success.
    pub async fn spawn(&self, req: SpawnRequest) -> Result<String, RemoteRunnerError> {
        let url = format!("{}/v1/agents/spawn", self.runner_url);
        debug!(run_id = %req.run_id, agent_type = %req.agent_type, %url, "spawning remote agent");

        let body = req.to_wire();
        let response = self
            .runner_auth(self.http.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| RemoteRunnerError::Transport(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(RemoteRunnerError::Http {
                status: status.as_u16(),
                body: text,
            });
        }

        let resp: SpawnResponse = response
            .json()
            .await
            .map_err(|e| RemoteRunnerError::Decode(e.to_string()))?;

        let remote_id = resp
            .remote_id()
            .ok_or_else(|| RemoteRunnerError::Decode("spawn response missing id".into()))?
            .to_string();
        info!(run_id = %req.run_id, remote_id = %remote_id, "remote agent spawned");
        Ok(remote_id)
    }

    /// `GET /v1/agents/{id}` — poll the job state and resource metadata.
    pub async fn status(&self, run_id: &str) -> Result<RunStatusResponse, RemoteRunnerError> {
        let url = format!("{}/v1/agents/{}", self.runner_url, run_id);
        debug!(%run_id, %url, "querying remote agent status");

        let response = self
            .runner_auth(self.http.get(&url))
            .send()
            .await
            .map_err(|e| RemoteRunnerError::Transport(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(RemoteRunnerError::Http {
                status: status.as_u16(),
                body: text,
            });
        }

        response
            .json::<RunStatusResponse>()
            .await
            .map_err(|e| RemoteRunnerError::Decode(e.to_string()))
    }

    /// `DELETE /v1/agents/{id}` — cancel the job and free all resources.
    pub async fn cancel(&self, run_id: &str) -> Result<(), RemoteRunnerError> {
        let url = format!("{}/v1/agents/{}", self.runner_url, run_id);
        debug!(%run_id, %url, "cancelling remote agent");

        let response = self
            .runner_auth(self.http.delete(&url))
            .send()
            .await
            .map_err(|e| RemoteRunnerError::Transport(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(RemoteRunnerError::Http {
                status: status.as_u16(),
                body: text,
            });
        }

        info!(%run_id, "remote agent cancelled");
        Ok(())
    }

    fn runner_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match std::env::var("APXM_RUNNER_IDENTITY_TOKEN") {
            Ok(token) if !token.trim().is_empty() => request.bearer_auth(token.trim().to_string()),
            _ => request,
        }
    }

    async fn prompt_remote(
        &self,
        remote_id: &str,
        message: &str,
    ) -> Result<AgentPromptResponse, RemoteRunnerError> {
        let url = format!("{}/v1/agents/{}/prompt", self.runner_url, remote_id);
        let response = self
            .runner_auth(self.http.post(&url))
            .json(&PromptBody { prompt: message })
            .send()
            .await
            .map_err(|e| RemoteRunnerError::Transport(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(RemoteRunnerError::Http {
                status: status.as_u16(),
                body: text,
            });
        }

        let body: PromptResponse = response
            .json()
            .await
            .map_err(|e| RemoteRunnerError::Decode(e.to_string()))?;
        let text = body.text.or(body.output_text).ok_or_else(|| {
            RemoteRunnerError::Decode("runner prompt response missing text".into())
        })?;
        let input_tokens = body
            .input_tokens
            .or_else(|| body.usage.as_ref().and_then(|usage| usage.input_tokens));
        let output_tokens = body
            .output_tokens
            .or_else(|| body.usage.as_ref().and_then(|usage| usage.output_tokens));
        let mut result = AgentPromptResponse::text(text);
        if let Some(session_id) = body.session_id {
            result = result.with_session_id(session_id);
        }
        if let Some(agent_session_id) = body.agent_session_id {
            result = result.with_agent_session_id(agent_session_id);
        }
        if let Some(turn) = body.turn {
            result = result.with_turn(turn);
        }
        if let Some(model) = body.model {
            result = result.with_model(model);
        }
        if let Some(stop_reason) = body.stop_reason {
            result = result.with_stop_reason(stop_reason);
        }
        Ok(result.with_token_usage(input_tokens, output_tokens))
    }
}

#[async_trait::async_trait]
impl AgentSpawner for RemoteAgentSpawner {
    fn route_candidates(&self) -> Vec<AgentRouteCandidate> {
        self.route_candidates.clone()
    }

    async fn spawn_external(
        &self,
        agent_name: &str,
        profile_name: &str,
        _cwd: &std::path::Path,
        mode: Option<&str>,
        model: Option<&str>,
        _aam_context: &apxm_core::types::aam::AamContext,
        spawn_context: &AgentSpawnContext,
        _extra_env: &std::collections::HashMap<String, String>,
    ) -> Result<Arc<tokio::sync::Mutex<dyn std::any::Any + Send + Sync>>, RuntimeError> {
        let execution_id = required_context(&spawn_context.execution_id, "execution_id")?;
        let workflow_id = required_context(&spawn_context.workflow_id, "workflow_id")?;
        required_context(&spawn_context.run_root, "run_root")?;
        required_context(&spawn_context.node_artifact_dir, "node_artifact_dir")?;
        required_context(&spawn_context.runner_artifact_dir, "runner_artifact_dir")?;
        required_context(&spawn_context.workdir_ref, "workdir_ref")?;

        let req = SpawnRequest {
            run_id: execution_id.clone(),
            agent_type: profile_name.to_string(),
            workflow_id: Some(workflow_id),
            credential_ids: Vec::new(),
            session_id: spawn_context.session_id.clone(),
            node_id: Some(spawn_context.node_id),
            node_name: spawn_context.node_name.clone(),
            run_root: spawn_context.run_root.clone(),
            node_artifact_dir: spawn_context.node_artifact_dir.clone(),
            runner_artifact_dir: spawn_context.runner_artifact_dir.clone(),
            context_ref: spawn_context.context_ref.clone(),
            workdir_ref: spawn_context.workdir_ref.clone(),
            mode: mode.map(ToOwned::to_owned),
            model: model.map(ToOwned::to_owned),
        };
        let url = format!("{}/v1/agents/spawn", self.runner_url);
        let body = {
            let mut body = req.to_wire();
            body.agent_name = Some(agent_name.to_string());
            body
        };
        let response = self
            .runner_auth(self.http.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| runner_runtime_error(format!("transport error: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(runner_runtime_error(format!(
                "runner returned HTTP {}: {text}",
                status.as_u16()
            )));
        }
        let resp: SpawnResponse = response
            .json()
            .await
            .map_err(|e| runner_runtime_error(format!("failed to decode spawn response: {e}")))?;
        let remote_id = resp
            .remote_id()
            .ok_or_else(|| runner_runtime_error("runner spawn response missing id".to_string()))?
            .to_string();
        let session = RemoteAgentSession {
            remote_id,
            execution_id: resp.execution_id.unwrap_or(execution_id),
            workflow_id: resp.workflow_id,
            profile_name: resp
                .agent_profile
                .unwrap_or_else(|| profile_name.to_string()),
            session_id: resp.session_id.or_else(|| spawn_context.session_id.clone()),
            agent_session_id: resp.agent_session_id,
            runner_artifact_dir: resp
                .runner_artifact_dir
                .or_else(|| spawn_context.runner_artifact_dir.clone()),
        };
        Ok(Arc::new(tokio::sync::Mutex::new(session)))
    }
}

#[async_trait::async_trait]
impl AgentPrompter for RemoteAgentSpawner {
    async fn prompt(
        &self,
        process: &AgentProcess,
        message: &str,
    ) -> Result<AgentPromptResponse, RuntimeError> {
        let ProcessKind::External { session, .. } = &process.kind else {
            return Err(runner_runtime_error(format!(
                "process '{}' is not an external runner session",
                process.name
            )));
        };
        let remote_id = {
            let guard = session.lock().await;
            let Some(remote) = guard.downcast_ref::<RemoteAgentSession>() else {
                return Err(runner_runtime_error(format!(
                    "process '{}' does not hold a runner session",
                    process.name
                )));
            };
            remote.remote_id.clone()
        };
        self.prompt_remote(&remote_id, message)
            .await
            .map_err(|error| runner_runtime_error(error.to_string()))
    }
}

fn required_context(value: &Option<String>, field: &str) -> Result<String, RuntimeError> {
    value
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or_else(|| runner_runtime_error(format!("runner spawn requires {field}")))
}

fn runner_runtime_error(message: String) -> RuntimeError {
    RuntimeError::Operation {
        op_type: AISOperationType::SpawnAgent,
        message,
    }
}

// ─── Error type ─────────────────────────────────────────────────────────────

/// Errors produced by `RemoteAgentSpawner`.
#[derive(Debug)]
pub enum RemoteRunnerError {
    /// Network or TLS failure before the runner responded.
    Transport(String),
    /// Runner returned a non-2xx status code.
    Http { status: u16, body: String },
    /// Runner response could not be deserialised.
    Decode(String),
}

impl std::fmt::Display for RemoteRunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
            Self::Http { status, body } => write!(f, "runner returned HTTP {status}: {body}"),
            Self::Decode(msg) => write!(f, "failed to decode runner response: {msg}"),
        }
    }
}

impl std::error::Error for RemoteRunnerError {}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(unsafe_code)]
    fn from_env_returns_none_when_unset() {
        // Guard: ensure APXM_RUNNER_URL is absent for this test.
        // SAFETY: single-threaded test; no concurrent env readers.
        unsafe { std::env::remove_var("APXM_RUNNER_URL") };
        assert!(RemoteAgentSpawner::from_env().is_none());
    }

    #[test]
    #[allow(unsafe_code)]
    fn from_env_returns_some_when_set() {
        // SAFETY: single-threaded test; no concurrent env readers.
        unsafe { std::env::set_var("APXM_RUNNER_URL", "http://localhost:9200") };
        let spawner = RemoteAgentSpawner::from_env();
        assert!(spawner.is_some());
        let s = spawner.unwrap();
        assert_eq!(s.runner_url, "http://localhost:9200");
        unsafe { std::env::remove_var("APXM_RUNNER_URL") };
    }

    #[test]
    fn trailing_slash_stripped_from_url() {
        let s = RemoteAgentSpawner::new("http://runner:9200/");
        assert_eq!(s.runner_url, "http://runner:9200");
    }

    #[test]
    fn spawn_request_wire_mapping() {
        let req = SpawnRequest {
            run_id: "exec-1".to_string(),
            agent_type: "test-profile".to_string(),
            workflow_id: Some("wf-abc".to_string()),
            credential_ids: vec!["cred-1".to_string()],
            session_id: Some("session-1".to_string()),
            node_id: Some(1),
            node_name: Some("agent".to_string()),
            run_root: Some("/workspace/runs/wf-abc/exec-1".to_string()),
            node_artifact_dir: Some("nodes/01_agent".to_string()),
            runner_artifact_dir: Some("nodes/01_agent/runner".to_string()),
            context_ref: Some("nodes/01_agent/runner/context".to_string()),
            workdir_ref: Some("nodes/01_agent/runner/workdir".to_string()),
            mode: Some("default".to_string()),
            model: Some("test-model".to_string()),
        };
        let wire = req.to_wire();
        assert_eq!(wire.execution_id, "exec-1");
        assert_eq!(wire.agent_profile, "test-profile");
        assert_eq!(wire.workflow_id, Some("wf-abc".to_string()));
        assert_eq!(wire.credential_refs, vec!["cred-1"]);
        assert_eq!(wire.session_id, Some("session-1".to_string()));
        assert_eq!(
            wire.workspace_ref,
            "nodes/01_agent/runner/workdir".to_string()
        );
        assert_eq!(
            wire.context_ref,
            Some("nodes/01_agent/runner/context".to_string())
        );
    }
}
