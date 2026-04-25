//! Graph-lifecycle guard for graph-aware LLM backends.
//!
//! `BackendGraphLifecycle` owns a single `register_graph` / `release_graph`
//! pairing for one graph execution. The happy path calls
//! `release().await` explicitly; the
//! `Drop` impl is the panic safety net that fires a best-effort release on a
//! detached `tokio::spawn`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use apxm_backends::llm::backends::traits::LLMBackend;
use apxm_core::constants::graph::metadata as graph_meta;
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::{
    ApxmGraphHints, GraphMetadata, GraphStatusSnapshot, NodeSpec, PriorityClass,
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
        let metadata = graph_metadata_from_dag(graph_id.clone(), exec_id, dag);

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

pub(crate) fn graph_metadata_from_dag(
    graph_id: impl Into<String>,
    exec_id: impl Into<String>,
    dag: &ExecutionDag,
) -> GraphMetadata {
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
                estimated_prompt_tokens: hints.compiler_hints.shared_prefix_est_tokens,
                downstream_nodes: if hints.downstream_nodes.is_empty() {
                    dag.edges
                        .iter()
                        .filter(|edge| edge.from == node.id)
                        .filter_map(|edge| u32::try_from(edge.to).ok())
                        .collect()
                } else {
                    hints.downstream_nodes
                },
                priority_class,
                reuse_group: hints.reuse_group,
                is_critical_path,
            }
        })
        .collect();

    GraphMetadata::new(graph_id, exec_id).with_nodes(nodes)
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
