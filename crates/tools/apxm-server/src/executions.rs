use std::path::Path as FsPath;
use std::sync::Arc;

use apxm_core::events::payload::{NodeMetricsPayload, NodeOutputPayload, RedactedContent};
use apxm_core::events::{ApxmEvent, EventEmitter};
use apxm_core::types::NodeMetrics;
use apxm_skill::SkillExecutionProvenance;
use axum::Json;
use axum::extract::{Path, State};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::execute::ExecuteResponse;
use crate::execution_index::{ExecutionIndex, IndexEntry};
use crate::helpers::now_ms;
use crate::state::AppState;

pub(crate) const EXECUTION_RECORDS_DIR: &str = "executions";
pub(crate) const EXECUTION_RECORD_EXTENSION: &str = "json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExecutionRecord {
    pub(crate) execution_id: String,
    pub(crate) skill_id: String,
    pub(crate) skill_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) entry_flow: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) air_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) artifact_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) parent_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) parent_skill_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) parent_skill_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) scope_id: Option<String>,
    pub(crate) session_id: String,
    pub(crate) session_dir: String,
    pub(crate) status: ExecutionStatus,
    pub(crate) started_at_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completed_at_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<ExecuteResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
    #[serde(default)]
    pub(crate) node_outputs: Vec<NodeOutputRecord>,
    #[serde(default)]
    pub(crate) node_metrics: Vec<NodeMetricsRecord>,
    /// Real (unredacted) token values captured on a successful run, keyed by
    /// token id. Persisted so a later `rerun-from-node` can seed the replay
    /// boundary with the prior run's upstream outputs. Empty when the run did
    /// not complete successfully or output capture was unavailable.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub(crate) token_values: std::collections::HashMap<u64, serde_json::Value>,
    /// Goal-convergence outcome for a goal pass: the typed gate
    /// verdict and the runtime decision derived from it. Absent for
    /// non-goal runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) goal: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeOutputRecord {
    pub(crate) node_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) node_name: Option<String>,
    pub(crate) observed_at_ms: u64,
    pub(crate) output: RedactedContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeMetricsRecord {
    pub(crate) node_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) node_name: Option<String>,
    pub(crate) observed_at_ms: u64,
    pub(crate) metrics: NodeMetrics,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NodeExecutionDetail {
    pub(crate) execution_id: String,
    pub(crate) skill_id: String,
    pub(crate) skill_version: String,
    pub(crate) session_id: String,
    pub(crate) session_dir: String,
    pub(crate) node_id: u64,
    pub(crate) outputs: Vec<NodeOutputRecord>,
    pub(crate) metrics: Vec<NodeMetricsRecord>,
}

#[derive(Clone)]
pub(crate) struct ExecutionStore {
    inner: Arc<DashMap<String, ExecutionRecord>>,
    index: ExecutionIndex,
}

