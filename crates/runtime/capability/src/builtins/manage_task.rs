//! `manage_task` — a native CRUD tool over the AAM goal tree.
//!
//! Tasks are AAM goals. Every mutation is applied to the live `Aam` handle and
//! written through to the durable [`ToolsStore`] (so the goal tree survives a
//! process restart). The tool is intentionally a thin, typed surface over the
//! existing `Aam` API — it does not introduce a second goal model.

use std::collections::HashMap;

use apxm_core::constants::capabilities::groups;
use apxm_core::types::goal::{Goal, GoalId, GoalStatus};
use apxm_core::types::values::{Number, Value};
use async_trait::async_trait;

use super::store::{TaskRow, ToolsStore, now_ms};
use apxm_aam::{Aam, CompletionPolicy, TransitionLabel};
use crate::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::RuntimeCapability,
};

const CAP: &str = apxm_core::constants::capabilities::MANAGE_TASK;

/// Native task-management capability backed by the AAM goal tree.
pub struct ManageTaskCapability {
    metadata: RuntimeCapability,
    aam: Aam,
    store: ToolsStore,
}

impl ManageTaskCapability {
    pub fn new(aam: Aam, store: ToolsStore) -> Self {
        Self {
            metadata: RuntimeCapability::new(
                CAP,
                "Create, update, list, complete, or cancel durable tasks (AAM goals). \
                 Tasks may be nested via parent_id and survive process restarts.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["create", "update", "list", "get", "complete", "cancel"]
                        },
                        "task_id": { "type": "string", "description": "Existing task id (uuid)" },
                        "parent_id": { "type": "string", "description": "Parent task id for nesting" },
                        "description": { "type": "string" },
                        "priority": { "type": "integer", "minimum": 0 },
                        "status": {
                            "type": "string",
                            "enum": ["pending", "active", "completed", "failed", "cancelled"]
                        },
                        "completion_policy": {
                            "type": "string",
                            "enum": ["all_children", "any_child", "manual"]
                        }
                    },
                    "required": ["action"]
                }),
            )
            .with_returns("object")
            .with_groups(vec![
                groups::TASK.to_string(),
                groups::AGENT_MANAGEMENT.to_string(),
            ]),
            aam,
            store,
        }
    }

    fn err(&self, message: impl Into<String>) -> apxm_core::error::RuntimeError {
        apxm_core::error::RuntimeError::Capability {
            capability: CAP.to_string(),
            message: message.into(),
        }
    }

    /// Resolve a task id string to a live `GoalId` by matching the goal tree.
    fn resolve(&self, task_id: &str) -> Option<GoalId> {
        self.aam
            .goals()
            .into_iter()
            .find(|g| g.id.to_string() == task_id)
            .map(|g| g.id)
    }

    fn fetch(&self, id: GoalId) -> Option<Goal> {
        self.aam.goals().into_iter().find(|g| g.id == id)
    }

    /// Write a goal through to the durable store, including any explicitly-set
    /// completion policy so it survives a restart.
    fn persist(&self, goal: &Goal) -> CapabilityResult<()> {
        let json =
            serde_json::to_string(goal).map_err(|e| self.err(format!("serialize task: {e}")))?;
        let row = TaskRow {
            id: goal.id.to_string(),
            parent_id: goal.parent_id.map(|p| p.to_string()),
            description: goal.description.clone(),
            priority: goal.priority as i64,
            status: wire_status(goal.status).to_string(),
            policy: self
                .aam
                .completion_policy(&goal.id)
                .map(|p| wire_policy(p).to_string()),
            json,
            updated_at_ms: now_ms(),
        };
        self.store.upsert_task(&row).map_err(|e| self.err(e))
    }

    fn goal_value(goal: &Goal) -> Value {
        let mut obj = HashMap::new();
        obj.insert("task_id".to_string(), Value::String(goal.id.to_string()));
        obj.insert(
            "description".to_string(),
            Value::String(goal.description.clone()),
        );
        obj.insert(
            "status".to_string(),
            Value::String(wire_status(goal.status).to_string()),
        );
        obj.insert(
            "priority".to_string(),
            Value::Number(Number::Integer(goal.priority as i64)),
        );
        obj.insert(
            "parent_id".to_string(),
            match goal.parent_id {
                Some(p) => Value::String(p.to_string()),
                None => Value::Null,
            },
        );
        Value::Object(obj)
    }

    fn ok_status(goal: &Goal) -> Value {
        let mut obj = HashMap::new();
        obj.insert("task_id".to_string(), Value::String(goal.id.to_string()));
        obj.insert(
            "status".to_string(),
            Value::String(wire_status(goal.status).to_string()),
        );
        Value::Object(obj)
    }

    fn do_create(&self, args: &HashMap<String, Value>) -> CapabilityResult<Value> {
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.err("'create' requires 'description'"))?
            .to_string();
        let priority = args.get("priority").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let parent = match args.get("parent_id").and_then(|v| v.as_str()) {
            Some(pid) => Some(
                self.resolve(pid)
                    .ok_or_else(|| self.err(format!("parent task '{pid}' not found")))?,
            ),
            None => None,
        };

        let goal = Goal {
            id: GoalId::new(),
            description,
            priority,
            status: GoalStatus::Pending,
            parent_id: parent,
        };
        let label = TransitionLabel::custom("manage_task:create");
        match parent {
            Some(parent_id) => {
                self.aam.add_child_goal(parent_id, goal.clone(), label);
            }
            None => {
                self.aam.add_goal(goal.clone(), label);
            }
        }
        if let Some(raw_policy) = args.get("completion_policy").and_then(|v| v.as_str()) {
            let policy = parse_policy(raw_policy).ok_or_else(|| {
                self.err("'completion_policy' must be all_children, any_child, or manual")
            })?;
            self.aam.set_completion_policy(goal.id, policy);
        }
        self.persist(&goal)?;
        Ok(Self::ok_status(&goal))
    }

    fn do_update(&self, args: &HashMap<String, Value>) -> CapabilityResult<Value> {
        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.err("'update' requires 'task_id'"))?;
        let id = self
            .resolve(task_id)
            .ok_or_else(|| self.err(format!("task '{task_id}' not found")))?;
        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let priority = args
            .get("priority")
            .and_then(|v| v.as_u64())
            .map(|p| p as u32);
        let status = match args.get("status").and_then(|v| v.as_str()) {
            Some(raw) => Some(parse_status(raw).ok_or_else(|| {
                self.err("'status' must be pending, active, completed, failed, or cancelled")
            })?),
            None => None,
        };
        let policy = match args.get("completion_policy").and_then(|v| v.as_str()) {
            Some(raw) => Some(parse_policy(raw).ok_or_else(|| {
                self.err("'completion_policy' must be all_children, any_child, or manual")
            })?),
            None => None,
        };
        if description.is_none() && priority.is_none() && status.is_none() && policy.is_none() {
            return Err(self.err(
                "'update' requires at least one of description, priority, status, or completion_policy",
            ));
        }

        self.aam
            .update_goal(
                id,
                description,
                priority,
                status,
                TransitionLabel::custom("manage_task:update"),
            )
            .ok_or_else(|| self.err(format!("task '{task_id}' not found")))?;
        if let Some(policy) = policy {
            self.aam.set_completion_policy(id, policy);
        }

        let goal = self
            .fetch(id)
            .ok_or_else(|| self.err("task vanished after update"))?;
        self.persist(&goal)?;
        if status == Some(GoalStatus::Completed) {
            self.propagate_parent_completion(&goal)?;
        }
        Ok(Self::ok_status(&goal))
    }

    fn apply_status(&self, task_id: &str, status: GoalStatus) -> CapabilityResult<Value> {
        let id = self
            .resolve(task_id)
            .ok_or_else(|| self.err(format!("task '{task_id}' not found")))?;
        self.aam
            .update_goal_status(id, status, TransitionLabel::custom("manage_task:update"))
            .ok_or_else(|| self.err(format!("task '{task_id}' not found")))?;
        let goal = self
            .fetch(id)
            .ok_or_else(|| self.err("task vanished after update"))?;
        self.persist(&goal)?;

        // Propagate completion to the parent if the policy is satisfied.
        if status == GoalStatus::Completed {
            self.propagate_parent_completion(&goal)?;
        }
        Ok(Self::ok_status(&goal))
    }

    fn propagate_parent_completion(&self, goal: &Goal) -> CapabilityResult<()> {
        if let Some(parent_id) = goal.parent_id
            && self
                .aam
                .propagate_completion(parent_id, TransitionLabel::custom("manage_task:propagate"))
                .is_some()
            && let Some(parent) = self.fetch(parent_id)
        {
            self.persist(&parent)?;
        }
        Ok(())
    }

    fn do_get(&self, args: &HashMap<String, Value>) -> CapabilityResult<Value> {
        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.err("'get' requires 'task_id'"))?;
        let id = self
            .resolve(task_id)
            .ok_or_else(|| self.err(format!("task '{task_id}' not found")))?;
        let goal = self
            .fetch(id)
            .ok_or_else(|| self.err(format!("task '{task_id}' not found")))?;
        Ok(Self::goal_value(&goal))
    }

    fn do_list(&self) -> CapabilityResult<Value> {
        let tasks: Vec<Value> = self.aam.goals().iter().map(Self::goal_value).collect();
        let mut obj = HashMap::new();
        obj.insert("tasks".to_string(), Value::Array(tasks));
        Ok(Value::Object(obj))
    }
}

