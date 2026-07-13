//! Workflow-file execution layer.
//!
//! Composes multiple `.air` workflow steps into workflows where:
//! - Workflow steps can depend on each other's outputs
//! - Dependents run as soon as their own prerequisites succeed
//! - Each step gets its own session/trace directory
//! - Users declare this with a `.apxmw` file

pub mod def;
pub mod optimization;
pub mod planner;
pub mod ready;
pub mod runner;
pub mod session;
pub mod template;
pub mod topo;

pub use def::{WorkflowDef, WorkflowParam, WorkflowStep};
pub use optimization::{
    WorkflowCheckpointBarrier, WorkflowCheckpointPlacement, WorkflowCriticalPathEvidence,
};
pub use planner::{WorkflowPlan, WorkflowPlanStep};
pub use ready::{
    ReadyWorkflowStep, WorkflowLegalityPrioritySource, WorkflowReadyQueue, WorkflowSchedulerMode,
    WorkflowSchedulerOptions,
};
pub use runner::{StepResult, StepStatus, WorkflowResult, WorkflowStatus};
pub use session::{
    write_workflow_background_started, write_workflow_checkpoint_barrier,
    write_workflow_session_finished, write_workflow_session_started, write_workflow_step_finished,
    write_workflow_step_started,
};
pub use template::resolve;
pub use topo::execution_phases;
