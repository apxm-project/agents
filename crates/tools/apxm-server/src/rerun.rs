//! Re-run endpoints (`/v1/runs/{id}/rerun` and `/v1/runs/{id}/rerun-from-node`).
//!
//! Re-execute a prior run identified by its `execution_id`. A run is
//! re-executable when the server can recover its source; today that is the
//! skill-backed path: every persisted [`ExecutionRecord`] carries the
//! `skill_id`/`skill_version` it ran, so a rerun re-invokes that skill on a fresh
//! session, reusing the original launch args (recovered from the prior run's
//! rollout `SessionMeta`) unless the caller overrides them.
//!
//! Raw `/v1/execute` runs are NOT re-executable here: their AIR source is never
//! persisted (only an event transcript is), so there is nothing to recompile.
//! Those return `422 Unprocessable Entity` telling the caller to resubmit AIR.
//!
//! ## rerun-from-node
//!
//! Genuine partial replay: the original run persists its real per-token outputs
//! on success ([`ExecutionRecord::token_values`]). A `rerun-from-node` recompiles
//! the same skill, loads those prior values, and stamps a replay seed
//! (`replay_from_node` + `replay_token_values`) into the new run's execution
//! metadata. The runtime then pre-completes the upstream nodes from those values
//! and re-executes only `from_node` and its descendants (see
//! `apxm_runtime::scheduler::ReplaySeed`). The response sets
//! `from_node_supported: true` when the seed could be built; it falls back to a
//! full re-run (`from_node_supported: false`) when the prior run did not persist
//! token values (e.g. it failed, or predates output capture).

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::execute::ExecuteResponse;
use crate::executions::ExecutionRecord;
use crate::skills::{SkillExecuteRequest, SkillExecuteResponse, execute_skill_by_id};
use crate::state::AppState;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct RerunRequest {
    /// Override the launch args for the rerun. When omitted, the original run's
    /// args are recovered from its rollout and reused.
    #[serde(default)]
    pub(crate) args: Option<Vec<String>>,
    /// Override the session id. When omitted a fresh session is allocated so the
    /// rerun does not clobber the original run's session output.
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    /// Minimum sandbox isolation, forwarded to the skill execution (fail-closed).
    #[serde(default)]
    pub(crate) sandbox_hint: Option<String>,
    /// Internal-only execution metadata threaded into the rerun (NOT
    /// caller-settable — `#[serde(skip)]`). `rerun-from-node` uses it to carry the
    /// partial-replay seed keys.
    #[serde(skip)]
    pub(crate) extra_metadata: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RerunFromNodeRequest {
    /// The node id in the prior run's graph to (conceptually) restart from.
    pub(crate) from_node: u64,
    #[serde(flatten)]
    pub(crate) base: RerunRequest,
}

#[derive(Debug, Serialize)]
pub(crate) struct RerunResponse {
    /// The original run this rerun was derived from.
    pub(crate) source_execution_id: String,
    /// The new run's execution id.
    pub(crate) execution_id: String,
    pub(crate) skill_id: String,
    pub(crate) skill_version: String,
    /// `false` for a whole-run rerun; `true` when a `rerun-from-node` ran as a
    /// genuine partial replay (only `from_node` + descendants re-executed).
    pub(crate) partial: bool,
    /// The node a rerun-from-node was asked to start at, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) from_node: Option<u64>,
    /// Whether the server honored a partial replay from `from_node`. `true` when
    /// the prior run's token values were available to seed the replay boundary;
    /// `false` when it fell back to a full re-run — see module docs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) from_node_supported: Option<bool>,
    /// The new run's result (results, stats, llm usage, …). Flattened so the
    /// rerun response mirrors the shape of a fresh skill execution response.
    #[serde(flatten)]
    pub(crate) result: ExecuteResponse,
}

/// `POST /v1/runs/{execution_id}/rerun` — re-execute a prior run.
pub(crate) async fn rerun_run(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
    body: Option<Json<RerunRequest>>,
) -> Result<Json<RerunResponse>, ApiError> {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let record = load_rerunnable_record(&state, &execution_id)?;
    let skill_result = rerun_skill(&state, &record, &req).await?;
    Ok(Json(RerunResponse {
        source_execution_id: execution_id,
        execution_id: skill_result.execution_id,
        skill_id: record.skill_id,
        skill_version: record.skill_version,
        partial: false,
        from_node: None,
        from_node_supported: None,
        result: skill_result.response,
    }))
}

/// `POST /v1/runs/{execution_id}/rerun-from-node` — partially replay a prior run
/// starting at a specific node.
///
/// The prior run's persisted token values seed the replay boundary so only
/// `from_node` and its descendants re-execute; upstream nodes are reused. When
/// the prior run has no persisted token values (it failed, or predates output
/// capture) this falls back to a full re-run and reports
/// `from_node_supported: false`.
pub(crate) async fn rerun_from_node(
    State(state): State<AppState>,
    Path(execution_id): Path<String>,
    Json(mut req): Json<RerunFromNodeRequest>,
) -> Result<Json<RerunResponse>, ApiError> {
    let record = load_rerunnable_record(&state, &execution_id)?;

    // Validate the requested node existed in the prior run so the caller gets a
    // clear error instead of a silently-ignored argument.
    let known_nodes = prior_run_node_ids(&state, &execution_id).await;
    if !known_nodes.is_empty() && !known_nodes.contains(&req.from_node) {
        return Err(ApiError::bad_request(format!(
            "node {} was not part of run {execution_id}; cannot rerun from it",
            req.from_node
        )));
    }

    // Build the partial-replay seed from the prior run's persisted token values.
    // The runtime recompiles the same skill, so node/token ids line up; it
    // computes the descendant set of `from_node` itself and pre-completes the
    // upstream nodes from these values. When no values were persisted, fall back
    // to a full re-run.
    let from_node_supported = build_replay_metadata(
        req.from_node,
        &record.token_values,
        &mut req.base.extra_metadata,
    );

    let skill_result = rerun_skill(&state, &record, &req.base).await?;
    Ok(Json(RerunResponse {
        source_execution_id: execution_id,
        execution_id: skill_result.execution_id,
        skill_id: record.skill_id,
        skill_version: record.skill_version,
        partial: from_node_supported,
        from_node: Some(req.from_node),
        from_node_supported: Some(from_node_supported),
        result: skill_result.response,
    }))
}

/// Look up a prior run's record and confirm it is re-executable (skill-backed).
fn load_rerunnable_record(
    state: &AppState,
    execution_id: &str,
) -> Result<ExecutionRecord, ApiError> {
    let record = state
        .execution_store
        .get(execution_id)
        .ok_or_else(|| ApiError::not_found(format!("run not found: {execution_id}")))?;
    if record.skill_id.trim().is_empty() {
        // Raw /v1/execute runs do not persist their AIR source, so they cannot be
        // recompiled + re-run from the server. 422: the request is well-formed
        // but the target is not re-executable.
        return Err(ApiError::unprocessable(format!(
            "run {execution_id} is not re-executable: its workflow source was not persisted; \
             resubmit the AIR to /v1/execute"
        )));
    }
    Ok(record)
}

/// Re-invoke the skill that produced `record`, reusing the original launch args
/// unless the request overrides them, on a fresh (or caller-supplied) session.
async fn rerun_skill(
    state: &AppState,
    record: &ExecutionRecord,
    req: &RerunRequest,
) -> Result<SkillExecuteResponse, ApiError> {
    let args = match &req.args {
        Some(a) => a.clone(),
        None => recover_prior_args(state, &record.execution_id)
            .await
            .unwrap_or_default(),
    };
    // The skill id carries its version when one was pinned, so the rerun targets
    // the same skill version the original ran.
    let skill_ref = if record.skill_version.trim().is_empty() {
        record.skill_id.clone()
    } else {
        format!("{}@{}", record.skill_id, record.skill_version)
    };
    let skill_req = SkillExecuteRequest {
        args,
        session_id: req.session_id.clone(),
        sandbox_hint: req.sandbox_hint.clone(),
        detach: false,
        idempotency_key: None,
        correlation_id: None,
        workflow_id: None,
        trace_id: None,
        extra_metadata: req.extra_metadata.clone(),
    };
    execute_skill_by_id(state, &skill_ref, skill_req).await
}

/// Recover the launch args a prior run was started with from its rollout
/// `SessionMeta` line. Returns `None` when no rollout is on disk for the run.
async fn recover_prior_args(state: &AppState, execution_id: &str) -> Option<Vec<String>> {
    let path = {
        let index = state.rollout_index.lock().await;
        match index.get(execution_id) {
            Ok(Some(entry)) => std::path::PathBuf::from(entry.file_path),
            _ => return None,
        }
    };
    let (lines, _) = apxm_rollout::load_rollout(&path).await.ok()?;
    lines.into_iter().find_map(|line| match line.payload {
        apxm_rollout::RolloutPayload::SessionMeta(meta) => Some(meta.args),
        _ => None,
    })
}

/// Stamp the partial-replay seed (`replay_from_node` + `replay_token_values`)
/// into `metadata` from a prior run's `token_values`. Returns `true` when a seed
/// was built (the runtime will partially replay), `false` when no token values
/// were available (the caller falls back to a full re-run). Leaves `metadata`
/// free of replay keys in the `false` case so a stale partial seed cannot leak.
fn build_replay_metadata(
    from_node: u64,
    token_values: &std::collections::HashMap<u64, serde_json::Value>,
    metadata: &mut std::collections::HashMap<String, String>,
) -> bool {
    if token_values.is_empty() {
        return false;
    }
    let Ok(values_json) = serde_json::to_string(token_values) else {
        return false;
    };
    metadata.insert(
        apxm_runtime::metadata_keys::REPLAY_FROM_NODE.to_string(),
        from_node.to_string(),
    );
    metadata.insert(
        apxm_runtime::metadata_keys::REPLAY_TOKEN_VALUES.to_string(),
        values_json,
    );
    true
}

/// The set of node ids that appeared in the prior run's graph (from its recorded
/// `operation_start` events). Empty when no events are available — callers treat
/// an empty set as "cannot validate" rather than "node unknown".
async fn prior_run_node_ids(
    state: &AppState,
    execution_id: &str,
) -> std::collections::HashSet<u64> {
    use apxm_core::events::payload::OperationStartPayload;
    crate::runs::events_for_run(state, execution_id)
        .await
        .iter()
        .filter_map(|event| {
            event
                .payload
                .downcast_ref::<OperationStartPayload>()
                .map(|payload| payload.node_id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::{DependencyType, Edge, ExecutionDag, Node, NodeMetadata, Value};
    use std::collections::HashMap;

    fn json(v: &str) -> serde_json::Value {
        serde_json::Value::String(v.to_string())
    }

    #[test]
    fn no_prior_values_means_full_rerun() {
        let mut metadata = HashMap::new();
        let supported = build_replay_metadata(2, &HashMap::new(), &mut metadata);
        assert!(!supported, "no token values -> not a partial replay");
        assert!(
            metadata.is_empty(),
            "no replay keys leak when a full re-run is chosen"
        );
    }

    #[test]
    fn prior_values_stamp_replay_seed_metadata() {
        let mut token_values = HashMap::new();
        token_values.insert(10u64, json("prior-output-of-node-1"));
        let mut metadata = HashMap::new();
        let supported = build_replay_metadata(2, &token_values, &mut metadata);

        assert!(
            supported,
            "token values present -> partial replay supported"
        );
        assert_eq!(
            metadata.get(apxm_runtime::metadata_keys::REPLAY_FROM_NODE),
            Some(&"2".to_string())
        );
        let values_json = metadata
            .get(apxm_runtime::metadata_keys::REPLAY_TOKEN_VALUES)
            .expect("replay_token_values stamped");
        let decoded: HashMap<u64, serde_json::Value> =
            serde_json::from_str(values_json).expect("values round-trip");
        assert_eq!(decoded.get(&10), Some(&json("prior-output-of-node-1")));
    }

    /// End-to-end contract: the metadata the server stamps is exactly what the
    /// runtime decodes into a `ReplaySeed` that re-executes only `from_node` and
    /// its descendants. This proves the upstream node (1) is NOT replayed.
    #[test]
    fn stamped_metadata_yields_seed_that_skips_upstream() {
        // Recompiled DAG (same ids as the prior run): 1 --t10--> 2 --t20--> 3.
        let mut dag = ExecutionDag::new();
        let mk = |id, inp: Vec<u64>, out: Vec<u64>| Node {
            id,
            op_type: AISOperationType::Nop,
            attributes: std::collections::HashMap::new(),
            input_tokens: inp,
            output_tokens: out,
            metadata: NodeMetadata::default(),
        };
        dag.add_node(mk(1, vec![], vec![10])).unwrap();
        dag.add_node(mk(2, vec![10], vec![20])).unwrap();
        dag.add_node(mk(3, vec![20], vec![30])).unwrap();
        dag.add_edge(Edge::new(1, 2, 10, DependencyType::Data))
            .unwrap();
        dag.add_edge(Edge::new(2, 3, 20, DependencyType::Data))
            .unwrap();
        dag.entry_nodes = dag.find_entry_nodes();
        dag.exit_nodes = dag.find_exit_nodes();

        // Server stamps the seed from the prior run's persisted token values.
        let mut token_values = HashMap::new();
        token_values.insert(10u64, json("prior"));
        let mut metadata = HashMap::new();
        assert!(build_replay_metadata(2, &token_values, &mut metadata));

        // Runtime decodes the stamped values and computes the seed.
        let from_node: u64 = metadata[apxm_runtime::metadata_keys::REPLAY_FROM_NODE]
            .parse()
            .unwrap();
        let raw: HashMap<String, Value> =
            serde_json::from_str(&metadata[apxm_runtime::metadata_keys::REPLAY_TOKEN_VALUES])
                .unwrap();
        let prior: HashMap<u64, Value> = raw
            .into_iter()
            .map(|(k, v)| (k.parse().unwrap(), v))
            .collect();
        let seed = apxm_runtime::scheduler::ReplaySeed::compute(&dag, from_node, &prior).unwrap();

        // Upstream node 1 is NOT re-executed; 2 and 3 are.
        assert!(
            seed.completed_nodes.contains(&1),
            "upstream node is pre-completed (not re-invoked)"
        );
        assert!(seed.replayed_nodes.contains(&2));
        assert!(seed.replayed_nodes.contains(&3));
        assert!(!seed.replayed_nodes.contains(&1));
        assert!(seed.is_complete(&dag), "boundary token 10 is seeded");
    }

    /// HTTP-level end-to-end coverage for `rerun-from-node`, driving the real
    /// axum `Router` (built by [`crate::build_app`]) via `tower::ServiceExt::oneshot`
    /// — no live LLM, no TCP bind.
    ///
    /// The flow is the genuine production path: a compiled skill (a real
    /// `.apxmobj` on disk) is executed through `POST /v1/skills/{id}/execute`, then
    /// partially replayed through `POST /v1/runs/{id}/rerun-from-node`. The
    /// skill's upstream node is an `INV_TOOL` backed by a registered,
    /// side-effect-free counting capability; the downstream node is a NOP
    /// passthrough. The capability's invocation counter is the witness: it must
    /// stay at 1 across both calls, proving the rerun pre-completed the upstream
    /// node from the prior run's token values instead of re-invoking it.
    ///
    /// `rerun-from-node` requires a *skill-backed* run (raw `/v1/execute` runs do
    /// not persist their AIR source, so they are not re-executable — see the
    /// module docs and [`load_rerunnable_record`]); the skill-execute entry is the
    /// smallest no-LLM path that persists the per-token outputs a partial replay
    /// needs.
    mod http_e2e {
        use std::collections::HashMap as Map;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::SystemTime;

        use apxm_artifact::{Artifact, ArtifactMetadata};
        use apxm_core::constants::graph::attrs as graph_attrs;
        use apxm_core::error::RuntimeError;
        use apxm_core::types::execution::{DagMetadata, ExecutionDag, Node, NodeMetadata};
        use apxm_core::types::operations::AISOperationType;
        use apxm_core::types::values::Value;
        use apxm_runtime::capability::executor::CapabilityExecutor;
        use apxm_runtime::capability::metadata::CapabilityMetadata;
        use apxm_runtime::{Runtime, RuntimeConfig, SchedulerConfig};
        use async_trait::async_trait;
        use axum::Router;
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use dashmap::DashMap;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        use crate::build_app;
        use crate::checkpoints::CheckpointStore;
        use crate::executions::ExecutionStore;
        use crate::skills::SkillLibrary;
        use crate::state::{AppState, InferenceLimiter};
        use crate::tasks::TaskQueueManager;

        const SKILL_ID: &str = "replay-counter-skill";
        const SKILL_VERSION: &str = "0.1.0";
        const ENTRY_FLOW: &str = "main";
        const COUNT_CAPABILITY: &str = "replay_counter";
        const CAPABILITY_OUTPUT: &str = "upstream-produced-value";
        /// Token produced by the upstream INV_TOOL node (node 1).
        const UPSTREAM_TOKEN: u64 = 10;
        /// Exit token produced by the downstream NOP node (node 2).
        const EXIT_TOKEN: u64 = 20;

        /// A read-only capability that counts how many times it is invoked. The
        /// shared counter is the proof that a partial replay does NOT re-run the
        /// upstream node.
        struct CountingCapability {
            metadata: CapabilityMetadata,
            invocations: Arc<AtomicUsize>,
        }

        impl CountingCapability {
            fn new(invocations: Arc<AtomicUsize>) -> Self {
                Self {
                    metadata: CapabilityMetadata::new(
                        COUNT_CAPABILITY,
                        "Counts invocations; used to prove non-re-execution on replay",
                        serde_json::json!({ "type": "object", "properties": {} }),
                    )
                    .with_returns("string")
                    .with_read_only(),
                    invocations,
                }
            }
        }

        #[async_trait]
        impl CapabilityExecutor for CountingCapability {
            async fn execute(&self, _args: Map<String, Value>) -> Result<Value, RuntimeError> {
                self.invocations.fetch_add(1, Ordering::SeqCst);
                Ok(Value::String(CAPABILITY_OUTPUT.to_string()))
            }

            fn metadata(&self) -> &CapabilityMetadata {
                &self.metadata
            }
        }

        fn tagged_blake3(bytes: &[u8]) -> String {
            format!("blake3:{}", blake3::hash(bytes).to_hex())
        }

        /// Two-node skill DAG: `INV_TOOL(replay_counter) --t10--> NOP --t20-->`.
        ///
        /// The NOP passes the upstream value through, so the exit token equals the
        /// capability's output. On a rerun-from-node=2, node 1 is pre-completed
        /// from the prior run's token 10, so the capability is never re-invoked.
        fn skill_artifact_bytes() -> Vec<u8> {
            let mut inv = Node {
                id: 1,
                op_type: AISOperationType::InvTool,
                attributes: Map::new(),
                input_tokens: vec![],
                output_tokens: vec![UPSTREAM_TOKEN],
                metadata: NodeMetadata::default(),
            };
            inv.attributes.insert(
                graph_attrs::CAPABILITY.to_string(),
                Value::String(COUNT_CAPABILITY.to_string()),
            );
            inv.attributes.insert(
                graph_attrs::PARAMS_JSON.to_string(),
                Value::String("{}".to_string()),
            );

            let nop = Node {
                id: 2,
                op_type: AISOperationType::Nop,
                attributes: Map::new(),
                input_tokens: vec![UPSTREAM_TOKEN],
                output_tokens: vec![EXIT_TOKEN],
                metadata: NodeMetadata::default(),
            };

            let dag = ExecutionDag {
                nodes: vec![inv, nop],
                edges: vec![apxm_core::types::Edge::new(
                    1,
                    2,
                    UPSTREAM_TOKEN,
                    apxm_core::types::DependencyType::Data,
                )],
                entry_nodes: vec![1],
                exit_nodes: vec![2],
                metadata: DagMetadata {
                    name: Some(ENTRY_FLOW.to_string()),
                    is_entry: true,
                    parameters: vec![],
                },
            };
            let artifact = Artifact::new(
                ArtifactMetadata::new(Some(SKILL_ID.to_string()), "test-compiler"),
                vec![dag],
            );
            artifact.to_bytes().expect("artifact bytes")
        }

        /// Write the skill package (`skill.toml` + `skill.apxmobj`) under `root`.
        fn write_skill(root: &std::path::Path) {
            let bytes = skill_artifact_bytes();
            let skill_dir = root.join("pkg");
            std::fs::create_dir_all(&skill_dir).expect("skill dir");
            std::fs::write(skill_dir.join("skill.apxmobj"), &bytes).expect("artifact");
            let artifact_hash = tagged_blake3(&bytes);
            std::fs::write(
                skill_dir.join("skill.toml"),
                format!(
                    r#"
skill_id = "{SKILL_ID}"
version = "{SKILL_VERSION}"
entry_flow = "{ENTRY_FLOW}"
artifact_hash = "{artifact_hash}"
allowed_tools = ["{COUNT_CAPABILITY}"]
side_effect_policy = "read_only"
timeout_ms = 30000
"#
                ),
            )
            .expect("manifest");
        }

        /// Build an `AppState` whose runtime has the counting capability
        /// registered and whose skill library points at `skill_root`. No LLM
        /// backend is registered; the skill graph touches none.
        async fn app_state(
            skill_root: std::path::PathBuf,
            invocations: Arc<AtomicUsize>,
        ) -> AppState {
            // Match the production server's scheduler config: capture per-token
            // outputs so the first run persists the values a `rerun-from-node`
            // needs to seed the replay boundary (see `startup::build_server_runtime`).
            let mut config = RuntimeConfig::in_memory();
            config.scheduler_config = SchedulerConfig::default().with_collect_all_outputs(true);
            let runtime = Runtime::new(config).await.expect("test runtime");
            runtime
                .capability_system()
                .register(Arc::new(CountingCapability::new(invocations)))
                .expect("register counting capability");

            let server_config = apxm_driver::ServerConfig::default();
            let hardening = crate::state::HardeningDefaults::for_config(&server_config);

            AppState {
                runtime: Arc::new(runtime),
                agent_registry: Arc::new(DashMap::new()),
                task_manager: TaskQueueManager::new(),
                checkpoint_store: CheckpointStore::new(),
                start_time: SystemTime::now(),
                a2a_tasks: Arc::new(DashMap::new()),
                skill_library: SkillLibrary::new(vec![skill_root]),
                execution_store: ExecutionStore::new(),
                run_event_bus: crate::runs::RunEventBus::new(),
                webhook_dispatcher: None,
                rollout_paths: Arc::new(apxm_rollout::RolloutPaths::new({
                    let dir = tempfile::tempdir().expect("rollout home");
                    let path = dir.path().to_path_buf();
                    std::mem::forget(dir);
                    path
                })),
                rollout_index: Arc::new(tokio::sync::Mutex::new(
                    apxm_rollout::IndexDb::open_in_memory().expect("rollout index"),
                )),
                rollout_registry: crate::rollout::RolloutRegistry::new(),
                inference_limiter: InferenceLimiter::unlimited_for_tests(),
                server_config,
                bind_addr: hardening.bind_addr,
                effective_require_auth: hardening.effective_require_auth,
                safety_state: hardening.safety_state,
                shutdown: hardening.shutdown,
                cancel_registry: Arc::new(DashMap::new()),
                goal_runs: crate::goal_runs::GoalRunRegistry::new(),
                session_registry: crate::conversations::SessionRegistry::new(),
            }
        }

        async fn post_json(
            app: Router,
            path: &str,
            body: serde_json::Value,
        ) -> (StatusCode, serde_json::Value) {
            let req = Request::builder()
                .method("POST")
                .uri(path)
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let resp = app.oneshot(req).await.unwrap();
            let status = resp.status();
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
            (status, json)
        }

        #[tokio::test]
        async fn rerun_from_node_partial_replay_skips_upstream_capability() {
            let skill_root = {
                let dir = tempfile::tempdir().expect("skill root");
                let path = dir.path().to_path_buf();
                std::mem::forget(dir);
                path
            };
            write_skill(&skill_root);

            let invocations = Arc::new(AtomicUsize::new(0));
            let state = app_state(skill_root, Arc::clone(&invocations)).await;

            // ── First run: execute the skill through the HTTP API. ──────────
            let (status, body) = post_json(
                build_app(state.clone()),
                &format!("/v1/skills/{SKILL_ID}/execute"),
                serde_json::json!({ "args": [] }),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "first run failed: {body:?}");
            let source_run_id = body["execution_id"]
                .as_str()
                .expect("first run execution_id")
                .to_string();
            assert_eq!(
                invocations.load(Ordering::SeqCst),
                1,
                "(a) the upstream capability runs exactly once on the first execution"
            );
            // (a) The first run persisted its per-token outputs (the seed source).
            let record = state
                .execution_store
                .get(&source_run_id)
                .expect("first run recorded");
            assert!(
                record.token_values.contains_key(&UPSTREAM_TOKEN),
                "(a) first run persists the upstream node's token value for replay"
            );

            // ── Partial replay from the downstream node (node 2). ───────────
            let (status, body) = post_json(
                build_app(state.clone()),
                &format!("/v1/runs/{source_run_id}/rerun-from-node"),
                serde_json::json!({ "from_node": 2 }),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "rerun failed: {body:?}");

            // (b) The server honored a genuine partial replay.
            assert_eq!(
                body["from_node_supported"],
                serde_json::Value::Bool(true),
                "(b) prior token values were available -> partial replay supported"
            );
            assert_eq!(
                body["partial"],
                serde_json::Value::Bool(true),
                "(b) the rerun ran as a partial replay"
            );
            assert_eq!(body["from_node"], serde_json::json!(2));

            // (c) Only node 2 re-executed: the upstream INV_TOOL capability was
            // NOT re-invoked, so the counter is still 1.
            assert_eq!(
                invocations.load(Ordering::SeqCst),
                1,
                "(c) rerun-from-node must NOT re-invoke the upstream capability; \
                 node 1 is pre-completed from the prior run's token values"
            );
        }
    }
}
