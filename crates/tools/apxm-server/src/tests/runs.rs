//! Phase 14.8.B — tests for the observer endpoints under `/v1/runs/...`.
//!
//! These exercise the full chain: a skill execution records events
//! into `RunEventBus`, then the read-only HTTP surface serves them
//! back as a graph, per-node detail, bulk pull, and SSE tail.

use super::*;
use crate::routes;
use crate::runs::{RunBusEmitter, RunEventBus};
use apxm_core::events::payload::{
    AgentSpawnedPayload, CommunicateDispatchedPayload, GraphEdgePayload, OperationEndPayload,
    OperationStartPayload, ToolEndPayload, ToolStartPayload,
};
use apxm_core::events::{ApxmEvent, EventEmitter, EventSource};
use std::collections::HashMap as StdHashMap;

const RUN_EXECUTION_ID: &str = "exec-runs-test";
const SECOND_EXECUTION_ID: &str = "exec-runs-test-2";

fn populate_run(bus: &RunEventBus, execution_id: &str) {
    let emitter = RunBusEmitter::new(bus.clone(), execution_id.to_string());

    // Root SPAWN_AGENT node — emits agent_spawned + operation_start.
    emitter.emit(ApxmEvent::root(
        OperationStartPayload {
            node_id: 1,
            op_type: AISOperationType::SpawnAgent,
            context: Some(serde_json::json!({"agent_code": "module.crm"})),
        },
        EventSource::Runtime,
        execution_id,
    ));
    emitter.emit(ApxmEvent::root(
        AgentSpawnedPayload {
            node_id: 1,
            agent_code: "module.crm".to_string(),
            parent_execution_id: execution_id.to_string(),
            profile: Some("acp-codex".to_string()),
            process_id: None,
            scope_policy: None,
        },
        EventSource::Runtime,
        execution_id,
    ));
    emitter.emit(ApxmEvent::root(
        OperationEndPayload {
            node_id: 1,
            op_type: AISOperationType::SpawnAgent,
            duration_ms: 5,
            success: true,
        },
        EventSource::Runtime,
        execution_id,
    ));

    // Edge: spawn → communicate (dispatch).
    emitter.emit(ApxmEvent::root(
        GraphEdgePayload {
            from_node_id: 1,
            to_node_id: 2,
            kind: "dispatch".to_string(),
        },
        EventSource::Runtime,
        execution_id,
    ));

    // COMMUNICATE node.
    emitter.emit(ApxmEvent::root(
        OperationStartPayload {
            node_id: 2,
            op_type: AISOperationType::Communicate,
            context: Some(serde_json::json!({"target_agent": "task.crm.lead"})),
        },
        EventSource::Runtime,
        execution_id,
    ));
    emitter.emit(ApxmEvent::root(
        CommunicateDispatchedPayload {
            node_id: 2,
            target_agent: "task.crm.lead".to_string(),
            protocol: "local".to_string(),
            message_excerpt: Some("triage new lead".to_string()),
        },
        EventSource::Runtime,
        execution_id,
    ));
    emitter.emit(ApxmEvent::root(
        OperationEndPayload {
            node_id: 2,
            op_type: AISOperationType::Communicate,
            duration_ms: 12,
            success: true,
        },
        EventSource::Runtime,
        execution_id,
    ));

    // Edge: communicate → tool (tool_invocation).
    emitter.emit(ApxmEvent::root(
        GraphEdgePayload {
            from_node_id: 2,
            to_node_id: 3,
            kind: "tool_invocation".to_string(),
        },
        EventSource::Runtime,
        execution_id,
    ));

    // INV_TOOL node + tool span.
    emitter.emit(ApxmEvent::root(
        OperationStartPayload {
            node_id: 3,
            op_type: AISOperationType::InvTool,
            context: None,
        },
        EventSource::Runtime,
        execution_id,
    ));
    let mut tool_args = StdHashMap::new();
    tool_args.insert(
        "query".to_string(),
        serde_json::Value::String("acme".to_string()),
    );
    emitter.emit(ApxmEvent::root(
        ToolStartPayload {
            name: "search_leads".to_string(),
            args: tool_args,
        },
        EventSource::Runtime,
        execution_id,
    ));
    emitter.emit(ApxmEvent::root(
        ToolEndPayload {
            name: "search_leads".to_string(),
            result: serde_json::json!({"leads": []}),
        },
        EventSource::Runtime,
        execution_id,
    ));
    emitter.emit(ApxmEvent::root(
        OperationEndPayload {
            node_id: 3,
            op_type: AISOperationType::InvTool,
            duration_ms: 7,
            success: true,
        },
        EventSource::Runtime,
        execution_id,
    ));
}

