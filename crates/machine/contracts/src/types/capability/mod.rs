//! Typed capability abstract machine schema.
//!
//! [`RuntimeCapabilityGrant`] is the grant projection carried into execution:
//! it is the only record here that can authorize external action.

mod common;
mod permission;
mod wire;

pub use common::{
    CapabilitySchemaError, RuntimeLimits, RuntimeSurfaceLimit, RuntimeSurfaceMode,
    RuntimeSurfacePolicy,
};
pub use permission::{PermissionDecision, PermissionOperation, PermissionScope, ResourceHandle};
pub use wire::{GrantStatus, RuntimeCapabilityGrant};
