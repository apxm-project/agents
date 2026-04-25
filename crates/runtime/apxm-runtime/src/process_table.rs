//! Process table — unified registry of all running agent processes and threads.
//!
//! The `ProcessTable` is the central data structure for the APXM process model.
//! It tracks all live agent processes (local and external) and their threads,
//! providing lookup, lifecycle management, and observability.

use apxm_core::constants::defaults::{DEFAULT_MAX_PROCESSES, DEFAULT_MAX_SPAWN_DEPTH};
use apxm_core::error::RuntimeError;
use apxm_core::types::operations::AISOperationType;
use dashmap::DashMap;
use std::sync::Arc;

use crate::process::{AgentProcess, ProcessId, ProcessKind, ProcessState};
use crate::thread::{AgentThread, ThreadId, ThreadState};

/// Token usage reported by a spawned agent prompt turn.
#[derive(Debug, Clone, Default)]
pub struct AgentPromptTokenUsage {
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
}

/// Provider-neutral response returned by a spawned agent prompter.
#[derive(Debug, Clone)]
pub struct AgentPromptResponse {
    pub text: String,
    pub session_id: Option<String>,
    pub agent_session_id: Option<String>,
    pub turn: Option<u64>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub token_usage: AgentPromptTokenUsage,
}

impl AgentPromptResponse {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            session_id: None,
            agent_session_id: None,
            turn: None,
            model: None,
            stop_reason: None,
            token_usage: AgentPromptTokenUsage::default(),
        }
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_agent_session_id(mut self, agent_session_id: impl Into<String>) -> Self {
        self.agent_session_id = Some(agent_session_id.into());
        self
    }

    pub fn with_turn(mut self, turn: u64) -> Self {
        self.turn = Some(turn);
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_stop_reason(mut self, stop_reason: impl Into<String>) -> Self {
        self.stop_reason = Some(stop_reason.into());
        self
    }

    pub fn with_token_usage(
        mut self,
        input_tokens: Option<usize>,
        output_tokens: Option<usize>,
    ) -> Self {
        self.token_usage = AgentPromptTokenUsage {
            input_tokens,
            output_tokens,
        };
        self
    }

    pub fn to_value(&self, agent_name: &str) -> apxm_core::types::values::Value {
        use apxm_core::constants::runtime::agent_result_keys as result_keys;
        use apxm_core::types::values::{Number, Value};

        let mut result = std::collections::HashMap::new();
        result.insert(
            result_keys::TEXT.to_string(),
            Value::String(self.text.clone()),
        );
        result.insert(
            result_keys::AGENT.to_string(),
            Value::String(agent_name.to_string()),
        );
        if let Some(stop_reason) = &self.stop_reason {
            result.insert(
                result_keys::STOP_REASON.to_string(),
                Value::String(stop_reason.clone()),
            );
        }
        if let Some(session_id) = &self.session_id {
            result.insert(
                result_keys::SESSION_ID.to_string(),
                Value::String(session_id.clone()),
            );
        }
        if let Some(agent_session_id) = &self.agent_session_id {
            result.insert(
                result_keys::AGENT_SESSION_ID.to_string(),
                Value::String(agent_session_id.clone()),
            );
        }
        if let Some(turn) = self.turn {
            result.insert(
                result_keys::TURN.to_string(),
                Value::Number(Number::Integer(turn as i64)),
            );
        }
        if let Some(model) = &self.model {
            result.insert(result_keys::MODEL.to_string(), Value::String(model.clone()));
        }
        if let Some(input) = self.token_usage.input_tokens {
            result.insert(
                result_keys::INPUT_TOKENS.to_string(),
                Value::Number(Number::Integer(input as i64)),
            );
        }
        if let Some(output) = self.token_usage.output_tokens {
            result.insert(
                result_keys::OUTPUT_TOKENS.to_string(),
                Value::Number(Number::Integer(output as i64)),
            );
        }

        Value::Object(result)
    }
}

/// Trait for spawning external agent processes.
///
/// Implemented by apxm-acp (which has access to AcpSession, AgentRegistry, etc.)
/// and injected into the ProcessTable by the driver during setup.
#[async_trait::async_trait]
pub trait AgentSpawner: Send + Sync {
    /// Spawn an external agent subprocess.
    ///
    /// Returns the session handle (type-erased AcpSession) wrapped in Arc<Mutex>.
    async fn spawn_external(
        &self,
        agent_name: &str,
        profile_name: &str,
        cwd: &std::path::Path,
        mode: Option<&str>,
        model: Option<&str>,
        aam_context: &apxm_core::types::aam::AamContext,
        extra_env: &std::collections::HashMap<String, String>,
    ) -> Result<Arc<tokio::sync::Mutex<dyn std::any::Any + Send + Sync>>, RuntimeError>;
}

