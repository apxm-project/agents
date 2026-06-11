use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::helpers::now_ms;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GoalRunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl GoalRunStatus {
    pub(crate) fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GoalRunRecord {
    pub(crate) goal_id: String,
    pub(crate) task: String,
    pub(crate) status: GoalRunStatus,
    pub(crate) started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completed_at_ms: Option<u64>,
    pub(crate) iteration: usize,
    pub(crate) max_iterations: usize,
    pub(crate) pass_execution_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current_execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) workflow_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bundle_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) artifacts: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) plan: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) planning: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) selection: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) control: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) goal: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) cancel_requested: bool,
}

#[derive(Clone, Default)]
pub(crate) struct GoalRunRegistry {
    inner: Arc<DashMap<String, GoalRunRecord>>,
}

impl GoalRunRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert_running(
        &self,
        goal_id: String,
        task: String,
        max_iterations: usize,
    ) -> GoalRunRecord {
        let record = GoalRunRecord {
            goal_id: goal_id.clone(),
            task,
            status: GoalRunStatus::Running,
            started_at_ms: now_ms(),
            completed_at_ms: None,
            iteration: 0,
            max_iterations,
            pass_execution_ids: Vec::new(),
            current_execution_id: None,
            session_id: None,
            session_dir: None,
            workflow_path: None,
            bundle_dir: None,
            artifacts: None,
            plan: None,
            planning: None,
            selection: None,
            control: None,
            goal: None,
            error: None,
            cancel_requested: false,
        };
        self.inner.insert(goal_id, record.clone());
        record
    }

    pub(crate) fn get(&self, goal_id: &str) -> Option<GoalRunRecord> {
        self.inner.get(goal_id).map(|entry| entry.clone())
    }

    pub(crate) fn list(&self) -> Vec<GoalRunRecord> {
        self.inner.iter().map(|entry| entry.clone()).collect()
    }

    pub(crate) fn set_current_pass(
        &self,
        goal_id: &str,
        iteration: usize,
        execution_id: String,
        session_id: String,
        session_dir: Option<String>,
        workflow_path: String,
        bundle_dir: String,
        artifacts: serde_json::Value,
        plan: serde_json::Value,
        planning: serde_json::Value,
        selection: Option<serde_json::Value>,
        control: serde_json::Value,
    ) -> Option<GoalRunRecord> {
        let mut entry = self.inner.get_mut(goal_id)?;
        entry.iteration = iteration;
        entry.current_execution_id = Some(execution_id.clone());
        entry.session_id = Some(session_id);
        entry.session_dir = session_dir;
        entry.workflow_path = Some(workflow_path);
        entry.bundle_dir = Some(bundle_dir);
        entry.artifacts = Some(artifacts);
        entry.plan = Some(plan);
        entry.planning = Some(planning);
        entry.selection = selection;
        entry.control = Some(control);
        entry.pass_execution_ids.push(execution_id);
        Some(entry.clone())
    }

    pub(crate) fn set_goal_outcome(
        &self,
        goal_id: &str,
        goal: serde_json::Value,
    ) -> Option<GoalRunRecord> {
        let mut entry = self.inner.get_mut(goal_id)?;
        entry.goal = Some(goal);
        Some(entry.clone())
    }

    pub(crate) fn finish(
        &self,
        goal_id: &str,
        status: GoalRunStatus,
        goal: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Option<GoalRunRecord> {
        let mut entry = self.inner.get_mut(goal_id)?;
        entry.status = status;
        entry.completed_at_ms = Some(now_ms());
        entry.current_execution_id = None;
        if goal.is_some() {
            entry.goal = goal;
        }
        entry.error = error;
        Some(entry.clone())
    }

    pub(crate) fn request_cancel(&self, goal_id: &str) -> Option<GoalRunRecord> {
        let mut entry = self.inner.get_mut(goal_id)?;
        entry.cancel_requested = true;
        Some(entry.clone())
    }
}
