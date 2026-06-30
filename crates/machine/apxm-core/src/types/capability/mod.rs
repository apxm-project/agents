//! Typed capability abstract machine schema.
//!
//! A **capability** is the first-class APXM unit composed of a [`ToolBinding`]
//! plus a [`PermissionPolicy`]. [`CapabilityGrant`] records are runtime-minted
//! authority objects; they are the only records that can authorize external action.

mod common;
mod definition;
mod grant;
mod permission;
mod policy;
mod tool_binding;
mod wire;

pub use common::{
    CapabilitySchemaError, Delegability, Lifecycle, LifecycleBound, PlannerVisibility,
    RuntimeLimits, RuntimeSurfaceLimit, RuntimeSurfaceMode, RuntimeSurfacePolicy, Sensitivity,
};
pub use definition::{
    CapabilityDefinition, CapabilityMetadata, CapabilityTemplateV1, CAPABILITY_DEFINITION_SCHEMA_V1,
    CAPABILITY_TEMPLATE_SCHEMA_V1,
};
pub use grant::{
    CapabilityGrant, GrantProvenance, GrantStatus, CAPABILITY_GRANT_SCHEMA_V1,
};
pub use permission::{
    PermissionEffect, PermissionOperation, PermissionPolicy, PermissionRule, PermissionScope,
    ResourceHandle, ResourceSelector,
};
pub use policy::{
    AuthMethod, Principal, PrincipalKind, PromptMode, PromptPolicy, RoleAssignment,
    RoleDefinition, SubjectContext, SubjectSelector,
};
pub use tool_binding::{ToolBinding, ToolBindingHandler, ToolBindingMetadata};
pub use wire::RuntimeCapabilityGrant;