impl ExecutionStore {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_index(ExecutionIndex::new())
    }

    pub(crate) fn with_index_max_entries(max_entries: usize) -> Self {
        Self::with_index(ExecutionIndex::with_capacity(max_entries))
    }

    pub(crate) fn with_index(index: ExecutionIndex) -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
            index,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn from_session_roots<I, P>(session_roots: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<FsPath>,
    {
        Self::from_session_roots_with_index_max_entries(
            session_roots,
            crate::execution_index::DEFAULT_MAX_ENTRIES,
        )
    }

    pub(crate) fn from_session_roots_with_index_max_entries<I, P>(
        session_roots: I,
        index_max_entries: usize,
    ) -> Self
    where
        I: IntoIterator<Item = P>,
        P: AsRef<FsPath>,
    {
        let store = Self::with_index_max_entries(index_max_entries);
        store.reload_from_session_roots(session_roots);
        store
    }

    pub(crate) fn reload_from_session_roots<I, P>(&self, session_roots: I) -> usize
    where
        I: IntoIterator<Item = P>,
        P: AsRef<FsPath>,
    {
        let roots: Vec<std::path::PathBuf> = session_roots
            .into_iter()
            .map(|root| root.as_ref().to_path_buf())
            .collect();
        // Seed the index from disk first — directory wins. This is cheap
        // because it only parses each snapshot once.
        self.index.reload_from_session_roots(roots.iter());

        let mut loaded = 0;
        for root in &roots {
            loaded += self.load_records_from_tree(root);
        }
        loaded
    }

    // Convenience overload (no provenance) used by in-crate tests
    // (`tests/skills_records.rs`, `tests/mcp.rs`). Production code paths
    // call `start_skill_execution_with_provenance{,_and_execution_id}`.
    #[allow(dead_code)]
    pub(crate) fn start_skill_execution(
        &self,
        skill_id: &str,
        skill_version: &str,
        session_id: &str,
        session_dir: &str,
    ) -> ExecutionRecord {
        self.start_skill_execution_with_provenance(
            SkillExecutionProvenance {
                skill_id: skill_id.to_string(),
                skill_version: skill_version.to_string(),
                ..SkillExecutionProvenance::default()
            },
            session_id,
            session_dir,
        )
    }

    pub(crate) fn start_skill_execution_with_provenance(
        &self,
        provenance: SkillExecutionProvenance,
        session_id: &str,
        session_dir: &str,
    ) -> ExecutionRecord {
        self.start_skill_execution_with_provenance_and_execution_id(
            uuid::Uuid::new_v4().to_string(),
            provenance,
            session_id,
            session_dir,
        )
    }

    pub(crate) fn start_skill_execution_with_provenance_and_execution_id(
        &self,
        execution_id: String,
        provenance: SkillExecutionProvenance,
        session_id: &str,
        session_dir: &str,
    ) -> ExecutionRecord {
        let record = ExecutionRecord {
            execution_id,
            skill_id: provenance.skill_id,
            skill_version: provenance.skill_version,
            entry_flow: provenance.entry_flow,
            source_hash: provenance.source_hash,
            air_hash: provenance.air_hash,
            artifact_hash: provenance.artifact_hash,
            parent_execution_id: provenance.parent_execution_id,
            parent_skill_id: provenance.parent_skill_id,
            parent_skill_version: provenance.parent_skill_version,
            scope_id: provenance.scope_id,
            session_id: session_id.to_string(),
            session_dir: session_dir.to_string(),
            status: ExecutionStatus::Running,
            started_at_ms: now_ms(),
            completed_at_ms: None,
            result: None,
            error: None,
            node_outputs: Vec::new(),
            node_metrics: Vec::new(),
            token_values: std::collections::HashMap::new(),
            goal: None,
        };
        self.inner
            .insert(record.execution_id.clone(), record.clone());
        persist_record_snapshot(&record);
        self.index.upsert_from_record(&record);
        record
    }

    pub(crate) fn complete_success(
        &self,
        execution_id: &str,
        result: ExecuteResponse,
    ) -> Option<ExecutionRecord> {
        let mut entry = self.inner.get_mut(execution_id)?;
        entry.status = ExecutionStatus::Succeeded;
        entry.completed_at_ms = Some(now_ms());
        entry.result = Some(result);
        entry.error = None;
        let record = entry.clone();
        drop(entry);
        persist_record_snapshot(&record);
        self.index.upsert_from_record(&record);
        Some(record)
    }

    pub(crate) fn complete_failure(
        &self,
        execution_id: &str,
        error: String,
    ) -> Option<ExecutionRecord> {
        let mut entry = self.inner.get_mut(execution_id)?;
        entry.status = ExecutionStatus::Failed;
        entry.completed_at_ms = Some(now_ms());
        entry.result = None;
        entry.error = Some(error);
        let record = entry.clone();
        drop(entry);
        persist_record_snapshot(&record);
        self.index.upsert_from_record(&record);
        Some(record)
    }

    /// Persist the run's real token values (keyed by token id) so a later
    /// `rerun-from-node` can seed the replay boundary. Called after
    /// `complete_success` when the runtime collected per-token outputs. No-op
    /// (returns `None`) when the record is unknown or `values` is empty.
    pub(crate) fn record_token_values(
        &self,
        execution_id: &str,
        values: std::collections::HashMap<u64, serde_json::Value>,
    ) -> Option<ExecutionRecord> {
        if values.is_empty() {
            return None;
        }
        let mut entry = self.inner.get_mut(execution_id)?;
        entry.token_values = values;
        let record = entry.clone();
        drop(entry);
        persist_record_snapshot(&record);
        self.index.upsert_from_record(&record);
        Some(record)
    }

    /// Attach the goal-convergence outcome (verdict + decision) to a settled
    /// record and re-persist it. Called after `complete_success`/`_failure` for
    /// goal passes.
    pub(crate) fn set_goal_outcome(
        &self,
        execution_id: &str,
        goal: serde_json::Value,
    ) -> Option<ExecutionRecord> {
        let mut entry = self.inner.get_mut(execution_id)?;
        entry.goal = Some(goal);
        let record = entry.clone();
        drop(entry);
        persist_record_snapshot(&record);
        self.index.upsert_from_record(&record);
        Some(record)
    }

    pub(crate) fn get(&self, execution_id: &str) -> Option<ExecutionRecord> {
        if let Some(entry) = self.inner.get(execution_id) {
            return Some(entry.clone());
        }
        // Fall back through the index: cheap metadata probe → resolve the
        // snapshot path → rehydrate the full record into the hot map.
        let entry: IndexEntry = self.index.get(execution_id)?;
        let path = entry.snapshot_path(execution_id);
        let record = read_execution_record_snapshot(&path)?;
        self.inner
            .insert(record.execution_id.clone(), record.clone());
        Some(record)
    }

    pub(crate) fn list(&self) -> Vec<ExecutionRecord> {
        let mut records: Vec<ExecutionRecord> = self
            .inner
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        records.sort_by(|left, right| {
            right
                .started_at_ms
                .cmp(&left.started_at_ms)
                .then_with(|| left.execution_id.cmp(&right.execution_id))
        });
        records
    }

    pub(crate) fn record_node_output(
        &self,
        execution_id: &str,
        node_id: u64,
        node_name: Option<String>,
        output: RedactedContent,
    ) -> Option<ExecutionRecord> {
        let mut entry = self.inner.get_mut(execution_id)?;
        entry.node_outputs.push(NodeOutputRecord {
            node_id,
            node_name,
            observed_at_ms: now_ms(),
            output,
        });
        let record = entry.clone();
        drop(entry);
        persist_record_snapshot(&record);
        self.index.upsert_from_record(&record);
        Some(record)
    }

    pub(crate) fn record_node_metrics(
        &self,
        execution_id: &str,
        node_id: u64,
        node_name: Option<String>,
        metrics: NodeMetrics,
    ) -> Option<ExecutionRecord> {
        let mut entry = self.inner.get_mut(execution_id)?;
        entry.node_metrics.push(NodeMetricsRecord {
            node_id,
            node_name,
            observed_at_ms: now_ms(),
            metrics,
        });
        let record = entry.clone();
        drop(entry);
        persist_record_snapshot(&record);
        self.index.upsert_from_record(&record);
        Some(record)
    }

    pub(crate) fn get_node(
        &self,
        execution_id: &str,
        node_id: u64,
    ) -> Result<NodeExecutionDetail, ExecutionNodeLookupError> {
        let record = self
            .get(execution_id)
            .ok_or_else(|| ExecutionNodeLookupError::ExecutionNotFound(execution_id.to_string()))?;
        let outputs: Vec<NodeOutputRecord> = record
            .node_outputs
            .iter()
            .filter(|output| output.node_id == node_id)
            .cloned()
            .collect();
        let metrics: Vec<NodeMetricsRecord> = record
            .node_metrics
            .iter()
            .filter(|metrics| metrics.node_id == node_id)
            .cloned()
            .collect();
        if outputs.is_empty() && metrics.is_empty() {
            return Err(ExecutionNodeLookupError::NodeNotFound {
                execution_id: execution_id.to_string(),
                node_id,
            });
        }
        Ok(NodeExecutionDetail {
            execution_id: record.execution_id,
            skill_id: record.skill_id,
            skill_version: record.skill_version,
            session_id: record.session_id,
            session_dir: record.session_dir,
            node_id,
            outputs,
            metrics,
        })
    }

    fn load_records_from_tree(&self, root: &FsPath) -> usize {
        if !root.is_dir() {
            return 0;
        }

        let mut loaded = 0;
        let Ok(entries) = std::fs::read_dir(root) else {
            tracing::warn!(path = %root.display(), "failed to read execution snapshot root");
            return 0;
        };

        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|name| name.to_str()) == Some(EXECUTION_RECORDS_DIR) {
                    loaded += self.load_records_from_execution_dir(&path);
                } else {
                    loaded += self.load_records_from_tree(&path);
                }
            }
        }

        loaded
    }

    fn load_records_from_execution_dir(&self, dir: &FsPath) -> usize {
        let Ok(entries) = std::fs::read_dir(dir) else {
            tracing::warn!(path = %dir.display(), "failed to read execution snapshot directory");
            return 0;
        };

        let mut loaded = 0;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !is_execution_record_snapshot(&path) {
                continue;
            }
            let Some(record) = read_execution_record_snapshot(&path) else {
                continue;
            };
            if self.inner.contains_key(&record.execution_id) {
                continue;
            }
            self.index.upsert_from_record(&record);
            self.inner.insert(record.execution_id.clone(), record);
            loaded += 1;
        }
        loaded
    }
}

