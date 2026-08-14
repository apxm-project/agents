//! Capability interception hooks.

use apxm_core::events::payload::{ApprovalRiskLevel, ToolCallCorrelation};
use apxm_core::types::capability::PermissionDecision;
use apxm_core::types::consent::{
    ConsentBroker, ConsentDecision, PermissionPrompt, PromptMode, RiskLevel,
};
use apxm_core::types::values::Value;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

// Imported directly from the interface crate (not via `crate::ExecutionEventEmitter`,
// which is only a re-export at the `apxm-runtime` lib root) — capability's
// interceptor pipeline depends on `apxm-capability-iface` for this trait, not
// on anything in `apxm-runtime`'s own `executor` module.
use super::metadata::RuntimeCapability;
use super::registry::CapabilityRegistry;
use apxm_capability_iface::events::ExecutionEventEmitter;

/// Default approval wait when `APXM_PERMISSION_TIMEOUT_SECS` is unset (matches server broker TTL).
pub const DEFAULT_PERMISSION_TIMEOUT_SECS: u64 = 120;

/// Execution-scoped context for approval-gated capability invocation.
///
/// An approval-gated invocation carries the identity that grants it:
/// `agent_code` names the acting agent and `grant_id` names the grant under
/// which the effect is admitted. Both are supplied by the caller; the gate
/// never substitutes a value for either one.
pub struct PreInvokeContext<'a> {
    pub registry: &'a CapabilityRegistry,
    pub call_id: &'a str,
    pub tool_call_correlation: Option<&'a ToolCallCorrelation>,
    pub consent_broker: &'a dyn ConsentBroker,
    pub event_emitter: Option<&'a dyn ExecutionEventEmitter>,
    pub host_id: Option<&'a str>,
    pub agent_code: Option<&'a str>,
    pub grant_id: Option<&'a str>,
    pub permission_timeout: Duration,
}

/// Identity field an approval-gated invocation failed to carry.
///
/// Approval evidence attributes an effect to the identity that granted it, so
/// an absent field is a denial rather than a substituted value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingInvocationIdentity {
    /// No acting agent was supplied.
    AgentCode,
    /// No grant was supplied.
    GrantId,
}

impl MissingInvocationIdentity {
    /// Exact context field name that was absent.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::AgentCode => "agent_code",
            Self::GrantId => "grant_id",
        }
    }
}

impl std::fmt::Display for MissingInvocationIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.field())
    }
}

/// Acting identity resolved from a [`PreInvokeContext`].
#[derive(Debug, Clone, Copy)]
pub struct InvocationIdentity<'a> {
    /// Agent the effect is attributed to.
    pub agent_code: &'a str,
    /// Grant the effect is admitted under.
    pub grant_id: &'a str,
}

impl<'a> PreInvokeContext<'a> {
    /// Resolve timeout from `APXM_PERMISSION_TIMEOUT_SECS` (default 120s).
    pub fn permission_timeout_from_env() -> Duration {
        std::env::var(apxm_core::constants::env::APXM_PERMISSION_TIMEOUT_SECS)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&s| s > 0)
            .map_or_else(
                || Duration::from_secs(DEFAULT_PERMISSION_TIMEOUT_SECS),
                Duration::from_secs,
            )
    }

    /// Resolve the identity an approval-gated effect is attributed to.
    ///
    /// # Errors
    ///
    /// Returns the absent field when the context carries no acting agent or no
    /// grant. Neither field has a substitute.
    pub fn invocation_identity(&self) -> Result<InvocationIdentity<'a>, MissingInvocationIdentity> {
        let agent_code = self
            .agent_code
            .filter(|value| !value.is_empty())
            .ok_or(MissingInvocationIdentity::AgentCode)?;
        let grant_id = self
            .grant_id
            .filter(|value| !value.is_empty())
            .ok_or(MissingInvocationIdentity::GrantId)?;
        Ok(InvocationIdentity {
            agent_code,
            grant_id,
        })
    }
}

/// Deterministic digest of capability args for consent signing.
pub fn args_digest_for(args: &HashMap<String, Value>) -> String {
    let json: HashMap<String, serde_json::Value> = args
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.to_json()
                    .unwrap_or_else(|_| serde_json::Value::String(v.to_string())),
            )
        })
        .collect();
    let canonical = serde_json::to_string(&json).unwrap_or_default();
    format!("blake3:{}", blake3::hash(canonical.as_bytes()).to_hex())
}

