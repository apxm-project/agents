//! `LinkAgentSpawner` — routes `SPAWN_AGENT` to a T2 LINK-RUNTIME host.
//!
//! When a host has been enrolled at the LINK-RUNTIME tier, an APXM-side
//! `SPAWN_AGENT` operation should delegate the ACP child to the host rather
//! than launching a local subprocess.  `LinkAgentSpawner` wraps the relay
//! channel registry and issues a `spawn_agent` Link frame to the host, then
//! returns a `RelaySessionHandle` that the prompter can drive over
//! `RelayTransport`.
//!
//! This spawner operates alongside `AcpAgentSpawner`: the runtime chooses
//! between them based on the presence of a `host_id` in the spawn context
//! and whether that host is currently link-connected.

use std::any::Any;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use apxm_acp::relay_transport::RelayTransport;
use apxm_core::error::RuntimeError;
use apxm_core::types::aam::AamContext;
use apxm_core::types::operations::AISOperationType;
use apxm_runtime::agent_router::AgentRouteCandidate;
use apxm_runtime::process_table::{AgentSpawnContext, AgentSpawner};
use tokio::sync::{Mutex, mpsc};
use tracing::debug;

/// A session handle for a relay-backed ACP child running on a T2 host.
///
/// The handle owns the `RelayTransport` so the prompter can borrow it to send
/// prompts and receive responses over the Link relay.
pub struct RelaySessionHandle {
    pub transport: RelayTransport,
    pub host_id: String,
    pub session_id: String,
}

/// Registry of active Link-connected hosts.
///
/// Each entry maps a `host_id` to the channel pair that bridges APXM frames
/// to the host's ACP child relay endpoint.  The relay connection handler
/// (in `os-listeners/src/relay.rs`) inserts entries when a host dials in and
/// removes them on disconnect.
pub struct LinkHostRegistry {
    /// host_id → (tx to host, rx from host)
    hosts: Mutex<HashMap<String, (mpsc::Sender<serde_json::Value>, mpsc::Receiver<serde_json::Value>)>>,
}

impl LinkHostRegistry {
    pub fn new() -> Self {
        Self {
            hosts: Mutex::new(HashMap::new()),
        }
    }

    /// Register a newly connected host.  Called by the relay connection handler.
    pub async fn insert(
        &self,
        host_id: String,
        tx: mpsc::Sender<serde_json::Value>,
        rx: mpsc::Receiver<serde_json::Value>,
    ) {
        self.hosts.lock().await.insert(host_id, (tx, rx));
    }

    /// Remove a disconnected host.
    pub async fn remove(&self, host_id: &str) {
        self.hosts.lock().await.remove(host_id);
    }

    /// Check whether a host is currently connected.
    pub async fn contains(&self, host_id: &str) -> bool {
        self.hosts.lock().await.contains_key(host_id)
    }

    /// Take the channel pair for a host, allowing the caller to build a
    /// `RelayTransport`.  Once taken, the registry no longer holds the channels
    /// (the active session owns them).
    pub async fn take(
        &self,
        host_id: &str,
    ) -> Option<(mpsc::Sender<serde_json::Value>, mpsc::Receiver<serde_json::Value>)> {
        self.hosts.lock().await.remove(host_id)
    }
}

impl Default for LinkHostRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawner that delegates `SPAWN_AGENT` to a T2 LINK-RUNTIME host.
pub struct LinkAgentSpawner {
    registry: Arc<LinkHostRegistry>,
}

impl LinkAgentSpawner {
    pub fn new(registry: Arc<LinkHostRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait::async_trait]
impl AgentSpawner for LinkAgentSpawner {
    fn route_candidates(&self) -> Vec<AgentRouteCandidate> {
        // Link-based agents are resolved by host_id at spawn time, not from a
        // static registry, so we return no static candidates here.
        vec![]
    }

    async fn spawn_external(
        &self,
        agent_name: &str,
        _profile_name: &str,
        _cwd: &Path,
        _mode: Option<&str>,
        _model: Option<&str>,
        _aam_context: &AamContext,
        spawn_context: &AgentSpawnContext,
        _extra_env: &HashMap<String, String>,
    ) -> Result<Arc<Mutex<dyn Any + Send + Sync>>, RuntimeError> {
        let host_id = spawn_context
            .host_id
            .as_deref()
            .ok_or_else(|| RuntimeError::Operation {
                op_type: AISOperationType::SpawnAgent,
                message: "LinkAgentSpawner requires a host_id in the spawn context".to_string(),
            })?;

        // TODO(band-d): replace direct registry access with
        // `HostDispatchGateway::open_agent_channel(host_id, agent_name, …)`.
        // `HostDispatchGateway` is available via `apxm_runtime::HostDispatchGateway`
        // (re-exported from `apxm-runtime`). The gateway returns an
        // `AgentChannelHandle` whose channel_id can then be resolved back to the
        // (tx, rx) pair from a channel store, decoupling the spawner from the
        // raw `LinkHostRegistry`.  The spawner should hold an
        // `Arc<dyn HostDispatchGateway>` rather than `Arc<LinkHostRegistry>`.
        let (tx, rx) = self
            .registry
            .take(host_id)
            .await
            .ok_or_else(|| RuntimeError::Operation {
                op_type: AISOperationType::SpawnAgent,
                message: format!("host '{host_id}' is not currently link-connected"),
            })?;

        debug!(host_id, agent_name, "spawning relay ACP session on T2 host");

        let session_id = uuid::Uuid::new_v4().to_string();
        let transport = RelayTransport::new(tx, rx);

        let handle = RelaySessionHandle {
            transport,
            host_id: host_id.to_string(),
            session_id,
        };

        Ok(Arc::new(Mutex::new(handle)))
    }
}
