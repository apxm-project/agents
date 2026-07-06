//! Typed capability abstract machine schema.
//!
//! A **capability** is the first-class APXM unit composed of a [`CapabilityBinding`]
//! plus a [`PermissionPolicy`]. [`CapabilityGrant`] records are runtime-minted
//! authority objects; they are the only records that can authorize external action.

mod capability_binding;
mod common;
mod definition;
mod grant;
mod permission;
mod permission_vocabulary;
mod policy;
mod wire;

pub use capability_binding::{
    CapabilityBinding, CapabilityBindingHandler, CapabilityBindingMetadata,
};
pub use common::{
    CapabilitySchemaError, Delegability, Lifecycle, LifecycleBound, PlannerVisibility,
    RuntimeLimits, RuntimeSurfaceLimit, RuntimeSurfaceMode, RuntimeSurfacePolicy, Sensitivity,
};
pub use definition::{
    CAPABILITY_DEFINITION_SCHEMA_V1, CAPABILITY_TEMPLATE_SCHEMA_V1, CapabilityDefinition,
    CapabilityMetadata, CapabilityTemplateV1,
};
pub use grant::{CAPABILITY_GRANT_SCHEMA_V1, CapabilityGrant, GrantProvenance, GrantStatus};
pub use permission::{
    PermissionEffect, PermissionOperation, PermissionPolicy, PermissionRule, PermissionScope,
    ResourceHandle, ResourceSelector,
};
pub use permission_vocabulary::{
    ApprovalPosture, AuditPayloadPolicy, CredentialScope, DecisionReason, GrantState,
    OperationClass, PERMISSION_POLICY_SCHEMA_V1, PermissionDecision, PermissionDecisionKind,
    PermissionPolicyV1, PermissionScopeKind, RiskLevel,
};
pub use policy::{
    AuthMethod, Principal, PrincipalKind, PromptMode, PromptPolicy, RoleAssignment, RoleDefinition,
    SubjectContext, SubjectSelector,
};
pub use wire::RuntimeCapabilityGrant;
