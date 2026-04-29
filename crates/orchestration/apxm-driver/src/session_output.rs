//! Session output writer for persisting execution results to disk.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use apxm_compiler::AirModule;
use apxm_core::constants;
use apxm_core::events::payload::EventPayload;
use apxm_core::events::payload::{OperationEndPayload, OperationStartPayload};
use apxm_core::events::{ApxmEvent, EventSource};
use apxm_core::paths::session_node_dir_name;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_core::types::{
    CompletedNodeInfo, LiveSessionState, NodeInfo, SessionManifest, SessionStatus,
};
use apxm_runtime::ExecutionEventEmitter;
use parking_lot::RwLock;

use crate::context_assembler::{ContextAssembler, WorkspaceNodeMetadata};
use crate::skill_resolver::SkillResolver;

const CLAUDE_PROFILE: &str = "claude";
const CODEX_PROFILE: &str = "codex";
const CLAUDE_CONTEXT_FILE: &str = "CLAUDE.md";
const CODEX_CONTEXT_FILE: &str = "AGENTS.md";

/// Writes session output files to a directory.
pub struct SessionOutputWriter {
    session_dir: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct SessionProvenance {
    pub scope_id: Option<String>,
    pub parent_execution_id: Option<String>,
    pub parent_session_dir: Option<String>,
    pub parent_scope_id: Option<String>,
    pub spawn_node_id: Option<u64>,
}

/// Serialize to pretty JSON and write to a file.
fn json_pretty_write(path: &Path, value: &(impl serde::Serialize + ?Sized)) -> io::Result<()> {
    let json =
        serde_json::to_string_pretty(value).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(path, json)
}

fn insert_optional_string(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), serde_json::Value::String(value));
    }
}

/// Pick the entry function's terminal output for `results.json::final_output`.
///
/// Single exit → that value's string form (or JSON-stringified non-string).
/// Multi-exit → newline-joined string forms in ascending node-id order so the
/// concatenation is deterministic for rubric matching.
/// No exit values → empty string + null id, matching the previous shape's
/// implicit behavior of not surfacing a result.
fn derive_final_output(
    exit_values: &HashMap<u64, Value>,
    fallback_outputs: &HashMap<u64, Value>,
) -> (Option<u64>, String) {
    let stringify = |v: &Value| -> String {
        match v {
            Value::String(s) => s.clone(),
            other => serde_json::to_string(other).unwrap_or_default(),
        }
    };
    if exit_values.is_empty() {
        // Some graphs don't surface a typed exit value; fall back to the
        // highest-numbered token output if anything was captured. Yields
        // ("", None) when the run produced no values at all.
        if let Some((id, v)) = fallback_outputs.iter().max_by_key(|(k, _)| **k) {
            return (Some(*id), stringify(v));
        }
        return (None, String::new());
    }
    if exit_values.len() == 1 {
        let (id, v) = exit_values.iter().next().unwrap();
        return (Some(*id), stringify(v));
    }
    let mut ids: Vec<u64> = exit_values.keys().copied().collect();
    ids.sort_unstable();
    let joined = ids
        .iter()
        .map(|id| stringify(exit_values.get(id).expect("known key")))
        .collect::<Vec<_>>()
        .join("\n");
    (None, joined)
}

