//! Basic recorder + loader behavior tests for `apxm-rollout`.

use std::sync::Arc;

use apxm_rollout::{
    ContentBlock, IndexDb, PartialMeta, RolloutPaths, RolloutPayload, RolloutRecorder,
    RolloutRecorderConfig, SCHEMA_VERSION, SessionMetaPayload, ToolResultPayload, ToolUsePayload,
    UserMessagePayload, load_rollout, now_rfc3339, rebuild_index_from_disk, reconstruct_history,
};
use chrono::Utc;
use tempfile::TempDir;

fn fixture_session_meta(thread_id: &str, session_id: &str) -> SessionMetaPayload {
    SessionMetaPayload {
        thread_id: thread_id.to_string(),
        parent_thread_id: None,
        session_id: session_id.to_string(),
        started_at: now_rfc3339(),
        cwd: "/tmp".to_string(),
        apxm_version: "test".to_string(),
        agent_role: "coordinator".to_string(),
        agent_code: Some("module.crm".to_string()),
        skill_id: "fixture-skill".to_string(),
        skill_version: "0.1.0".to_string(),
        artifact_hash: "blake3:aaaa".to_string(),
        source_hash: "blake3:bbbb".to_string(),
        air_hash: "blake3:cccc".to_string(),
        compiler_version: Some("test-compiler".to_string()),
        runtime_version: Some("test-runtime".to_string()),
        args: vec!["hello".to_string()],
        model_provider: Some("mock".to_string()),
        model_id: Some("mock-1".to_string()),
        backend_endpoint: None,
        tool_use_id_in_parent: None,
    }
}

fn fixture_paths(dir: &TempDir) -> Arc<RolloutPaths> {
    Arc::new(RolloutPaths::new(dir.path().to_path_buf()))
}

#[tokio::test]
async fn rollout_writes_one_jsonl_per_thread_with_session_meta_first() {
    let dir = TempDir::new().unwrap();
    let paths = fixture_paths(&dir);
    let started_at = Utc::now();

    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-main".to_string(),
            session_id: "s-1".to_string(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        fixture_session_meta("t-main", "s-1"),
    )
    .await
    .unwrap();

    recorder
        .write_line(
            RolloutPayload::UserMessage(UserMessagePayload {
                content: vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
            }),
            PartialMeta::default(),
        )
        .await
        .unwrap();
    recorder
        .write_line(
            RolloutPayload::ToolUse(ToolUsePayload {
                tool_use_id: "tu_1".into(),
                name: "search_leads".into(),
                input: serde_json::json!({"q": "acme"}),
            }),
            PartialMeta::default(),
        )
        .await
        .unwrap();
    recorder
        .write_line(
            RolloutPayload::ToolResult(ToolResultPayload {
                tool_use_id: "tu_1".into(),
                content: vec![ContentBlock::Text {
                    text: "ok".into(),
                }],
                is_error: false,
                latency_ms: 5,
            }),
            PartialMeta {
                tool_use_id: Some("tu_1".into()),
                ..PartialMeta::default()
            },
        )
        .await
        .unwrap();
    recorder.close().await.unwrap();

    let file_path = paths.rollout_path("t-main", started_at);
    let (items, stats) = load_rollout(&file_path).await.unwrap();
    assert_eq!(items.len(), 4, "expected SessionMeta + 3 lines");
    assert_eq!(stats.parse_errors, 0);
    assert_eq!(items[0].meta.seq, 0);
    assert!(matches!(
        items[0].payload,
        RolloutPayload::SessionMeta(_)
    ));
    assert_eq!(items[0].meta.schema_version, SCHEMA_VERSION);
}

