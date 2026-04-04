//! Session output writer for persisting execution results to disk.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use apxm_core::constants;
use apxm_core::paths::session_node_dir_name;
use apxm_core::types::SessionManifest;
use apxm_core::types::values::Value;
use apxm_events::payload::{EventPayload, OperationEndPayload, OperationStartPayload};
use apxm_events::{ApxmEvent, EventSource};
use apxm_runtime::ExecutionEventEmitter;

use crate::context_assembler::{ContextAssembler, WorkspaceNodeMetadata};
use crate::skill_resolver::SkillResolver;

/// Writes session output files to a directory.
pub struct SessionOutputWriter {
    session_dir: PathBuf,
}

/// Serialize to pretty JSON and write to a file.
fn json_pretty_write(path: &Path, value: &(impl serde::Serialize + ?Sized)) -> io::Result<()> {
    let json =
        serde_json::to_string_pretty(value).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
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
            self.session_dir
                .join(constants::session::files::INPUT_GRAPH),
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
            &self
                .session_dir
                .join(constants::session::files::NODE_STATUSES),
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
        self.write_manifest(
            execution_id,
            graph_name,
            status,
            duration_ms,
            node_count,
            success,
        )?;

        if let (Some(all_outputs), Some(node_map)) = (all_outputs, node_map) {
            self.write_results(all_outputs, node_map, exit_values)?;
        }

        self.write_metrics(metrics_json)?;
        self.write_node_statuses(node_statuses)?;

        // Write final live.json with correct completed/failed status.
        // This is the definitive fix: live.json must not stay "running" after execution ends.
        let live = serde_json::json!({
            "status": status,
            "current_node_id": null,
            "completed": node_statuses.len(),
            "total": node_statuses.len(),
            "elapsed_ms": duration_ms,
            "success": success,
        });
        let tmp_path = self.session_dir.join(".live.json.tmp");
        json_pretty_write(&tmp_path, &live)?;
        let live_path = self.session_dir.join(constants::session::files::LIVE);
        fs::rename(&tmp_path, &live_path)?;

        Ok(())
    }

    /// Write final live.json AND manifest with completed/failed status.
    /// Call this on error paths where finalize() won't be reached.
    /// Requires execution_id and graph_name so both files are consistent.
    pub fn finalize_live(&self, success: bool) -> io::Result<()> {
        self.finalize_live_with_id(success, None, None)
    }

    /// Full error-path finalization with execution id and graph name for manifest.
    pub fn finalize_live_with_id(
        &self,
        success: bool,
        execution_id: Option<&str>,
        graph_name: Option<&str>,
    ) -> io::Result<()> {
        let status = if success {
            constants::session::status::COMPLETED
        } else {
            constants::session::status::FAILED
        };

        // Write live.json
        let live = serde_json::json!({
            "status": status,
            "current_node_id": null,
            "completed": null,
            "total": null,
            "elapsed_ms": null,
            "success": success,
        });
        let tmp_path = self.session_dir.join(".live.json.tmp");
        json_pretty_write(&tmp_path, &live)?;
        let live_path = self.session_dir.join(constants::session::files::LIVE);
        fs::rename(&tmp_path, &live_path)?;

        // Also update manifest.json so it doesn't stay at "running"
        if let Some(exec_id) = execution_id {
            self.write_manifest(exec_id, graph_name, status, 0, 0, success)?;
        }

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
        let line =
            serde_json::to_string(event).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
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
    total: AtomicU64,
    seq: AtomicU64,
    current_node_id: AtomicI64,
    node_metadata: Arc<HashMap<u64, WorkspaceNodeMetadata>>,
    node_traces: Mutex<HashMap<u64, FileEventSink>>,
    node_llm_tokens: Mutex<HashMap<u64, Vec<String>>>,
    skill_resolver: Option<SkillResolver>,
    context_assembler: Option<ContextAssembler>,
}

