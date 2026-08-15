//! Execution-time capability metadata.

use apxm_core::types::capability::PermissionOperation;
use apxm_core::types::values::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Declares one grant selector that constrains a capability invocation.
///
/// `selector` names the typed selector carried in [`PermissionScope`]. When
/// `argument` is present, a caller-supplied string or string array must be a
/// subset of that selector. An absent argument leaves the executor to apply
/// the trusted grant context to its result set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrantScopeSelector {
    pub selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
}

/// Declares one runtime quota a grant must carry for capability execution.
///
/// `argument` optionally binds a caller-provided upper bound to the named
/// quota. The executor still receives the trusted grant context and must cap
/// generated output when the caller leaves that argument absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrantRuntimeQuota {
    pub quota: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument: Option<String>,
}

/// Typed resource and quota requirements for grants that admit a capability.
///
/// This declaration is capability metadata, not a capability-specific
/// runtime branch. The executor uses the matching grants supplied in the
/// trusted invocation context to constrain any result that has no direct
/// request argument.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityGrantScopeRequirements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_selectors: Vec<CapabilityGrantScopeSelector>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_quotas: Vec<CapabilityGrantRuntimeQuota>,
}

impl CapabilityGrantScopeRequirements {
    /// Returns whether this capability declares any resource-scoped grant requirements.
    pub fn is_empty(&self) -> bool {
        self.resource_kind.is_none()
            && self.scope_selectors.is_empty()
            && self.runtime_quotas.is_empty()
    }
}

/// Execution-time capability metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeCapability {
    /// Unique capability name
    pub name: String,

    /// Human-readable description
    pub description: String,

    /// JSON Schema for input parameters
    pub parameters_schema: serde_json::Value,

    /// Expected return type description
    pub returns: String,

    /// Estimated execution cost (arbitrary units)
    #[serde(default)]
    pub cost_estimate: f64,

    /// Estimated latency in milliseconds
    #[serde(default = "default_latency")]
    pub latency_estimate_ms: u64,

    /// Whether this capability requires authentication
    #[serde(default)]
    pub requires_auth: bool,

    /// Whether invocation must pass the approval gate.
    #[serde(default)]
    pub requires_approval: bool,

    /// Tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,

    /// Tool groups used for least-privilege LLM exposure.
    #[serde(default)]
    pub groups: Vec<String>,

    /// Whether this capability is read-only (safe for full parallel execution).
    #[serde(default)]
    pub read_only: bool,

    /// Operations that require an active runtime-minted grant even when the
    /// capability is otherwise read-only.
    #[serde(default)]
    pub required_grant_operations: Vec<PermissionOperation>,

    /// Resource selectors and quotas that matching grants must carry.
    #[serde(default)]
    pub grant_scope_requirements: CapabilityGrantScopeRequirements,

    /// Additional metadata
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

fn default_latency() -> u64 {
    100
}

impl RuntimeCapability {
    /// Create new capability metadata with required fields
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters_schema: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters_schema,
            returns: "object".to_string(),
            cost_estimate: 0.0,
            latency_estimate_ms: default_latency(),
            requires_auth: false,
            requires_approval: false,
            tags: Vec::new(),
            groups: Vec::new(),
            read_only: false,
            required_grant_operations: Vec::new(),
            grant_scope_requirements: CapabilityGrantScopeRequirements::default(),
            metadata: HashMap::new(),
        }
    }

    /// Set return type description
    pub fn with_returns(mut self, returns: impl Into<String>) -> Self {
        self.returns = returns.into();
        self
    }

    /// Set cost estimate
    pub fn with_cost(mut self, cost: f64) -> Self {
        self.cost_estimate = cost;
        self
    }

    /// Set latency estimate
    pub fn with_latency(mut self, latency_ms: u64) -> Self {
        self.latency_estimate_ms = latency_ms;
        self
    }

    /// Mark as requiring authentication
    pub fn with_auth(mut self) -> Self {
        self.requires_auth = true;
        self
    }

    /// Mark as requiring explicit approval before invocation.
    pub fn with_requires_approval(mut self) -> Self {
        self.requires_approval = true;
        self
    }

    /// Add tags
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Assign tool groups for grouped capability exposure.
    pub fn with_groups(mut self, groups: Vec<String>) -> Self {
        self.groups = groups;
        self
    }

    /// Mark this capability as read-only (safe for full parallel execution).
    pub fn with_read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Require active grants containing every supplied permission operation.
    pub fn with_required_grant_operations(mut self, operations: Vec<PermissionOperation>) -> Self {
        self.required_grant_operations = operations;
        self
    }

    /// Require matching grant resource selectors and runtime quotas.
    pub fn with_grant_scope_requirements(
        mut self,
        requirements: CapabilityGrantScopeRequirements,
    ) -> Self {
        self.grant_scope_requirements = requirements;
        self
    }

    /// Add metadata entry
    pub fn with_metadata(mut self, key: String, value: Value) -> Self {
        self.metadata.insert(key, value);
        self
    }
}