/// Approval gate for capabilities declaring `requires_approval` in metadata.
///
/// Called from `CapabilitySystem::invoke_with_timeout` when a [`PreInvokeContext`]
/// is supplied. Fail-closed on deny or timeout.
pub async fn pre_invoke_ctx(
    ctx: &PreInvokeContext<'_>,
    name: &str,
    args: &HashMap<String, Value>,
) -> InterceptDecision {
    let Some(cap) = ctx.registry.get(name) else {
        return InterceptDecision::allow();
    };
    pre_invoke_policy_ctx(ctx, name, args, cap.metadata().requires_approval).await
}

/// Approval gate for an invocation whose policy is supplied by the caller.
///
/// Script-backed capabilities are artifact-scoped rather than process-wide, so
/// their policy metadata is admitted here without registering an executor in
/// the global capability registry.
pub(crate) async fn pre_invoke_policy_ctx(
    ctx: &PreInvokeContext<'_>,
    name: &str,
    args: &HashMap<String, Value>,
    requires_approval: bool,
) -> InterceptDecision {
    if !requires_approval {
        return InterceptDecision::allow();
    }

    let identity = match ctx.invocation_identity() {
        Ok(identity) => identity,
        Err(missing) => {
            return InterceptDecision::deny(format!(
                "capability '{name}' requires approval but the invocation context carries no {missing}"
            ));
        }
    };

    let prompt_id = uuid::Uuid::new_v4().to_string();
    let args_digest = args_digest_for(args);
    let args_preview = serde_json::to_value(args).unwrap_or_else(|_| serde_json::json!({}));
    let expires_at = (chrono::Utc::now()
        + chrono::Duration::seconds(
            i64::try_from(ctx.permission_timeout.as_secs()).unwrap_or(i64::MAX),
        ))
    .to_rfc3339();

    if let Some(emitter) = ctx.event_emitter {
        emitter.emit_approval_request_with_correlation(
            identity.agent_code,
            name,
            &prompt_id,
            ApprovalRiskLevel::High,
            ctx.tool_call_correlation,
        );
    }

    let prompt = PermissionPrompt {
        prompt_id: prompt_id.clone(),
        call_id: ctx.call_id.to_string(),
        grant_id: identity.grant_id.to_string(),
        capability_id: name.to_string(),
        capability_binding: name.to_string(),
        host_id: ctx.host_id.map(str::to_string),
        operation: "invoke".to_string(),
        mode: PromptMode::Confirm,
        subject: None,
        args_digest,
        args_preview,
        risk_level: RiskLevel::High,
        expires_at,
        channel_id: None,
        description: Some(format!("Capability '{name}' requires explicit approval")),
        target_ref: None,
        resource: None,
        diff_ref: None,
    };

    let decision = ctx
        .consent_broker
        .request_consent(prompt, ctx.permission_timeout)
        .await;

    let resolution = decision.resolution();
    if let Some(emitter) = ctx.event_emitter {
        emitter.emit_approval_resolved_with_correlation(
            &prompt_id,
            resolution,
            ctx.tool_call_correlation,
        );
    }

    match decision {
        ConsentDecision::Approved { .. } => InterceptDecision::allow(),
        ConsentDecision::Denied { reason } => InterceptDecision::deny(reason),
        ConsentDecision::Expired => InterceptDecision::deny(format!(
            "approval for capability '{name}' timed out after {}s",
            ctx.permission_timeout.as_secs()
        )),
    }
}

/// Collect capability names that declare `requires_auth`.
pub fn requires_auth_names(metadata: &[RuntimeCapability]) -> HashSet<String> {
    metadata
        .iter()
        .filter(|m| m.requires_auth)
        .map(|m| m.name.clone())
        .collect()
}

/// Collect capability names that declare `requires_approval`.
pub fn requires_approval_names(metadata: &[RuntimeCapability]) -> HashSet<String> {
    metadata
        .iter()
        .filter(|m| m.requires_approval)
        .map(|m| m.name.clone())
        .collect()
}

