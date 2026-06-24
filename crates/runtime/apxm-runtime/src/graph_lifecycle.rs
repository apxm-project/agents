//! Graph-lifecycle guard for graph-aware LLM backends.
//!
//! `BackendGraphLifecycle` owns a single `register_graph` / `release_graph`
//! pairing for one graph execution. The happy path calls
//! `release().await` explicitly; the
//! `Drop` impl is the panic safety net that fires a best-effort release on a
//! detached `tokio::spawn`.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::dispatch::v1::{
    BackendCapabilityRequirements, DispatchIrV1, TelemetryContract, lower_graph,
};
use anyhow::{Context, Result};
use apxm_backends::llm::backends::traits::LLMBackend;
use apxm_core::constants::graph::{attrs as graph_attrs, metadata as graph_meta};
use apxm_core::constants::llm::apxm as apxm_llm;
use apxm_core::constants::llm::apxm::dispatch_fields as df;
use apxm_core::constants::llm::apxm::telemetry_metrics as tm;
use apxm_core::types::execution::{ExecutionDag, Node};
use apxm_core::types::{
    ApxmGraphHints, GraphMetadata, GraphStatusSnapshot, NodeGraphMetrics, NodeSpec, PriorityClass,
};
use tokio::sync::Mutex;

/// Best-effort registration guard for one graph execution.
///
/// Constructed via [`BackendGraphLifecycle::register`], which calls
/// `LLMBackend::register_graph` once. Either `release().await` is called on
/// the happy path, or `Drop` fires a detached release task as a safety net.
/// Double-release is prevented by an atomic flag.
pub struct BackendGraphLifecycle {
    backend: Arc<dyn LLMBackend>,
    graph_id: String,
    released: AtomicBool,
    /// Status snapshot captured just before release (if available).
    pre_release_status: Mutex<Option<GraphStatusSnapshot>>,
}

impl BackendGraphLifecycle {
    /// Register `dag` with `backend` under `graph_id` / `exec_id` and
    /// return the guard.
    pub async fn register(
        backend: Arc<dyn LLMBackend>,
        graph_id: String,
        exec_id: String,
        dag: &ExecutionDag,
    ) -> Result<Self> {
        let dispatch_ir = graph_dispatch_ir_from_dag(graph_id.clone(), exec_id, dag);
        Self::register_dispatch_ir(backend, &dispatch_ir).await
    }

    /// Register a graph using the runtime-owned dispatch plan as the source
    /// of the existing backend registration payload.
    pub(crate) async fn register_dispatch_ir(
        backend: Arc<dyn LLMBackend>,
        dispatch_ir: &DispatchIrV1,
    ) -> Result<Self> {
        let metadata = graph_metadata_from_dispatch_ir(dispatch_ir);
        let graph_id = metadata.graph_id.clone();

        backend
            .register_graph(metadata)
            .await
            .with_context(|| format!("register_graph failed for graph_id={graph_id}"))?;

        tracing::debug!(
            graph_id = %graph_id,
            backend = %backend.name(),
            "Registered graph with backend"
        );

        Ok(Self {
            backend,
            graph_id,
            released: AtomicBool::new(false),
            pre_release_status: Mutex::new(None),
        })
    }

    /// Explicit (happy-path) release. Idempotent: subsequent calls (and the
    /// `Drop` guard) become no-ops.
    ///
    /// Captures graph status via `get_graph_status` before issuing the release.
    /// The snapshot is available via `take_status()` afterwards.
    pub async fn release(&self) -> Result<()> {
        if !self.released.swap(true, Ordering::AcqRel) {
            // Capture pin telemetry before releasing blocks.
            let status = self
                .backend
                .get_graph_status(&self.graph_id)
                .await
                .ok()
                .flatten();
            *self.pre_release_status.lock().await = status;

            self.backend
                .release_graph(&self.graph_id)
                .await
                .with_context(|| format!("release_graph failed for graph_id={}", self.graph_id))?;
            tracing::debug!(
                graph_id = %self.graph_id,
                backend = %self.backend.name(),
                "Released graph from backend"
            );
        }
        Ok(())
    }

    /// Return (and consume) the pre-release status snapshot, if one was captured.
    pub async fn take_status(&self) -> Option<GraphStatusSnapshot> {
        self.pre_release_status.lock().await.take()
    }

    /// Graph id this guard owns.
    pub fn graph_id(&self) -> &str {
        &self.graph_id
    }
}

