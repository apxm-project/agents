//! Capability interception hooks.

use apxm_core::events::payload::ToolCallCorrelation;
use apxm_core::types::capability::PermissionDecision;
use apxm_core::types::consent::{
    ConsentBroker, ConsentDecision, PermissionPrompt, PromptMode, RiskLevel,
};
use apxm_core::types::values::Value;
use apxm_program::capability::CapabilityInvocationAuthority;
use async_trait::async_trait;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

const MAX_APPROVAL_PREVIEW_BYTES: usize = 8 * 1024;
const MAX_APPROVAL_PREVIEW_DEPTH: usize = 4;
const MAX_APPROVAL_PREVIEW_STRING_BYTES: usize = 512;
const MAX_APPROVAL_PREVIEW_ITEMS: usize = 32;

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
    let json: BTreeMap<String, serde_json::Value> = args
        .iter()
        .map(|(k, v)| (k.clone(), canonical_arg_value(v)))
        .collect();
    let canonical = serde_json::to_string(&json).unwrap_or_default();
    format!("blake3:{}", blake3::hash(canonical.as_bytes()).to_hex())
}

fn canonical_arg_value(value: &Value) -> serde_json::Value {
    match value {
        Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(canonical_arg_value).collect())
        }
        Value::Object(values) => {
            let mut object = serde_json::Map::new();
            for (key, value) in values {
                object.insert(key.clone(), canonical_arg_value(value));
            }
            serde_json::Value::Object(object)
        }
        Value::Token(id) => serde_json::json!({"$token": id}),
        _ => value
            .to_json()
            .unwrap_or_else(|_| serde_json::Value::String(value.to_string())),
    }
}

fn sensitive_preview_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "authorization",
        "api_key",
        "apikey",
        "bearer",
        "cookie",
        "credential",
        "password",
        "secret",
        "token",
    ]
    .iter()
    .any(|needle| key == *needle || key.contains(needle))
}