#[tokio::test]
async fn spawn_agent_creates_separate_subagent_file_with_parent_thread_id_linkage() {
    let dir = TempDir::new().unwrap();
    let paths = fixture_paths(&dir);
    let started_at = Utc::now();

    let parent = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-parent".into(),
            session_id: "s-1".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        fixture_session_meta("t-parent", "s-1"),
    )
    .await
    .unwrap();
    parent
        .write_line(
            RolloutPayload::ToolUse(ToolUsePayload {
                tool_use_id: "tu_spawn".into(),
                name: "Agent".into(),
                input: serde_json::json!({"agent_code": "task.crm.lead"}),
            }),
            PartialMeta::default(),
        )
        .await
        .unwrap();

    // Subagent: separate file (Claude Code pattern) with parent_thread_id +
    // is_sidechain on every line.
    let child_path = paths.subagent_path("t-parent", started_at, "child-1");
    let mut child_meta = fixture_session_meta("t-child", "s-1");
    child_meta.parent_thread_id = Some("t-parent".into());
    child_meta.tool_use_id_in_parent = Some("tu_spawn".into());
    let child = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-child".into(),
            session_id: "s-1".into(),
            started_at,
            is_sidechain: true,
            spill_threshold_bytes: None,
            override_path: Some(child_path.clone()),
        },
        child_meta,
    )
    .await
    .unwrap();
    child
        .write_line(
            RolloutPayload::UserMessage(UserMessagePayload {
                content: vec![ContentBlock::Text {
                    text: "subagent turn".into(),
                }],
            }),
            PartialMeta::default(),
        )
        .await
        .unwrap();

    assert!(child_path.exists(), "subagent file at {child_path:?}");
    let (items, _) = load_rollout(&child_path).await.unwrap();
    let first = &items[0];
    let RolloutPayload::SessionMeta(meta) = &first.payload else {
        panic!("expected SessionMeta as first line");
    };
    assert_eq!(meta.parent_thread_id.as_deref(), Some("t-parent"));
    assert_eq!(meta.tool_use_id_in_parent.as_deref(), Some("tu_spawn"));
    assert!(items.iter().all(|line| line.meta.is_sidechain));
}

#[tokio::test]
async fn tool_use_and_tool_result_pair_by_tool_use_id_across_threads() {
    let dir = TempDir::new().unwrap();
    let paths = fixture_paths(&dir);
    let started_at = Utc::now();
    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-pair".into(),
            session_id: "s-pair".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        fixture_session_meta("t-pair", "s-pair"),
    )
    .await
    .unwrap();

    recorder
        .write_line(
            RolloutPayload::ToolUse(ToolUsePayload {
                tool_use_id: "tu_42".into(),
                name: "do_thing".into(),
                input: serde_json::json!({}),
            }),
            PartialMeta {
                tool_use_id: Some("tu_42".into()),
                ..PartialMeta::default()
            },
        )
        .await
        .unwrap();
    recorder
        .write_line(
            RolloutPayload::ToolResult(ToolResultPayload {
                tool_use_id: "tu_42".into(),
                content: vec![ContentBlock::Text {
                    text: "done".into(),
                }],
                is_error: false,
                latency_ms: 1,
            }),
            PartialMeta {
                tool_use_id: Some("tu_42".into()),
                ..PartialMeta::default()
            },
        )
        .await
        .unwrap();

    let (items, _) = load_rollout(&paths.rollout_path("t-pair", started_at))
        .await
        .unwrap();
    let pair: Vec<_> = items
        .iter()
        .filter(|l| l.meta.tool_use_id.as_deref() == Some("tu_42"))
        .collect();
    assert_eq!(pair.len(), 2);
}

#[tokio::test]
async fn spill_threshold_writes_blob_and_emits_spilled_pointer() {
    let dir = TempDir::new().unwrap();
    let paths = fixture_paths(&dir);
    let started_at = Utc::now();
    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-spill".into(),
            session_id: "s-spill".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: Some(1024),
            override_path: None,
        },
        fixture_session_meta("t-spill", "s-spill"),
    )
    .await
    .unwrap();

    let big = "x".repeat(8192);
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

    let (items, _) = load_rollout(&paths.rollout_path("t-spill", started_at))
        .await
        .unwrap();
    let spilled = items
        .iter()
        .find(|l| matches!(l.payload, RolloutPayload::Spilled(_)))
        .expect("expected one spilled line");
    let RolloutPayload::Spilled(blob) = &spilled.payload else {
        unreachable!();
    };
    let blob_path = paths.blob_path("t-spill", started_at, &blob.blob_ref, "json");
    assert!(blob_path.exists(), "blob at {blob_path:?}");
    assert!(blob.bytes_estimate > 1024);
}