pub(crate) fn graph_dispatch_ir_from_dag(
    graph_id: impl Into<String>,
    exec_id: impl Into<String>,
    dag: &ExecutionDag,
) -> DispatchIrV1 {
    let graph_id = graph_id.into();
    let exec_id = exec_id.into();
    let metadata = graph_metadata_from_dag(graph_id.clone(), exec_id.clone(), dag);
    let node_hints = graph_hints_from_dag(&graph_id, &exec_id, dag);
    lower_graph(
        &metadata,
        &node_hints,
        BackendCapabilityRequirements {
            backend: None,
            protocol: None,
            required: vec![
                df::GRAPH_REGISTRATION.to_owned(),
                df::REQUEST_HINTS.to_owned(),
            ],
            optional: vec![
                df::PRIORITY.to_owned(),
                df::PREFIX_COHORTS.to_owned(),
                df::PIN_RELEASE.to_owned(),
                df::BACKEND_CACHE_STATE.to_owned(),
            ],
        },
        TelemetryContract {
            required_labels: vec![apxm_llm::GRAPH_ID.to_owned(), apxm_llm::NODE_ID.to_owned()],
            requested_metrics: vec![
                tm::SCHEDULER_POLICY.to_owned(),
                tm::GRAPH_STATUS.to_owned(),
                tm::PINNED_BLOCKS_PEAK.to_owned(),
            ],
        },
    )
}

pub(crate) fn graph_metadata_from_dispatch_ir(ir: &DispatchIrV1) -> GraphMetadata {
    let nodes = ir
        .nodes
        .iter()
        .map(|node| {
            let is_critical_path = node.registration_is_critical_path.unwrap_or(
                matches!(node.priority_class, Some(PriorityClass::CriticalPath)),
            );
            NodeSpec {
                node_id: node.node_id,
                node_name: node
                    .registration_node_name
                    .clone()
                    .or_else(|| node.node_name.clone()),
                estimated_prompt_tokens: node.registration_estimated_prompt_tokens,
                downstream_nodes: node.downstream_nodes.clone(),
                priority_class: node.priority_class,
                reuse_group: node.reuse_group.clone(),
                graph_metrics: NodeGraphMetrics {
                    fanout_count: node.fanout_count,
                    remaining_path_len: node.remaining_path_len,
                    latency_class: node.latency_class,
                    batch_group: node.batch_group.clone(),
                    stage_index: node.stage_index,
                    estimated_dynamic_tokens: node.estimated_dynamic_tokens,
                },
                is_critical_path,
            }
        })
        .collect();

    GraphMetadata {
        graph_id: ir.graph.graph_id.clone(),
        execution_id: ir.graph.execution_id.clone(),
        critical_path_length: ir.graph.critical_path_length,
        node_count: ir.graph.node_count,
        max_parallelism: ir.graph.max_parallelism,
        nodes,
        default_pin_ttl_ms: ir.graph.default_pin_ttl_ms,
    }
}

fn graph_hints_from_dag(
    graph_id: &str,
    exec_id: &str,
    dag: &ExecutionDag,
) -> HashMap<u32, ApxmGraphHints> {
    let graph_shape = analyze_graph_shape(dag);
    dag.nodes
        .iter()
        .filter_map(|node| {
            let node_id = u32::try_from(node.id).ok()?;
            let node_label = node.metadata.name.clone().unwrap_or_else(|| {
                format!("{}{}", graph_meta::GENERATED_NODE_NAME_PREFIX, node.id)
            });
            let mut hints = ApxmGraphHints::from_node_attrs(
                graph_id.to_owned(),
                node_label.clone(),
                &node.attributes,
            );
            hints.execution_id = Some(exec_id.to_owned());
            hints.node_id = Some(node_id);
            hints.node_name = Some(node_label);
            if hints.priority_class.is_none() {
                let priority = i64::from(node.metadata.priority);
                hints.priority_class = Some(
                    if priority >= graph_meta::CRITICAL_PATH_PRIORITY_THRESHOLD {
                        PriorityClass::CriticalPath
                    } else {
                        PriorityClass::Parallel
                    },
                );
            }
            if hints.downstream_nodes.is_empty() {
                hints.downstream_nodes = graph_shape
                    .downstream
                    .get(&node.id)
                    .cloned()
                    .unwrap_or_default();
            }
            Some((node_id, hints))
        })
        .collect()
}