/// Build the only argument representation allowed into an approval prompt.
/// Sensitive fields are replaced, nested data is bounded, and the final
/// serialized preview has a hard byte ceiling. The full arguments remain
/// represented by `args_digest`.
fn redacted_bounded_preview(args: &HashMap<String, Value>) -> serde_json::Value {
    fn sanitize(value: serde_json::Value, depth: usize) -> serde_json::Value {
        if depth >= MAX_APPROVAL_PREVIEW_DEPTH {
            return serde_json::json!("[REDACTED: depth limit]");
        }
        match value {
            serde_json::Value::String(mut text) => {
                if text.len() > MAX_APPROVAL_PREVIEW_STRING_BYTES {
                    let mut limit = MAX_APPROVAL_PREVIEW_STRING_BYTES;
                    while !text.is_char_boundary(limit) {
                        limit -= 1;
                    }
                    text.truncate(limit);
                    text.push_str("...[truncated]");
                }
                serde_json::Value::String(text)
            }
            serde_json::Value::Array(values) => serde_json::Value::Array(
                values
                    .into_iter()
                    .take(MAX_APPROVAL_PREVIEW_ITEMS)
                    .map(|value| sanitize(value, depth + 1))
                    .collect(),
            ),
            serde_json::Value::Object(values) => serde_json::Value::Object(
                values
                    .into_iter()
                    .take(MAX_APPROVAL_PREVIEW_ITEMS)
                    .map(|(key, value)| {
                        let value = if sensitive_preview_key(&key) {
                            serde_json::json!("[REDACTED]")
                        } else {
                            sanitize(value, depth + 1)
                        };
                        (key, value)
                    })
                    .collect(),
            ),
            other => other,
        }
    }

    let mut object = serde_json::Map::new();
    let mut keys: Vec<_> = args.keys().collect();
    keys.sort();
    for key in keys.into_iter().take(MAX_APPROVAL_PREVIEW_ITEMS) {
        let value = if sensitive_preview_key(key) {
            serde_json::json!("[REDACTED]")
        } else {
            sanitize(canonical_arg_value(&args[key]), 0)
        };
        object.insert(key.clone(), value);
    }
    let preview = serde_json::Value::Object(object);
    let encoded = serde_json::to_vec(&preview).unwrap_or_default();
    if encoded.len() <= MAX_APPROVAL_PREVIEW_BYTES {
        preview
    } else {
        serde_json::json!({"_redacted": "approval preview exceeds byte limit"})
    }
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
    let args_preview = redacted_bounded_preview(args);
    let expires_at = (chrono::Utc::now()
        + chrono::Duration::seconds(
            i64::try_from(ctx.permission_timeout.as_secs()).unwrap_or(i64::MAX),
        ))
    .to_rfc3339();

    // One policy risk decides both the emitted event and the prompt. Stating it
    // twice let the two drift the moment either became policy-derived.
    let risk_level = RiskLevel::High;

    if let Some(emitter) = ctx.event_emitter {
        emitter.emit_approval_request_with_correlation(
            identity.agent_code,
            name,
            &prompt_id,
            risk_level.to_approval(),
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
        risk_level,
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

    /// Called before an invocation that crossed the typed canonical request
    /// boundary. The authority is a trusted projection of admission, not a
    /// caller-controlled argument. The default preserves compatibility for
    /// interceptors that only need the capability name and arguments.
    async fn pre_invoke_with_authority(
        &self,
        name: &str,
        args: &HashMap<String, Value>,
        _authority: &CapabilityInvocationAuthority,
    ) -> InterceptDecision {
        self.pre_invoke(name, args).await
    }

    /// Called after capability execution (success path).
    async fn post_invoke(&self, _name: &str, _result: &Value) {}
}

/// Permission gate registered at the trusted invoke chokepoint. A capability
/// that declares `requires_auth` is denied unless the typed invocation carries
/// validated authority from admission. Caller-controlled arguments (including
/// `credential` and Authorization headers) are never proof of admission. The
/// historical `strict` argument remains for source compatibility but is
/// intentionally ignored: authentication cannot be advisory at an effect
/// boundary.
pub struct PermissionInterceptor {
    requires_auth: HashSet<String>,
    _strict_compat: bool,
}

impl PermissionInterceptor {
    /// Build the gate from the set of capability names that declare
    /// `requires_auth`. `strict` denies (vs. warns) on a missing credential.
    pub fn new(requires_auth: HashSet<String>, strict: bool) -> Self {
        Self {
            requires_auth,
            _strict_compat: strict,
        }
    }
}

#[async_trait]
impl CapabilityInterceptor for PermissionInterceptor {
    fn name(&self) -> &'static str {
        "permission"
    }

    async fn pre_invoke(&self, name: &str, args: &HashMap<String, Value>) -> InterceptDecision {
        let _ = args;
        if self.requires_auth.contains(name) {
            return InterceptDecision::deny(format!(
                "capability '{name}' requires typed invocation authority from admission"
            ));
        }
        InterceptDecision::allow()
    }

    async fn pre_invoke_with_authority(
        &self,
        name: &str,
        _args: &HashMap<String, Value>,
        authority: &CapabilityInvocationAuthority,
    ) -> InterceptDecision {
        if !self.requires_auth.contains(name) {
            return InterceptDecision::allow();
        }
        if let Err(error) = authority.validate() {
            return InterceptDecision::deny(format!(
                "capability '{name}' carries invalid invocation authority: {error}"
            ));
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

    #[test]
    fn consent_argument_digest_is_order_independent() {
        let mut first = HashMap::new();
        first.insert("b".to_string(), Value::String("two".to_string()));
        first.insert("a".to_string(), Value::String("one".to_string()));
        let mut second = HashMap::new();
        second.insert("a".to_string(), Value::String("one".to_string()));
        second.insert("b".to_string(), Value::String("two".to_string()));
        assert_eq!(args_digest_for(&first), args_digest_for(&second));

        let mut nested_first = HashMap::new();
        nested_first.insert("b".to_string(), Value::String("two".to_string()));
        nested_first.insert("a".to_string(), Value::String("one".to_string()));
        let mut nested_second = HashMap::new();
        nested_second.insert("a".to_string(), Value::String("one".to_string()));
        nested_second.insert("b".to_string(), Value::String("two".to_string()));
        let mut outer_first = HashMap::new();
        outer_first.insert("nested".to_string(), Value::Object(nested_first));
        let mut outer_second = HashMap::new();
        outer_second.insert("nested".to_string(), Value::Object(nested_second));
        assert_eq!(
            args_digest_for(&outer_first),
            args_digest_for(&outer_second)
        );
    }

    #[tokio::test]
    async fn auth_interceptor_requires_typed_authority_not_caller_arguments() {
        let interceptor = PermissionInterceptor::new(HashSet::from(["provider".to_owned()]), true);
        let caller_args = HashMap::from([(
            "credential".to_owned(),
            Value::String("caller-controlled".to_owned()),
        )]);
        let denied = interceptor.pre_invoke("provider", &caller_args).await;
        assert!(denied.denial_reason().is_some());

        let authority = CapabilityInvocationAuthority::new(
            "principal.user.1",
            "agent.identity.1",
            "grant.provider.1",
            Vec::new(),
        )
        .expect("valid authority");
        let allowed = interceptor
            .pre_invoke_with_authority("provider", &caller_args, &authority)
            .await;
        assert_eq!(allowed, InterceptDecision::allow());
    }

    #[test]
    fn approval_preview_redacts_credentials_and_is_bounded() {
        let args = HashMap::from([
            (
                "credential".to_owned(),
                Value::String("connection-secret-id".to_owned()),
            ),
            (
                "headers".to_owned(),
                Value::Object(HashMap::from([(
                    "Authorization".to_owned(),
                    Value::String("Bearer super-secret".to_owned()),
                )])),
            ),
            (
                "body".to_owned(),
                Value::String("x".repeat(MAX_APPROVAL_PREVIEW_STRING_BYTES + 100)),
            ),
        ]);
        let preview = redacted_bounded_preview(&args);
        let wire = serde_json::to_vec(&preview).expect("preview serializes");
        assert!(wire.len() <= MAX_APPROVAL_PREVIEW_BYTES);
        let text = String::from_utf8(wire).expect("preview is UTF-8");
        assert!(!text.contains("connection-secret-id"));
        assert!(!text.contains("super-secret"));
        assert!(text.contains("[REDACTED]"));
        assert!(text.contains("truncated"));
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