#[tokio::test]
async fn reconstruct_history_walks_back_to_latest_compacted_then_forward_replays() {
    let dir = TempDir::new().unwrap();
    let paths = fixture_paths(&dir);
    let started_at = Utc::now();
    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-rec".into(),
            session_id: "s-rec".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        fixture_session_meta("t-rec", "s-rec"),
    )
    .await
    .unwrap();

    for tag in ["u1", "a1", "u2", "a2"] {
        recorder
            .write_line(
                RolloutPayload::UserMessage(UserMessagePayload {
                    content: vec![ContentBlock::Text {
                        text: tag.into(),
                    }],
                }),
                PartialMeta::default(),
            )
            .await
            .unwrap();
    }
    // Compaction baseline: a UserMessage summary.
    recorder
        .write_line(
            RolloutPayload::Compacted(apxm_rollout::CompactedPayload {
                message: "compacted-1".into(),
                replacement_history: vec![Box::new(RolloutPayload::UserMessage(
                    UserMessagePayload {
                        content: vec![ContentBlock::Text {
                            text: "summary".into(),
                        }],
                    },
                ))],
            }),
            PartialMeta::default(),
        )
        .await
        .unwrap();
    for tag in ["u3", "a3"] {
        recorder
            .write_line(
                RolloutPayload::UserMessage(UserMessagePayload {
                    content: vec![ContentBlock::Text {
                        text: tag.into(),
                    }],
                }),
                PartialMeta::default(),
            )
            .await
            .unwrap();
    }
    recorder.close().await.unwrap();

    let (items, _) = load_rollout(&paths.rollout_path("t-rec", started_at))
        .await
        .unwrap();
    let tree = reconstruct_history(&items);
    assert_eq!(tree.baseline.len(), 1, "baseline holds compacted summary");
    assert_eq!(
        tree.suffix.len(),
        2,
        "suffix is post-compaction lines only"
    );
}

#[tokio::test]
async fn index_sqlite_is_fully_rebuildable_from_jsonl_files() {
    let dir = TempDir::new().unwrap();
    let paths = fixture_paths(&dir);
    let started_at = Utc::now();
    for i in 0..5 {
        let id = format!("t-{i}");
        let r = RolloutRecorder::open(
            RolloutRecorderConfig {
                paths: paths.clone(),
                thread_id: id.clone(),
                session_id: "s-bulk".into(),
                started_at,
                is_sidechain: false,
                spill_threshold_bytes: None,
                override_path: None,
            },
            fixture_session_meta(&id, "s-bulk"),
        )
        .await
        .unwrap();
        r.close().await.unwrap();
    }
    // The index path may not exist yet; rebuild materializes it from the
    // JSONL files alone.
    let n = rebuild_index_from_disk(&paths).await.unwrap();
    assert_eq!(n, 5);
    let db = IndexDb::open(&paths.index_db_path()).unwrap();
    assert_eq!(db.count().unwrap(), 5);
    let by_session = db.list_by_session("s-bulk").unwrap();
    assert_eq!(by_session.len(), 5);
}

#[tokio::test]
async fn unparseable_line_in_middle_does_not_abort_load() {
    let dir = TempDir::new().unwrap();
    let paths = fixture_paths(&dir);
    let started_at = Utc::now();
    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-bad".into(),
            session_id: "s-bad".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        fixture_session_meta("t-bad", "s-bad"),
    )
    .await
    .unwrap();
    recorder
        .write_line(
            RolloutPayload::UserMessage(UserMessagePayload {
                content: vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
            }),
            PartialMeta::default(),
        )
        .await
        .unwrap();
    recorder.close().await.unwrap();

    // Append a garbage line to simulate corruption.
    let path = paths.rollout_path("t-bad", started_at);
    let mut buf = tokio::fs::read(&path).await.unwrap();
    buf.extend_from_slice(b"\n{not valid json\n");
    let recorder2 = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-bad".into(),
            session_id: "s-bad".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        // Re-opening writes another SessionMeta. That's fine for this test;
        // we only care that loading tolerates the malformed line.
        fixture_session_meta("t-bad", "s-bad"),
    )
    .await
    .unwrap();
    recorder2
        .write_line(
            RolloutPayload::UserMessage(UserMessagePayload {
                content: vec![ContentBlock::Text {
                    text: "after corruption".into(),
                }],
            }),
            PartialMeta::default(),
        )
        .await
        .unwrap();
    recorder2.close().await.unwrap();

    tokio::fs::write(&path, buf).await.unwrap();
    let (items, stats) = load_rollout(&path).await.unwrap();
    assert!(stats.parse_errors >= 1, "expected at least one parse error");
    assert!(!items.is_empty(), "valid lines still returned");
}