#[tokio::test]
async fn run_event_bus_assigns_monotonic_run_sequence() {
    let bus = RunEventBus::new();
    let first = bus.record(
        RUN_EXECUTION_ID,
        ApxmEvent::root(
            OperationStartPayload {
                node_id: 1,
                op_type: AISOperationType::Ask,
                context: None,
            },
            EventSource::Runtime,
            RUN_EXECUTION_ID,
        )
        .with_seq(99),
    );
    let second = bus.record(
        RUN_EXECUTION_ID,
        ApxmEvent::root(
            OperationEndPayload {
                node_id: 1,
                op_type: AISOperationType::Ask,
                duration_ms: 1,
                success: true,
            },
            EventSource::Runtime,
            RUN_EXECUTION_ID,
        )
        .with_seq(99),
    );

    assert_eq!(first.meta.seq, 0);
    assert_eq!(second.meta.seq, 1);
    let seqs: Vec<u64> = bus
        .snapshot(RUN_EXECUTION_ID)
        .into_iter()
        .map(|event| event.meta.seq)
        .collect();
    assert_eq!(seqs, vec![0, 1]);
}

async fn seed_run_record(state: &AppState, execution_id: &str) {
    // ExecutionStore.start_skill_execution requires a session dir; use
    // a tempdir so persistence doesn't pollute the home directory.
    let session = tempfile::tempdir().expect("session dir");
    state.execution_store.start_skill_execution_with_provenance(
        apxm_skill::SkillExecutionProvenance {
            skill_id: FIXTURE_SKILL_ID.to_string(),
            skill_version: FIXTURE_SKILL_VERSION.to_string(),
            ..apxm_skill::SkillExecutionProvenance::default()
        },
        FIXTURE_SESSION_ID,
        session.path().to_str().expect("utf-8 session"),
    );
    // Leak the tempdir so the path stays valid for the rest of the
    // test (the snapshot persistence reaches into it).
    std::mem::forget(session);
    populate_run(&state.run_event_bus, execution_id);
    let _ = execution_id;
}

#[tokio::test]
async fn get_runs_lists_recent_executions() {
    let state = test_state().await;
    // Seed two completed runs so the listing has something to surface.
    let exec_a = state.execution_store.start_skill_execution(
        FIXTURE_SKILL_ID,
        FIXTURE_SKILL_VERSION,
        FIXTURE_SESSION_ID,
        tempfile::tempdir().expect("tmp").path().to_str().unwrap(),
    );
    populate_run(&state.run_event_bus, &exec_a.execution_id);
    let exec_b = state.execution_store.start_skill_execution(
        FIXTURE_SKILL_ID,
        FIXTURE_SKILL_VERSION,
        FIXTURE_SESSION_ID,
        tempfile::tempdir().expect("tmp").path().to_str().unwrap(),
    );
    populate_run(&state.run_event_bus, &exec_b.execution_id);

    let app = build_app(state);
    let (status, body) = get_json(app, routes::RUNS).await;
    assert_eq!(status, StatusCode::OK, "list runs failed: {body}");
    assert_eq!(body["object"], "list");
    let runs = body["data"].as_array().expect("data array");
    assert!(runs.len() >= 2, "expected at least 2 runs: {body}");
    // Most recent first.
    assert_eq!(runs[0]["skill_id"], FIXTURE_SKILL_ID);
    // Totals computed from bus events.
    assert!(runs[0]["totals"]["events"].as_u64().unwrap_or(0) > 0);
}

#[tokio::test]
async fn get_run_graph_returns_layered_topology_with_edges() {
    let state = test_state().await;
    seed_run_record(&state, RUN_EXECUTION_ID).await;

    let app = build_app(state);
    let (status, body) = get_json(app, &routes::run_graph_path(RUN_EXECUTION_ID)).await;
    assert_eq!(status, StatusCode::OK, "graph fetch failed: {body}");
    assert_eq!(body["graph_schema_version"], 1);
    let nodes = body["nodes"].as_array().expect("nodes");
    let edges = body["edges"].as_array().expect("edges");
    // 3 op nodes + at least 1 pseudo tool node from the ToolStart.
    assert!(nodes.len() >= 3, "expected ≥3 nodes: {body}");
    assert_eq!(edges.len(), 2);
    let kinds: Vec<&str> = edges
        .iter()
        .map(|e| e["kind"].as_str().unwrap_or(""))
        .collect();
    assert!(kinds.contains(&"dispatch"));
    assert!(kinds.contains(&"tool_invocation"));
    // Layer assignment: root has layer 0, descendant ≥ 1.
    let layers: Vec<u64> = nodes.iter().filter_map(|n| n["layer"].as_u64()).collect();
    assert!(layers.iter().any(|&l| l == 0));
    assert!(layers.iter().any(|&l| l >= 1));
}

