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
pub enum HostTier {
    /// No Link; public webhook in, `/proxy` out. Agent runs APXM-side.
    #[serde(rename = "DIRECT")]
    Direct,
    /// Host-dialed Link. Host contributes tools/events. Agent runs APXM-side.
    #[serde(rename = "LINK-TOOLS")]
    LinkTools,
    /// Host-dialed Link. Host also forks an ACP child locally.
    #[serde(rename = "LINK-RUNTIME")]
    LinkRuntime,
}

impl std::fmt::Display for HostTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostTier::Direct => f.write_str("DIRECT"),
            HostTier::LinkTools => f.write_str("LINK-TOOLS"),
            HostTier::LinkRuntime => f.write_str("LINK-RUNTIME"),
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
    #[error("DIRECT host must use custody=inject")]
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
    if let Some(hint) = manifest.tier_hint
        && hint != tier
        && !(hint == HostTier::LinkTools && tier == HostTier::LinkRuntime)
    {
        return Err(EnrollError::TierMismatch {
            hint,
            computed: tier,
        });
    }

    // LINK-RUNTIME requires a confinement block
    if tier == HostTier::LinkRuntime && manifest.confinement.is_none() {
        return Err(EnrollError::MissingConfinement);
    }

    // DIRECT egress keeps credentials APXM-side and injects them through /proxy.
    if tier == HostTier::Direct && manifest.custody != Some(HostCustody::Inject) {
        return Err(EnrollError::CustodyMismatch);
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

#[path = "generated_host_execution_manifest.rs"]
mod generated_host_execution_manifest;

pub use generated_host_execution_manifest::HostExecutionManifest;

// These product-neutral effect records are owned by Agents because the runtime
// creates the request and consumes the terminal result. Downstream host SDKs
// implement this wire contract; Agents must not depend on any downstream SDK.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostTypedPayload {
    pub schema_id: String,
    pub value: serde_json::Value,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostEffectRequest {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: u64,
    pub invocation_id: String,
    pub call_id: String,
    pub capability_id: String,
    pub agent_identity_ref: String,
    pub host_op: String,
    pub capability_binding: String,
    pub implementation_ref: String,
    pub request_digest: String,
    pub idempotency_key: String,
    pub expected_host_key_id: String,
    pub grant_refs: Vec<String>,
    pub approval_refs: Vec<String>,
    pub args: serde_json::Value,
    pub acting_principal_attestation: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostEffectOutcome {
    #[serde(rename = "committed")]
    Committed,
    #[serde(rename = "deduplicated")]
    Deduplicated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostEffectRejectionCategory {
    #[serde(rename = "validation")]
    Validation,
    #[serde(rename = "not_found")]
    NotFound,
    #[serde(rename = "authority")]
    Authority,
    #[serde(rename = "approval")]
    Approval,
    #[serde(rename = "conflict")]
    Conflict,
    #[serde(rename = "unsupported")]
    Unsupported,
    #[serde(rename = "unavailable")]
    Unavailable,
    #[serde(rename = "outcome_unknown")]
    OutcomeUnknown,
    #[serde(rename = "internal")]
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostEffectCommit {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: u64,
    pub invocation_id: String,
    pub call_id: String,
    pub request_digest: String,
    pub idempotency_key: String,
    pub effect_digest: String,
    pub effect_outcome: HostEffectOutcome,
    pub result: HostTypedPayload,
    pub host_key_id: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostEffectRejection {
    pub execution_id: String,
    pub graph_id: String,
    pub node_id: u64,
    pub invocation_id: String,
    pub call_id: String,
    pub request_digest: String,
    pub idempotency_key: String,
    pub category: HostEffectRejectionCategory,
    pub failure: HostTypedPayload,
}

/// Terminal result of one downstream effect dispatch. This is an internal SDK
/// seam over the two closed wire records and adds no serialized discriminant.
#[derive(Debug, Clone)]
pub enum HostEffectAdapterResult {
    Commit(HostEffectCommit),
    Rejection(HostEffectRejection),
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

    /// Execute a negotiated AHI v2 host effect and preserve its typed terminal result.
    ///
    /// This is deliberately separate from `call_tool`: Link v1 tool responses
    /// never become replay authority.
    async fn execute_effect(
        &self,
        _host_id: &str,
        _effect: HostEffectRequest,
    ) -> Result<HostEffectAdapterResult, HostDispatchError> {
        Err(HostDispatchError::Transport(
            "durable host effects are not configured".into(),
        ))
    }

    async fn request_relay_egress(
        &self,
        host_id: &str,
        request: HostProxyRequest,
    ) -> Result<HostProxyResult, HostDispatchError>;

    async fn request_permission(
        &self,
        host_id: &str,
        prompt: serde_json::Value,
        timeout_ms: u64,
    ) -> Result<HostPromptApproval, HostDispatchError>;

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
#[serde(deny_unknown_fields)]
pub struct HostToolCall {
    pub call_id: String,
    pub capability_id: String,
    pub host_op: String,
    pub capability_binding: String,
    pub args: serde_json::Value,
    pub args_digest: String,
    pub grant_ref: Option<String>,
    pub acting_principal_attestation: serde_json::Value,
    pub timeout_ms: u64,
    pub idempotency_key: Option<String>,
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
    pub credential: serde_json::Value,
    pub method: String,
    pub url: String,
    pub headers: Option<serde_json::Value>,
    pub body_b64: Option<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostProxyResult {
    pub call_id: String,
    pub status: Option<u16>,
    pub headers: Option<serde_json::Value>,
    pub body_b64: Option<String>,
    #[serde(rename = "ref")]
    pub result_ref: Option<serde_json::Value>,
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPromptApproval {
    pub call_id: String,
    pub grant_id: String,
    pub approved: bool,
    pub approvals: Vec<serde_json::Value>,
    pub reason: Option<String>,
    pub host_pubkey_hex: Option<String>,
    pub args_digest: Option<String>,
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

    fn valid_host_tool_call_json() -> serde_json::Value {
        serde_json::json!({
            "call_id": "call-123",
            "capability_id": "capability-456",
            "host_op": "documents.read",
            "capability_binding": "binding-789",
            "args": {"document_id": "document-012"},
            "args_digest": "sha256:args",
            "grant_ref": "grant-345",
            "timeout_ms": 5_000,
            "idempotency_key": "idempotency-678",
            "acting_principal_attestation": {
                "proof": ["opaque", 7, true],
                "nested": {"bytes": "AAE="}
            }
        })
    }

    #[test]
    fn host_tool_call_accepts_an_opaque_acting_principal_attestation() {
        let encoded = valid_host_tool_call_json();
        let expected = encoded["acting_principal_attestation"].clone();

        let call: HostToolCall = serde_json::from_value(encoded).unwrap();
        let round_tripped = serde_json::to_value(call).unwrap();

        assert_eq!(round_tripped["acting_principal_attestation"], expected);
    }

    #[test]
    fn host_tool_call_rejects_a_missing_acting_principal_attestation() {
        let mut encoded = valid_host_tool_call_json();
        encoded
            .as_object_mut()
            .unwrap()
            .remove("acting_principal_attestation");

        assert!(serde_json::from_value::<HostToolCall>(encoded).is_err());
    }

    #[test]
    fn host_tool_call_rejects_subject() {
        let mut encoded = valid_host_tool_call_json();
        encoded["subject"] = serde_json::json!("caller-supplied-subject");

        assert!(serde_json::from_value::<HostToolCall>(encoded).is_err());
    }

    #[test]
    fn host_tool_call_rejects_end_user_subject() {
        let mut encoded = valid_host_tool_call_json();
        encoded["end_user_subject"] = serde_json::json!("caller-supplied-subject");

        assert!(serde_json::from_value::<HostToolCall>(encoded).is_err());
    }

    #[test]
    fn host_tool_call_rejects_any_other_unknown_field() {
        let mut encoded = valid_host_tool_call_json();
        encoded["unexpected_contract_extension"] = serde_json::json!(true);

        assert!(serde_json::from_value::<HostToolCall>(encoded).is_err());
    }

    #[test]
    fn host_effect_dispatch_preserves_a_typed_rejection() {
        let terminal = HostEffectAdapterResult::Rejection(HostEffectRejection {
            execution_id: "execution-1".into(),
            graph_id: "graph-1".into(),
            node_id: 7,
            invocation_id: "invocation-1".into(),
            call_id: "call-1".into(),
            request_digest: format!("sha256:{}", "a".repeat(64)),
            idempotency_key: "effect-1".into(),
            category: HostEffectRejectionCategory::Conflict,
            failure: HostTypedPayload {
                schema_id: "apxm.example-effect-failure.v1".into(),
                value: serde_json::json!({"code": "example.effect.conflict"}),
                digest: format!("sha256:{}", "b".repeat(64)),
            },
        });

        let HostEffectAdapterResult::Rejection(rejection) = terminal else {
            panic!("typed Host rejection was collapsed into a commit");
        };
        assert_eq!(rejection.category, HostEffectRejectionCategory::Conflict);
        assert_eq!(
            rejection.failure.schema_id,
            "apxm.example-effect-failure.v1"
        );
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
        let direct = TransportManifest {
            host_id: Some("hybrid".into()),
            ingress: IngressConfig {
                public_url: Some("https://hooks.hybrid.com/apxm".into()),
                webhook_base_path: None,
            },
            runtime: RuntimeConfig { local_agent: false },
            tier_hint: None,
            custody: Some(HostCustody::Inject),
            push: None,
            confinement: None,
            labels: vec![],
        };
        let link = TransportManifest {
            custody: Some(HostCustody::Token),
            ..direct.clone()
        };
        let q_direct = TierQuery {
            transport: "direct",
            op_kind: None,
        };
        let q_link = TierQuery {
            transport: "link",
            op_kind: Some(OpKind::Standard),
        };
        assert_eq!(select_tier(&direct, &q_direct).unwrap(), HostTier::Direct);
        assert_eq!(select_tier(&link, &q_link).unwrap(), HostTier::LinkTools);
    }

    #[test]
    fn direct_with_token_custody_fails_closed() {
        let mut m = direct_manifest();
        m.custody = Some(HostCustody::Token);
        let q = TierQuery {
            transport: "direct",
            op_kind: None,
        };
        assert!(matches!(
            select_tier(&m, &q),
            Err(EnrollError::CustodyMismatch)
        ));
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