fn persist_record_snapshot(record: &ExecutionRecord) {
    let path = execution_record_snapshot_path(&record.session_dir, &record.execution_id);
    let Ok(bytes) = serde_json::to_vec_pretty(record) else {
        tracing::warn!(
            execution_id = %record.execution_id,
            "failed to serialize execution record snapshot"
        );
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            tracing::warn!(
                execution_id = %record.execution_id,
                path = %parent.display(),
                %error,
                "failed to create execution record snapshot directory"
            );
            return;
        }
    }
    let temp_path = path.with_extension(format!("{EXECUTION_RECORD_EXTENSION}.tmp"));
    if let Err(error) = std::fs::write(&temp_path, bytes) {
        tracing::warn!(
            execution_id = %record.execution_id,
            path = %temp_path.display(),
            %error,
            "failed to write execution record snapshot"
        );
        return;
    }
    if let Err(error) = std::fs::rename(&temp_path, &path) {
        tracing::warn!(
            execution_id = %record.execution_id,
            from = %temp_path.display(),
            to = %path.display(),
            %error,
            "failed to persist execution record snapshot"
        );
    }
}

pub(crate) fn execution_record_snapshot_path(
    session_dir: &str,
    execution_id: &str,
) -> std::path::PathBuf {
    std::path::Path::new(session_dir)
        .join(EXECUTION_RECORDS_DIR)
        .join(execution_record_snapshot_file_name(execution_id))
}

