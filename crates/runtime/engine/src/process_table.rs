//! Process table — unified registry of all running agent processes and threads.
//!
//! The `ProcessTable` is the central data structure for the APXM process model.
//! It tracks all live agent processes (local and external) and their threads,
//! providing lookup, lifecycle management, and observability.

use apxm_core::constants::defaults::{DEFAULT_MAX_PROCESSES, DEFAULT_MAX_SPAWN_DEPTH};
use apxm_core::error::RuntimeError;
use apxm_core::types::operations::AISOperationType;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;

use crate::agent_router::AgentRouteCandidate;
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

/// Structured execution/node context required to spawn an external agent.
///
/// Production runner-backed spawners need durable run observability paths and
/// node identity; passing only `cwd` and ad-hoc environment strings is not
/// sufficient for a fail-closed remote execution boundary.
#[derive(Debug, Clone, Default)]
pub struct AgentSpawnContext {
    pub execution_id: Option<String>,
    pub workflow_id: Option<String>,
    pub trace_id: Option<String>,
    pub session_id: Option<String>,
    pub session_dir: Option<String>,
    pub run_root: Option<String>,
    pub node_id: u64,
    pub node_name: Option<String>,
    pub node_workspace: Option<String>,
    pub node_artifact_dir: Option<String>,
    pub runner_artifact_dir: Option<String>,
    pub context_ref: Option<String>,
    pub workdir_ref: Option<String>,
    /// Set for LINK-RUNTIME spawns; selects the `LinkAgentSpawner` path.
    pub host_id: Option<String>,
}

/// Trait for spawning external agent processes.
///
/// Implemented by apxm-acp (which has access to AcpSession, AgentRegistry, etc.)
/// and injected into the ProcessTable by the driver during setup.
#[async_trait::async_trait]
pub trait AgentSpawner: Send + Sync {
    /// Return APXM route candidates currently available to this spawner.
    fn route_candidates(&self) -> Vec<AgentRouteCandidate>;