#[async_trait]
impl CapabilityExecutor for ManageTaskCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| self.err("missing required 'action'"))?;
        match action {
            "create" => self.do_create(&args),
            "update" => self.do_update(&args),
            "complete" => {
                let task_id = args
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| self.err("'complete' requires 'task_id'"))?
                    .to_string();
                self.apply_status(&task_id, GoalStatus::Completed)
            }
            "cancel" => {
                let task_id = args
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| self.err("'cancel' requires 'task_id'"))?
                    .to_string();
                self.apply_status(&task_id, GoalStatus::Cancelled)
            }
            "get" => self.do_get(&args),
            "list" => self.do_list(),
            other => Err(self.err(format!("unknown action '{other}'"))),
        }
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

fn wire_status(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Pending => "pending",
        GoalStatus::Active => "active",
        GoalStatus::Completed => "completed",
        GoalStatus::Failed => "failed",
        GoalStatus::Cancelled => "cancelled",
    }
}

fn parse_status(s: &str) -> Option<GoalStatus> {
    match s.to_ascii_lowercase().as_str() {
        "pending" => Some(GoalStatus::Pending),
        "active" => Some(GoalStatus::Active),
        "completed" => Some(GoalStatus::Completed),
        "failed" => Some(GoalStatus::Failed),
        "cancelled" => Some(GoalStatus::Cancelled),
        _ => None,
    }
}

/// Parse a completion-policy wire token (`all_children` / `any_child` /
/// `manual`). Public so the restore path can rehydrate persisted policies.
pub fn parse_policy(s: &str) -> Option<CompletionPolicy> {
    match s.to_ascii_lowercase().as_str() {
        "all_children" => Some(CompletionPolicy::AllChildren),
        "any_child" => Some(CompletionPolicy::AnyChild),
        "manual" => Some(CompletionPolicy::Manual),
        _ => None,
    }
}

/// Wire token for a completion policy (inverse of [`parse_policy`]).
pub fn wire_policy(policy: CompletionPolicy) -> &'static str {
    match policy {
        CompletionPolicy::AllChildren => "all_children",
        CompletionPolicy::AnyChild => "any_child",
        CompletionPolicy::Manual => "manual",
    }
}
