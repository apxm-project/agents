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
    CAPABILITY_DEFINITION_SCHEMA, CAPABILITY_TEMPLATE_SCHEMA, CapabilityDefinition,
    CapabilityMetadata, CapabilityTemplate,
};
pub use grant::{CAPABILITY_GRANT_SCHEMA, CapabilityGrant, GrantProvenance, GrantStatus};
pub use permission::{
    PermissionDecision, PermissionOperation, PermissionPolicy, PermissionRule, PermissionScope,
    ResourceHandle, ResourceSelector,
};
pub use permission_vocabulary::{
    ApprovalPosture, AuditPayloadPolicy, CredentialScope, GrantState, OperationClass,
    PermissionScopeKind, RiskLevel,
};
pub use policy::{
    AuthMethod, Principal, PrincipalKind, PromptMode, PromptPolicy, RoleAssignment, RoleDefinition,
    SubjectContext, SubjectSelector,
};
pub use wire::RuntimeCapabilityGrant;