/// Trait for sending prompts to external agent processes.
///
/// Implemented by apxm-acp and injected into the ProcessTable by the driver.
#[async_trait::async_trait]
pub trait AgentPrompter: Send + Sync {
    /// Send a prompt to a named agent process and return the response.
    async fn prompt(
        &self,
        process: &AgentProcess,
        message: &str,
    ) -> Result<AgentPromptResponse, RuntimeError>;
}

/// Unified registry of all running agent processes and their threads.
pub struct ProcessTable {
    processes: DashMap<ProcessId, AgentProcess>,
    name_index: DashMap<String, ProcessId>,
    threads: DashMap<ThreadId, AgentThread>,
    max_processes: usize,
    max_spawn_depth: usize,
    /// Injected spawner for creating external agent processes.
    agent_spawner: tokio::sync::RwLock<Option<Arc<dyn AgentSpawner>>>,
    /// Injected prompter for sending messages to external agents.
    agent_prompter: tokio::sync::RwLock<Option<Arc<dyn AgentPrompter>>>,
}

impl ProcessTable {
    /// Create a new empty process table with default limits.
    pub fn new() -> Self {
        Self {
            processes: DashMap::new(),
            name_index: DashMap::new(),
            threads: DashMap::new(),
            max_processes: DEFAULT_MAX_PROCESSES,
            max_spawn_depth: DEFAULT_MAX_SPAWN_DEPTH,
            agent_spawner: tokio::sync::RwLock::new(None),
            agent_prompter: tokio::sync::RwLock::new(None),
        }
    }

    /// Create a process table with a custom process limit.
    pub fn with_max_processes(max: usize) -> Self {
        Self {
            max_processes: max,
            ..Self::new()
        }
    }

    /// Create a process table with custom process and spawn depth limits.
    pub fn with_limits(max_processes: usize, max_spawn_depth: usize) -> Self {
        Self {
            max_processes,
            max_spawn_depth,
            ..Self::new()
        }
    }

    /// Set the agent spawner (called by the driver during setup).
    pub async fn set_agent_spawner(&self, spawner: Arc<dyn AgentSpawner>) {
        *self.agent_spawner.write().await = Some(spawner);
    }

    /// Set the agent prompter (called by the driver during setup).
    pub async fn set_agent_prompter(&self, prompter: Arc<dyn AgentPrompter>) {
        *self.agent_prompter.write().await = Some(prompter);
    }

    /// Get a reference to the agent spawner.
    pub async fn agent_spawner(&self) -> Option<Arc<dyn AgentSpawner>> {
        self.agent_spawner.read().await.clone()
    }

    /// Get a reference to the agent prompter.
    pub async fn agent_prompter(&self) -> Option<Arc<dyn AgentPrompter>> {
        self.agent_prompter.read().await.clone()
    }

    /// Spawn a new local agent process.
    pub fn spawn_local(
        &self,
        name: String,
        parent_id: Option<ProcessId>,
    ) -> Result<ProcessId, RuntimeError> {
        let reservation = self.reserve_spawn_slot(name.clone(), &parent_id)?;

        let id = uuid::Uuid::now_v7().to_string();
        let process = AgentProcess {
            id: id.clone(),
            name,
            parent_id,
            kind: ProcessKind::Local,
            state: ProcessState::Running,
            spawned_at: std::time::Instant::now(),
        };

        Ok(reservation.commit(process))
    }

    /// Register an external agent process (spawned via AgentSpawner).
    pub fn register_external(
        &self,
        name: String,
        parent_id: Option<ProcessId>,
        session: Arc<tokio::sync::Mutex<dyn std::any::Any + Send + Sync>>,
        profile_name: String,
    ) -> Result<ProcessId, RuntimeError> {
        let reservation = self.reserve_spawn_slot(name.clone(), &parent_id)?;

        let id = uuid::Uuid::now_v7().to_string();
        let process = AgentProcess {
            id: id.clone(),
            name,
            parent_id,
            kind: ProcessKind::External {
                session,
                profile_name,
            },
            state: ProcessState::Running,
            spawned_at: std::time::Instant::now(),
        };

        Ok(reservation.commit(process))
    }

    /// Look up a process by agent name.
    pub fn get_by_name(
        &self,
        name: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, ProcessId, AgentProcess>> {
        let id = self.name_index.get(name)?;
        self.processes.get(id.value())
    }

    /// Look up a process by ID.
    pub fn get(&self, id: &str) -> Option<dashmap::mapref::one::Ref<'_, ProcessId, AgentProcess>> {
        self.processes.get(id)
    }