#[cfg(test)]
mod final_output_tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::String(v.to_string())
    }

    #[test]
    fn single_exit_returns_id_and_string() {
        let exit: HashMap<u64, Value> = [(7u64, s("Paris"))].into_iter().collect();
        let outs: HashMap<u64, Value> = HashMap::new();
        let (id, out) = derive_final_output(&exit, &outs);
        assert_eq!(id, Some(7));
        assert_eq!(out, "Paris");
    }

    #[test]
    fn multi_exit_concatenates_in_id_order() {
        let exit: HashMap<u64, Value> = [(2u64, s("b")), (1u64, s("a"))].into_iter().collect();
        let outs: HashMap<u64, Value> = HashMap::new();
        let (id, out) = derive_final_output(&exit, &outs);
        assert_eq!(id, None);
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn empty_exit_falls_back_to_highest_output_id() {
        let exit: HashMap<u64, Value> = HashMap::new();
        let outs: HashMap<u64, Value> = [(3u64, s("low")), (9u64, s("hi"))].into_iter().collect();
        let (id, out) = derive_final_output(&exit, &outs);
        assert_eq!(id, Some(9));
        assert_eq!(out, "hi");
    }

    #[test]
    fn empty_everywhere_returns_empty_string() {
        let (id, out) = derive_final_output(&HashMap::new(), &HashMap::new());
        assert_eq!(id, None);
        assert_eq!(out, "");
    }

    #[test]
    fn non_string_value_serialises_via_json() {
        let exit: HashMap<u64, Value> = [(5u64, Value::Bool(true))].into_iter().collect();
        let (_, out) = derive_final_output(&exit, &HashMap::new());
        assert_eq!(out, "true");
    }
}