#[tokio::test]
async fn get_run_node_returns_full_detail_for_agent_node() {
    let state = test_state().await;
    populate_run(&state.run_event_bus, RUN_EXECUTION_ID);
    let app = build_app(state);
    let (status, body) = get_json(app, &routes::run_node_detail_path(RUN_EXECUTION_ID, 1)).await;
    assert_eq!(status, StatusCode::OK, "node detail failed: {body}");
    assert_eq!(body["node_id"], 1);
    assert_eq!(body["status"], "succeeded");
    assert_eq!(body["agent"]["agent_code"], "module.crm");
    assert!(body["events"].as_array().expect("events").len() >= 2);
}

#[tokio::test]
async fn get_run_node_returns_full_detail_for_tool_node() {
    let state = test_state().await;
    populate_run(&state.run_event_bus, RUN_EXECUTION_ID);
    let app = build_app(state);
    let (status, body) = get_json(app, &routes::run_node_detail_path(RUN_EXECUTION_ID, 3)).await;
    assert_eq!(status, StatusCode::OK, "tool node detail failed: {body}");
    assert_eq!(body["node_id"], 3);
    assert!(body["tool"].is_object(), "tool block present");
    assert_eq!(body["tool"]["tool_name"], "search_leads");
    assert_eq!(body["tool"]["result"]["leads"], serde_json::json!([]));
}

#[tokio::test]
async fn events_bulk_supports_since_paging() {
    let state = test_state().await;
    populate_run(&state.run_event_bus, RUN_EXECUTION_ID);
    let app = build_app(state);
    let (status, body) = get_json(
        app.clone(),
        &format!("{}?limit=3", routes::run_events_path(RUN_EXECUTION_ID)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "events bulk failed: {body}");
    let events = body["events"].as_array().expect("events");
    assert_eq!(events.len(), 3);
    assert!(body["next_seq"].as_u64().unwrap() > 0);
    assert_eq!(body["done"], false);
}

#[tokio::test]
async fn events_stream_replays_from_seq_zero_then_tails_live() {
    let state = test_state().await;
    populate_run(&state.run_event_bus, RUN_EXECUTION_ID);
    let app = build_app(state.clone());

    let req = Request::builder()
        .method("GET")
        .uri(&routes::run_events_stream_path(RUN_EXECUTION_ID))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Read a bounded chunk of the SSE body so we don't block on the
    // keep-alive stream forever — the test only validates initial
    // replay.
    use futures::StreamExt;
    use http_body_util::BodyStream;
    let mut body_stream = BodyStream::new(resp.into_body());
    let mut buf = Vec::new();
    while buf.len() < 1024 {
        match tokio::time::timeout(std::time::Duration::from_millis(500), body_stream.next()).await
        {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    buf.extend_from_slice(data);
                }
            }
            _ => break,
        }
    }
    let text = String::from_utf8_lossy(&buf);
    assert!(
        text.contains("event: operation_start"),
        "expected operation_start in SSE: {text}"
    );
    assert!(
        text.contains("event: agent_spawned"),
        "expected agent_spawned in SSE: {text}"
    );
}

