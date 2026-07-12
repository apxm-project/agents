//! Capability interception hooks.

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
pub struct PreInvokeContext<'a> {
    pub registry: &'a CapabilityRegistry,
    pub call_id: &'a str,
    pub consent_broker: &'a dyn ConsentBroker,
    pub event_emitter: Option<&'a dyn ExecutionEventEmitter>,
    pub host_id: Option<&'a str>,
    pub agent_code: Option<&'a str>,
    pub grant_id: Option<&'a str>,
    pub permission_timeout: Duration,
}

impl<'a> PreInvokeContext<'a> {
    /// Resolve timeout from `APXM_PERMISSION_TIMEOUT_SECS` (default 120s).
    pub fn permission_timeout_from_env() -> Duration {
        std::env::var(apxm_core::constants::env::APXM_PERMISSION_TIMEOUT_SECS)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&s| s > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(DEFAULT_PERMISSION_TIMEOUT_SECS))
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
        return InterceptDecision::Allow;
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
        return InterceptDecision::Allow;
    }

    let prompt_id = uuid::Uuid::new_v4().to_string();
    let args_digest = args_digest_for(args);
    let args_preview = serde_json::to_value(args).unwrap_or_else(|_| serde_json::json!({}));
    let expires_at = (chrono::Utc::now()
        + chrono::Duration::seconds(ctx.permission_timeout.as_secs() as i64))
    .to_rfc3339();

    let agent_code = ctx.agent_code.unwrap_or("runtime");
    if let Some(emitter) = ctx.event_emitter {
        emitter.emit_approval_request(agent_code, name, &prompt_id, "high");
    }

    let prompt = PermissionPrompt {
        prompt_id: prompt_id.clone(),
        call_id: ctx.call_id.to_string(),
        grant_id: ctx.grant_id.unwrap_or("runtime-grant").to_string(),
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
        emitter.emit_approval_resolved(&prompt_id, resolution);
    }

    match decision {
        ConsentDecision::Approved { .. } => InterceptDecision::Allow,
        ConsentDecision::Denied { reason } => InterceptDecision::Deny { reason },
        ConsentDecision::Expired => InterceptDecision::Deny {
            reason: format!(
                "approval for capability '{name}' timed out after {}s",
                ctx.permission_timeout.as_secs()
            ),
        },
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
#[derive(Debug, Clone)]
pub enum InterceptDecision {
    /// Continue execution unchanged.
    Allow,
    /// Deny invocation with a reason.
    Deny { reason: String },
    /// Continue execution with updated arguments.
    EditArgs { args: HashMap<String, Value> },
}

/// Optional interception hook around capability execution.
#[async_trait]
pub trait CapabilityInterceptor: Send + Sync {
    /// Stable interceptor name for deduplication.
    fn name(&self) -> &str {
        "interceptor"
    }

    /// Called before capability execution.
    async fn pre_invoke(&self, _name: &str, _args: &HashMap<String, Value>) -> InterceptDecision {
        InterceptDecision::Allow
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
    fn name(&self) -> &str {
        "permission"
    }

    async fn pre_invoke(&self, name: &str, args: &HashMap<String, Value>) -> InterceptDecision {
        if self.requires_auth.contains(name) && !Self::args_have_credential(args) {
            if self.strict {
                return InterceptDecision::Deny {
                    reason: format!(
                        "capability '{name}' requires authentication but no credential was \
                         resolved; bind one (e.g. `--tool-auth {name}=<connection_id>`)"
                    ),
                };
            }
            tracing::warn!(
                capability = %name,
                "requires_auth capability invoked without a resolved credential \
                 (advisory; set APXM_REQUIRE_AUTH_STRICT=1 to enforce)"
            );
        }
        InterceptDecision::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{CapabilityExecutor, EchoCapability};
    use apxm_core::types::consent::{
        ApprovalEvidence, ApprovalResolution, ConsentBroker, ConsentDecision,
        InteractiveApproval,
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
            _risk_level: &str,
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
    }

    #[async_trait::async_trait]
    impl ConsentBroker for StubBroker {
        async fn request_consent(
            &self,
            _prompt: PermissionPrompt,
            _timeout: Duration,
        ) -> ConsentDecision {
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

    #[tokio::test]
    async fn pre_invoke_ctx_allows_on_approval() {
        let registry = CapabilityRegistry::new();
        registry.register(gated_echo()).unwrap();
        let broker = StubBroker {
            decision: ConsentDecision::Approved {
                evidence: ApprovalEvidence::Interactive(InteractiveApproval {
                    decided_at: "2026-01-01T00:00:00Z".into(),
                    responder_subject: Some("user".into()),
                }),
            },
        };
        let emitter = RecordingEmitter::new();
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-1",
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: Some("host-1"),
            agent_code: Some("agent-a"),
            grant_id: None,
            permission_timeout: Duration::from_secs(5),
        };
        let mut args = HashMap::new();
        args.insert("message".to_string(), Value::String("hi".to_string()));
        let decision = pre_invoke_ctx(&ctx, "gated-echo", &args).await;
        assert!(matches!(decision, InterceptDecision::Allow));
        assert_eq!(emitter.requests.lock().len(), 1);
        assert_eq!(emitter.resolutions.lock()[0].1, ApprovalResolution::Approved);
    }

    #[tokio::test]
    async fn pre_invoke_ctx_denies_on_timeout() {
        let registry = CapabilityRegistry::new();
        registry.register(gated_echo()).unwrap();
        let broker = StubBroker {
            decision: ConsentDecision::Expired,
        };
        let emitter = RecordingEmitter::new();
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-2",
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: None,
            agent_code: None,
            grant_id: None,
            permission_timeout: Duration::from_secs(1),
        };
        let decision = pre_invoke_ctx(&ctx, "gated-echo", &HashMap::new()).await;
        assert!(matches!(decision, InterceptDecision::Deny { .. }));
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
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: None,
            agent_code: None,
            grant_id: None,
            permission_timeout: Duration::from_secs(1),
        };

        let decision = pre_invoke_ctx(&ctx, "gated-echo", &HashMap::new()).await;

        assert!(matches!(
            decision,
            InterceptDecision::Deny { ref reason }
                if reason == apxm_core::types::consent::APPROVAL_BROKER_UNAVAILABLE_REASON
        ));
        assert_eq!(emitter.resolutions.lock()[0].1, ApprovalResolution::Denied);
    }

    #[tokio::test]
    async fn pre_invoke_ctx_emits_denied_for_explicit_rejection() {
        let registry = CapabilityRegistry::new();
        registry.register(gated_echo()).unwrap();
        let broker = StubBroker {
            decision: ConsentDecision::Denied {
                reason: "operator rejected".into(),
            },
        };
        let emitter = RecordingEmitter::new();
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-4",
            consent_broker: &broker,
            event_emitter: Some(&emitter),
            host_id: None,
            agent_code: None,
            grant_id: None,
            permission_timeout: Duration::from_secs(1),
        };

        let decision = pre_invoke_ctx(&ctx, "gated-echo", &HashMap::new()).await;

        assert!(matches!(
            decision,
            InterceptDecision::Deny { ref reason } if reason == "operator rejected"
        ));
        assert_eq!(emitter.resolutions.lock()[0].1, ApprovalResolution::Denied);
    }

    #[tokio::test]
    async fn pre_invoke_ctx_skips_non_gated_capabilities() {
        let registry = CapabilityRegistry::new();
        registry.register(Arc::new(EchoCapability::new())).unwrap();
        let broker = StubBroker {
            decision: ConsentDecision::Denied {
                reason: "should not run".into(),
            },
        };
        let ctx = PreInvokeContext {
            registry: &registry,
            call_id: "call-5",
            consent_broker: &broker,
            event_emitter: None,
            host_id: None,
            agent_code: None,
            grant_id: None,
            permission_timeout: Duration::from_secs(1),
        };
        let decision = pre_invoke_ctx(&ctx, "echo", &HashMap::new()).await;
        assert!(matches!(decision, InterceptDecision::Allow));
    }
}
