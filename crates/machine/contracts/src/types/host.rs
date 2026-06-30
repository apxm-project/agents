//! Universal APXM host contract types.
//!
//! Shared by agents (runtime decisions), server (admission/ledger), os (relay
//! routing), auth (enrollment/custody), and studio (authoring/catalog).
//! These types are the normalized runtime contract — not the full authoring
//! `HostAdapterCard`.

use serde::{Deserialize, Serialize};

// ── Tier ─────────────────────────────────────────────────────────────────────

/// Where the agent process executes. Selected deterministically by
/// [`select_tier`]; never self-asserted as authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostTier {
    /// No Link; public webhook in, `/proxy` out. Agent runs APXM-side.
    Direct,
    /// Host-dialed Link. Host contributes tools/events. Agent runs APXM-side.
    LinkTools,
    /// Host-dialed Link. Host also forks an ACP child locally.
    LinkRuntime,
}

impl std::fmt::Display for HostTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostTier::Direct => f.write_str("direct"),
            HostTier::LinkTools => f.write_str("link-tools"),
            HostTier::LinkRuntime => f.write_str("link-runtime"),
        }
    }
}

// ── Custody ───────────────────────────────────────────────────────────────────

/// How credentials flow to the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostCustody {
    /// Secret stays APXM-side; `/proxy` injects and returns only the upstream
    /// body. Used by T0 DIRECT hosts.
    Inject,
    /// Host receives an APXM-minted PoP JWT (60s, host-bound, scoped).
    /// Used by LINK-TOOLS and LINK-RUNTIME hosts.
    Token,
}

// ── Confinement ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfinementMechanism {
    None,
    OsSandbox,
    Container,
    Vm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfinementNetwork {
    Isolated,
    EgressAllowlist,
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfinementFsScope {
    WorkdirOnly,
    HostHome,
    HostRoot,
}

/// Confinement posture declared in `transport.toml [confinement]`.
/// Required for LINK-RUNTIME hosts; bound to the host principal at enrollment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfinementProfile {
    pub mechanism: ConfinementMechanism,
    pub network: ConfinementNetwork,
    pub fs_scope: ConfinementFsScope,
    pub writable: bool,
}

impl ConfinementProfile {
    /// Returns true if this profile satisfies the minimum required profile.
    pub fn satisfies(&self, min: &ConfinementProfile) -> bool {
        self.mechanism_strength() >= min.mechanism_strength()
            && self.network_strength() >= min.network_strength()
            && self.fs_strength() >= min.fs_strength()
    }

    fn mechanism_strength(&self) -> u8 {
        match self.mechanism {
            ConfinementMechanism::None => 0,
            ConfinementMechanism::OsSandbox => 1,
            ConfinementMechanism::Container => 2,
            ConfinementMechanism::Vm => 3,
        }
    }

    fn network_strength(&self) -> u8 {
        match self.network {
            ConfinementNetwork::Open => 0,
            ConfinementNetwork::EgressAllowlist => 1,
            ConfinementNetwork::Isolated => 2,
        }
    }

    fn fs_strength(&self) -> u8 {
        match self.fs_scope {
            ConfinementFsScope::HostRoot => 0,
            ConfinementFsScope::HostHome => 1,
            ConfinementFsScope::WorkdirOnly => 2,
        }
    }
}

// ── TransportManifest ─────────────────────────────────────────────────────────

/// Parsed `transport.toml` — the single declarative input to `select_tier`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TransportManifest {
    /// Identity of the enrolled Link stream. Present ⇒ a Link exists.
    pub host_id: Option<String>,
    /// Self-documenting tier assertion; must match computed tier or enrollment fails.
    pub tier_hint: Option<HostTier>,
    /// Credential custody mode.
    pub custody: Option<HostCustody>,
    #[serde(default)]
    pub ingress: IngressConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub push: Option<PushConfig>,
    pub confinement: Option<ConfinementProfile>,
    /// Labels for host pool/routing decisions.
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IngressConfig {
    pub public_url: Option<String>,
    pub webhook_base_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub local_agent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushConfig {
    pub provider: String, // "apns" | "fcm"
}

/// Operation context used by `select_tier` to determine which tier applies
/// to a specific trigger or capability.
#[derive(Debug, Clone)]
pub struct TierQuery<'a> {
    pub transport: &'a str, // "direct" | "link"
    pub op_kind: Option<OpKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Spawn,
    HostFs,
    HostExec,
    Process,
    Watch,
    LocalListener,
    Standard,
}

