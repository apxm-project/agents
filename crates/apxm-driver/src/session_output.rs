//! Session output writer for persisting execution results to disk.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use apxm_core::constants;
use apxm_core::types::values::Value;
use apxm_events::{ApxmEvent, EventSource};
use apxm_events::payload::{
    EventPayload, OperationStartPayload, OperationEndPayload,
};
use apxm_runtime::ExecutionEventEmitter;


/// Writes session output files to a directory.
pub struct SessionOutputWriter {
    session_dir: PathBuf,
}

use apxm_core::types::SessionManifest;

/// Serialize to pretty JSON and write to a file.
fn json_pretty_write(path: &Path, value: &(impl serde::Serialize + ?Sized)) -> io::Result<()> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(path, json)
}

/// Result key names used in session results JSON.
mod result_keys {
    pub const NODE_OUTPUTS: &str = "node_outputs";
    pub const TOKEN_VALUES: &str = "token_values";
    pub const EXIT_VALUES: &str = "exit_values";
}

impl SessionOutputWriter {
    /// Create a new writer, creating the session directory.
    pub fn new(base_dir: &Path, execution_id: &str) -> io::Result<Self> {
        let session_dir = base_dir.join(execution_id);
        fs::create_dir_all(&session_dir)?;
        Ok(Self { session_dir })
    }

    /// Path to the session directory.
    pub fn session_dir(&self) -> &Path {
        &self.session_dir
    }

    /// Write the execution manifest with the given status/duration/node_count.
    /// Use `status = constants::session::status::RUNNING` before execution,
    /// and `COMPLETED`/`FAILED` after.
    pub fn write_manifest(
        &self,
        execution_id: &str,
        graph_name: Option<&str>,
        status: &str,
        duration_ms: u128,
        node_count: usize,
        success: bool,
    ) -> io::Result<()> {
        let manifest = SessionManifest {
            execution_id: execution_id.to_string(),
            graph_name: graph_name.map(|s| s.to_string()),
            timestamp: chrono::Utc::now().to_rfc3339(),
            status: status.to_string(),
            duration_ms,
            node_count,
            success,
        };
        json_pretty_write(
            &self.session_dir.join(constants::session::files::MANIFEST),
            &manifest,
        )
    }

    /// Copy the input graph for reproducibility.
    pub fn write_input_graph(&self, graph_json: &str) -> io::Result<()> {
        fs::write(
            self.session_dir.join(constants::session::files::INPUT_GRAPH),
            graph_json,
        )
    }

    /// Write all node outputs and token values.
    pub fn write_results(
        &self,
        all_outputs: &HashMap<u64, Value>,
        node_map: &HashMap<u64, Vec<u64>>,
        exit_values: &HashMap<u64, Value>,
    ) -> io::Result<()> {
        let results = serde_json::json!({
            result_keys::NODE_OUTPUTS: node_map,
            result_keys::TOKEN_VALUES: all_outputs.iter()
                .map(|(k, v)| (k.to_string(), serde_json::to_value(v).unwrap_or_default()))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
            result_keys::EXIT_VALUES: exit_values.iter()
                .map(|(k, v)| (k.to_string(), serde_json::to_value(v).unwrap_or_default()))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
        });
        json_pretty_write(
            &self.session_dir.join(constants::session::files::RESULTS),
            &results,
        )
    }

    /// Write metrics JSON.
    pub fn write_metrics(&self, metrics_json: &serde_json::Value) -> io::Result<()> {
        json_pretty_write(
            &self.session_dir.join(constants::session::files::METRICS),
            metrics_json,
        )
    }

    /// Write per-node execution statuses.
    pub fn write_node_statuses(&self, statuses: &[apxm_core::types::NodeStatus]) -> io::Result<()> {
        json_pretty_write(
            &self.session_dir.join(constants::session::files::NODE_STATUSES),
            statuses,
        )
    }

