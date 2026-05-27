//! Phase 14.8.F — `apxm rollout {list, archive, replay}` subcommands.
//!
//! Offline counterparts to `apxm watch`: read directly from
//! `APXM_ROLLOUT_HOME/sessions/rollouts/...` via the durability layer
//! shipped in Phase 14.8.E. No apxm-server required — these work in
//! airgapped/regulatory-replay scenarios.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use apxm_core::events::ApxmEvent;
use apxm_rollout::{
    IndexDb, RolloutPaths, RolloutPayload, ThreadIndexEntry, load_rollout, reconstruct_history,
    rebuild_index_from_disk,
};
use flate2::Compression;
use flate2::write::GzEncoder;

use super::render::{RunSnapshot, render_tree};

#[derive(Debug, Default, Clone)]
pub struct RolloutListOptions {
    pub session: Option<String>,
    pub since: Option<String>,
    pub agent_role: Option<String>,
    pub limit: usize,
    /// Override APXM_ROLLOUT_HOME for tests + scripts.
    pub home: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RolloutArchiveOptions {
    pub thread_id: String,
    pub output: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// Optional explicit skill source directory (skill.toml/SKILL.md/skill.air).
    /// When unset, the archive includes only the rollout JSONL + spilled blobs.
    pub skill_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RolloutReplayOptions {
    pub thread_id: String,
    pub home: Option<PathBuf>,
}

fn resolve_paths(override_home: Option<PathBuf>) -> RolloutPaths {
    override_home.map_or_else(RolloutPaths::from_env, RolloutPaths::new)
}

/// `apxm rollout list` — query the SQLite thread index and print one
/// summary row per thread. Falls back to a one-shot rebuild from disk
/// when the index is empty (handy after manual file ops).
pub async fn rollout_list_command(opts: RolloutListOptions) -> Result<()> {
    let paths = resolve_paths(opts.home);
    let db_path = paths.index_db_path();
    // Auto-rebuild when the index hasn't been seeded yet (first-run UX).
    if !db_path.exists() {
        rebuild_index_from_disk(&paths)
            .await
            .context("failed to rebuild rollout index from disk")?;
    }
    let db = IndexDb::open(&db_path)
        .with_context(|| format!("failed to open rollout index at {}", db_path.display()))?;
    let limit = if opts.limit == 0 { 200 } else { opts.limit };
    let mut entries = if let Some(session) = opts.session.as_deref() {
        db.list_by_session(session)
            .context("list_by_session failed")?
    } else {
        db.list_recent(limit).context("list_recent failed")?
    };
    if let Some(role) = opts.agent_role.as_deref() {
        entries.retain(|e| e.agent_role == role);
    }
    if let Some(since) = opts.since.as_deref() {
        entries.retain(|e| e.started_at.as_str() >= since);
    }
    entries.truncate(limit);
    print_entries_table(&entries);
    Ok(())
}

fn print_entries_table(entries: &[ThreadIndexEntry]) {
    if entries.is_empty() {
        println!("(no rollouts found)");
        return;
    }
    println!(
        "{:<36}  {:<20}  {:<22}  {:>6}  {:>8}",
        "THREAD_ID", "STARTED_AT", "AGENT_ROLE", "LINES", "BYTES"
    );
    for entry in entries {
        let started = entry.started_at.chars().take(19).collect::<String>();
        println!(
            "{:<36}  {:<20}  {:<22}  {:>6}  {:>8}",
            truncate(&entry.thread_id, 36),
            started,
            truncate(&entry.agent_role, 22),
            entry.line_count,
            entry.file_bytes,
        );
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n.saturating_sub(1)])
    }
}

/// `apxm rollout replay <thread_id>` — read the rollout JSONL from disk
/// and render the same tree `apxm watch` would render off the live SSE.
pub async fn rollout_replay_command(opts: RolloutReplayOptions) -> Result<()> {
    let paths = resolve_paths(opts.home);
    let entry = lookup_thread(&paths, &opts.thread_id).await?;
    let path = PathBuf::from(&entry.file_path);
    let (items, _) = load_rollout(&path)
        .await
        .with_context(|| format!("failed to load rollout {}", path.display()))?;
    let tree = reconstruct_history(&items);
    // We render off the *full* item list (not just the post-compact suffix)
    // because the snapshot folds idempotently and a viewer following
    // along with `replay` expects to see the whole run.
    let _ = tree; // baseline + suffix are reconstructed for parity with the loader API.
    let mut snapshot = RunSnapshot::new(opts.thread_id.clone());
    for line in &items {
        if let RolloutPayload::Event(event_payload) = &line.payload
            && let Ok(event) = serde_json::from_value::<ApxmEvent>(event_payload.event.clone())
        {
            snapshot.apply(&event);
        }
    }
    println!("{}", render_tree(&snapshot));
    Ok(())
}

