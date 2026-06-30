//! Capability interception hooks.

use apxm_core::types::values::Value;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

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

/// Production permission gate, registered by the host at the trusted
/// invoke chokepoint to complement the in-handler write boundary. It activates
/// the `requires_auth` capability-metadata flag: a capability that declares it
/// needs authentication but is invoked without a resolved credential is denied in
/// strict mode, or warned about otherwise (advisory is the default so legitimate
/// unauthenticated tools keep working). Because every tool call — both the graph
/// `INV_TOOL` path and the in-`ASK`-node model loop — funnels through
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