impl SessionEventEmitter {
    /// Create a new emitter writing to the given session directory.
    pub fn new(
        session_dir: &Path,
        trace_id: String,
        input_graph_json: Option<&str>,
        project_root: Option<&Path>,
    ) -> io::Result<Self> {
        let trace_path = session_dir.join(constants::session::files::TRACE);
        let sink = FileEventSink::new(&trace_path)?;
        fs::create_dir_all(session_dir.join(constants::session::files::NODES_DIR))?;

        let mut node_metadata = HashMap::new();
        let mut graph_edges = Vec::new();
        if let Some(json) = input_graph_json
            && let Ok(graph) = apxm_graph::ApxmGraph::from_json(json)
        {
            for node in graph.nodes {
                node_metadata.insert(
                    node.id,
                    WorkspaceNodeMetadata {
                        name: node.name,
                        op_type: node.op,
                        attributes: node.attributes,
                    },
                );
            }
            graph_edges = graph
                .edges
                .into_iter()
                .map(|edge| (edge.from, edge.to))
                .collect();
        }

        let node_metadata = Arc::new(node_metadata);
        let graph_edges = Arc::new(graph_edges);
        let skill_resolver = project_root.and_then(|root| SkillResolver::new(root).ok());
        let context_assembler = project_root.map(|root| {
            ContextAssembler::new(
                session_dir.to_path_buf(),
                trace_id.clone(),
                root.to_path_buf(),
                Arc::clone(&node_metadata),
                Arc::clone(&graph_edges),
            )
        });

        let emitter = Self {
            sink: Mutex::new(sink),
            session_dir: session_dir.to_path_buf(),
            trace_id: Arc::from(trace_id),
            start_time: Instant::now(),
            completed: AtomicU64::new(0),
            total: AtomicU64::new(0),
            seq: AtomicU64::new(0),
            current_node_id: AtomicI64::new(-1),
            node_metadata,
            node_traces: Mutex::new(HashMap::new()),
            node_llm_tokens: Mutex::new(HashMap::new()),
            skill_resolver,
            context_assembler,
        };

        emitter.write_live(None)?;
        Ok(emitter)
    }

    fn node_workspace_dir(&self, node_id: u64) -> Option<PathBuf> {
        self.node_metadata.get(&node_id).map(|meta| {
            self.session_dir
                .join(constants::session::files::NODES_DIR)
                .join(session_node_dir_name(node_id, &meta.name))
        })
    }