    /// Spawn an external agent process or remote runner job.
    ///
    /// Returns the session handle wrapped in Arc<Mutex>. The concrete handle is
    /// adapter-specific, for example a local ACP session or a remote runner job
    /// session.
    async fn spawn_external(
        &self,
        agent_name: &str,
        profile_name: &str,
        cwd: &std::path::Path,
        mode: Option<&str>,
        model: Option<&str>,
        aam_context: &apxm_core::types::aam::AamContext,
        spawn_context: &AgentSpawnContext,
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
    registry_state: parking_lot::Mutex<RegistryState>,
    threads: DashMap<ThreadId, AgentThread>,
    max_processes: usize,
    max_spawn_depth: usize,
    /// Injected spawner for creating external agent processes.
    agent_spawner: tokio::sync::RwLock<Option<Arc<dyn AgentSpawner>>>,
    /// Injected prompter for sending messages to external agents.
    agent_prompter: tokio::sync::RwLock<Option<Arc<dyn AgentPrompter>>>,
}

#[derive(Default)]
struct RegistryState {
    reserved_names: HashMap<String, u64>,
    generation: u64,
}

impl ProcessTable {
    /// Create a new empty process table with default limits.
    pub fn new() -> Self {
        Self {
            processes: DashMap::new(),
            name_index: DashMap::new(),
            registry_state: parking_lot::Mutex::new(RegistryState::default()),
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

    /// Set the agent spawner during driver setup.
    pub async fn set_agent_spawner(&self, spawner: Arc<dyn AgentSpawner>) {
        *self.agent_spawner.write().await = Some(spawner);
    }

    /// Set the agent prompter during driver setup.
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

        reservation.commit(process)
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

        reservation.commit(process)
    }

    /// Look up a process by agent name.
    pub fn get_by_name(
        &self,
        name: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, ProcessId, AgentProcess>> {
        let _registry = self.registry_state.lock();
        let id = self.name_index.get(name)?;
        self.processes.get(id.value())
    }

    /// Look up a process by ID.
    pub fn get(&self, id: &str) -> Option<dashmap::mapref::one::Ref<'_, ProcessId, AgentProcess>> {
        let _registry = self.registry_state.lock();
        self.processes.get(id)
    }

    /// Register a new thread within a process.
    pub fn register_thread(
        &self,
        process_id: ProcessId,
        node_id: u64,
        op_type: AISOperationType,
    ) -> Result<ThreadId, RuntimeError> {
        let _registry = self.registry_state.lock();
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
    /// The process entry is fully deleted so that Arc references to the
    /// session handle are dropped, allowing AcpSession's Drop impl to fire.
    pub fn close(&self, name: &str) -> bool {
        let _registry = self.registry_state.lock();
        if let Some((_, id)) = self.name_index.remove(name) {
            let removed = self.processes.remove(&id).is_some();
            debug_assert!(removed, "name index referenced a missing process");
            self.debug_assert_registry_consistent();
            removed
        } else {
            false
        }
    }

    /// Close all processes.
    pub fn close_all(&self) {
        let mut registry = self.registry_state.lock();
        registry.generation = registry
            .generation
            .checked_add(1)
            .expect("process table reservation generation overflowed");
        registry.reserved_names.clear();
        self.name_index.clear();
        self.processes.clear();
        self.debug_assert_registry_consistent();
    }

    /// List active process names (lightweight — only reads the name index).
    pub fn list_process_names(&self) -> Vec<String> {
        let _registry = self.registry_state.lock();
        self.name_index.iter().map(|e| e.key().clone()).collect()
    }

    /// List all processes (for observability).
    pub fn list_processes(&self) -> Vec<(ProcessId, String, bool)> {
        let _registry = self.registry_state.lock();
        self.processes
            .iter()
            .map(|entry| {
                let p = entry.value();
                let is_running = matches!(p.state, ProcessState::Running | ProcessState::Idle);
                (p.id.clone(), p.name.clone(), is_running)
            })
            .collect()
    }

    /// Count active external processes by ACP profile name.
    pub fn external_profile_counts(&self) -> HashMap<String, usize> {
        let _registry = self.registry_state.lock();
        let mut counts = HashMap::new();
        for entry in self.processes.iter() {
            let process = entry.value();
            if !matches!(process.state, ProcessState::Running | ProcessState::Idle) {
                continue;
            }
            if let ProcessKind::External { profile_name, .. } = &process.kind {
                *counts.entry(profile_name.clone()).or_insert(0) += 1;
            }
        }
        counts
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
        let _registry = self.registry_state.lock();
        self.name_index.len()
    }

    fn check_capacity(&self, registry: &RegistryState) -> Result<(), RuntimeError> {
        let reserved_or_active = self.processes.len() + registry.reserved_names.len();
        if reserved_or_active >= self.max_processes {
            return Err(RuntimeError::State(format!(
                "Process table full: {} active processes (max {})",
                reserved_or_active, self.max_processes
            )));
        }
        Ok(())
    }

    fn check_name_available(
        &self,
        registry: &RegistryState,
        name: &str,
    ) -> Result<(), RuntimeError> {
        if self.name_index.contains_key(name) || registry.reserved_names.contains_key(name) {
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
    /// Capacity, spawn depth, and name availability are checked while holding
    /// the registry lock. Reservations count against capacity but remain
    /// separate from the live process maps until committed.
    pub fn reserve_spawn_slot(
        &self,
        name: String,
        parent_id: &Option<ProcessId>,
    ) -> Result<SpawnReservation<'_>, RuntimeError> {
        let mut registry = self.registry_state.lock();
        self.check_capacity(&registry)?;
        self.check_spawn_depth(parent_id)?;
        self.check_name_available(&registry, &name)?;
        let generation = registry.generation;
        registry.reserved_names.insert(name.clone(), generation);
        Ok(SpawnReservation {
            table: self,
            name,
            generation,
            committed: false,
        })
    }

    fn debug_assert_registry_consistent(&self) {
        #[cfg(debug_assertions)]
        {
            debug_assert_eq!(self.processes.len(), self.name_index.len());
            for entry in self.name_index.iter() {
                let process = self
                    .processes
                    .get(entry.value())
                    .expect("name index referenced a missing process");
                debug_assert_eq!(process.name, *entry.key());
            }
            for entry in self.processes.iter() {
                let indexed_id = self
                    .name_index
                    .get(&entry.name)
                    .expect("process was missing from the name index");
                debug_assert_eq!(*indexed_id, *entry.key());
            }
        }
    }
}

/// RAII guard for a spawn slot. Rolls back on Drop unless committed.
pub struct SpawnReservation<'a> {
    table: &'a ProcessTable,
    name: String,
    generation: u64,
    committed: bool,
}

impl<'a> Drop for SpawnReservation<'a> {
    fn drop(&mut self) {
        if !self.committed {
            let mut registry = self.table.registry_state.lock();
            if registry.reserved_names.get(&self.name) == Some(&self.generation) {
                registry.reserved_names.remove(&self.name);
            }
        }
    }
}

impl<'a> SpawnReservation<'a> {
    /// Commit the reservation by publishing both live-process indexes.
    pub fn commit(mut self, process: AgentProcess) -> Result<ProcessId, RuntimeError> {
        let id = process.id.clone();
        let mut registry = self.table.registry_state.lock();

        if registry.generation != self.generation
            || registry.reserved_names.get(&self.name) != Some(&self.generation)
        {
            return Err(RuntimeError::State(format!(
                "Spawn reservation for agent process '{}' was invalidated",
                self.name
            )));
        }
        if process.name != self.name {
            return Err(RuntimeError::State(format!(
                "Spawn reservation name '{}' does not match process name '{}'",
                self.name, process.name
            )));
        }
        if self.table.processes.contains_key(&id) {
            return Err(RuntimeError::State(format!(
                "Agent process id '{}' already exists",
                id
            )));
        }
        if self.table.name_index.contains_key(&self.name) {
            return Err(RuntimeError::State(format!(
                "Agent process '{}' already exists",
                self.name
            )));
        }

        self.table.processes.insert(id.clone(), process);
        self.table.name_index.insert(self.name.clone(), id.clone());
        registry.reserved_names.remove(&self.name);
        self.committed = true;
        self.table.debug_assert_registry_consistent();
        Ok(id)
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
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::thread;

    struct DropProbe(Arc<AtomicBool>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn external_session() -> Arc<tokio::sync::Mutex<dyn std::any::Any + Send + Sync>> {
        Arc::new(tokio::sync::Mutex::new(()))
    }

    fn local_process(name: &str) -> AgentProcess {
        AgentProcess {
            id: uuid::Uuid::now_v7().to_string(),
            name: name.to_string(),
            parent_id: None,
            kind: ProcessKind::Local,
            state: ProcessState::Running,
            spawned_at: std::time::Instant::now(),
        }
    }

    fn assert_state_error_contains(result: Result<ProcessId, RuntimeError>, expected: &str) {
        match result {
            Err(RuntimeError::State(message)) => assert!(
                message.contains(expected),
                "expected state error containing '{expected}', got '{message}'"
            ),
            Err(error) => panic!("expected state error, got {error}"),
            Ok(id) => panic!("expected failure, created process {id}"),
        }
    }

    #[test]
    fn concurrent_same_name_spawn_observes_reservation_and_rollback() {
        let table = Arc::new(ProcessTable::with_max_processes(2));
        let (reserved_tx, reserved_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        thread::scope(|scope| {
            let reserving_table = Arc::clone(&table);
            scope.spawn(move || {
                let reservation = reserving_table
                    .reserve_spawn_slot("shared".to_string(), &None)
                    .expect("first reservation should succeed");
                reserved_tx.send(()).expect("signal reservation");
                release_rx.recv().expect("wait for competing spawn");
                drop(reservation);
            });

            reserved_rx.recv().expect("wait for reservation");
            assert_state_error_contains(
                table.spawn_local("shared".to_string(), None),
                "already exists",
            );
            assert_eq!(table.active_count(), 0);
            assert!(table.name_index.is_empty());
            assert!(table.processes.is_empty());
            release_tx.send(()).expect("release reservation");
        });

        let id = table
            .spawn_local("shared".to_string(), None)
            .expect("dropped reservation should release the name");
        assert_eq!(table.get_by_name("shared").unwrap().id, id);
        table.debug_assert_registry_consistent();
    }

    #[test]
    fn concurrent_same_name_spawns_have_one_winner() {
        const WORKERS: usize = 16;
        let table = Arc::new(ProcessTable::with_max_processes(WORKERS));
        let start = Arc::new(Barrier::new(WORKERS));

        let results = thread::scope(|scope| {
            let mut handles = Vec::with_capacity(WORKERS);
            for _ in 0..WORKERS {
                let table = Arc::clone(&table);
                let start = Arc::clone(&start);
                handles.push(scope.spawn(move || {
                    start.wait();
                    table.spawn_local("shared".to_string(), None)
                }));
            }
            handles
                .into_iter()
                .map(|handle| handle.join().expect("spawn worker panicked"))
                .collect::<Vec<_>>()
        });

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(table.active_count(), 1);
        assert_eq!(table.name_index.len(), 1);
        assert_eq!(table.processes.len(), 1);
        table.debug_assert_registry_consistent();
    }

    #[test]
    fn concurrent_capacity_check_counts_in_flight_reservation() {
        let table = Arc::new(ProcessTable::with_max_processes(1));
        let (reserved_tx, reserved_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        thread::scope(|scope| {
            let reserving_table = Arc::clone(&table);
            scope.spawn(move || {
                let reservation = reserving_table
                    .reserve_spawn_slot("reserved".to_string(), &None)
                    .expect("capacity reservation should succeed");
                reserved_tx.send(()).expect("signal reservation");
                release_rx.recv().expect("wait for competing registration");
                drop(reservation);
            });

            reserved_rx.recv().expect("wait for reservation");
            let rejected_session_dropped = Arc::new(AtomicBool::new(false));
            let rejected_session: Arc<tokio::sync::Mutex<dyn std::any::Any + Send + Sync>> =
                Arc::new(tokio::sync::Mutex::new(DropProbe(Arc::clone(
                    &rejected_session_dropped,
                ))));
            assert_state_error_contains(
                table.register_external(
                    "external".to_string(),
                    None,
                    rejected_session,
                    "test-profile".to_string(),
                ),
                "Process table full",
            );
            assert!(rejected_session_dropped.load(Ordering::SeqCst));
            assert_eq!(table.active_count(), 0);
            release_tx.send(()).expect("release reservation");
        });

        table
            .register_external(
                "external".to_string(),
                None,
                external_session(),
                "test-profile".to_string(),
            )
            .expect("released capacity should be reusable");
        assert_eq!(table.active_count(), 1);
        table.debug_assert_registry_consistent();
    }

    #[test]
    fn concurrent_spawns_do_not_exceed_capacity() {
        const CAPACITY: usize = 4;
        const WORKERS: usize = 16;
        let table = Arc::new(ProcessTable::with_max_processes(CAPACITY));
        let start = Arc::new(Barrier::new(WORKERS));

        let results = thread::scope(|scope| {
            let mut handles = Vec::with_capacity(WORKERS);
            for worker in 0..WORKERS {
                let table = Arc::clone(&table);
                let start = Arc::clone(&start);
                handles.push(scope.spawn(move || {
                    start.wait();
                    let name = format!("agent-{worker}");
                    if worker % 2 == 0 {
                        table.spawn_local(name, None)
                    } else {
                        table.register_external(
                            name,
                            None,
                            external_session(),
                            "test-profile".to_string(),
                        )
                    }
                }));
            }
            handles
                .into_iter()
                .map(|handle| handle.join().expect("spawn worker panicked"))
                .collect::<Vec<_>>()
        });

        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            CAPACITY
        );
        assert_eq!(table.active_count(), CAPACITY);
        assert_eq!(table.name_index.len(), CAPACITY);
        assert_eq!(table.processes.len(), CAPACITY);
        table.debug_assert_registry_consistent();
    }

    #[test]
    fn close_all_clears_indexes_drops_sessions_and_invalidates_reservations() {
        let table = ProcessTable::with_max_processes(4);
        let local_id = table
            .spawn_local("local".to_string(), None)
            .expect("local spawn");
        let session_dropped = Arc::new(AtomicBool::new(false));
        let session: Arc<tokio::sync::Mutex<dyn std::any::Any + Send + Sync>> = Arc::new(
            tokio::sync::Mutex::new(DropProbe(Arc::clone(&session_dropped))),
        );
        let external_id = table
            .register_external(
                "external".to_string(),
                None,
                session,
                "test-profile".to_string(),
            )
            .expect("external registration");
        let pending = table
            .reserve_spawn_slot("pending".to_string(), &None)
            .expect("pending reservation");

        table.close_all();

        assert_eq!(table.active_count(), 0);
        assert!(table.list_process_names().is_empty());
        assert!(table.list_processes().is_empty());
        assert!(table.get(&local_id).is_none());
        assert!(table.get(&external_id).is_none());
        assert!(table.name_index.is_empty());
        assert!(table.processes.is_empty());
        assert!(table.registry_state.lock().reserved_names.is_empty());
        assert!(session_dropped.load(Ordering::SeqCst));

        assert_state_error_contains(pending.commit(local_process("pending")), "was invalidated");
        assert_eq!(table.active_count(), 0);
        assert!(table.name_index.is_empty());
        assert!(table.processes.is_empty());
        table.debug_assert_registry_consistent();
    }
}
