//! Generates a tiny rollout file and prints the bytes — only useful as
//! `cargo test -p apxm-rollout sample_output -- --nocapture` for
//! eyeballing the wire format.

use std::sync::Arc;

use apxm_rollout::{
    ContentBlock, PartialMeta, RolloutPaths, RolloutPayload, RolloutRecorder,
    RolloutRecorderConfig, SessionMetaPayload, ToolUsePayload, UserMessagePayload, now_rfc3339,
};
use chrono::Utc;
use tempfile::TempDir;

#[tokio::test]
async fn sample_output() {
    let dir = TempDir::new().unwrap();
    let paths = Arc::new(RolloutPaths::new(dir.path().to_path_buf()));
    let started_at = Utc::now();
    let session_meta = SessionMetaPayload {
        thread_id: "exec-sample".to_string(),
        parent_thread_id: None,
        session_id: "sample-session".to_string(),
        started_at: now_rfc3339(),
        cwd: "/tmp/example".to_string(),
        apxm_version: "0.0.1".to_string(),
        agent_role: "coordinator".to_string(),
        agent_code: Some("module.crm".to_string()),
        skill_id: "fixture-skill".to_string(),
        skill_version: "0.1.0".to_string(),
        artifact_hash: "blake3:aaaa".to_string(),
        source_hash: "blake3:bbbb".to_string(),
        air_hash: "blake3:cccc".to_string(),
        compiler_version: Some("c-1".to_string()),
        runtime_version: Some("r-1".to_string()),
        args: vec!["hello".to_string()],
        model_provider: Some("mock".to_string()),
        model_id: Some("mock-1".to_string()),
        backend_endpoint: None,
        tool_use_id_in_parent: None,
    };
    let recorder = RolloutRecorder::open(
        RolloutRecorderConfig {
            paths: paths.clone(),
            thread_id: "exec-sample".into(),
            session_id: "sample-session".into(),
            started_at,
            is_sidechain: false,
            spill_threshold_bytes: None,
            override_path: None,
        },
        session_meta,
    )
    .await
    .unwrap();
    recorder
        .write_line(
            RolloutPayload::UserMessage(UserMessagePayload {
                content: vec![ContentBlock::Text {
                    text: "hello CLIC".into(),
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
            PartialMeta {
                tool_use_id: Some("tu_1".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    recorder.close().await.unwrap();
    let path = paths.rollout_path("exec-sample", started_at);
    let bytes = std::fs::read(&path).unwrap();
    println!("{}", String::from_utf8_lossy(&bytes));
}