/// Decision returned from `pre_invoke`.
///
/// The chokepoint speaks the one permission vocabulary rather than a parallel
/// spelling of it, and it can only allow or deny: `Ask` is a *pre*-chokepoint
/// posture that the approval gate has already resolved into one of the two by
/// the time an interceptor runs, so an `Ask` arriving here has no broker left
/// to raise it and becomes a denial carrying the reason the gate gave.
///
/// The vocabulary is deliberately this narrow. This is the only decision that
/// gates capability execution, so any third outcome — notably one that
/// rewrote the arguments after admission — would be a power to change what
/// runs, held by whatever registered an interceptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterceptDecision(PermissionDecision);

impl InterceptDecision {
    /// Continue execution unchanged.
    #[must_use]
    pub const fn allow() -> Self {
        Self(PermissionDecision::allow())
    }

    /// Refuse the invocation, naming why.
    #[must_use]
    pub fn deny(reason: impl Into<String>) -> Self {
        Self(PermissionDecision::deny(reason))
    }

    /// Project a resolved permission decision onto the chokepoint, failing
    /// closed: anything that is not an outright `Allow` denies here.
    #[must_use]
    pub fn from_effect(effect: PermissionDecision) -> Self {
        if effect.is_allow() {
            return Self::allow();
        }
        Self(PermissionDecision::deny(
            effect
                .reason()
                .map_or_else(|| effect.as_str().to_string(), str::to_string),
        ))
    }

    /// The decision in the shared permission vocabulary.
    #[must_use]
    pub const fn effect(&self) -> &PermissionDecision {
        &self.0
    }

    /// Why the invocation was refused, or `None` when it was allowed.
    #[must_use]
    pub fn denial_reason(&self) -> Option<&str> {
        if self.0.is_allow() {
            return None;
        }
        Some(self.0.reason().unwrap_or_else(|| self.0.as_str()))
    }
}

/// Optional interception hook around capability execution.
#[async_trait]
pub trait CapabilityInterceptor: Send + Sync {
    /// Stable interceptor name for deduplication.
    fn name(&self) -> &'static str {
        "interceptor"
    }

    /// Called before capability execution.
    async fn pre_invoke(&self, _name: &str, _args: &HashMap<String, Value>) -> InterceptDecision {
        InterceptDecision::allow()
    }

    /// Called after capability execution (success path).
    async fn post_invoke(&self, _name: &str, _result: &Value) {}
}

/// Permission gate registered at the trusted invoke chokepoint to complement
/// the in-handler write boundary. It activates
/// the `requires_auth` capability-metadata flag: a capability that declares it
/// needs authentication but is invoked without a resolved credential is denied in
/// strict mode, or warned about otherwise (advisory is the default so legitimate
/// unauthenticated tools keep working). Because every tool call — both the graph
/// `INV_CAP` path and the in-`ASK`-node model loop — funnels through
/// `invoke_with_timeout`, this is a single always-invoked policy-enforcement
/// point. It is composed by the trusted runtime, never by the AIR program.
pub struct PermissionInterceptor {
    requires_auth: HashSet<String>,
    strict: bool,
}

impl PermissionInterceptor {
    /// Build the gate from the set of capability names that declare
    /// `requires_auth`. `strict` denies (vs. warns) on a missing credential.
    pub fn new(requires_auth: HashSet<String>, strict: bool) -> Self {
        Self {
            requires_auth,
            strict,
        }
    }

    /// Whether the call carries a resolved credential: an explicit `credential`
    /// connection id, a top-level auth arg, or an injected
    /// `headers.Authorization` (the provider.call resolve-and-inject shape).
    fn args_have_credential(args: &HashMap<String, Value>) -> bool {
        if args.contains_key("credential")
            || args.contains_key("authorization")
            || args.contains_key("api_key")
        {
            return true;
        }
        if let Some(Value::Object(headers)) = args.get("headers") {
            return headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("authorization"));
        }
        false
    }
}