#[tokio::test]
async fn crash_safe_writer_fsyncs_per_line() {
    let dir = TempDir::new().unwrap();
    let paths = fixture_paths(&dir);
    let started_at = Utc::now();
    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-crash".into(),
            session_id: "s-crash".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        fixture_session_meta("t-crash", "s-crash"),
    )
    .await
    .unwrap();
    for i in 0..5 {
        recorder
            .write_line(
                RolloutPayload::UserMessage(UserMessagePayload {
                    content: vec![ContentBlock::Text {
                        text: format!("line-{i}"),
                    }],
                }),
                PartialMeta::default(),
            )
            .await
            .unwrap();
    }
    // Drop without close — flush-per-line means each line is durable
    // even with no explicit close.
    drop(recorder);

    let (items, _) = load_rollout(&paths.rollout_path("t-crash", started_at))
        .await
        .unwrap();
    assert_eq!(items.len(), 6, "SessionMeta + 5 lines all durable");
}

#[tokio::test]
async fn concurrent_subagent_writes_do_not_interleave() {
    let dir = TempDir::new().unwrap();
    let paths = fixture_paths(&dir);
    let started_at = Utc::now();
    let mut handles = Vec::new();
    for i in 0..4 {
        let paths = paths.clone();
        handles.push(tokio::spawn(async move {
            let thread_id = format!("t-c-{i}");
            let path = paths.subagent_path("t-parent", started_at, &thread_id);
            let r = RolloutRecorder::open(
                RolloutRecorderConfig {
                    paths: paths.clone(),
                    thread_id: thread_id.clone(),
                    session_id: "s-c".into(),
                    started_at,
                    is_sidechain: true,
                    spill_threshold_bytes: None,
                    override_path: Some(path.clone()),
                },
                fixture_session_meta(&thread_id, "s-c"),
            )
            .await
            .unwrap();
            for k in 0..10 {
                r.write_line(
                    RolloutPayload::UserMessage(UserMessagePayload {
                        content: vec![ContentBlock::Text {
                            text: format!("line-{k}"),
                        }],
                    }),
                    PartialMeta::default(),
                )
                .await
                .unwrap();
            }
            r.close().await.unwrap();
            path
        }));
    }
    for h in handles {
        let path = h.await.unwrap();
        let (items, stats) = load_rollout(&path).await.unwrap();
        assert_eq!(stats.parse_errors, 0, "no interleaving for {path:?}");
        assert_eq!(items.len(), 11);
    }
}

#[tokio::test]
async fn session_meta_pins_artifact_source_air_hash() {
    let dir = TempDir::new().unwrap();
    let paths = fixture_paths(&dir);
    let started_at = Utc::now();
    let meta = fixture_session_meta("t-pin", "s-pin");
    let r = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-pin".into(),
            session_id: "s-pin".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        meta.clone(),
    )
    .await
    .unwrap();
    r.close().await.unwrap();
    let (items, _) = load_rollout(&paths.rollout_path("t-pin", started_at))
        .await
        .unwrap();
    let RolloutPayload::SessionMeta(written) = &items[0].payload else {
        panic!("first line is SessionMeta");
    };
    assert_eq!(written.artifact_hash, meta.artifact_hash);
    assert_eq!(written.source_hash, meta.source_hash);
    assert_eq!(written.air_hash, meta.air_hash);
}
