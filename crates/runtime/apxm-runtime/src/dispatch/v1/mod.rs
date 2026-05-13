//! `DispatchIrV1` — the first internal version of APXM's compiled-graph
//! intent contract for graph-aware inference backends.
//!
//! This module is internal to `apxm-runtime`. It is not re-exported from the
//! crate root and is not part of any public ABI. Promotion to a public
//! contract is gated on benchmark proof and a serious second backend per
//! `.apxm/docs/design/dispatch-ir.md`.

mod lower;
mod plan;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use lower::{derive_apxm_hints, lower_graph};
#[allow(unused_imports)]
pub(crate) use plan::{
    BackendCapabilityRequirements, DispatchIrV1, GraphDispatchPlan, NodeDispatchPlan,
    TelemetryContract, DISPATCH_IR_V1_SCHEMA_VERSION,
};