pub(crate) fn graph_metadata_from_dag(
    graph_id: impl Into<String>,
    exec_id: impl Into<String>,
    dag: &ExecutionDag,
) -> GraphMetadata {
    let graph_shape = analyze_graph_shape(dag);
    let nodes = dag
        .nodes
        .iter()
        .map(|node| {
            let node_label = node.metadata.name.clone().unwrap_or_else(|| {
                format!("{}{}", graph_meta::GENERATED_NODE_NAME_PREFIX, node.id)
            });
            let hints =
                ApxmGraphHints::from_node_attrs(String::new(), node_label, &node.attributes);
            let priority_class = hints.priority_class.or_else(|| {
                let priority = i64::from(node.metadata.priority);
                if priority >= graph_meta::CRITICAL_PATH_PRIORITY_THRESHOLD {
                    Some(PriorityClass::CriticalPath)
                } else {
                    Some(PriorityClass::Parallel)
                }
            });
            let is_critical_path = matches!(priority_class, Some(PriorityClass::CriticalPath));

            NodeSpec {
                node_id: node.id as u32,
                node_name: node.metadata.name.clone(),
                estimated_prompt_tokens: estimated_prompt_tokens(node, &hints),
                downstream_nodes: if hints.downstream_nodes.is_empty() {
                    graph_shape
                        .downstream
                        .get(&node.id)
                        .cloned()
                        .unwrap_or_default()
                } else {
                    hints.downstream_nodes
                },
                priority_class,
                reuse_group: hints.reuse_group,
                graph_metrics: hints.graph_metrics,
                is_critical_path,
            }
        })
        .collect();

    let mut metadata = GraphMetadata::new(graph_id, exec_id).with_nodes(nodes);
    metadata.critical_path_length = graph_shape.critical_path_length;
    metadata.max_parallelism = graph_shape.max_parallelism;
    metadata
}

#[derive(Debug, Default)]
struct GraphShape {
    downstream: HashMap<u64, Vec<u32>>,
    critical_path_length: Option<u32>,
    max_parallelism: Option<u32>,
}

fn estimated_prompt_tokens(node: &Node, hints: &ApxmGraphHints) -> Option<u32> {
    node.attributes
        .get(graph_attrs::EST_TEMPLATE_TOKENS)
        .and_then(|value| value.as_u64())
        .or_else(|| hints.compiler_hints.shared_prefix_est_tokens.map(u64::from))
        .and_then(|value| u32::try_from(value).ok())
}

fn analyze_graph_shape(dag: &ExecutionDag) -> GraphShape {
    if dag.nodes.is_empty() {
        return GraphShape::default();
    }

    let mut downstream: HashMap<u64, Vec<u32>> = HashMap::new();
    let mut incoming_count: HashMap<u64, usize> =
        dag.nodes.iter().map(|node| (node.id, 0)).collect();

    for edge in &dag.edges {
        if let Ok(target) = u32::try_from(edge.to) {
            let targets = downstream.entry(edge.from).or_default();
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        *incoming_count.entry(edge.to).or_insert(0) += 1;
    }

    for targets in downstream.values_mut() {
        targets.sort_unstable();
    }

    let mut queue = VecDeque::new();
    let mut level: HashMap<u64, u32> = HashMap::new();
    for node in &dag.nodes {
        if incoming_count.get(&node.id).copied().unwrap_or_default() == 0 {
            queue.push_back(node.id);
            level.insert(node.id, 1);
        }
    }

    let mut visited = 0usize;
    let mut level_widths: HashMap<u32, u32> = HashMap::new();
    let mut critical_path_length = 0u32;
    while let Some(node_id) = queue.pop_front() {
        visited += 1;
        let node_level = level.get(&node_id).copied().unwrap_or(1);
        critical_path_length = critical_path_length.max(node_level);
        *level_widths.entry(node_level).or_insert(0) += 1;

        for target in downstream.get(&node_id).into_iter().flatten() {
            let target_id = u64::from(*target);
            let next_level = node_level.saturating_add(1);
            level
                .entry(target_id)
                .and_modify(|current| *current = (*current).max(next_level))
                .or_insert(next_level);

            if let Some(count) = incoming_count.get_mut(&target_id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    queue.push_back(target_id);
                }
            }
        }
    }

    let max_parallelism = level_widths.values().copied().max();
    GraphShape {
        downstream,
        critical_path_length: if visited == dag.nodes.len() {
            Some(critical_path_length)
        } else {
            None
        },
        max_parallelism,
    }
}

impl Drop for BackendGraphLifecycle {
    fn drop(&mut self) {
        if !self.released.swap(true, Ordering::AcqRel) {
            // Panic / early-return safety net: `Drop` cannot `.await`, so
            // we detach a best-effort release. If the tokio runtime is
            // already shutting down, `tokio::spawn` will silently no-op
            // and the pin will leak — that's acceptable for the panic path
            // (the happy path always calls `release().await`).
            let backend = self.backend.clone();
            let graph_id = self.graph_id.clone();
            // `tokio::spawn` requires a current runtime; guard against
            // being dropped outside one (e.g. some unit tests).
            if tokio::runtime::Handle::try_current().is_ok() {
                tokio::spawn(async move {
                    if let Err(e) = backend.release_graph(&graph_id).await {
                        tracing::warn!(
                            graph_id = %graph_id,
                            error = %e,
                            "Drop-path release_graph failed (non-fatal)"
                        );
                    }
                });
            }
        }
    }
}
