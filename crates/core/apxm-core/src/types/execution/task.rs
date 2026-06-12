//! Task -- the fundamental unit of AI work in APXM.
//!
//! A task represents a schedulable piece of AI computation, from a single
//! LLM prompt to a composed pipeline of operations.  Tasks fire when all
//! dependencies are satisfied, and independent tasks are automatically
//! parallelized by the dataflow scheduler.
//!
//! # Design
//!
//! A [`Task`] wraps one or more APXM [`Node`]s into a logical unit of AI
//! work.  A [`TaskDag`] organises tasks into a dependency graph that can
//! be validated (cycle detection, missing-dependency checks) and then lowered
//! to an [`ExecutionDag`] for the runtime scheduler.
//!
//! # Background & Inspiration
//!
//! The task abstraction draws on two traditions:
//!
//! - **HPC dataflow** (Gao et al., CAPSL) -- fine-grained, dependency-driven
//!   firing rules and automatic parallelism.
//! - **Cognitive science** (Baars & Franklin, Global Workspace Theory) --
//!   specialized processors that fire when conditions are met and broadcast
//!   results to a shared workspace.
//!
//! APXM tasks are structurally isomorphic to both: a unit of work that fires
//! when its inputs are ready, produces typed outputs, and composes into
//! dependency graphs.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::{DagMetadata, DependencyType, Edge, ExecutionDag, Node, NodeId};
use crate::constants::graph::attrs as graph_attrs;
use crate::error::runtime::RuntimeError;
use crate::types::AISOperationType;

// ---------------------------------------------------------------------------
// TaskId
// ---------------------------------------------------------------------------

/// Unique identifier for a task.
pub type TaskId = u64;

// ---------------------------------------------------------------------------
// TaskMetadata
// ---------------------------------------------------------------------------

/// Metadata associated with a [`Task`].
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TaskMetadata {
    /// Scheduling priority (higher = more important).
    #[serde(default)]
    pub priority: u32,
    /// Optional JSON Schema describing the expected output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_output_schema: Option<String>,
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

/// The fundamental unit of AI work in APXM.
///
/// A task represents a schedulable piece of AI computation -- from a
/// single LLM prompt to a composed pipeline of operations.  Tasks fire
/// when all dependencies are satisfied, and independent tasks are
/// automatically parallelized by the dataflow scheduler.
///
/// Each task maps to one or more underlying APXM [`Node`]s.  The
/// [`TaskDag`] manages inter-task dependencies and can be lowered to
/// an [`ExecutionDag`] for scheduling.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    /// Unique identifier.
    pub id: TaskId,
    /// Human-readable name.
    pub name: String,
    /// Description of what this task does.
    pub description: String,
    /// The underlying APXM node IDs that implement this task.
    pub nodes: Vec<NodeId>,
    /// Tasks that must complete before this one fires.
    pub depends_on: Vec<TaskId>,
    /// Scheduling and output metadata.
    pub metadata: TaskMetadata,
}

impl Task {
    /// Creates a new task with the given id, name, and description.
    pub fn new(id: TaskId, name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            description: description.into(),
            nodes: Vec::new(),
            depends_on: Vec::new(),
            metadata: TaskMetadata::default(),
        }
    }

    /// Adds an underlying APXM node to this task.
    pub fn add_node(mut self, node_id: NodeId) -> Self {
        self.nodes.push(node_id);
        self
    }

    /// Declares a dependency on another task.
    pub fn add_dependency(mut self, dep: TaskId) -> Self {
        self.depends_on.push(dep);
        self
    }

    /// Sets scheduling priority.
    pub fn with_priority(mut self, priority: u32) -> Self {
        self.metadata.priority = priority;
        self
    }

    /// Sets the expected output JSON Schema.
    pub fn with_expected_output_schema(mut self, schema: impl Into<String>) -> Self {
        self.metadata.expected_output_schema = Some(schema.into());
        self
    }
}

// ---------------------------------------------------------------------------
// TaskDag
// ---------------------------------------------------------------------------

/// A directed acyclic graph of tasks.
///
/// Validates dependencies, detects cycles at construction time, and can be
/// lowered to an [`ExecutionDag`] for the dataflow scheduler.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskDag {
    /// Human-readable name for this DAG.
    pub name: String,
    /// The tasks in this graph.
    pub tasks: Vec<Task>,
    /// DAG-level metadata (forwarded when lowering to [`ExecutionDag`]).
    pub metadata: DagMetadata,
}