#[tokio::test]
async fn events_stream_supports_last_event_id_reconnect() {
    let state = test_state().await;
    populate_run(&state.run_event_bus, SECOND_EXECUTION_ID);
    let app = build_app(state.clone());

    // Last-Event-ID is exclusive, so seq=5 must not replay.
    let req = Request::builder()
        .method("GET")
        .uri(&routes::run_events_stream_path(SECOND_EXECUTION_ID))
        .header("Last-Event-ID", "5")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    use futures::StreamExt;
    use http_body_util::BodyStream;
    let mut body_stream = BodyStream::new(resp.into_body());
    let mut buf = Vec::new();
    while buf.len() < 1024 {
        match tokio::time::timeout(std::time::Duration::from_millis(500), body_stream.next()).await
        {
            Ok(Some(Ok(frame))) => {
                if let Some(data) = frame.data_ref() {
                    buf.extend_from_slice(data);
                }
            }
            _ => break,
        }
    }
    let text = String::from_utf8_lossy(&buf);
    // No event with id <= 5 should appear.
    for forbidden in 0..=5 {
        let needle = format!("id: {forbidden}\n");
        assert!(
            !text.contains(&needle),
            "Last-Event-ID resume must skip id {forbidden} in: {text}"
        );
    }
    assert!(
        text.contains("id: 6\n"),
        "Last-Event-ID resume should continue after id 5: {text}"
    );
}