/// Simple .air emitter for session output (avoids circular dependency on Compiler).
fn emit_air_simple(module: &AirModule) -> String {
    let mut out = String::new();
    out.push_str("; Agent IR (.air) — canonical intermediate representation\n");
    out.push_str(&format!("; graph: {}\n", module.name));
    for (k, v) in &module.metadata {
        out.push_str(&format!("; {}: {}\n", k, v));
    }
    out.push('\n');
    if !module.parameters.is_empty() {
        for p in &module.parameters {
            out.push_str(&format!("; param %{}: {}\n", p.name, p.type_name));
        }
        out.push('\n');
    }
    for node in &module.nodes {
        let op = node.op.to_string().to_lowercase();
        let mut attrs = vec![];
        for (k, v) in &node.attributes {
            if !k.starts_with('_') {
                attrs.push(format!("{} = {}", k, v));
            }
        }
        let attr_str = if attrs.is_empty() {
            String::new()
        } else {
            format!(" {}", attrs.join(", "))
        };
        out.push_str(&format!(
            "  %{} = {}({}){}\n",
            node.id, op, node.name, attr_str
        ));
    }
    if !module.edges.is_empty() {
        out.push('\n');
        for edge in &module.edges {
            let dep = format!("{:?}", edge.dependency).to_lowercase();
            out.push_str(&format!(
                "  edge %{} -> %{} [{}]\n",
                edge.from, edge.to, dep
            ));
        }
    }
    out
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

    pub fn write_manifest(
        &self,
        execution_id: &str,
        graph_name: Option<&str>,
        status: SessionStatus,
        duration_ms: u128,
        node_count: usize,
        success: bool,
    ) -> io::Result<()> {
        self.write_manifest_with_provenance(
            execution_id,
            graph_name,
            status,
            duration_ms,
            node_count,
            success,
            &SessionProvenance::default(),
        )
    }

    pub fn write_manifest_with_provenance(
        &self,
        execution_id: &str,
        graph_name: Option<&str>,
        status: SessionStatus,
        duration_ms: u128,
        node_count: usize,
        success: bool,
        provenance: &SessionProvenance,
    ) -> io::Result<()> {
        let manifest = SessionManifest {
            execution_id: execution_id.to_string(),
            graph_name: graph_name.map(|s| s.to_string()),
            timestamp: chrono::Utc::now().to_rfc3339(),
            status,
            duration_ms,
            node_count,
            success,
            scope_id: provenance.scope_id.clone(),
            parent_execution_id: provenance.parent_execution_id.clone(),
            parent_session_dir: provenance.parent_session_dir.clone(),
            parent_scope_id: provenance.parent_scope_id.clone(),
            spawn_node_id: provenance.spawn_node_id,
        };
        json_pretty_write(
            &self.session_dir.join(constants::session::files::MANIFEST),
            &manifest,
        )
    }

    /// Write the input graph in .air format for reproducibility.
    pub fn write_input_graph(&self, module: &AirModule) -> io::Result<()> {
        // Emit .air format using a simple inline emitter (to avoid circular dependency on Compiler)
        let air_text = emit_air_simple(module);
        fs::write(
            self.session_dir
                .join(constants::session::files::INPUT_GRAPH),
            air_text,
        )
    }

    fn write_results(
        &self,
        all_outputs: &HashMap<u64, Value>,
        node_map: &HashMap<u64, Vec<u64>>,
        exit_values: &HashMap<u64, Value>,
    ) -> io::Result<()> {
        // Phase C contract: tier-3 quality_eval reads `final_output` directly,
        // without having to parse `exit_values` and follow `node_map`. When the
        // entry function returns one value we surface its node id + string
        // form; for multi-return entries we concatenate (newline-separated) so
        // a rubric can still apply. Non-string returns serialise via JSON.
        let (final_node_id, final_output) = derive_final_output(exit_values, all_outputs);
        use constants::session::results_keys as rk;
        let results = serde_json::json!({
            rk::NODE_OUTPUTS: node_map,
            rk::TOKEN_VALUES: all_outputs.iter()
                .map(|(k, v)| (k.to_string(), serde_json::to_value(v).unwrap_or_default()))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
            rk::EXIT_VALUES: exit_values.iter()
                .map(|(k, v)| (k.to_string(), serde_json::to_value(v).unwrap_or_default()))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
            rk::FINAL_NODE_ID: final_node_id,
            rk::FINAL_OUTPUT: final_output,
        });
        json_pretty_write(
            &self.session_dir.join(constants::session::files::RESULTS),
            &results,
        )
    }

    fn write_metrics(&self, metrics_json: &serde_json::Value) -> io::Result<()> {
        json_pretty_write(
            &self.session_dir.join(constants::session::files::METRICS),
            metrics_json,
        )
    }

    fn write_node_statuses(&self, statuses: &[apxm_core::types::NodeStatus]) -> io::Result<()> {
        json_pretty_write(
            &self
                .session_dir
                .join(constants::session::files::NODE_STATUSES),
            statuses,
        )
    }

    fn write_episodic_entries(
        &self,
        entries: &[apxm_runtime::memory::EpisodicEntry],
    ) -> io::Result<()> {
        let path = self.session_dir.join("episodic.ndjson");
        let mut file = BufWriter::new(fs::File::create(&path)?);
        for entry in entries {
            let line = serde_json::to_string(entry)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            writeln!(file, "{}", line)?;
        }
        file.flush()?;
        Ok(())
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
        episodic_entries: Option<&[apxm_runtime::memory::EpisodicEntry]>,
    ) -> io::Result<()> {
        self.finalize_with_provenance(
            execution_id,
            graph_name,
            duration_ms,
            node_count,
            success,
            all_outputs,
            node_map,
            exit_values,
            metrics_json,
            node_statuses,
            episodic_entries,
            &SessionProvenance::default(),
        )
    }

    pub fn finalize_with_provenance(
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
        episodic_entries: Option<&[apxm_runtime::memory::EpisodicEntry]>,
        provenance: &SessionProvenance,
    ) -> io::Result<()> {
        let status = if success {
            SessionStatus::Completed
        } else {
            SessionStatus::Failed
        };
        self.write_manifest_with_provenance(
            execution_id,
            graph_name,
            status,
            duration_ms,
            node_count,
            success,
            provenance,
        )?;

        if let (Some(all_outputs), Some(node_map)) = (all_outputs, node_map) {
            self.write_results(all_outputs, node_map, exit_values)?;
        }

        self.write_metrics(metrics_json)?;
        self.write_node_statuses(node_statuses)?;

        // Export episodic entries for this execution
        if let Some(entries) = episodic_entries {
            self.write_episodic_entries(entries)?;
        }

        // Write final live.json with correct completed/failed status.
        let live = LiveSessionState {
            status,
            running_nodes: Vec::new(),
            completed_nodes: Vec::new(),
            completed: node_statuses.len(),
            total: Some(node_statuses.len()),
            elapsed_ms: duration_ms,
            success,
            current_phase: None,
        };
        let tmp_path = self.session_dir.join(".live.json.tmp");
        json_pretty_write(&tmp_path, &live)?;
        let live_path = self.session_dir.join(constants::session::files::LIVE);
        fs::rename(&tmp_path, &live_path)?;

        Ok(())
    }

    /// Write final live.json AND manifest with completed/failed status.
    /// Call this on error paths where finalize() won't be reached.
    pub fn finalize_live_with_id(
        &self,
        success: bool,
        execution_id: Option<&str>,
        graph_name: Option<&str>,
    ) -> io::Result<()> {
        self.finalize_live_with_id_and_provenance(
            success,
            execution_id,
            graph_name,
            &SessionProvenance::default(),
        )
    }

    pub fn finalize_live_with_id_and_provenance(
        &self,
        success: bool,
        execution_id: Option<&str>,
        graph_name: Option<&str>,
        provenance: &SessionProvenance,
    ) -> io::Result<()> {
        let status = if success {
            SessionStatus::Completed
        } else {
            SessionStatus::Failed
        };

        // Write live.json
        let live = LiveSessionState {
            status,
            running_nodes: Vec::new(),
            completed_nodes: Vec::new(),
            completed: 0,
            total: None,
            elapsed_ms: 0,
            success,
            current_phase: None,
        };
        let tmp_path = self.session_dir.join(".live.json.tmp");
        json_pretty_write(&tmp_path, &live)?;
        let live_path = self.session_dir.join(constants::session::files::LIVE);
        fs::rename(&tmp_path, &live_path)?;

        // Also update manifest.json so it doesn't stay at "running"
        if let Some(exec_id) = execution_id {
            self.write_manifest_with_provenance(
                exec_id, graph_name, status, 0, 0, success, provenance,
            )?;
        }

        Ok(())
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
    current_scope_id: RwLock<Option<String>>,
    node_metadata: Arc<HashMap<u64, WorkspaceNodeMetadata>>,
    node_traces: Mutex<HashMap<u64, FileEventSink>>,
    node_llm_tokens: Mutex<HashMap<u64, Vec<String>>>,
    provenance: SessionProvenance,
    skill_resolver: Option<SkillResolver>,
    context_assembler: parking_lot::Mutex<Option<ContextAssembler>>,
    running_nodes: Mutex<Vec<NodeInfo>>,
    completed_nodes: Mutex<Vec<CompletedNodeInfo>>,
}

impl SessionEventEmitter {
    /// Create a new emitter writing to the given session directory.
    pub fn new(
        session_dir: &Path,
        trace_id: String,
        input_graph: Option<&AirModule>,
        project_root: Option<&Path>,
    ) -> io::Result<Self> {
        Self::new_with_provenance(
            session_dir,
            trace_id,
            input_graph,
            project_root,
            SessionProvenance::default(),
        )
    }

    pub fn new_with_provenance(
        session_dir: &Path,
        trace_id: String,
        input_graph: Option<&AirModule>,
        project_root: Option<&Path>,
        provenance: SessionProvenance,
    ) -> io::Result<Self> {
        let trace_path = session_dir.join(constants::session::files::TRACE);
        let sink = FileEventSink::new(&trace_path)?;
        fs::create_dir_all(session_dir.join(constants::session::files::NODES_DIR))?;

        let mut node_metadata = HashMap::new();
        let mut graph_edges = Vec::new();
        if let Some(graph) = input_graph {
            for node in &graph.nodes {
                node_metadata.insert(
                    node.id,
                    WorkspaceNodeMetadata {
                        name: node.name.clone(),
                        op_type: node.op,
                        attributes: node.attributes.clone(),
                    },
                );
            }
            graph_edges = graph
                .edges
                .iter()
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
            current_scope_id: RwLock::new(provenance.scope_id.clone()),
            node_metadata,
            node_traces: Mutex::new(HashMap::new()),
            node_llm_tokens: Mutex::new(HashMap::new()),
            provenance,
            skill_resolver,
            context_assembler: parking_lot::Mutex::new(context_assembler),
            running_nodes: Mutex::new(Vec::new()),
            completed_nodes: Mutex::new(Vec::new()),
        };

        emitter.write_live(None)?;
        Ok(emitter)
    }

    /// Update the memory system reference in the context assembler
    pub fn set_memory(&self, memory: Arc<apxm_runtime::memory::MemorySystem>) {
        if let Some(mut assembler_opt) = self.context_assembler.try_lock() {
            if let Some(assembler) = assembler_opt.take() {
                *assembler_opt = Some(assembler.with_memory(memory));
            }
        }
    }

    fn node_workspace_dir(&self, node_id: u64) -> Option<PathBuf> {
        self.node_metadata.get(&node_id).map(|meta| {
            self.session_dir
                .join(constants::session::files::NODES_DIR)
                .join(session_node_dir_name(node_id, &meta.name))
        })
    }

    fn write_trace_event<P: EventPayload>(&self, payload: P) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let event = ApxmEvent::root(payload, EventSource::Runtime, &*self.trace_id)
            .with_scope_id(self.current_scope_id())
            .with_seq(seq);
        if let Ok(mut sink) = self.sink.lock() {
            let _ = sink.write_event(&event);
            let _ = sink.flush();
        }
    }

    fn write_node_trace_event<P: EventPayload>(&self, node_id: u64, payload: P) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let event = ApxmEvent::root(payload, EventSource::Runtime, &*self.trace_id)
            .with_scope_id(self.current_scope_id())
            .with_seq(seq);
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

        let mut node_info = serde_json::json!({
            "id": node_id,
            "name": meta.name,
            "op": meta.op_type,
            "attributes": serde_json::to_value(&meta.attributes).unwrap_or_default(),
        });
        if let Some(object) = node_info.as_object_mut() {
            insert_optional_string(
                object,
                constants::session::node::SCOPE_ID,
                self.current_scope_id(),
            );
            insert_optional_string(
                object,
                constants::session::node::PARENT_EXECUTION_ID,
                self.provenance.parent_execution_id.clone(),
            );
            insert_optional_string(
                object,
                constants::session::node::PARENT_SESSION_DIR,
                self.provenance.parent_session_dir.clone(),
            );
            insert_optional_string(
                object,
                constants::session::node::PARENT_SCOPE_ID,
                self.provenance.parent_scope_id.clone(),
            );
            if let Some(spawn_node_id) = self.provenance.spawn_node_id {
                object.insert(
                    constants::session::node::SPAWN_NODE_ID.to_string(),
                    serde_json::Value::Number(spawn_node_id.into()),
                );
            }
        }
        let _ = json_pretty_write(
            &node_dir.join(constants::session::node::NODE_JSON),
            &node_info,
        );

        let live = serde_json::json!({
            "status": SessionStatus::Running,
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

        if let Some(assembler) = self.context_assembler.lock().as_ref() {
            if let Some(profile) = meta
                .attributes
                .get(constants::graph::attrs::PROFILE)
                .and_then(|v| v.as_str())
            {
                match profile {
                    CLAUDE_PROFILE => {
                        if let Ok(contents) =
                            assembler.assemble_claude_md(node_id, meta, &skill_names)
                        {
                            let _ = fs::write(node_dir.join(CLAUDE_CONTEXT_FILE), contents);
                        }
                    }
                    CODEX_PROFILE => {
                        if let Ok(contents) =
                            assembler.assemble_agents_md(node_id, meta, &skill_names)
                        {
                            let _ = fs::write(node_dir.join(CODEX_CONTEXT_FILE), contents);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn write_live(&self, current_node_id: Option<u64>) -> io::Result<()> {
        if let Some(id) = current_node_id {
            self.current_node_id
                .store(id.min(i64::MAX as u64) as i64, Ordering::Relaxed);
        }

        let completed = self.completed.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        let elapsed_ms = self.start_time.elapsed().as_millis();

        let running_nodes = if let Ok(guard) = self.running_nodes.lock() {
            guard.clone()
        } else {
            Vec::new()
        };
        let recent_completed = if let Ok(guard) = self.completed_nodes.lock() {
            guard.iter().rev().take(10).cloned().collect()
        } else {
            Vec::new()
        };

        let live = LiveSessionState {
            status: SessionStatus::Running,
            running_nodes,
            completed_nodes: recent_completed,
            completed: completed as usize,
            total: if total > 0 {
                Some(total as usize)
            } else {
                None
            },
            elapsed_ms,
            success: false,
            current_phase: None,
        };

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
            "status": if success { SessionStatus::Completed } else { SessionStatus::Failed },
            "duration_ms": duration.as_millis(),
            "retries": 0,
            "error": if success { serde_json::Value::Null } else { serde_json::Value::String("operation failed".to_string()) },
        });
        let live = serde_json::json!({
            "status": if success { SessionStatus::Completed } else { SessionStatus::Failed },
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
            SessionStatus::Completed
        } else {
            SessionStatus::Failed
        };
        let completed = self.completed.load(Ordering::Relaxed);
        let total = self.total.load(Ordering::Relaxed);
        let elapsed_ms = self.start_time.elapsed().as_millis();

        let live = LiveSessionState {
            status,
            running_nodes: Vec::new(),
            completed_nodes: Vec::new(),
            completed: completed as usize,
            total: if total > 0 {
                Some(total as usize)
            } else {
                None
            },
            elapsed_ms,
            success,
            current_phase: None,
        };
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
    fn set_current_scope_id(&self, scope_id: Option<String>) {
        *self.current_scope_id.write() = scope_id;
    }

    fn current_scope_id(&self) -> Option<String> {
        self.current_scope_id.read().clone()
    }

    fn emit_llm_token(&self, content: &str) {
        self.write_trace_event(apxm_core::events::payload::TokenPayload {
            text: content.to_string(),
        });
    }

    fn emit_llm_token_for_node(&self, node_id: u64, content: &str) {
        let payload = apxm_core::events::payload::TokenPayload {
            text: content.to_string(),
        };
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
        self.write_trace_event(apxm_core::events::payload::ToolStartPayload {
            name: name.to_string(),
            args: args_json,
        });
    }

    fn emit_tool_end(&self, name: &str, result: &apxm_core::types::values::Value) {
        self.write_trace_event(apxm_core::events::payload::ToolEndPayload {
            name: name.to_string(),
            result: serde_json::to_value(result).unwrap_or_default(),
        });
    }

    fn emit_operation_start(&self, node_id: u64, op_type: AISOperationType) {
        self.ensure_node_workspace(node_id);

        if let Some(meta) = self.node_metadata.get(&node_id) {
            let node_info = NodeInfo {
                id: node_id,
                name: meta.name.clone(),
                op: op_type,
            };
            if let Ok(mut running) = self.running_nodes.lock() {
                running.push(node_info);
            }
        }

        let payload = OperationStartPayload { node_id, op_type };
        self.write_trace_event(payload.clone());
        self.write_node_trace_event(node_id, payload);
        let _ = self.write_live(Some(node_id));
    }

    fn emit_operation_end(
        &self,
        node_id: u64,
        op_type: AISOperationType,
        duration: std::time::Duration,
        success: bool,
        tokens: Option<apxm_runtime::TokenUsageSummary>,
        timing: Option<apxm_core::types::TimingBreakdown>,
    ) {
        self.completed.fetch_add(1, Ordering::Relaxed);

        if let Ok(mut running) = self.running_nodes.lock() {
            running.retain(|n| n.id != node_id);
        }

        if let Some(meta) = self.node_metadata.get(&node_id) {
            let status = if success {
                SessionStatus::Completed
            } else {
                SessionStatus::Failed
            };
            let (input_tokens, output_tokens) = match &tokens {
                Some(usage) => (Some(usage.input_tokens), Some(usage.output_tokens)),
                None => (None, None),
            };
            let (prefill_ms, decode_ms) = match &timing {
                Some(t) => (Some(t.prefill_ms), Some(t.decode_ms)),
                None => (None, None),
            };
            let completed_info = CompletedNodeInfo {
                id: node_id,
                name: meta.name.clone(),
                op: op_type,
                duration_ms: duration.as_millis() as u64,
                status,
                input_tokens,
                output_tokens,
                prefill_ms,
                decode_ms,
            };
            if let Ok(mut completed) = self.completed_nodes.lock() {
                completed.push(completed_info);
            }
        }

        let payload = OperationEndPayload {
            node_id,
            op_type,
            duration_ms: duration.as_millis() as u64,
            success,
        };
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

    fn emit_node_metrics(&self, node_id: u64, metrics: &apxm_core::types::NodeMetrics) {
        let Some(node_dir) = self.node_workspace_dir(node_id) else {
            return;
        };
        let _ = json_pretty_write(
            &node_dir.join(constants::session::node::METRICS_JSON),
            metrics,
        );
    }

    fn emit_plan_created(&self, plan_id: &str, steps: usize) {
        self.write_trace_event(apxm_core::events::payload::PlanCreatedPayload {
            plan_id: plan_id.to_string(),
            steps,
        });
    }

    fn emit_plan_step_started(&self, plan_id: &str, step_index: usize) {
        self.write_trace_event(apxm_core::events::payload::PlanStepStartedPayload {
            plan_id: plan_id.to_string(),
            step_index,
        });
    }

    fn emit_plan_step_completed(&self, plan_id: &str, step_index: usize, success: bool) {
        self.write_trace_event(apxm_core::events::payload::PlanStepCompletedPayload {
            plan_id: plan_id.to_string(),
            step_index,
            success,
        });
    }

    fn emit_memory_read(&self, scope: &str, key: &str) {
        self.write_trace_event(apxm_core::events::payload::MemoryReadPayload {
            scope: scope.to_string(),
            key: key.to_string(),
        });
    }

    fn emit_memory_write(&self, scope: &str, key: &str) {
        self.write_trace_event(apxm_core::events::payload::MemoryWritePayload {
            scope: scope.to_string(),
            key: key.to_string(),
        });
    }

    fn emit_checkpoint_saved(&self, checkpoint_id: &str) {
        self.write_trace_event(apxm_core::events::payload::CheckpointSavedPayload {
            checkpoint_id: checkpoint_id.to_string(),
        });
    }

    fn emit_checkpoint_restored(&self, checkpoint_id: &str) {
        self.write_trace_event(apxm_core::events::payload::CheckpointRestoredPayload {
            checkpoint_id: checkpoint_id.to_string(),
        });
    }

    fn emit_scheduler_decision(&self, node_id: u64, delay: std::time::Duration, reason: &str) {
        self.write_trace_event(apxm_core::events::payload::SchedulerDecisionPayload {
            node_id,
            delay_ms: delay.as_millis() as u64,
            reason: reason.to_string(),
        });
    }

    fn emit_head_of_line_block(
        &self,
        blocker_node: u64,
        blocked_node: u64,
        wait_ms: u64,
        reason: &str,
    ) {
        self.write_trace_event(apxm_core::events::payload::HeadOfLineBlockPayload {
            blocker_node,
            blocked_node,
            wait_ms,
            reason: reason.to_string(),
        });
    }

    fn emit_gpu_utilization(&self, gpu_id: u32, utilization_pct: f32, memory_pct: f32) {
        self.write_trace_event(apxm_core::events::payload::GpuUtilizationPayload {
            gpu_id,
            utilization_pct,
            memory_pct,
        });
    }

    fn emit_token_usage(&self, node_id: u64, input_tokens: usize, output_tokens: usize) {
        self.write_trace_event(apxm_core::events::payload::TokenUsagePayload {
            node_id,
            input_tokens,
            output_tokens,
        });
    }

    fn emit_memoization_hit(&self, node_id: u64) {
        self.write_trace_event(apxm_core::events::payload::MemoizationHitPayload { node_id });
    }
}
