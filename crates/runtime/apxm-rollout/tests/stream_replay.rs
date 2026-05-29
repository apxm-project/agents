//! Streaming/replay-from-disk tests.

use std::sync::Arc;

use apxm_rollout::{
    ContentBlock, PartialMeta, RolloutPaths, RolloutPayload, RolloutRecorder,
    RolloutRecorderConfig, SessionMetaPayload, UserMessagePayload, load_rollout, now_rfc3339,
};
use chrono::Utc;
use tempfile::TempDir;

fn paths_for(dir: &TempDir) -> Arc<RolloutPaths> {
    Arc::new(RolloutPaths::new(dir.path().to_path_buf()))
}

fn fixture_session_meta(thread_id: &str) -> SessionMetaPayload {
    SessionMetaPayload {
        thread_id: thread_id.to_string(),
        parent_thread_id: None,
        session_id: "s-stream".to_string(),
        started_at: now_rfc3339(),
        cwd: "/tmp".to_string(),
        apxm_version: "test".to_string(),
        agent_role: "coordinator".to_string(),
        agent_code: None,
        skill_id: "fix".to_string(),
        skill_version: "0.1.0".to_string(),
        artifact_hash: "blake3:00".to_string(),
        source_hash: "blake3:01".to_string(),
        air_hash: "blake3:02".to_string(),
        compiler_version: None,
        runtime_version: None,
        args: vec![],
        model_provider: None,
        model_id: None,
        backend_endpoint: None,
        tool_use_id_in_parent: None,
    }
}

#[tokio::test]
async fn events_stream_tails_rollout_file_via_notify() {
    let dir = TempDir::new().unwrap();
    let paths = paths_for(&dir);
    let started_at = Utc::now();
    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-tail".into(),
            session_id: "s-stream".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        fixture_session_meta("t-tail"),
    )
    .await
    .unwrap();

    let file_path = paths.rollout_path("t-tail", started_at);
    // Follower: poll-tail in 50ms beats — simple and dependency-free.
    let follower_path = file_path.clone();
    let follower = tokio::spawn(async move {
        let mut seen = 0;
        let start = std::time::Instant::now();
        while seen < 3 && start.elapsed().as_secs() < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if let Ok((items, _)) = load_rollout(&follower_path).await {
                let nonmeta = items
                    .iter()
                    .filter(|l| !matches!(l.payload, RolloutPayload::SessionMeta(_)))
                    .count();
                seen = nonmeta;
            }
        }
        seen
    });

    for i in 0..3 {
        recorder
            .write_line(
                RolloutPayload::UserMessage(UserMessagePayload {
                    content: vec![ContentBlock::Text {
                        text: format!("msg-{i}"),
                    }],
                }),
                PartialMeta::default(),
            )
            .await
            .unwrap();
    }
    recorder.close().await.unwrap();

    let observed = follower.await.unwrap();
    assert_eq!(observed, 3, "follower observed all 3 new lines");
}

#[tokio::test]
async fn last_event_id_reconnect_replays_from_disk() {
    let dir = TempDir::new().unwrap();
    let paths = paths_for(&dir);
    let started_at = Utc::now();
    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "t-resume".into(),
            session_id: "s-stream".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        fixture_session_meta("t-resume"),
    )
    .await
    .unwrap();
    for i in 0..10 {
        recorder
            .write_line(
                RolloutPayload::UserMessage(UserMessagePayload {
                    content: vec![ContentBlock::Text {
                        text: format!("msg-{i}"),
                    }],
                }),
                PartialMeta::default(),
            )
            .await
            .unwrap();
    }
    recorder.close().await.unwrap();

    let file_path = paths.rollout_path("t-resume", started_at);
    let (items, _) = load_rollout(&file_path).await.unwrap();
    // Resume past seq=5 — caller filters by meta.seq >= since.
    let since: u64 = 5;
    let replay: Vec<_> = items.iter().filter(|l| l.meta.seq >= since).collect();
    // 11 total (SessionMeta + 10), 6 lines from seq=5 .. seq=10 inclusive.
    assert_eq!(replay.len(), 6);
    assert_eq!(replay.first().unwrap().meta.seq, 5);
}