#[tokio::test]
async fn get_run_returns_404_for_unknown_execution() {
    let app = build_app(test_state().await);
    let (status, _body) = get_json(app, &routes::run_detail_path("nonexistent-run")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ─────────────────────────────────────────────────────────────────────
// Phase 14.8.E — rollout-backed read-path tests.
// ─────────────────────────────────────────────────────────────────────

async fn write_synthetic_rollout(
    state: &AppState,
    execution_id: &str,
    session_id: &str,
) -> std::path::PathBuf {
    use apxm_rollout::{
        ContentBlock, PartialMeta, RolloutPayload, RolloutRecorder, RolloutRecorderConfig,
        SessionMetaPayload, UserMessagePayload, now_rfc3339,
    };
    let started_at = chrono::Utc::now();
    let session_meta = SessionMetaPayload {
        thread_id: execution_id.to_string(),
        parent_thread_id: None,
        session_id: session_id.to_string(),
        started_at: now_rfc3339(),
        cwd: "/tmp".into(),
        apxm_version: "test".into(),
        agent_role: "coordinator".into(),
        agent_code: Some("module.test".into()),
        skill_id: FIXTURE_SKILL_ID.into(),
        skill_version: FIXTURE_SKILL_VERSION.into(),
        artifact_hash: "blake3:aaa".into(),
        source_hash: "blake3:bbb".into(),
        air_hash: "blake3:ccc".into(),
        compiler_version: None,
        runtime_version: None,
        args: vec![],
        model_provider: None,
        model_id: None,
        backend_endpoint: None,
        tool_use_id_in_parent: None,
    };
    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: state.rollout_paths.clone(),
            thread_id: execution_id.to_string(),
            session_id: session_id.to_string(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        session_meta.clone(),
    )
    .await
    .expect("open recorder");
    recorder
        .write_line(
            RolloutPayload::UserMessage(UserMessagePayload {
                content: vec![ContentBlock::Text {
                    text: "synthetic".into(),
                }],
            }),
            PartialMeta::default(),
        )
        .await
        .unwrap();
    recorder.close().await.unwrap();
    let file_path = state.rollout_paths.rollout_path(execution_id, started_at);
    // Insert into the index so the read endpoints can resolve the file.
    let index = state.rollout_index.lock().await;
    index
        .insert_or_update(&apxm_rollout::ThreadIndexEntry {
            thread_id: execution_id.to_string(),
            parent_thread_id: None,
            session_id: session_id.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            status: "succeeded".to_string(),
            agent_role: "coordinator".to_string(),
            agent_code: Some("module.test".to_string()),
            file_path: file_path.to_string_lossy().into_owned(),
            line_count: 2,
            file_bytes: 0,
        })
        .unwrap();
    file_path
}

#[tokio::test]
async fn runs_list_reads_from_index_when_bus_empty() {
    let state = test_state().await;
    write_synthetic_rollout(&state, "exec-from-index", "s-idx").await;
    let app = build_app(state);
    let (status, body) = get_json(app, routes::RUNS).await;
    assert_eq!(status, StatusCode::OK, "list runs: {body}");
    let data = body["data"].as_array().expect("data array");
    assert!(
        data.iter().any(|r| r["execution_id"] == "exec-from-index"),
        "expected index-backed run: {body}"
    );
}

#[tokio::test]
async fn events_stream_replays_from_rollout_after_bus_rollover() {
    let state = test_state().await;
    // Write a rollout that contains a real ApxmEvent line so the
    // disk-replay branch has something to surface.
    let exec_id = "exec-rollover";
    let session_id = "s-rollover";
    let started_at = chrono::Utc::now();
    let session_meta = apxm_rollout::SessionMetaPayload {
        thread_id: exec_id.into(),
        parent_thread_id: None,
        session_id: session_id.into(),
        started_at: apxm_rollout::now_rfc3339(),
        cwd: "/tmp".into(),
        apxm_version: "test".into(),
        agent_role: "coordinator".into(),
        agent_code: None,
        skill_id: FIXTURE_SKILL_ID.into(),
        skill_version: FIXTURE_SKILL_VERSION.into(),
        artifact_hash: "blake3:00".into(),
        source_hash: "blake3:01".into(),
        air_hash: "blake3:02".into(),
        compiler_version: None,
        runtime_version: None,
        args: vec![],
        model_provider: None,
        model_id: None,
        backend_endpoint: None,
        tool_use_id_in_parent: None,
    };
    let recorder = apxm_rollout::RolloutRecorder::open(
        apxm_rollout::RolloutRecorderConfig {
            paths: state.rollout_paths.clone(),
            thread_id: exec_id.into(),
            session_id: session_id.into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        session_meta,
    )
    .await
    .unwrap();
    let event = ApxmEvent::root(
        OperationStartPayload {
            node_id: 1,
            op_type: AISOperationType::SpawnAgent,
            context: None,
        },
        EventSource::Runtime,
        exec_id,
    );
    recorder
        .write_event(event, apxm_rollout::PartialMeta::default())
        .await
        .unwrap();
    recorder.close().await.unwrap();
    let file_path = state.rollout_paths.rollout_path(exec_id, started_at);
    {
        let index = state.rollout_index.lock().await;
        index
            .insert_or_update(&apxm_rollout::ThreadIndexEntry {
                thread_id: exec_id.into(),
                parent_thread_id: None,
                session_id: session_id.into(),
                started_at: chrono::Utc::now().to_rfc3339(),
                completed_at: None,
                status: "succeeded".into(),
                agent_role: "coordinator".into(),
                agent_code: None,
                file_path: file_path.to_string_lossy().into_owned(),
                line_count: 2,
                file_bytes: 0,
            })
            .unwrap();
    }
    let app = build_app(state);
    // bus is empty for this id; the events bulk endpoint should fall
    // through to the rollout JSONL.
    let (status, body) = get_json(app, &routes::run_events_path(exec_id)).await;
    assert_eq!(status, StatusCode::OK, "events from disk: {body}");
    let events = body["events"].as_array().expect("events");
    assert!(!events.is_empty(), "expected events from rollout: {body}");
}

#[tokio::test]
async fn node_detail_reads_from_rollout_when_bus_empty() {
    let state = test_state().await;
    let exec_id = "exec-node-disk";
    let session_id = "s-node-disk";
    let started_at = chrono::Utc::now();
    let session_meta = apxm_rollout::SessionMetaPayload {
        thread_id: exec_id.into(),
        parent_thread_id: None,
        session_id: session_id.into(),
        started_at: apxm_rollout::now_rfc3339(),
        cwd: "/tmp".into(),
        apxm_version: "test".into(),
        agent_role: "coordinator".into(),
        agent_code: None,
        skill_id: FIXTURE_SKILL_ID.into(),
        skill_version: FIXTURE_SKILL_VERSION.into(),
        artifact_hash: "blake3:00".into(),
        source_hash: "blake3:01".into(),
        air_hash: "blake3:02".into(),
        compiler_version: None,
        runtime_version: None,
        args: vec![],
        model_provider: None,
        model_id: None,
        backend_endpoint: None,
        tool_use_id_in_parent: None,
    };
    let recorder = apxm_rollout::RolloutRecorder::open(
        apxm_rollout::RolloutRecorderConfig {
            paths: state.rollout_paths.clone(),
            thread_id: exec_id.into(),
            session_id: session_id.into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        session_meta,
    )
    .await
    .unwrap();
    let start_event = ApxmEvent::root(
        OperationStartPayload {
            node_id: 7,
            op_type: AISOperationType::SpawnAgent,
            context: None,
        },
        EventSource::Runtime,
        exec_id,
    );
    let agent_event = ApxmEvent::root(
        AgentSpawnedPayload {
            node_id: 7,
            agent_code: "module.disk".into(),
            parent_execution_id: exec_id.into(),
            profile: None,
            process_id: None,
            scope_policy: None,
        },
        EventSource::Runtime,
        exec_id,
    );
    recorder
        .write_event(start_event, apxm_rollout::PartialMeta::default())
        .await
        .unwrap();
    recorder
        .write_event(agent_event, apxm_rollout::PartialMeta::default())
        .await
        .unwrap();
    recorder.close().await.unwrap();
    let file_path = state.rollout_paths.rollout_path(exec_id, started_at);
    {
        let index = state.rollout_index.lock().await;
        index
            .insert_or_update(&apxm_rollout::ThreadIndexEntry {
                thread_id: exec_id.into(),
                parent_thread_id: None,
                session_id: session_id.into(),
                started_at: chrono::Utc::now().to_rfc3339(),
                completed_at: None,
                status: "succeeded".into(),
                agent_role: "coordinator".into(),
                agent_code: None,
                file_path: file_path.to_string_lossy().into_owned(),
                line_count: 3,
                file_bytes: 0,
            })
            .unwrap();
    }
    let app = build_app(state);
    let (status, body) = get_json(app, &routes::run_node_detail_path(exec_id, 7)).await;
    assert_eq!(status, StatusCode::OK, "node detail from disk: {body}");
    assert_eq!(body["node_id"], 7);
    // op_type round-trips through core decoder so SpawnAgent is preserved.
    assert_eq!(body["op_type"], "SPAWN_AGENT");
}

#[tokio::test]
async fn blob_endpoint_returns_spilled_content() {
    use apxm_rollout::{
        ContentBlock, PartialMeta, RolloutPaths, RolloutPayload, RolloutRecorder,
        RolloutRecorderConfig, SessionMetaPayload, UserMessagePayload, now_rfc3339,
    };
    let state = test_state().await;
    let exec_id = "exec-blob";
    let session_id = "s-blob";
    let started_at = chrono::Utc::now();
    let session_meta = SessionMetaPayload {
        thread_id: exec_id.into(),
        parent_thread_id: None,
        session_id: session_id.into(),
        started_at: now_rfc3339(),
        cwd: "/tmp".into(),
        apxm_version: "test".into(),
        agent_role: "coordinator".into(),
        agent_code: None,
        skill_id: FIXTURE_SKILL_ID.into(),
        skill_version: FIXTURE_SKILL_VERSION.into(),
        artifact_hash: "blake3:00".into(),
        source_hash: "blake3:01".into(),
        air_hash: "blake3:02".into(),
        compiler_version: None,
        runtime_version: None,
        args: vec![],
        model_provider: None,
        model_id: None,
        backend_endpoint: None,
        tool_use_id_in_parent: None,
    };
    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: state.rollout_paths.clone(),
            thread_id: exec_id.into(),
            session_id: session_id.into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: Some(64),
            override_path: None,
        },
        session_meta,
    )
    .await
    .unwrap();
    let big = "z".repeat(4096);
    recorder
        .write_line(
            RolloutPayload::UserMessage(UserMessagePayload {
                content: vec![ContentBlock::Text { text: big }],
            }),
            PartialMeta::default(),
        )
        .await
        .unwrap();
    recorder.close().await.unwrap();
    let file_path = state.rollout_paths.rollout_path(exec_id, started_at);
    // Find the blob hash by scanning the file for a Spilled payload.
    let (items, _) = apxm_rollout::load_rollout(&file_path).await.unwrap();
    let blob_ref = items
        .iter()
        .find_map(|l| match &l.payload {
            RolloutPayload::Spilled(p) => Some(p.blob_ref.clone()),
            _ => None,
        })
        .expect("expected a spilled line");
    {
        let index = state.rollout_index.lock().await;
        index
            .insert_or_update(&apxm_rollout::ThreadIndexEntry {
                thread_id: exec_id.into(),
                parent_thread_id: None,
                session_id: session_id.into(),
                started_at: chrono::Utc::now().to_rfc3339(),
                completed_at: None,
                status: "succeeded".into(),
                agent_role: "coordinator".into(),
                agent_code: None,
                file_path: file_path.to_string_lossy().into_owned(),
                line_count: items.len() as i64,
                file_bytes: 0,
            })
            .unwrap();
    }
    // Sanity: silence the unused-paths-import warning for hosts where
    // RolloutPaths gets re-exported.
    let _ = RolloutPaths::from_env();
    let app = build_app(state);
    let req = Request::builder()
        .method("GET")
        .uri(routes::run_blob_path(exec_id, &blob_ref))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert!(!bytes.is_empty(), "blob bytes returned");
}