fn execution_record_snapshot_file_name(execution_id: &str) -> String {
    format!("{execution_id}.{EXECUTION_RECORD_EXTENSION}")
}

fn is_execution_record_snapshot(path: &FsPath) -> bool {
    path.is_file()
        && path.file_name().and_then(|name| name.to_str())
            != Some(crate::execution_index::INDEX_FILE_NAME)
        && path.extension().and_then(|extension| extension.to_str())
            == Some(EXECUTION_RECORD_EXTENSION)
}

fn read_execution_record_snapshot(path: &FsPath) -> Option<ExecutionRecord> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "failed to read execution record snapshot"
            );
            return None;
        }
    };
    let record: ExecutionRecord = match serde_json::from_slice(&bytes) {
        Ok(record) => record,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "failed to parse execution record snapshot"
            );
            return None;
        }
    };
    let expected_file_name = execution_record_snapshot_file_name(&record.execution_id);
    if path.file_name().and_then(|name| name.to_str()) != Some(expected_file_name.as_str()) {
        tracing::warn!(
            path = %path.display(),
            execution_id = %record.execution_id,
            "execution record snapshot filename does not match record id"
        );
        return None;
    }
    Some(record)
}

pub(crate) async fn list_executions(State(state): State<AppState>) -> Json<Vec<ExecutionRecord>> {
    Json(state.execution_store.list())
}

pub(crate) async fn get_execution(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
) -> Result<Json<ExecutionRecord>, ApiError> {
    state
        .execution_store
        .get(&execution_id)
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("execution not found: {execution_id}")))
}

pub(crate) async fn get_execution_node(
    State(state): State<AppState>,
    Path((execution_id, node_id)): Path<(String, u64)>,
) -> Result<Json<NodeExecutionDetail>, ApiError> {
    state
        .execution_store
        .get_node(&execution_id, node_id)
        .map(Json)
        .map_err(execution_node_lookup_error)
}

#[derive(Debug)]
pub(crate) enum ExecutionNodeLookupError {
    ExecutionNotFound(String),
    NodeNotFound { execution_id: String, node_id: u64 },
}

fn execution_node_lookup_error(error: ExecutionNodeLookupError) -> ApiError {
    match error {
        ExecutionNodeLookupError::ExecutionNotFound(execution_id) => {
            ApiError::not_found(format!("execution not found: {execution_id}"))
        }
        ExecutionNodeLookupError::NodeNotFound {
            execution_id,
            node_id,
        } => ApiError::not_found(format!(
            "node output not found for execution {execution_id}: {node_id}"
        )),
    }
}

pub(crate) struct ExecutionRecordingEmitter {
    execution_store: ExecutionStore,
    execution_id: String,
}

impl ExecutionRecordingEmitter {
    pub(crate) fn new(execution_store: ExecutionStore, execution_id: impl Into<String>) -> Self {
        Self {
            execution_store,
            execution_id: execution_id.into(),
        }
    }
}

impl EventEmitter for ExecutionRecordingEmitter {
    fn emit(&self, event: ApxmEvent) {
        if let Some(payload) = event.payload.downcast_ref::<NodeOutputPayload>() {
            self.execution_store.record_node_output(
                &self.execution_id,
                payload.node_id,
                payload.node_name.clone(),
                payload.output.clone(),
            );
        }
        if let Some(payload) = event.payload.downcast_ref::<NodeMetricsPayload>() {
            self.execution_store.record_node_metrics(
                &self.execution_id,
                payload.node_id,
                payload.node_name.clone(),
                payload.metrics.clone(),
            );
        }
    }
}