    /// Register a new thread within a process.
    pub fn register_thread(
        &self,
        process_id: ProcessId,
        node_id: u64,
        op_type: AISOperationType,
    ) -> Result<ThreadId, RuntimeError> {
        if !self.processes.contains_key(&process_id) {
            return Err(RuntimeError::State(format!(
                "Cannot register thread: process '{}' not found",
                process_id
            )));
        }

        let id = uuid::Uuid::now_v7().to_string();
        let thread = AgentThread {
            id: id.clone(),
            process_id,
            node_id,
            op_type,
            state: ThreadState::Running,
            started_at: std::time::Instant::now(),
        };

        self.threads.insert(id.clone(), thread);
        Ok(id)
    }

    /// Mark a thread as completed.
    pub fn complete_thread(&self, thread_id: &str) {
        if let Some(mut thread) = self.threads.get_mut(thread_id) {
            thread.state = ThreadState::Completed;
        }
    }

    /// Mark a thread as failed.
    pub fn fail_thread(&self, thread_id: &str, error: String) {
        if let Some(mut thread) = self.threads.get_mut(thread_id) {
            thread.state = ThreadState::Failed { error };
        }
    }

    /// Close a named process (remove from both name index and process map).
    ///
    /// The process entry is fully removed so that Arc references to the
    /// session handle are dropped, allowing AcpSession's Drop impl to fire.
    pub fn close(&self, name: &str) -> bool {
        if let Some((_, id)) = self.name_index.remove(name) {
            self.processes.remove(&id);
            true
        } else {
            false
        }
    }

    /// Close all processes.
    pub fn close_all(&self) {
        let names: Vec<String> = self.name_index.iter().map(|e| e.key().clone()).collect();
        for name in names {
            self.close(&name);
        }
    }

    /// List active process names (lightweight — only reads the name index).
    pub fn list_process_names(&self) -> Vec<String> {
        self.name_index.iter().map(|e| e.key().clone()).collect()
    }

    /// List all processes (for observability).
    pub fn list_processes(&self) -> Vec<(ProcessId, String, bool)> {
        self.processes
            .iter()
            .map(|entry| {
                let p = entry.value();
                let is_running = matches!(p.state, ProcessState::Running | ProcessState::Idle);
                (p.id.clone(), p.name.clone(), is_running)
            })
            .collect()
    }

    /// List threads for a specific process.
    pub fn list_threads(&self, process_id: &str) -> Vec<(ThreadId, u64, AISOperationType)> {
        self.threads
            .iter()
            .filter(|entry| entry.value().process_id == process_id)
            .map(|entry| {
                let t = entry.value();
                (t.id.clone(), t.node_id, t.op_type)
            })
            .collect()
    }

    /// Number of active (non-terminated) processes.
    pub fn active_count(&self) -> usize {
        self.name_index.len()
    }

    fn check_capacity(&self) -> Result<(), RuntimeError> {
        if self.name_index.len() >= self.max_processes {
            return Err(RuntimeError::State(format!(
                "Process table full: {} active processes (max {})",
                self.name_index.len(),
                self.max_processes
            )));
        }
        Ok(())
    }

    fn check_name_available(&self, name: &str) -> Result<(), RuntimeError> {
        if self.name_index.contains_key(name) {
            return Err(RuntimeError::State(format!(
                "Agent process '{}' already exists",
                name
            )));
        }
        Ok(())
    }

    /// Walk the parent chain to compute spawn depth. Bounded by max_processes to prevent loops.
    fn compute_depth(&self, parent_id: &Option<ProcessId>) -> usize {
        let mut depth = 0;
        let mut current = parent_id.clone();
        while let Some(pid) = current {
            depth += 1;
            current = self.processes.get(&pid).and_then(|p| p.parent_id.clone());
            if depth > self.max_processes {
                break;
            }
        }
        depth
    }

    fn check_spawn_depth(&self, parent_id: &Option<ProcessId>) -> Result<(), RuntimeError> {
        let depth = self.compute_depth(parent_id);
        if depth >= self.max_spawn_depth {
            return Err(RuntimeError::State(format!(
                "Agent spawn depth limit exceeded: depth {} (max {}). \
                 Recursive SPAWN_AGENT chains are limited to prevent runaway spawning.",
                depth, self.max_spawn_depth
            )));
        }
        Ok(())
    }

    /// Reserve a spawn slot with RAII rollback semantics.
    ///
    /// Checks capacity, spawn depth, and name availability atomically.
    /// The returned `SpawnReservation` holds a placeholder in the name index.
    /// If dropped without calling `commit()`, the placeholder is removed.
    pub fn reserve_spawn_slot(
        &self,
        name: String,
        parent_id: &Option<ProcessId>,
    ) -> Result<SpawnReservation<'_>, RuntimeError> {
        self.check_capacity()?;
        self.check_spawn_depth(parent_id)?;
        self.check_name_available(&name)?;
        // Insert placeholder to reserve the name
        self.name_index.insert(name.clone(), String::new());
        Ok(SpawnReservation {
            table: self,
            name,
            committed: false,
        })
    }
}