    fn write_trace_event(&self, payload: EventPayload) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let event = ApxmEvent::new(payload, EventSource::Runtime, &*self.trace_id).with_seq(seq);
        if let Ok(mut sink) = self.sink.lock() {
            let _ = sink.write_event(&event);
            let _ = sink.flush();
        }
    }

    fn write_node_trace_event(&self, node_id: u64, payload: EventPayload) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let event = ApxmEvent::new(payload, EventSource::Runtime, &*self.trace_id).with_seq(seq);
        if let Ok(mut traces) = self.node_traces.lock()
            && let Some(sink) = traces.get_mut(&node_id)
        {
            let _ = sink.write_event(&event);
            let _ = sink.flush();
        }
    }

    fn ensure_node_workspace(&self, node_id: u64) {
        let Some(meta) = self.node_metadata.get(&node_id) else {
            return;
        };
        let Some(node_dir) = self.node_workspace_dir(node_id) else {
            return;
        };
        if fs::create_dir_all(&node_dir).is_err() {
            return;
        }

        let node_info = serde_json::json!({
            "id": node_id,
            "name": meta.name,
            "op": format!("{:?}", meta.op_type),
            "attributes": serde_json::to_value(&meta.attributes).unwrap_or_default(),
        });
        let _ = json_pretty_write(
            &node_dir.join(constants::session::node::NODE_JSON),
            &node_info,
        );

        let live = serde_json::json!({
            "status": constants::session::status::RUNNING,
            "started_at_ms": self.start_time.elapsed().as_millis(),
            "elapsed_ms": 0,
        });
        let _ = json_pretty_write(&node_dir.join(constants::session::node::LIVE_JSON), &live);

        if let Ok(mut traces) = self.node_traces.lock()
            && !traces.contains_key(&node_id)
            && let Ok(sink) =
                FileEventSink::new(&node_dir.join(constants::session::node::TRACE_NDJSON))
        {
            traces.insert(node_id, sink);
        }
        if let Ok(mut tokens) = self.node_llm_tokens.lock() {
            tokens.entry(node_id).or_default();
        }

        let mut skill_names = Vec::new();
        if let Some(resolver) = &self.skill_resolver {
            let resolved = resolver.resolve(meta.op_type, &meta.attributes);
            if !resolved.is_empty() {
                let skills_dir = node_dir.join(constants::session::node::SKILLS_DIR);
                let _ = fs::create_dir_all(&skills_dir);
                for skill_path in resolved {
                    let skill_name = resolver.skill_name(&skill_path);
                    let _ = resolver.copy_skill_to(&skill_path, &skills_dir);
                    skill_names.push(skill_name);
                }
            }
        }

        if let Some(assembler) = &self.context_assembler {
            if let Some(profile) = meta
                .attributes
                .get(constants::graph::attrs::PROFILE)
                .and_then(|v| v.as_str())
            {
                match profile {
                    "claude" => {
                        if let Ok(contents) =
                            assembler.assemble_claude_md(node_id, meta, &skill_names)
                        {
                            let _ = fs::write(node_dir.join("CLAUDE.md"), contents);
                        }
                    }
                    "codex" => {
                        if let Ok(contents) =
                            assembler.assemble_agents_md(node_id, meta, &skill_names)
                        {
                            let _ = fs::write(node_dir.join("AGENTS.md"), contents);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn write_live(&self, current_node_id: Option<u64>) -> io::Result<()> {
        if let Some(id) = current_node_id {
            self.current_node_id.store(id as i64, Ordering::Relaxed);
        }
        self.write_live_with_status(current_node_id, constants::session::status::RUNNING, false)
    }

    fn write_live_with_status(
        &self,
        current_node_id: Option<u64>,
        status: &str,
        success: bool,
    ) -> io::Result<()> {
        let completed = self.completed.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        let elapsed_ms = self.start_time.elapsed().as_millis();

        let live = serde_json::json!({
            "status": status,
            "current_node_id": current_node_id,
            "completed": completed,
            "total": if total > 0 { Some(total) } else { None::<u64> },
            "elapsed_ms": elapsed_ms,
            "success": success,
        });

        let tmp_path = self.session_dir.join(".live.json.tmp");
        json_pretty_write(&tmp_path, &live)?;
        let live_path = self.session_dir.join(constants::session::files::LIVE);
        fs::rename(&tmp_path, &live_path)?;
        Ok(())
    }

    fn write_node_completion(&self, node_id: u64, duration: std::time::Duration, success: bool) {
        let Some(node_dir) = self.node_workspace_dir(node_id) else {
            return;
        };
        let status = serde_json::json!({
            "status": if success { constants::session::status::COMPLETED } else { constants::session::status::FAILED },
            "duration_ms": duration.as_millis(),
            "retries": 0,
            "error": if success { serde_json::Value::Null } else { serde_json::Value::String("operation failed".to_string()) },
        });
        let live = serde_json::json!({
            "status": if success { constants::session::status::COMPLETED } else { constants::session::status::FAILED },
            "duration_ms": duration.as_millis(),
        });
        let _ = json_pretty_write(
            &node_dir.join(constants::session::node::STATUS_JSON),
            &status,
        );
        let _ = json_pretty_write(&node_dir.join(constants::session::node::LIVE_JSON), &live);

        if let Ok(mut traces) = self.node_traces.lock()
            && let Some(mut sink) = traces.remove(&node_id)
        {
            let _ = sink.flush();
        }

        if let Ok(mut tokens) = self.node_llm_tokens.lock()
            && let Some(chunks) = tokens.remove(&node_id)
        {
            let _ = fs::write(
                node_dir.join(constants::session::node::RESPONSE_TXT),
                chunks.join(""),
            );
        }
    }

    pub fn tick(&self) {
        let node_id = self.current_node_id.load(Ordering::Relaxed);
        let current = if node_id >= 0 {
            Some(node_id as u64)
        } else {
            None
        };
        let _ = self.write_live(current);
    }

    pub fn finalize_live(&self, success: bool) -> io::Result<()> {
        let status = if success {
            constants::session::status::COMPLETED
        } else {
            constants::session::status::FAILED
        };
        let completed = self.completed.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        let elapsed_ms = self.start_time.elapsed().as_millis();

        let live = serde_json::json!({
            "status": status,
            "current_node_id": null,
            "completed": completed,
            "total": if total > 0 { Some(total) } else { None::<u64> },
            "elapsed_ms": elapsed_ms,
            "success": success,
        });
        let tmp_path = self.session_dir.join(".live.json.tmp");
        json_pretty_write(&tmp_path, &live)?;
        let live_path = self.session_dir.join(constants::session::files::LIVE);
        fs::rename(&tmp_path, &live_path)
    }

    pub fn set_total_nodes(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }
}

impl ExecutionEventEmitter for SessionEventEmitter {
    fn emit_llm_token(&self, content: &str) {
        self.write_trace_event(EventPayload::Token(apxm_events::payload::TokenPayload {
            text: content.to_string(),
        }));
    }

    fn emit_llm_token_for_node(&self, node_id: u64, content: &str) {
        let payload = EventPayload::Token(apxm_events::payload::TokenPayload {
            text: content.to_string(),
        });
        self.write_trace_event(payload.clone());
        self.write_node_trace_event(node_id, payload);
        if let Ok(mut tokens) = self.node_llm_tokens.lock() {
            tokens.entry(node_id).or_default().push(content.to_string());
        }
    }

    fn emit_llm_prompt(&self, node_id: u64, prompt: &str) {
        let Some(node_dir) = self.node_workspace_dir(node_id) else {
            return;
        };
        let _ = fs::write(node_dir.join(constants::session::node::PROMPT_TXT), prompt);
        if let Ok(mut tokens) = self.node_llm_tokens.lock() {
            tokens.insert(node_id, Vec::new());
        }
    }

    fn emit_tool_start(&self, name: &str, args: &HashMap<String, apxm_core::types::values::Value>) {
        let args_json = args
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or_default()))
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
        self.ensure_node_workspace(node_id);
        let payload = EventPayload::OperationStart(OperationStartPayload {
            node_id,
            op_type: op_type.to_string(),
        });
        self.write_trace_event(payload.clone());
        self.write_node_trace_event(node_id, payload);
        let _ = self.write_live(Some(node_id));
    }

    fn emit_operation_end(
        &self,
        node_id: u64,
        op_type: &str,
        duration: std::time::Duration,
        success: bool,
    ) {
        self.completed.fetch_add(1, Ordering::Relaxed);
        let payload = EventPayload::OperationEnd(OperationEndPayload {
            node_id,
            op_type: op_type.to_string(),
            duration_ms: duration.as_millis() as u64,
            success,
        });
        self.write_trace_event(payload.clone());
        self.write_node_trace_event(node_id, payload);
        self.write_node_completion(node_id, duration, success);
        let _ = self.write_live(Some(node_id));
    }

    fn emit_node_output(&self, node_id: u64, value: &Value) {
        let Some(node_dir) = self.node_workspace_dir(node_id) else {
            return;
        };
        let output_json = serde_json::to_value(value).unwrap_or_default();
        let _ = json_pretty_write(
            &node_dir.join(constants::session::node::OUTPUT_JSON),
            &output_json,
        );
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
