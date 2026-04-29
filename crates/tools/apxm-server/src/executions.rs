use std::sync::Arc;

use apxm_core::events::payload::{NodeMetricsPayload, NodeOutputPayload, RedactedContent};
use apxm_core::events::{ApxmEvent, EventEmitter};
use apxm_core::types::NodeMetrics;
use axum::Json;
use axum::extract::{Path, State};
use dashmap::DashMap;
use serde::Serialize;

use crate::error::ApiError;
use crate::execute::ExecuteResponse;
use crate::helpers::now_ms;
use crate::state::AppState;

pub(crate) const EXECUTION_RECORDS_DIR: &str = "executions";
pub(crate) const EXECUTION_RECORD_EXTENSION: &str = "json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExecutionStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ExecutionRecord {
    pub(crate) execution_id: String,
    pub(crate) skill_id: String,
    pub(crate) skill_version: String,
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
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct NodeOutputRecord {
    pub(crate) node_id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) node_name: Option<String>,
    pub(crate) observed_at_ms: u64,
    pub(crate) output: RedactedContent,
}

#[derive(Debug, Clone, Serialize)]
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
}

impl ExecutionStore {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn start_skill_execution(
        &self,
        skill_id: &str,
        skill_version: &str,
        session_id: &str,
        session_dir: &str,
    ) -> ExecutionRecord {
        let record = ExecutionRecord {
            execution_id: uuid::Uuid::new_v4().to_string(),
            skill_id: skill_id.to_string(),
            skill_version: skill_version.to_string(),
            session_id: session_id.to_string(),
            session_dir: session_dir.to_string(),
            status: ExecutionStatus::Running,
            started_at_ms: now_ms(),
            completed_at_ms: None,
            result: None,
            error: None,
            node_outputs: Vec::new(),
            node_metrics: Vec::new(),
        };
        self.inner
            .insert(record.execution_id.clone(), record.clone());
        persist_record_snapshot(&record);
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
        Some(record)
    }

    pub(crate) fn get(&self, execution_id: &str) -> Option<ExecutionRecord> {
        self.inner.get(execution_id).map(|entry| entry.clone())
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
        .join(format!("{execution_id}.{EXECUTION_RECORD_EXTENSION}"))
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

#[cfg(test)]
mod tests {
    use apxm_core::events::EventSource;

    use super::*;

    const TEST_SKILL_ID: &str = "checkout-context-triage";
    const TEST_SKILL_VERSION: &str = "0.1.0";
    const TEST_SESSION_ID: &str = "session-1";
    const TEST_NODE_ID: u64 = 7;
    const TEST_NODE_NAME: &str = "fetch_context";
    const TEST_NODE_OUTPUT: &str = "private context";

    #[test]
    fn execution_recording_emitter_persists_node_names() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = ExecutionStore::new();
        let record = store.start_skill_execution(
            TEST_SKILL_ID,
            TEST_SKILL_VERSION,
            TEST_SESSION_ID,
            temp.path().to_str().expect("utf-8 temp path"),
        );
        let emitter = ExecutionRecordingEmitter::new(store.clone(), record.execution_id.clone());

        emitter.emit(ApxmEvent::root(
            NodeOutputPayload {
                node_id: TEST_NODE_ID,
                node_name: Some(TEST_NODE_NAME.to_string()),
                output: RedactedContent::from_text(TEST_NODE_OUTPUT),
            },
            EventSource::Runtime,
            &record.execution_id,
        ));
        emitter.emit(ApxmEvent::root(
            NodeMetricsPayload {
                node_id: TEST_NODE_ID,
                node_name: Some(TEST_NODE_NAME.to_string()),
                metrics: NodeMetrics::new(TEST_NODE_ID),
            },
            EventSource::Runtime,
            &record.execution_id,
        ));

        let detail = store
            .get_node(&record.execution_id, TEST_NODE_ID)
            .expect("node execution detail");
        assert_eq!(detail.outputs[0].node_name.as_deref(), Some(TEST_NODE_NAME));
        assert_eq!(detail.metrics[0].node_name.as_deref(), Some(TEST_NODE_NAME));
    }
}
