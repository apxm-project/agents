//! Graph-lifecycle guard for vLLM-style graph-aware backends.
//!
//! `VllmGraphLifecycle` owns a single `register_graph` / `release_graph`
//! pairing for one graph execution. The happy path calls
//! `release().await` explicitly; the
//! `Drop` impl is the panic safety net that fires a best-effort release on a
//! detached `tokio::spawn`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use apxm_backends::llm::backends::traits::LLMBackend;
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::{GraphMetadata, NodeSpec};
use serde_json::Value;

/// Best-effort registration guard for one graph execution.
///
/// Constructed via [`VllmGraphLifecycle::register`], which calls
/// `LLMBackend::register_graph` once. Either `release().await` is called on
/// the happy path, or `Drop` fires a detached release task as a safety net.
/// Double-release is prevented by an atomic flag.
pub struct VllmGraphLifecycle {
    backend: Arc<dyn LLMBackend>,
    graph_id: String,
    released: AtomicBool,
}

impl VllmGraphLifecycle {
    /// Register `dag` with `backend` under `graph_id` / `exec_id` and
    /// return the guard. The metadata payload is constructed from the DAG's
    /// LLM-eligible nodes (Ask / Think / Reason); non-LLM nodes are skipped
    /// in the per-node spec list but still counted toward `node_count`.
    pub async fn register(
        backend: Arc<dyn LLMBackend>,
        graph_id: String,
        exec_id: String,
        dag: &ExecutionDag,
    ) -> Result<Self> {
        let nodes: Vec<NodeSpec> = dag
            .nodes
            .iter()
            .map(|n| NodeSpec {
                node_id: n.id as u32,
                node_name: n.metadata.name.clone(),
                estimated_prompt_tokens: None,
                downstream_nodes: dag
                    .edges
                    .iter()
                    .filter(|e| e.from == n.id)
                    .map(|e| e.to as u32)
                    .collect(),
                priority_class: None,
                reuse_group: None,
                is_critical_path: false,
            })
            .collect();

        let metadata = GraphMetadata::new(graph_id.clone(), exec_id).with_nodes(nodes);

        let payload: Value = serde_json::to_value(&metadata)
            .context("failed to serialize GraphMetadata for register_graph")?;

        backend
            .register_graph(payload)
            .await
            .with_context(|| format!("register_graph failed for graph_id={graph_id}"))?;

        tracing::debug!(
            graph_id = %graph_id,
            backend = %backend.name(),
            "Registered graph with graph-aware backend"
        );

        Ok(Self {
            backend,
            graph_id,
            released: AtomicBool::new(false),
        })
    }

    /// Explicit (happy-path) release. Idempotent: subsequent calls (and the
    /// `Drop` guard) become no-ops.
    pub async fn release(&self) -> Result<()> {
        if !self.released.swap(true, Ordering::AcqRel) {
            self.backend
                .release_graph(&self.graph_id)
                .await
                .with_context(|| format!("release_graph failed for graph_id={}", self.graph_id))?;
            tracing::debug!(
                graph_id = %self.graph_id,
                backend = %self.backend.name(),
                "Released graph from graph-aware backend"
            );
        }
        Ok(())
    }

    /// Graph id this guard owns.
    pub fn graph_id(&self) -> &str {
        &self.graph_id
    }
}

impl Drop for VllmGraphLifecycle {
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