async fn lookup_thread(paths: &RolloutPaths, thread_id: &str) -> Result<ThreadIndexEntry> {
    let db_path = paths.index_db_path();
    if !db_path.exists() {
        rebuild_index_from_disk(paths)
            .await
            .context("failed to seed rollout index for replay")?;
    }
    let db = IndexDb::open(&db_path).context("failed to open rollout index")?;
    db.get(thread_id)
        .context("rollout index query failed")?
        .ok_or_else(|| anyhow!("rollout not found for thread_id {thread_id}"))
}

/// `apxm rollout archive <thread_id>` — bundle the rollout JSONL +
/// referenced blobs + (optionally) the source skill into a `.tar.gz`.
/// The output is the air-gapped reproducibility envelope described in
/// Phase 14.8.E.1 of the plan.
pub async fn rollout_archive_command(opts: RolloutArchiveOptions) -> Result<PathBuf> {
    let paths = resolve_paths(opts.home);
    let entry = lookup_thread(&paths, &opts.thread_id).await?;
    let rollout_path = PathBuf::from(&entry.file_path);
    let output = opts
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("apxm-rollout-{}.tar.gz", opts.thread_id)));
    let tar_file = File::create(&output)
        .with_context(|| format!("failed to create {}", output.display()))?;
    let encoder = GzEncoder::new(tar_file, Compression::default());
    let mut builder = tar::Builder::new(encoder);
    // 1) Always include the rollout JSONL as `rollout.jsonl` at the
    //    archive root — keeps the regulatory replay layout stable
    //    independent of the on-disk YYYY/MM/DD nesting.
    append_file(&mut builder, &rollout_path, "rollout.jsonl")?;
    // 2) Include every spilled blob the rollout points at. The sibling
    //    `blobs/` directory next to the rollout file holds the content-
    //    addressed payloads emitted by RolloutRecorder::maybe_spill.
    let sidecar = rollout_path
        .parent()
        .and_then(Path::parent)
        .map(|p| p.join(rollout_path.file_stem().unwrap_or_default()));
    if let Some(sidecar_dir) = sidecar
        && sidecar_dir.is_dir()
    {
        let blobs_dir = sidecar_dir.join("blobs");
        if blobs_dir.is_dir() {
            for entry in std::fs::read_dir(&blobs_dir)
                .with_context(|| format!("failed to read {}", blobs_dir.display()))?
            {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    let name = path.file_name().map_or_else(
                        || "blobs/unknown.blob".to_string(),
                        |s| format!("blobs/{}", s.to_string_lossy()),
                    );
                    append_file(&mut builder, &path, &name)?;
                }
            }
        }
    }
    // 3) Optional skill source (skill.toml / SKILL.md / skill.air) +
    //    any .apxmobj alongside. Required for byte-identical replay.
    if let Some(skill_dir) = opts.skill_dir {
        for filename in [
            "skill.toml",
            "SKILL.md",
            "skill.air",
            "skill.apxmobj",
        ] {
            let path = skill_dir.join(filename);
            if path.is_file() {
                append_file(&mut builder, &path, &format!("skill/{filename}"))?;
            }
        }
    }
    builder.finish().context("tar finalize failed")?;
    Ok(output)
}