    /// Finalize session after execution: write manifest, results, metrics, node statuses.
    pub fn finalize(
        &self,
        execution_id: &str,
        graph_name: Option<&str>,
        duration_ms: u128,
        node_count: usize,
        success: bool,
        all_outputs: Option<&HashMap<u64, Value>>,
        node_map: Option<&HashMap<u64, Vec<u64>>>,
        exit_values: &HashMap<u64, Value>,
        metrics_json: &serde_json::Value,
        node_statuses: &[apxm_core::types::NodeStatus],
    ) -> io::Result<()> {
        let status = if success {
            constants::session::status::COMPLETED
        } else {
            constants::session::status::FAILED
        };
        self.write_manifest(execution_id, graph_name, status, duration_ms, node_count, success)?;

        if let (Some(all_outputs), Some(node_map)) = (all_outputs, node_map) {
            self.write_results(all_outputs, node_map, exit_values)?;
        }

        self.write_metrics(metrics_json)?;
        self.write_node_statuses(node_statuses)?;
        Ok(())
    }

    /// Path where events should be written.
    pub fn events_path(&self) -> PathBuf {
        self.session_dir.join(constants::session::files::TRACE)
    }
}

/// Appends events as JSONL to a file.
pub struct FileEventSink {
    writer: BufWriter<fs::File>,
}

impl FileEventSink {
    /// Create a new sink writing to the given path.
    pub fn new(path: &Path) -> io::Result<Self> {
        let file = fs::File::create(path)?;
        Ok(Self {
            writer: BufWriter::new(file),
        })
    }

    /// Write a single event as one JSON line.
    pub fn write_event(&mut self, event: &ApxmEvent) -> io::Result<()> {
        let line = serde_json::to_string(event)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        writeln!(self.writer, "{}", line)
    }

    /// Flush buffered writes.
    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

/// Live session event emitter that writes events to trace.ndjson and live.json
/// during execution.
pub struct SessionEventEmitter {
    sink: Mutex<FileEventSink>,
    session_dir: PathBuf,
    trace_id: Arc<str>,
    start_time: Instant,
    completed: AtomicU64,
    seq: AtomicU64,
}

impl SessionEventEmitter {
    /// Create a new emitter writing to the given session directory.
    ///
    /// The session directory must already exist. Creates `trace.ndjson` and
    /// writes the initial `live.json`.
    pub fn new(session_dir: &Path, trace_id: String) -> io::Result<Self> {
        let trace_path = session_dir.join(constants::session::files::TRACE);
        let sink = FileEventSink::new(&trace_path)?;

        let emitter = Self {
            sink: Mutex::new(sink),
            session_dir: session_dir.to_path_buf(),
            trace_id: Arc::from(trace_id),
            start_time: Instant::now(),
            completed: AtomicU64::new(0),
            seq: AtomicU64::new(0),
        };

        // Write initial live.json
        emitter.write_live(None)?;

        Ok(emitter)
    }

    fn write_trace_event(&self, payload: EventPayload) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let event = ApxmEvent::new(payload, EventSource::Runtime, &*self.trace_id)
            .with_seq(seq);
        if let Ok(mut sink) = self.sink.lock() {
            let _ = sink.write_event(&event);
            let _ = sink.flush();
        }
    }

    fn write_live(&self, current_node_id: Option<u64>) -> io::Result<()> {
        let completed = self.completed.load(Ordering::Relaxed);
        let elapsed_ms = self.start_time.elapsed().as_millis();

        // Always report "running" — the manifest handles final status after execution.
        let live = serde_json::json!({
            "status": constants::session::status::RUNNING,
            "current_node_id": current_node_id,
            "completed": completed,
            "elapsed_ms": elapsed_ms,
        });

        // Atomic write: write to tmp, then rename
        let tmp_path = self.session_dir.join(".live.json.tmp");
        json_pretty_write(&tmp_path, &live)?;
        let live_path = self.session_dir.join(constants::session::files::LIVE);
        fs::rename(&tmp_path, &live_path)?;

        Ok(())
    }
}

impl ExecutionEventEmitter for SessionEventEmitter {
    fn emit_llm_token(&self, content: &str) {
        self.write_trace_event(EventPayload::Token(
            apxm_events::payload::TokenPayload {
                text: content.to_string(),
            },
        ));
    }

