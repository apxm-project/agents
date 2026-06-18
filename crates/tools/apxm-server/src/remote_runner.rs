//! Remote agent spawner seam.
//!
//! When `APXM_RUNNER_URL` is set, `RemoteAgentSpawner` delegates ACP agent
//! launch, status queries, and cancellation to the apxm-runner service via its
//! HTTP API.  When the env var is absent the local subprocess path (configured
//! via `configure_agent_registry`) remains active — dev compatibility is
//! preserved with no code change required at call sites.
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
use tracing::{debug, info};

/// Payload sent to `POST /v1/agents/spawn`.
///
/// Field names follow the runner-api.md contract verbatim so they serialise
/// correctly over the wire without extra mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnRequest {
    /// Caller-assigned execution identifier; used as the runner's
    /// `execution_id` so logs and artifacts can be correlated.
    pub run_id: String,
    /// Agent profile name: `"codex"`, `"claude"`, or `"custom"`.
    pub agent_type: String,
    /// Optional parent workflow identifier for correlation.
    pub workflow_id: Option<String>,
    /// Initial prompt delivered to the agent immediately after spawn.
    pub prompt: String,
    /// Opaque credential references forwarded to the runner so it can
    /// materialise short-lived secrets before launching the container.
    pub credential_ids: Vec<String>,
}

impl SpawnRequest {
    /// Wire format expected by the runner.
    fn to_wire(&self) -> RunnerSpawnBody {
        RunnerSpawnBody {
            execution_id: self.run_id.clone(),
            workflow_id: self.workflow_id.clone(),
            agent_profile: self.agent_type.clone(),
            // workspace_ref and resource_limits are runner-controlled defaults;
            // they can be extended here when spec 0015 adds brokered runners.
            workspace_ref: String::new(),
            credential_refs: self.credential_ids.clone(),
            resource_limits: None,
        }
    }
}

/// Runner-API spawn body (matches runner-api.md § Spawn request exactly).
#[derive(Debug, Serialize)]
struct RunnerSpawnBody {
    execution_id: String,
    workflow_id: Option<String>,
    agent_profile: String,
    workspace_ref: String,
    credential_refs: Vec<String>,
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
    pub id: String,
    pub status: RunStatus,
    pub agent_profile: Option<String>,
    pub workflow_id: Option<String>,
}

/// Runner spawn response — the server returns the newly created job id.
#[derive(Debug, Deserialize)]
struct SpawnResponse {
    id: String,
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
        }
    }

    /// Try to construct a spawner from the `APXM_RUNNER_URL` environment
    /// variable.  Returns `None` when the variable is absent or empty so the
    /// caller can fall back to the local subprocess path.
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("APXM_RUNNER_URL")
            .ok()
            .filter(|v| !v.is_empty())?;
        info!(runner_url = %url, "remote agent runner configured");
        Some(Self::new(url))
    }

    /// `POST /v1/agents/spawn` — create an isolated ACP job on the runner.
    ///
    /// Returns the opaque run id assigned by the runner on success.
    pub async fn spawn(&self, req: SpawnRequest) -> Result<String, RemoteRunnerError> {
        let url = format!("{}/v1/agents/spawn", self.runner_url);
        debug!(run_id = %req.run_id, agent_type = %req.agent_type, %url, "spawning remote agent");

        let body = req.to_wire();
        let response = self
            .http
            .post(&url)
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

        info!(run_id = %req.run_id, remote_id = %resp.id, "remote agent spawned");
        Ok(resp.id)
    }

    /// `GET /v1/agents/{id}` — poll the job state and resource metadata.
    pub async fn status(&self, run_id: &str) -> Result<RunStatusResponse, RemoteRunnerError> {
        let url = format!("{}/v1/agents/{}", self.runner_url, run_id);
        debug!(%run_id, %url, "querying remote agent status");

        let response = self
            .http
            .get(&url)
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
            .http
            .delete(&url)
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
            agent_type: "codex".to_string(),
            workflow_id: Some("wf-abc".to_string()),
            prompt: "hello".to_string(),
            credential_ids: vec!["cred-1".to_string()],
        };
        let wire = req.to_wire();
        assert_eq!(wire.execution_id, "exec-1");
        assert_eq!(wire.agent_profile, "codex");
        assert_eq!(wire.workflow_id, Some("wf-abc".to_string()));
        assert_eq!(wire.credential_refs, vec!["cred-1"]);
    }
}