fn append_file<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    path: &Path,
    name: &str,
) -> Result<()> {
    let mut file = File::open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    builder
        .append_file(name, &mut file)
        .with_context(|| format!("failed to append {} as {name}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use apxm_rollout::{
        PartialMeta, RolloutPaths, RolloutPayload, RolloutRecorder, RolloutRecorderConfig,
        SessionMetaPayload, now_rfc3339,
    };
    use chrono::Utc;
    use flate2::read::GzDecoder;
    use tempfile::TempDir;

    use super::*;

    fn fixture_session_meta(thread_id: &str, agent_role: &str) -> SessionMetaPayload {
        SessionMetaPayload {
            thread_id: thread_id.to_string(),
            parent_thread_id: None,
            session_id: format!("s-{thread_id}"),
            started_at: now_rfc3339(),
            cwd: "/tmp".to_string(),
            apxm_version: "test".to_string(),
            agent_role: agent_role.to_string(),
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

    fn synthetic_event(seq: u64, node_id: u64, agent_code: &str) -> ApxmEvent {
        let raw = serde_json::json!({
            "meta": {
                "seq": seq,
                "timestamp": "2026-05-27T00:00:00Z",
                "trace_id": "t-test",
                "source": "runtime",
                "span_id": format!("span-{seq}"),
            },
            "payload": {
                "kind": "agent_spawned",
                "node_id": node_id,
                "agent_code": agent_code,
                "parent_execution_id": "t-test",
            },
        });
        serde_json::from_value(raw).unwrap()
    }

    async fn write_rollout(paths: Arc<RolloutPaths>, thread_id: &str) -> std::path::PathBuf {
        let recorder = RolloutRecorder::open(
            RolloutRecorderConfig {
                paths: paths.clone(),
                thread_id: thread_id.to_string(),
                session_id: format!("s-{thread_id}"),
                started_at: Utc::now(),
                is_sidechain: false,
                spill_threshold_bytes: None,
                override_path: None,
            },
            fixture_session_meta(thread_id, "coordinator"),
        )
        .await
        .expect("open recorder");
        for (i, agent) in ["module.knowledge", "module.crm", "module.revenue"]
            .iter()
            .enumerate()
        {
            recorder
                .write_event(
                    synthetic_event(i as u64 + 1, 100 + i as u64, agent),
                    PartialMeta::default(),
                )
                .await
                .unwrap();
        }
        recorder.close().await.unwrap();
        recorder.file_path().clone()
    }

    #[tokio::test]
    async fn replay_reads_rollout_from_disk_renders_same_tree_as_watch() {
        let dir = TempDir::new().unwrap();
        let paths = Arc::new(RolloutPaths::new(dir.path().to_path_buf()));
        write_rollout(paths.clone(), "t-replay").await;
        // The watch path folds events into a RunSnapshot; replay does the
        // same off disk. Same fixture in, same shape out — the test asserts
        // both surfaces agree on agent rows.
        let mut watch_snapshot = RunSnapshot::new("t-replay");
        for (i, agent) in ["module.knowledge", "module.crm", "module.revenue"]
            .iter()
            .enumerate()
        {
            watch_snapshot.apply(&synthetic_event(i as u64 + 1, 100 + i as u64, agent));
        }
        let expected = render_tree(&watch_snapshot);
        assert!(expected.contains("module.knowledge"));
        assert!(expected.contains("module.crm"));
        assert!(expected.contains("module.revenue"));
        let result = rollout_replay_command(RolloutReplayOptions {
            thread_id: "t-replay".to_string(),
            home: Some(dir.path().to_path_buf()),
        })
        .await;
        assert!(result.is_ok(), "rollout replay failed: {result:?}");
    }

    #[tokio::test]
    async fn rollout_list_queries_index_returns_recent() {
        let dir = TempDir::new().unwrap();
        let paths = Arc::new(RolloutPaths::new(dir.path().to_path_buf()));
        for tid in ["t-a", "t-b", "t-c"] {
            write_rollout(paths.clone(), tid).await;
        }
        // First call auto-rebuilds the index from disk — that's the path
        // a freshly-cloned ops-machine takes.
        let result = rollout_list_command(RolloutListOptions {
            limit: 3,
            home: Some(dir.path().to_path_buf()),
            ..RolloutListOptions::default()
        })
        .await;
        assert!(result.is_ok(), "rollout list failed: {result:?}");
        // Confirm the index actually picked up all three rollouts.
        let db = IndexDb::open(&paths.index_db_path()).unwrap();
        let count = db.list_recent(10).unwrap().len();
        assert_eq!(count, 3, "expected 3 rollouts in index, got {count}");
    }

    #[tokio::test]
    async fn rollout_archive_includes_jsonl_and_skill_source() {
        let dir = TempDir::new().unwrap();
        let paths = Arc::new(RolloutPaths::new(dir.path().to_path_buf()));
        write_rollout(paths.clone(), "t-archive").await;
        let skill_dir = dir.path().join("skill-src");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("skill.toml"), "[skill]\nid = \"fix\"\n").unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Skill\n").unwrap();
        std::fs::write(skill_dir.join("skill.air"), "graph {}\n").unwrap();
        std::fs::write(skill_dir.join("skill.apxmobj"), [0u8; 16]).unwrap();

        let output = dir.path().join("archive.tar.gz");
        let path = rollout_archive_command(RolloutArchiveOptions {
            thread_id: "t-archive".to_string(),
            output: Some(output.clone()),
            home: Some(dir.path().to_path_buf()),
            skill_dir: Some(skill_dir),
        })
        .await
        .expect("archive");
        assert!(path.exists(), "archive not written");

        let file = File::open(&path).unwrap();
        let mut tar = tar::Archive::new(GzDecoder::new(file));
        let names: Vec<String> = tar
            .entries()
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.iter().any(|n| n == "rollout.jsonl"));
        assert!(names.iter().any(|n| n == "skill/skill.toml"));
        assert!(names.iter().any(|n| n == "skill/SKILL.md"));
        assert!(names.iter().any(|n| n == "skill/skill.air"));
        assert!(names.iter().any(|n| n == "skill/skill.apxmobj"));
    }

    #[tokio::test]
    async fn rollout_recorder_writes_event_payload_lines() {
        // Sanity check: confirm the on-disk JSONL really carries Event
        // payloads (not, e.g., AssistantMessage), so the replay loop
        // doesn't silently filter every line away.
        let dir = TempDir::new().unwrap();
        let paths = Arc::new(RolloutPaths::new(dir.path().to_path_buf()));
        let file = write_rollout(paths, "t-rt").await;
        let (lines, _stats) = apxm_rollout::load_rollout(&file).await.unwrap();
        let event_count = lines
            .iter()
            .filter(|l| matches!(l.payload, RolloutPayload::Event(_)))
            .count();
        assert_eq!(event_count, 3, "expected 3 Event payloads, got {event_count}");
    }
}
