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
use apxm_runtime::host_dispatch::{HostDispatchGateway, SpawnOffer};
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
    gateway: Arc<dyn HostDispatchGateway>,
    registry: Arc<LinkHostRegistry>,
}

impl LinkAgentSpawner {
    pub fn new(gateway: Arc<dyn HostDispatchGateway>, registry: Arc<LinkHostRegistry>) -> Self {
        Self { gateway, registry }
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

        // Open the agent channel through the HostDispatchGateway. This performs
        // the policy check, attestation, and logging side effects, and yields the
        // `channel_id` that identifies this session. The raw `(tx, rx)` relay pair
        // is then resolved from `LinkHostRegistry` (the gateway does not surface
        // the underlying channels; agents never own live Link attachments).
        let channel_id = uuid::Uuid::new_v4().to_string();
        let spawn_offer = SpawnOffer {
            lease_id: uuid::Uuid::new_v4().to_string(),
            channel_id: channel_id.clone(),
            accept_by_ms: 0,
            signed_spawn_envelope: String::new(),
            profile: _profile_name.to_string(),
            mode: _mode.map(str::to_string),
            model: _model.map(str::to_string),
            workdir_ref: _cwd.to_str().map(str::to_string),
            attestation_nonce: uuid::Uuid::new_v4().to_string(),
            extra_env: _extra_env.clone(),
        };

        let channel_handle = self
            .gateway
            .open_agent_channel(host_id, spawn_offer)
            .await
            .map_err(|e| RuntimeError::Operation {
                op_type: AISOperationType::SpawnAgent,
                message: format!("failed to open agent channel on host '{host_id}': {e}"),
            })?;

        let (tx, rx) = self
            .registry
            .take(host_id)
            .await
            .ok_or_else(|| RuntimeError::Operation {
                op_type: AISOperationType::SpawnAgent,
                message: format!("host '{host_id}' is not currently link-connected"),
            })?;

        debug!(host_id, agent_name, "spawning relay ACP session on T2 host");

        let transport = RelayTransport::new(tx, rx);

        let handle = RelaySessionHandle {
            transport,
            host_id: host_id.to_string(),
            session_id: channel_handle.channel_id,
        };

        Ok(Arc::new(Mutex::new(handle)))
    }
}