#[async_trait]
impl CapabilityInterceptor for PermissionInterceptor {
    fn name(&self) -> &'static str {
        "permission"
    }

    async fn pre_invoke(&self, name: &str, args: &HashMap<String, Value>) -> InterceptDecision {
        if self.requires_auth.contains(name) && !Self::args_have_credential(args) {
            if self.strict {
                return InterceptDecision::deny(format!(
                    "capability '{name}' requires authentication but no credential was \
                     resolved; bind one (e.g. `--tool-auth {name}=<connection_id>`)"
                ));
            }
            tracing::warn!(
                capability = %name,
                "requires_auth capability invoked without a resolved credential \
                 (advisory; set APXM_REQUIRE_AUTH_STRICT=1 to enforce)"
            );
        }
        InterceptDecision::allow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{CapabilityExecutor, EchoCapability};
    use apxm_core::types::consent::{
        ApprovalEvidence, ApprovalResolution, ConsentBroker, ConsentDecision, InteractiveApproval,
    };
    use std::sync::Arc;

    struct RecordingEmitter {
        requests: parking_lot::Mutex<Vec<(String, String, String)>>,
        resolutions: parking_lot::Mutex<Vec<(String, ApprovalResolution)>>,
    }

    impl RecordingEmitter {
        fn new() -> Self {
            Self {
                requests: parking_lot::Mutex::new(Vec::new()),
                resolutions: parking_lot::Mutex::new(Vec::new()),
            }
        }
    }

    impl ExecutionEventEmitter for RecordingEmitter {
        fn emit_llm_token(&self, _content: &str) {}
        fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}
        fn emit_tool_end(&self, _name: &str, _result: &Value) {}

        fn emit_approval_request(
            &self,
            agent_code: &str,
            tool_name: &str,
            approval_id: &str,
            _risk_level: apxm_core::events::payload::ApprovalRiskLevel,
        ) {
            self.requests.lock().push((
                agent_code.to_string(),
                tool_name.to_string(),
                approval_id.to_string(),
            ));
        }

        fn emit_approval_resolved(&self, approval_id: &str, decision: ApprovalResolution) {
            self.resolutions
                .lock()
                .push((approval_id.to_string(), decision));
        }
    }

    struct StubBroker {
        decision: ConsentDecision,
        prompts: parking_lot::Mutex<Vec<PermissionPrompt>>,
    }

    impl StubBroker {
        fn new(decision: ConsentDecision) -> Self {
            Self {
                decision,
                prompts: parking_lot::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl ConsentBroker for StubBroker {
        async fn request_consent(
            &self,
            prompt: PermissionPrompt,
            _timeout: Duration,
        ) -> ConsentDecision {
            self.prompts.lock().push(prompt);
            self.decision.clone()
        }
    }

    fn gated_echo() -> Arc<dyn CapabilityExecutor> {
        let schema = serde_json::json!({"type": "object"});
        let meta = RuntimeCapability::new("gated-echo", "echo with approval", schema)
            .with_requires_approval();
        struct GatedEcho {
            meta: RuntimeCapability,
        }
        #[async_trait::async_trait]
        impl CapabilityExecutor for GatedEcho {
            async fn execute(
                &self,
                args: HashMap<String, Value>,
            ) -> Result<Value, apxm_core::error::RuntimeError> {
                EchoCapability::new().execute(args).await
            }
            fn metadata(&self) -> &RuntimeCapability {
                &self.meta
            }
        }
        Arc::new(GatedEcho { meta })
    }

    /// The chokepoint that gates every capability invocation can say exactly
    /// two things. It carries no third outcome — in particular none that
    /// rewrites the arguments after admission — so whatever registered an
    /// interceptor cannot change what runs, only whether it runs.
    #[test]
    fn the_chokepoint_can_only_allow_or_deny() {
        assert_eq!(InterceptDecision::allow().denial_reason(), None);
        assert_eq!(
            InterceptDecision::deny("not admitted").denial_reason(),
            Some("not admitted")
        );
        assert_eq!(
            InterceptDecision::from_effect(PermissionDecision::allow()),
            InterceptDecision::allow()
        );
        // `Ask` has no broker left here, so it refuses and says why.
        assert_eq!(
            InterceptDecision::from_effect(PermissionDecision::ask("confirm each send"))
                .denial_reason(),
            Some("confirm each send")
        );
        // A refusal that gives no reason still refuses.
        assert_eq!(
            InterceptDecision::from_effect(PermissionDecision::Deny { reason: None })
                .denial_reason(),
            Some("deny")
        );
    }

    /// An interceptor sees the arguments and decides on them; the arguments the
    /// implementation receives are the ones admission carried, unchanged.
    #[tokio::test]
    async fn an_interceptor_cannot_change_the_arguments_it_inspects() {
        struct Inspector;
        #[async_trait]
        impl CapabilityInterceptor for Inspector {
            async fn pre_invoke(
                &self,
                _name: &str,
                args: &HashMap<String, Value>,
            ) -> InterceptDecision {
                assert_eq!(args.get("message"), Some(&Value::String("hi".into())));
                InterceptDecision::allow()
            }
        }

        let system = crate::CapabilitySystem::new();
        system.register(Arc::new(EchoCapability::new())).unwrap();
        system.register_interceptor(Arc::new(Inspector));
        let mut args = HashMap::new();
        args.insert("message".to_string(), Value::String("hi".to_string()));

        let result = system.invoke("echo", args).await.expect("echo runs");
        assert!(
            result.to_string().contains("hi"),
            "the implementation received the authored argument: {result}"
        );
    }

    #[tokio::test]
    async fn pre_invoke_ctx_allows_on_approval() {
        let registry = CapabilityRegistry::new();
        registry.register(gated_echo()).unwrap();
        let broker = StubBroker::new(ConsentDecision::Approved {
            evidence: ApprovalEvidence::Interactive(InteractiveApproval {
                decided_at: "2026-01-01T00:00:00Z".into(),
                responder_subject: Some("user".into()),
            }),
        });
        let emitter = RecordingEmitter::new();
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-1",
            tool_call_correlation: None,
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: Some("host-1"),
            agent_code: Some("agent-a"),
            grant_id: Some("grant-a"),
            permission_timeout: Duration::from_secs(5),
        };
        let mut args = HashMap::new();
        args.insert("message".to_string(), Value::String("hi".to_string()));
        let decision = pre_invoke_ctx(&ctx, "gated-echo", &args).await;
        assert_eq!(decision, InterceptDecision::allow());
        assert_eq!(emitter.requests.lock().len(), 1);
        assert_eq!(
            emitter.resolutions.lock()[0].1,
            ApprovalResolution::Approved
        );
    }

    #[tokio::test]
    async fn pre_invoke_ctx_attributes_the_prompt_to_the_supplied_identity() {
        let registry = CapabilityRegistry::new();
        registry.register(gated_echo()).unwrap();
        let broker = StubBroker::new(ConsentDecision::Approved {
            evidence: ApprovalEvidence::Interactive(InteractiveApproval {
                decided_at: "2026-01-01T00:00:00Z".into(),
                responder_subject: Some("user".into()),
            }),
        });
        let emitter = RecordingEmitter::new();
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-identity",
            tool_call_correlation: None,
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: Some("host-1"),
            agent_code: Some("agent-a"),
            grant_id: Some("grant-a"),
            permission_timeout: Duration::from_secs(5),
        };

        let decision = pre_invoke_ctx(&ctx, "gated-echo", &HashMap::new()).await;

        assert_eq!(decision, InterceptDecision::allow());
        let prompts = broker.prompts.lock();
        assert_eq!(prompts.len(), 1);
        assert_eq!(
            prompts[0].grant_id, "grant-a",
            "the prompt carries the grant supplied by the caller"
        );
        assert_eq!(
            emitter.requests.lock()[0].0,
            "agent-a",
            "the approval request names the agent supplied by the caller"
        );
    }

    #[tokio::test]
    async fn pre_invoke_ctx_denies_a_gated_call_carrying_no_grant() {
        let registry = CapabilityRegistry::new();
        registry.register(gated_echo()).unwrap();
        let broker = StubBroker::new(ConsentDecision::Approved {
            evidence: ApprovalEvidence::Interactive(InteractiveApproval {
                decided_at: "2026-01-01T00:00:00Z".into(),
                responder_subject: Some("user".into()),
            }),
        });
        let emitter = RecordingEmitter::new();
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-no-grant",
            tool_call_correlation: None,
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: Some("host-1"),
            agent_code: Some("agent-a"),
            grant_id: None,
            permission_timeout: Duration::from_secs(5),
        };

        let decision = pre_invoke_ctx(&ctx, "gated-echo", &HashMap::new()).await;

        assert!(
            decision
                .denial_reason()
                .is_some_and(|reason| reason.contains("grant_id")),
            "a gated call carrying no grant is denied and names the absent field"
        );
        assert!(
            broker.prompts.lock().is_empty(),
            "no prompt is raised for an invocation with no grant"
        );
        assert!(
            emitter.requests.lock().is_empty(),
            "no approval request attributes the effect to a substituted identity"
        );
    }

    #[tokio::test]
    async fn pre_invoke_ctx_denies_a_gated_call_carrying_no_agent_code() {
        let registry = CapabilityRegistry::new();
        registry.register(gated_echo()).unwrap();
        let broker = StubBroker::new(ConsentDecision::Approved {
            evidence: ApprovalEvidence::Interactive(InteractiveApproval {
                decided_at: "2026-01-01T00:00:00Z".into(),
                responder_subject: Some("user".into()),
            }),
        });
        let emitter = RecordingEmitter::new();
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-no-agent",
            tool_call_correlation: None,
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: Some("host-1"),
            agent_code: None,
            grant_id: Some("grant-a"),
            permission_timeout: Duration::from_secs(5),
        };

        let decision = pre_invoke_ctx(&ctx, "gated-echo", &HashMap::new()).await;

        assert!(
            decision
                .denial_reason()
                .is_some_and(|reason| reason.contains("agent_code")),
            "a gated call carrying no acting agent is denied and names the absent field"
        );
        assert!(
            emitter.requests.lock().is_empty(),
            "no approval request attributes the effect to a substituted identity"
        );
    }

    #[tokio::test]
    async fn pre_invoke_ctx_denies_on_timeout() {
        let registry = CapabilityRegistry::new();
        registry.register(gated_echo()).unwrap();
        let broker = StubBroker::new(ConsentDecision::Expired);
        let emitter = RecordingEmitter::new();
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-2",
            tool_call_correlation: None,
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: None,
            agent_code: Some("agent-a"),
            grant_id: Some("grant-a"),
            permission_timeout: Duration::from_secs(1),
        };
        let decision = pre_invoke_ctx(&ctx, "gated-echo", &HashMap::new()).await;
        assert!(decision.denial_reason().is_some());
        assert_eq!(emitter.resolutions.lock()[0].1, ApprovalResolution::Expired);
    }

    #[tokio::test]
    async fn pre_invoke_ctx_denies_with_unavailable_broker_and_emits_public_denied_state() {
        let registry = CapabilityRegistry::new();
        registry.register(gated_echo()).unwrap();
        let broker = apxm_core::types::consent::UnavailableConsentBroker;
        let emitter = RecordingEmitter::new();
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-3",
            tool_call_correlation: None,
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: None,
            agent_code: Some("agent-a"),
            grant_id: Some("grant-a"),
            permission_timeout: Duration::from_secs(1),
        };

        let decision = pre_invoke_ctx(&ctx, "gated-echo", &HashMap::new()).await;

        assert_eq!(
            decision.denial_reason(),
            Some(apxm_core::types::consent::APPROVAL_BROKER_UNAVAILABLE_REASON)
        );
        assert_eq!(emitter.resolutions.lock()[0].1, ApprovalResolution::Denied);
    }

    #[tokio::test]
    async fn pre_invoke_ctx_emits_denied_for_explicit_rejection() {
        let registry = CapabilityRegistry::new();
        registry.register(gated_echo()).unwrap();
        let broker = StubBroker::new(ConsentDecision::Denied {
            reason: "operator rejected".into(),
        });
        let emitter = RecordingEmitter::new();
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-4",
            tool_call_correlation: None,
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: None,
            agent_code: Some("agent-a"),
            grant_id: Some("grant-a"),
            permission_timeout: Duration::from_secs(1),
        };

        let decision = pre_invoke_ctx(&ctx, "gated-echo", &HashMap::new()).await;

        assert_eq!(decision.denial_reason(), Some("operator rejected"));
        assert_eq!(emitter.resolutions.lock()[0].1, ApprovalResolution::Denied);
    }

    #[tokio::test]
    async fn pre_invoke_ctx_skips_non_gated_capabilities() {
        let registry = CapabilityRegistry::new();
        registry.register(Arc::new(EchoCapability::new())).unwrap();
        let broker = StubBroker::new(ConsentDecision::Denied {
            reason: "should not run".into(),
        });
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-5",
            tool_call_correlation: None,
            consent_broker: &broker,
            event_emitter: None,
            host_id: None,
            agent_code: None,
            grant_id: None,
            permission_timeout: Duration::from_secs(1),
        };
        let decision = pre_invoke_ctx(&ctx, "echo", &HashMap::new()).await;
        assert_eq!(decision, InterceptDecision::allow());
    }
}
