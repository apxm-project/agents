//! The canonical Agent Program runtime driver.
//!
//! It executes a compiled five-operation AIR node graph through the kernel's
//! injected ports and commits atomically through the one Execution Commit port.
//! It is generic over the effects: it selects no implementation, holds no
//! router or registry, and never falls back or picks a first-available target.

pub mod driver;
pub mod ports;

pub use driver::{
    ExecutionError, ExecutionPorts, ExecutionRequest, NodeOutcome, RunReport, execute,
};
pub use ports::{
    CapabilityOutcome, CapabilityPort, CapabilityRequest, CompositionOutcome, CompositionPort,
    CompositionRequest, EventAwait, EventOutcome, EventPort,
};