/// RAII guard for a spawn slot. Rolls back on Drop unless committed.
///
/// Inspired by Codex's `SpawnReservation` pattern — atomically reserve a slot
/// before doing work, auto-rollback on Drop if spawn fails partway.
pub struct SpawnReservation<'a> {
    table: &'a ProcessTable,
    name: String,
    committed: bool,
}

impl<'a> Drop for SpawnReservation<'a> {
    fn drop(&mut self) {
        if !self.committed {
            self.table.name_index.remove(&self.name);
        }
    }
}

impl<'a> SpawnReservation<'a> {
    /// Commit the reservation — insert the process and finalize.
    pub fn commit(mut self, process: AgentProcess) -> ProcessId {
        let id = process.id.clone();
        self.table.name_index.insert(self.name.clone(), id.clone());
        self.table.processes.insert(id.clone(), process);
        self.committed = true;
        id
    }
}

impl Default for ProcessTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_local_process() {
        let table = ProcessTable::new();
        let id = table.spawn_local("test_agent".to_string(), None).unwrap();
        assert!(!id.is_empty());
        assert!(table.get_by_name("test_agent").is_some());
        assert_eq!(table.active_count(), 1);
    }

    #[test]
    fn duplicate_name_rejected() {
        let table = ProcessTable::new();
        table.spawn_local("agent_a".to_string(), None).unwrap();
        let result = table.spawn_local("agent_a".to_string(), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn close_process() {
        let table = ProcessTable::new();
        table.spawn_local("agent_b".to_string(), None).unwrap();
        assert!(table.close("agent_b"));
        assert!(table.get_by_name("agent_b").is_none());
        assert_eq!(table.active_count(), 0);
    }

    #[test]
    fn thread_lifecycle() {
        let table = ProcessTable::new();
        let pid = table.spawn_local("agent_c".to_string(), None).unwrap();
        let tid = table
            .register_thread(pid.clone(), 1, AISOperationType::Ask)
            .unwrap();
        assert_eq!(table.list_threads(&pid).len(), 1);

        table.complete_thread(&tid);
        let threads = table.list_threads(&pid);
        assert_eq!(threads.len(), 1);
    }

    #[test]
    fn max_processes_enforced() {
        let table = ProcessTable::with_max_processes(2);
        table.spawn_local("a1".to_string(), None).unwrap();
        table.spawn_local("a2".to_string(), None).unwrap();
        let result = table.spawn_local("a3".to_string(), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("full"));
    }

    #[test]
    fn list_processes() {
        let table = ProcessTable::new();
        table.spawn_local("alpha".to_string(), None).unwrap();
        table.spawn_local("beta".to_string(), None).unwrap();
        let list = table.list_processes();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|(_, _, running)| *running));
    }

    #[test]
    fn close_all() {
        let table = ProcessTable::new();
        table.spawn_local("x1".to_string(), None).unwrap();
        table.spawn_local("x2".to_string(), None).unwrap();
        table.close_all();
        assert_eq!(table.active_count(), 0);
    }

    #[test]
    fn spawn_depth_limit_enforced() {
        let table = ProcessTable::with_limits(32, 2);
        let p1 = table.spawn_local("a".into(), None).unwrap(); // depth 0
        let p2 = table.spawn_local("b".into(), Some(p1)).unwrap(); // depth 1
        let err = table.spawn_local("c".into(), Some(p2)); // depth 2 -> rejected
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("depth limit"));
    }

    #[test]
    fn spawn_depth_within_limit() {
        let table = ProcessTable::with_limits(32, 4);
        let p1 = table.spawn_local("a".into(), None).unwrap();
        let p2 = table.spawn_local("b".into(), Some(p1)).unwrap();
        let p3 = table.spawn_local("c".into(), Some(p2)).unwrap();
        assert!(table.spawn_local("d".into(), Some(p3)).is_ok());
    }

    #[test]
    fn reservation_rollback_on_drop() {
        let table = ProcessTable::new();
        {
            let _res = table.reserve_spawn_slot("x".into(), &None).unwrap();
            // Drop without commit
        }
        assert!(table.get_by_name("x").is_none());
        // Name should be available again
        assert!(table.spawn_local("x".into(), None).is_ok());
    }

    #[test]
    fn reservation_commit_succeeds() {
        let table = ProcessTable::new();
        let reservation = table.reserve_spawn_slot("committed".into(), &None).unwrap();
        let process = AgentProcess {
            id: "test-id".to_string(),
            name: "committed".to_string(),
            parent_id: None,
            kind: ProcessKind::Local,
            state: ProcessState::Running,
            spawned_at: std::time::Instant::now(),
        };
        let id = reservation.commit(process);
        assert_eq!(id, "test-id");
        assert!(table.get_by_name("committed").is_some());
    }
}
