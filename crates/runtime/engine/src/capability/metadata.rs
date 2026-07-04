//! Runtime capability metadata — execution projection of contract definitions.
//!
//! `RuntimeCapability` (and its `apxm_aam::CapabilityRecord` conversion)
//! moved to `apxm-capability-iface`: it's the return type of
//! `CapabilityFacade::get_metadata`/`list_capabilities`, and once it's
//! foreign to this crate the orphan rule no longer permits the
//! `AamCapabilityRecord` conversion impl here either (neither type is local
//! to `apxm-runtime` anymore) — so that impl moved with it.

// Re-exported so `crate::capability::metadata::RuntimeCapability` and
// `apxm_runtime::capability::metadata::RuntimeCapability` keep working.
pub use apxm_capability_iface::RuntimeCapability;
