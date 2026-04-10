//! Multi-graph workflow execution layer.
//!
//! Composes multiple `.apxm` graphs into workflows where:
//! - Graphs can depend on each other's outputs
//! - Independent graphs run in parallel (tokio::spawn)
//! - Each graph gets its own session/trace directory
//! - Users declare this with a `.apxmw` file

pub mod def;
pub mod runner;
pub mod template;
pub mod topo;

pub use def::{GraphStep, WorkflowDef, WorkflowParam};
pub use runner::{StepResult, StepStatus, WorkflowResult, WorkflowRunner, WorkflowStatus};
pub use template::resolve;
pub use topo::execution_phases;