/// Error returned when a host cannot be admitted.
#[derive(Debug, thiserror::Error)]
pub enum EnrollError {
    #[error("host has no public ingress and no enrolled host_id — rejected (fail-closed)")]
    NoReachability,
    #[error("tier_hint {hint} does not match computed tier {computed}")]
    TierMismatch { hint: HostTier, computed: HostTier },
    #[error("LINK-RUNTIME host missing required [confinement] block")]
    MissingConfinement,
    #[error("DIRECT host must use custody=inject, got custody=token")]
    CustodyMismatch,
}

/// Deterministic, fail-closed tier selection.
///
/// Rule (first match wins; predicates are mutually exclusive):
/// 1. `transport="direct"` AND `ingress.public_url` is set → T0 DIRECT
/// 2. `runtime.local_agent=true` AND op needs local execution → LINK-RUNTIME
/// 3. `host_id` present → LINK-TOOLS
/// 4. Otherwise → EnrollError (fail-closed)
pub fn select_tier(
    manifest: &TransportManifest,
    query: &TierQuery<'_>,
) -> Result<HostTier, EnrollError> {
    let tier = if query.transport == "direct" && manifest.ingress.public_url.is_some() {
        HostTier::Direct
    } else if manifest.runtime.local_agent && is_local_exec_op(query.op_kind) {
        HostTier::LinkRuntime
    } else if manifest.host_id.is_some() {
        HostTier::LinkTools
    } else {
        return Err(EnrollError::NoReachability);
    };

    // Validate tier_hint consistency
    if let Some(hint) = manifest.tier_hint {
        if hint != tier && !(hint == HostTier::LinkTools && tier == HostTier::LinkRuntime) {
            return Err(EnrollError::TierMismatch {
                hint,
                computed: tier,
            });
        }
    }

    // LINK-RUNTIME requires a confinement block
    if tier == HostTier::LinkRuntime && manifest.confinement.is_none() {
        return Err(EnrollError::MissingConfinement);
    }

    Ok(tier)
}

fn is_local_exec_op(kind: Option<OpKind>) -> bool {
    matches!(
        kind,
        Some(
            OpKind::Spawn
                | OpKind::HostFs
                | OpKind::HostExec
                | OpKind::Process
                | OpKind::Watch
                | OpKind::LocalListener
        )
    )
}

// ── HostExecutionManifest ─────────────────────────────────────────────────────

