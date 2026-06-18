//! Unified Plan structures shared across the system
//!
//! This module provides a single source of truth for plan-related types,
//! eliminating the previous inconsistency between OuterPlan (chat) and
//! PlanOutput (runtime).

use serde::{Deserialize, Serialize};

use crate::types::execution::TaskDag;

/// Structured plan with steps, result summary, and optional inner plan
///
/// This structure is used throughout the system:
/// - In planning workflows that generate ApxmGraph subgraphs
/// - In apxm-runtime PLAN operation for LLM-based planning
/// - Supports multi-level planning with inner plan execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Ordered list of plan steps
    #[serde(rename = "plan")]
    pub steps: Vec<PlanStep>,

    /// Summary of what will be accomplished
    pub result: String,

    /// Optional inner plan (AIR payload to be compiled and executed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner_plan: Option<InnerPlanPayload>,
}

/// A single step in a plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    /// Clear, actionable step description
    pub description: String,

    /// Execution priority (0-100, higher = more urgent)
    #[serde(default)]
    pub priority: u32,

    /// List of step descriptions that must complete first
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

/// Inner plan payload
///
/// Represents either AIR text or a structured `TaskDag` that should
/// be compiled and executed as part of multi-level planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InnerPlanPayload {
    /// Raw AIR text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub air: Option<String>,
    /// Optional structured task DAG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_dag: Option<TaskDag>,
}

impl Plan {
    /// Create a new plan with steps and result
    pub fn new(steps: Vec<PlanStep>, result: String) -> Self {
        Self {
            steps,
            result,
            inner_plan: None,
        }
    }

    /// Create a plan with an inner AIR payload.
    pub fn with_inner_air(mut self, air: String) -> Self {
        self.inner_plan = Some(InnerPlanPayload {
            air: Some(air),
            task_dag: None,
        });
        self
    }

    /// Create a plan with structured inner task DAG.
    pub fn with_inner_task_dag(mut self, task_dag: TaskDag) -> Self {
        self.inner_plan = Some(InnerPlanPayload {
            air: None,
            task_dag: Some(task_dag),
        });
        self
    }

    /// Check if this plan has an inner plan
    pub fn has_inner_plan(&self) -> bool {
        self.inner_plan
            .as_ref()
            .map(InnerPlanPayload::has_payload)
            .unwrap_or(false)
    }
}

impl InnerPlanPayload {
    /// Returns true when this inner plan contains either AIR text or a task DAG.
    pub fn has_payload(&self) -> bool {
        self.air
            .as_ref()
            .map(|air| !air.trim().is_empty())
            .unwrap_or(false)
            || self.task_dag.is_some()
    }
}

impl PlanStep {
    /// Create a new plan step
    pub fn new(description: String, priority: u32) -> Self {
        Self {
            description,
            priority,
            dependencies: Vec::new(),
        }
    }

    /// Add a dependency to this step
    pub fn with_dependency(mut self, dep: String) -> Self {
        self.dependencies.push(dep);
        self
    }

    /// Add multiple dependencies
    pub fn with_dependencies(mut self, deps: Vec<String>) -> Self {
        self.dependencies.extend(deps);
        self
    }
}