impl TaskDag {
    /// Creates a new empty task DAG.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tasks: Vec::new(),
            metadata: DagMetadata::default(),
        }
    }

    /// Adds a task to the DAG.
    pub fn add_task(mut self, task: Task) -> Self {
        self.tasks.push(task);
        self
    }

    /// Serialize this task DAG as JSON.
    pub fn to_json(&self) -> Result<String, RuntimeError> {
        serde_json::to_string(self)
            .map_err(|e| RuntimeError::Serialization(format!("failed to serialize TaskDag: {e}")))
    }

    /// Deserialize a task DAG from JSON.
    pub fn from_json(payload: &str) -> Result<Self, RuntimeError> {
        serde_json::from_str(payload)
            .map_err(|e| RuntimeError::Serialization(format!("failed to deserialize TaskDag: {e}")))
    }

    /// Validates the task DAG.
    ///
    /// Checks for:
    /// - Duplicate task IDs
    /// - Missing dependency references
    /// - Dependency cycles (via Kahn's algorithm)
    pub fn validate(&self) -> Result<(), RuntimeError> {
        let ids: HashMap<TaskId, &Task> = self.tasks.iter().map(|t| (t.id, t)).collect();

        // Check for duplicate IDs
        if ids.len() != self.tasks.len() {
            return Err(RuntimeError::State(
                "TaskDag contains duplicate task IDs".to_string(),
            ));
        }

        // Check all dependency references exist
        for task in &self.tasks {
            for dep in &task.depends_on {
                if !ids.contains_key(dep) {
                    return Err(RuntimeError::State(format!(
                        "task '{}' (id={}) depends on unknown task id={}",
                        task.name, task.id, dep
                    )));
                }
            }
        }

        // Cycle detection via topological sort (Kahn's algorithm)
        let mut in_degree: HashMap<TaskId, usize> = self.tasks.iter().map(|t| (t.id, 0)).collect();
        let mut adjacency: HashMap<TaskId, Vec<TaskId>> =
            self.tasks.iter().map(|t| (t.id, Vec::new())).collect();

        for task in &self.tasks {
            for dep in &task.depends_on {
                adjacency.get_mut(dep).unwrap().push(task.id);
                *in_degree.get_mut(&task.id).unwrap() += 1;
            }
        }

        let mut queue: VecDeque<TaskId> = in_degree
            .iter()
            .filter_map(|(&id, &deg)| (deg == 0).then_some(id))
            .collect();
        let mut visited = 0usize;

        while let Some(id) = queue.pop_front() {
            visited += 1;
            for &target in adjacency.get(&id).unwrap_or(&Vec::new()) {
                let deg = in_degree.get_mut(&target).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(target);
                }
            }
        }

        if visited != self.tasks.len() {
            return Err(RuntimeError::State(format!(
                "TaskDag '{}' contains a dependency cycle",
                self.name
            )));
        }

        Ok(())
    }

    /// Lowers this task DAG to an [`ExecutionDag`] for the dataflow
    /// scheduler.
    ///
    /// Each task's nodes are added to the execution DAG, and dependency
    /// edges are created between the last node of a dependency task and
    /// the first node of the dependent task.
    ///
    /// Tasks without any underlying nodes get a synthetic `Ask` node
    /// created from their description.
    pub fn to_execution_dag(&self) -> Result<ExecutionDag, RuntimeError> {
        self.validate()?;

        let mut dag = ExecutionDag::new();
        dag.metadata = self.metadata.clone();
        dag.metadata.name = Some(self.name.clone());

        let mut token_counter: u64 = 1;
        // Track first and last node IDs for each task (for edge wiring).
        let mut task_first_node: HashMap<TaskId, NodeId> = HashMap::new();
        let mut task_last_node: HashMap<TaskId, NodeId> = HashMap::new();

        for task in &self.tasks {
            if task.nodes.is_empty() {
                // Synthesize a node from the task description.
                let node_id = task.id * 1000; // avoid collisions
                let mut node = Node::new(node_id, AISOperationType::Ask);
                node.set_attribute(
                    graph_attrs::PROMPT.to_string(),
                    crate::types::Value::String(task.description.clone()),
                );
                node.metadata.priority = task.metadata.priority;
                node.metadata.task_source_id = Some(task.id);
                dag.add_node(node)?;
                task_first_node.insert(task.id, node_id);
                task_last_node.insert(task.id, node_id);
            } else {
                for &node_id in &task.nodes {
                    if dag.get_node(node_id).is_none() {
                        let mut node = Node::new(node_id, AISOperationType::Ask);
                        node.set_attribute(
                            graph_attrs::PROMPT.to_string(),
                            crate::types::Value::String(task.description.clone()),
                        );
                        node.metadata.priority = task.metadata.priority;
                        node.metadata.task_source_id = Some(task.id);
                        dag.add_node(node)?;
                    } else if let Some(existing) = dag.get_node_mut(node_id) {
                        existing.metadata.task_source_id = Some(task.id);
                    }
                }

                // Add all specified nodes and wire them sequentially.
                let first = task.nodes[0];
                let last = *task.nodes.last().unwrap();
                task_first_node.insert(task.id, first);
                task_last_node.insert(task.id, last);

                // Wire sequential edges within the task.
                for pair in task.nodes.windows(2) {
                    let token_id = token_counter;
                    token_counter += 1;
                    // Add output token to source, input token to target.
                    if let Some(src) = dag.get_node_mut(pair[0]) {
                        src.add_output_token(token_id);
                    }
                    if let Some(tgt) = dag.get_node_mut(pair[1]) {
                        tgt.add_input_token(token_id);
                    }
                    dag.add_edge(Edge::new(pair[0], pair[1], token_id, DependencyType::Data))?;
                }
            }
        }

        // Wire inter-task dependency edges.
        for task in &self.tasks {
            for dep_id in &task.depends_on {
                let from_node = *task_last_node.get(dep_id).unwrap();
                let to_node = *task_first_node.get(&task.id).unwrap();
                let token_id = token_counter;
                token_counter += 1;

                if let Some(src) = dag.get_node_mut(from_node) {
                    src.add_output_token(token_id);
                }
                if let Some(tgt) = dag.get_node_mut(to_node) {
                    tgt.add_input_token(token_id);
                }
                dag.add_edge(Edge::new(
                    from_node,
                    to_node,
                    token_id,
                    DependencyType::Data,
                ))?;
            }
        }

        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        Ok(dag)
    }
}