    fn emit_tool_start(&self, name: &str, args: &HashMap<String, apxm_core::types::values::Value>) {
        let args_json = args
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    serde_json::to_value(v).unwrap_or_default(),
                )
            })
            .collect();
        self.write_trace_event(EventPayload::ToolStart(
            apxm_events::payload::ToolStartPayload {
                name: name.to_string(),
                args: args_json,
            },
        ));
    }

    fn emit_tool_end(&self, name: &str, result: &apxm_core::types::values::Value) {
        self.write_trace_event(EventPayload::ToolEnd(
            apxm_events::payload::ToolEndPayload {
                name: name.to_string(),
                result: serde_json::to_value(result).unwrap_or_default(),
            },
        ));
    }

    fn emit_operation_start(&self, node_id: u64, op_type: &str) {
        self.write_trace_event(EventPayload::OperationStart(OperationStartPayload {
            node_id,
            op_type: op_type.to_string(),
        }));
        let _ = self.write_live(Some(node_id));
    }

    fn emit_operation_end(&self, node_id: u64, op_type: &str, duration: std::time::Duration, success: bool) {
        self.completed.fetch_add(1, Ordering::Relaxed);
        self.write_trace_event(EventPayload::OperationEnd(OperationEndPayload {
            node_id,
            op_type: op_type.to_string(),
            duration_ms: duration.as_millis() as u64,
            success,
        }));
        let _ = self.write_live(Some(node_id));
    }

    fn emit_plan_created(&self, plan_id: &str, steps: usize) {
        self.write_trace_event(EventPayload::PlanCreated(
            apxm_events::payload::PlanCreatedPayload {
                plan_id: plan_id.to_string(),
                steps,
            },
        ));
    }

    fn emit_plan_step_started(&self, plan_id: &str, step_index: usize) {
        self.write_trace_event(EventPayload::PlanStepStarted(
            apxm_events::payload::PlanStepStartedPayload {
                plan_id: plan_id.to_string(),
                step_index,
            },
        ));
    }

    fn emit_plan_step_completed(&self, plan_id: &str, step_index: usize, success: bool) {
        self.write_trace_event(EventPayload::PlanStepCompleted(
            apxm_events::payload::PlanStepCompletedPayload {
                plan_id: plan_id.to_string(),
                step_index,
                success,
            },
        ));
    }

    fn emit_memory_read(&self, scope: &str, key: &str) {
        self.write_trace_event(EventPayload::MemoryRead(
            apxm_events::payload::MemoryReadPayload {
                scope: scope.to_string(),
                key: key.to_string(),
            },
        ));
    }

    fn emit_memory_write(&self, scope: &str, key: &str) {
        self.write_trace_event(EventPayload::MemoryWrite(
            apxm_events::payload::MemoryWritePayload {
                scope: scope.to_string(),
                key: key.to_string(),
            },
        ));
    }

    fn emit_checkpoint_saved(&self, checkpoint_id: &str) {
        self.write_trace_event(EventPayload::CheckpointSaved(
            apxm_events::payload::CheckpointSavedPayload {
                checkpoint_id: checkpoint_id.to_string(),
            },
        ));
    }

    fn emit_checkpoint_restored(&self, checkpoint_id: &str) {
        self.write_trace_event(EventPayload::CheckpointRestored(
            apxm_events::payload::CheckpointRestoredPayload {
                checkpoint_id: checkpoint_id.to_string(),
            },
        ));
    }

    fn emit_scheduler_decision(&self, node_id: u64, delay: std::time::Duration, reason: &str) {
        self.write_trace_event(EventPayload::SchedulerDecision(
            apxm_events::payload::SchedulerDecisionPayload {
                node_id,
                delay_ms: delay.as_millis() as u64,
                reason: reason.to_string(),
            },
        ));
    }

    fn emit_gpu_utilization(&self, gpu_id: u32, utilization_pct: f32, memory_pct: f32) {
        self.write_trace_event(EventPayload::GpuUtilization(
            apxm_events::payload::GpuUtilizationPayload {
                gpu_id,
                utilization_pct,
                memory_pct,
            },
        ));
    }

    fn emit_token_usage(&self, node_id: u64, input_tokens: usize, output_tokens: usize) {
        self.write_trace_event(EventPayload::TokenUsage(
            apxm_events::payload::TokenUsagePayload {
                node_id,
                input_tokens,
                output_tokens,
            },
        ));
    }

    fn emit_memoization_hit(&self, node_id: u64) {
        self.write_trace_event(EventPayload::MemoizationHit(
            apxm_events::payload::MemoizationHitPayload { node_id },
        ));
    }
}