/// Normalized runtime manifest consumed by agents. Rendered from the full
/// `HostAdapterCard` by the host-sdk manifest renderer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostExecutionManifest {
    pub host_id: String,
    pub tier: HostTier,
    pub custody_mode: HostCustody,
    pub transports: Vec<TransportRef>,
    pub capabilities: Vec<HostCapabilityDecl>,
    pub triggers: Vec<HostTriggerDecl>,
    pub runtime: Option<RuntimeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportRef {
    pub transport_ref: String,
    pub mode: String,
    pub host_id: Option<String>,
    pub direct_ingress: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCapabilityDecl {
    pub capability_id: String,
    pub host_id: String,
    pub transport_ref: String,
    pub host_op: String,
    pub tool_binding: String,
    pub read_only: bool,
    pub idempotency: HostIdempotency,
    pub timeout_ms: u64,
    pub min_confinement: Option<ConfinementProfile>,
    pub token_scope_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostIdempotency {
    None,
    Optional,
    Required,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostTriggerDecl {
    pub trigger_id: String,
    pub transport_ref: String,
    pub event_type: String,
    pub qos: HostQos,
    pub idempotency_source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostQos {
    Safety,
    Bulk,
}

// ── HostDispatchGateway ───────────────────────────────────────────────────────

/// The agents-facing syscall interface for host work. Interface defined here;
/// implemented by `os` (HostDispatchGatewayImpl). Agents call through this;
/// they never own live Link attachments.
#[async_trait::async_trait]
pub trait HostDispatchGateway: Send + Sync {
    async fn call_tool(
        &self,
        host_id: &str,
        call: HostToolCall,
    ) -> Result<HostToolResult, HostDispatchError>;

    async fn request_relay_egress(
        &self,
        host_id: &str,
        request: HostProxyRequest,
    ) -> Result<HostProxyResult, HostDispatchError>;

    async fn open_agent_channel(
        &self,
        host_id: &str,
        spawn_offer: SpawnOffer,
    ) -> Result<AgentChannelHandle, HostDispatchError>;

    async fn send_agent_frame(
        &self,
        channel_id: &str,
        frame: Vec<u8>,
    ) -> Result<(), HostDispatchError>;

    async fn close_agent_channel(
        &self,
        channel_id: &str,
        reason: &str,
    ) -> Result<(), HostDispatchError>;

    async fn take_relay_channel(
        &self,
        channel_id: &str,
    ) -> Option<(
        tokio::sync::mpsc::Sender<serde_json::Value>,
        tokio::sync::mpsc::Receiver<serde_json::Value>,
    )> {
        let _ = channel_id;
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostToolCall {
    pub call_id: String,
    pub capability_id: String,
    pub host_op: String,
    pub tool_binding: String,
    pub args: serde_json::Value,
    pub args_digest: String,
    pub grant_ref: Option<String>,
    pub timeout_ms: u64,
    pub idempotency_key: Option<String>,
    pub subject: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostToolResult {
    pub ok: bool,
    pub value: Option<serde_json::Value>,
    pub error: Option<HostToolError>,
    pub result_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostToolError {
    pub class: String,
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostProxyRequest {
    pub call_id: String,
    pub method: String,
    pub url_ref: String,
    pub headers: Option<serde_json::Value>,
    pub deadline_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostProxyResult {
    pub ok: bool,
    pub status: Option<u16>,
    pub body_ref: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnOffer {
    pub lease_id: String,
    pub channel_id: String,
    pub accept_by_ms: u64,
    pub signed_spawn_envelope: String,
    pub profile: String,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub workdir_ref: Option<String>,
    pub attestation_nonce: String,
    pub extra_env: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct AgentChannelHandle {
    pub channel_id: String,
    pub host_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum HostDispatchError {
    #[error("host {host_id} not attached")]
    NotAttached { host_id: String },
    #[error("call timed out after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },
    #[error("host returned error: {message}")]
    HostError { message: String },
    #[error("transport error: {0}")]
    Transport(String),
    #[error("confinement attestation rejected: {reason}")]
    ConfinementRejected { reason: String },
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn direct_manifest() -> TransportManifest {
        TransportManifest {
            host_id: Some("saas-prod".into()),
            tier_hint: Some(HostTier::Direct),
            custody: Some(HostCustody::Inject),
            ingress: IngressConfig {
                public_url: Some("https://hooks.saas.com/apxm".into()),
                webhook_base_path: None,
            },
            runtime: RuntimeConfig { local_agent: false },
            push: None,
            confinement: None,
            labels: vec![],
        }
    }

    fn link_tools_manifest() -> TransportManifest {
        TransportManifest {
            host_id: Some("browser-x7f".into()),
            tier_hint: Some(HostTier::LinkTools),
            custody: Some(HostCustody::Token),
            ingress: IngressConfig::default(),
            runtime: RuntimeConfig { local_agent: false },
            push: None,
            confinement: None,
            labels: vec![],
        }
    }

    fn link_runtime_manifest() -> TransportManifest {
        TransportManifest {
            host_id: Some("ide-abc".into()),
            tier_hint: Some(HostTier::LinkRuntime),
            custody: Some(HostCustody::Token),
            ingress: IngressConfig::default(),
            runtime: RuntimeConfig { local_agent: true },
            push: None,
            confinement: Some(ConfinementProfile {
                mechanism: ConfinementMechanism::OsSandbox,
                network: ConfinementNetwork::EgressAllowlist,
                fs_scope: ConfinementFsScope::WorkdirOnly,
                writable: true,
            }),
            labels: vec![],
        }
    }

    #[test]
    fn direct_public_ingress_resolves_to_direct() {
        let m = direct_manifest();
        let q = TierQuery {
            transport: "direct",
            op_kind: None,
        };
        assert_eq!(select_tier(&m, &q).unwrap(), HostTier::Direct);
    }

    #[test]
    fn enrolled_host_no_local_runtime_resolves_to_link_tools() {
        let m = link_tools_manifest();
        let q = TierQuery {
            transport: "link",
            op_kind: Some(OpKind::Standard),
        };
        assert_eq!(select_tier(&m, &q).unwrap(), HostTier::LinkTools);
    }

    #[test]
    fn local_agent_spawn_op_resolves_to_link_runtime() {
        let m = link_runtime_manifest();
        let q = TierQuery {
            transport: "link",
            op_kind: Some(OpKind::Spawn),
        };
        assert_eq!(select_tier(&m, &q).unwrap(), HostTier::LinkRuntime);
    }

    #[test]
    fn no_ingress_no_host_id_fails_closed() {
        let m = TransportManifest::default();
        let q = TierQuery {
            transport: "link",
            op_kind: None,
        };
        assert!(matches!(
            select_tier(&m, &q),
            Err(EnrollError::NoReachability)
        ));
    }

    #[test]
    fn link_runtime_without_confinement_fails() {
        let mut m = link_runtime_manifest();
        m.confinement = None;
        let q = TierQuery {
            transport: "link",
            op_kind: Some(OpKind::Spawn),
        };
        assert!(matches!(
            select_tier(&m, &q),
            Err(EnrollError::MissingConfinement)
        ));
    }

    #[test]
    fn mixed_host_selects_tier_per_capability() {
        let m = TransportManifest {
            host_id: Some("hybrid".into()),
            ingress: IngressConfig {
                public_url: Some("https://hooks.hybrid.com/apxm".into()),
                webhook_base_path: None,
            },
            runtime: RuntimeConfig { local_agent: false },
            tier_hint: None,
            custody: Some(HostCustody::Token),
            push: None,
            confinement: None,
            labels: vec![],
        };
        let q_direct = TierQuery {
            transport: "direct",
            op_kind: None,
        };
        let q_link = TierQuery {
            transport: "link",
            op_kind: Some(OpKind::Standard),
        };
        assert_eq!(select_tier(&m, &q_direct).unwrap(), HostTier::Direct);
        assert_eq!(select_tier(&m, &q_link).unwrap(), HostTier::LinkTools);
    }

    // ── Spec-vector conformance: the 3 canonical tier archetypes ─────────────
    //
    // These tests pin the exact (manifest, query) → tier mapping that the spec
    // defines. They are the normative reference; other tests may extend coverage
    // but must not contradict these vectors.

    #[test]
    fn spec_vector_t0_direct_no_link_webhook_in_proxy_out() {
        // T0 DIRECT: no Link; public webhook in, /proxy out. Agent runs APXM-side.
        // Requirement: transport="direct" AND ingress.public_url is set → T0
        let m = TransportManifest {
            host_id: None, // no Link identity required for T0
            tier_hint: None,
            custody: Some(HostCustody::Inject), // T0 uses custody=inject
            ingress: IngressConfig {
                public_url: Some("https://hooks.saas.example/apxm".into()),
                webhook_base_path: None,
            },
            runtime: RuntimeConfig { local_agent: false },
            push: None,
            confinement: None,
            labels: vec![],
        };
        let q = TierQuery {
            transport: "direct",
            op_kind: None,
        };
        assert_eq!(
            select_tier(&m, &q).unwrap(),
            HostTier::Direct,
            "spec vector T0: public-ingress direct transport must yield DIRECT"
        );
    }

    #[test]
    fn spec_vector_t1_link_tools_host_dialed_link_no_local_runtime() {
        // LINK-TOOLS: host-dialed Link. Host contributes tools/events. Agent runs APXM-side.
        // Requirement: host_id present, runtime.local_agent=false (or Standard op_kind)
        let m = TransportManifest {
            host_id: Some("browser-x7f".into()),
            tier_hint: None,
            custody: Some(HostCustody::Token),
            ingress: IngressConfig::default(),
            runtime: RuntimeConfig { local_agent: false },
            push: None,
            confinement: None,
            labels: vec![],
        };
        let q = TierQuery {
            transport: "link",
            op_kind: Some(OpKind::Standard),
        };
        assert_eq!(
            select_tier(&m, &q).unwrap(),
            HostTier::LinkTools,
            "spec: LINK-TOOLS — enrolled host without local runtime must yield LINK-TOOLS"
        );
    }

    #[test]
    fn spec_vector_t2_link_runtime_host_forks_acp_child_locally() {
        // LINK-RUNTIME: host-dialed Link. Host also forks an ACP child locally.
        // Requirement: runtime.local_agent=true AND op requires local execution AND confinement set
        let m = TransportManifest {
            host_id: Some("ide-abc".into()),
            tier_hint: None,
            custody: Some(HostCustody::Token),
            ingress: IngressConfig::default(),
            runtime: RuntimeConfig { local_agent: true },
            push: None,
            confinement: Some(ConfinementProfile {
                mechanism: ConfinementMechanism::OsSandbox,
                network: ConfinementNetwork::EgressAllowlist,
                fs_scope: ConfinementFsScope::WorkdirOnly,
                writable: true,
            }),
            labels: vec![],
        };
        let q = TierQuery {
            transport: "link",
            op_kind: Some(OpKind::Spawn),
        };
        assert_eq!(
            select_tier(&m, &q).unwrap(),
            HostTier::LinkRuntime,
            "spec: LINK-RUNTIME — local_agent host with Spawn op and confinement must yield LINK-RUNTIME"
        );
    }

    #[test]
    fn confinement_satisfaction_ordering() {
        let strong = ConfinementProfile {
            mechanism: ConfinementMechanism::Container,
            network: ConfinementNetwork::Isolated,
            fs_scope: ConfinementFsScope::WorkdirOnly,
            writable: false,
        };
        let weak = ConfinementProfile {
            mechanism: ConfinementMechanism::OsSandbox,
            network: ConfinementNetwork::EgressAllowlist,
            fs_scope: ConfinementFsScope::WorkdirOnly,
            writable: true,
        };
        assert!(strong.satisfies(&weak));
        assert!(!weak.satisfies(&strong));
    }
}
