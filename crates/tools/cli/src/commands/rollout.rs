//!.F — `apxm rollout {list, archive, replay}` subcommands.
//!
//! Offline counterparts to `apxm watch`: read directly from
//! `APXM_ROLLOUT_HOME/sessions/rollouts/...` via the durability layer
//! shipped in. No apxm-server required — these work in
//! airgapped/regulatory-replay scenarios.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use apxm_core::events::ApxmEvent;
use apxm_rollout::{
    CompactionReport, IndexDb, RetentionPolicy, RolloutPaths, RolloutPayload, ThreadIndexEntry,
    compact, load_rollout, rebuild_index_from_disk, reconstruct_history,
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
    /// Optional explicit instruction skill directory.
    /// When unset, the archive includes only the rollout JSONL + spilled blobs.
    pub skill_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct RolloutReplayOptions {
    pub thread_id: String,
    pub home: Option<PathBuf>,
}

///  retention/compaction options — see [`apxm_rollout::retention`].
#[derive(Debug, Default, Clone)]
pub struct RolloutCompactOptions {
    pub home: Option<PathBuf>,
    /// Override the rollout max-age policy (days). Falls back to
    /// `APXM_RETENTION_ROLLOUT_MAX_AGE_DAYS` / the built-in default.
    pub max_age_days: Option<u64>,
    /// Override the blob GC grace period (hours). Falls back to
    /// `APXM_RETENTION_BLOB_GC_GRACE_HOURS` / the built-in default.
    pub blob_grace_hours: Option<u64>,
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
        let RolloutPayload::Event(event_payload) = &line.payload else {
            continue;
        };
        let event = serde_json::from_value::<ApxmEvent>(event_payload.event.clone()).with_context(
            || {
                format!(
                    "rollout {} seq {} carries an undecodable {} event; \
                     replay is evidence, so an unreadable record fails the command \
                     instead of rendering a tree that silently omits it",
                    path.display(),
                    line.meta.seq,
                    event_payload.event_kind,
                )
            },
        )?;
        snapshot.apply(&event);
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
/// referenced blobs + (optionally) instruction skill markdown into a `.tar.gz`.
/// The output is an air-gapped reproducibility envelope.
pub async fn rollout_archive_command(opts: RolloutArchiveOptions) -> Result<PathBuf> {
    let paths = resolve_paths(opts.home);
    let entry = lookup_thread(&paths, &opts.thread_id).await?;
    let rollout_path = PathBuf::from(&entry.file_path);
    let output = opts
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("apxm-rollout-{}.tar.gz", opts.thread_id)));
    let tar_file =
        File::create(&output).with_context(|| format!("failed to create {}", output.display()))?;
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
    // 3) Optional instruction skill markdown. Executable program content is
    //    captured by package/workflow metadata, not skill artifacts.
    if let Some(skill_dir) = opts.skill_dir {
        let filename = "SKILL.md";
        let path = skill_dir.join(filename);
        if path.is_file() {
            append_file(&mut builder, &path, &format!("skill/{filename}"))?;
        }
    }
    builder.finish().context("tar finalize failed")?;
    Ok(output)
}

/// `apxm rollout compact` —  retention pass over agents-owned durable
/// state: archives rollout JSONL bodies older than the max-age policy
/// (index row survives with `status = archived`) and collects blobs no
/// longer referenced by any rollout. Never touches the memory tier — this
/// crate has no dependency on `apxm-memory`/`apxm-backends`.
pub async fn rollout_compact_command(opts: RolloutCompactOptions) -> Result<CompactionReport> {
    let paths = resolve_paths(opts.home);
    let mut policy = RetentionPolicy::from_env();
    if let Some(days) = opts.max_age_days {
        policy.rollout_max_age = std::time::Duration::from_secs(days * 24 * 3600);
    }
    if let Some(hours) = opts.blob_grace_hours {
        policy.blob_gc_grace = std::time::Duration::from_secs(hours * 3600);
    }
    let report = compact(&paths, &policy)
        .await
        .context("retention compaction failed")?;
    println!(
        "rollouts: scanned={} archived={} bytes_reclaimed={}",
        report.rollouts.scanned, report.rollouts.archived, report.rollouts.bytes_reclaimed
    );
    println!(
        "blobs:    scanned={} deleted={} retained={} bytes_reclaimed={}",
        report.blobs.scanned,
        report.blobs.deleted,
        report.blobs.retained,
        report.blobs.bytes_reclaimed
    );
    Ok(report)
}

fn append_file<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    path: &Path,
    name: &str,
) -> Result<()> {
    let mut file =
        File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    builder
        .append_file(name, &mut file)
        .with_context(|| format!("failed to append {} as {name}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use apxm_core::events::EventSource;
    use apxm_core::events::payload::TokenPayload;
    use apxm_rollout::{PartialMeta, RolloutRecorder, RolloutRecorderConfig, SessionMetaPayload};
    use chrono::Utc;

    use super::*;

    fn session_meta(thread_id: &str, started_at: chrono::DateTime<Utc>) -> SessionMetaPayload {
        SessionMetaPayload {
            thread_id: thread_id.to_string(),
            parent_thread_id: None,
            session_id: format!("session-{thread_id}"),
            started_at: started_at.to_rfc3339(),
            cwd: "/tmp".to_string(),
            apxm_version: "0.1.0".to_string(),
            agent_role: "test-agent".to_string(),
            agent_code: None,
            program_package_id: "package".to_string(),
            program_package_digest: "digest".to_string(),
            artifact_hash: "artifact".to_string(),
            source_hash: "source".to_string(),
            air_hash: "air".to_string(),
            compiler_version: None,
            runtime_version: None,
            args: vec![],
            model_provider: None,
            model_id: None,
            backend_endpoint: None,
            tool_use_id_in_parent: None,
        }
    }

    /// `apxm rollout replay` renders persisted evidence, so a record it cannot
    /// decode must fail the command rather than be dropped from the rendered
    /// tree.
    ///
    /// The concrete record here is a `capability_effect_receipt` whose
    /// `dispatch_path` is `ask_tool` — a name this runtime's
    /// `CapabilityEffectDispatchPath` does not admit, because a capability
    /// effect is dispatched only by a graph `INV_CAP` operation. Such a line
    /// can exist on disk from a writer that predates that closure. Replay must
    /// surface it: there is no reader that translates the removed name onto a
    /// canonical one, and there is no silent skip that would let an operator
    /// read a tree missing a committed external effect and believe it complete.
    #[tokio::test]
    async fn replay_fails_closed_on_undecodable_persisted_evidence() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().to_path_buf();
        let paths = Arc::new(RolloutPaths::new(home.clone()));
        let thread_id = "thread-undecodable-receipt";
        let started_at = Utc::now();

        let recorder = RolloutRecorder::open(
            RolloutRecorderConfig {
                paths: paths.clone(),
                thread_id: thread_id.to_string(),
                session_id: format!("session-{thread_id}"),
                started_at,
                is_sidechain: false,
                spill_threshold_bytes: None,
                override_path: None,
            },
            session_meta(thread_id, started_at),
        )
        .await
        .expect("open recorder");

        // A decodable line, so the failure below cannot be confused with an
        // empty or structurally broken rollout.
        recorder
            .write_event(
                ApxmEvent::root(
                    TokenPayload {
                        text: "first".to_string(),
                        generation: None,
                    },
                    EventSource::Runtime,
                    thread_id,
                ),
                PartialMeta::default(),
            )
            .await
            .expect("write decodable event");

        // The receipt is written as a raw payload line rather than through
        // `ApxmEvent`, because the typed enum can no longer construct
        // `ask_tool` — which is exactly the point: only a previously written
        // record can carry it.
        recorder
            .write_line(
                RolloutPayload::Event(apxm_rollout::EventMsgPayload {
                    event_kind: "capability_effect_receipt".to_string(),
                    event: serde_json::json!({
                        "meta": {
                            "seq": 2,
                            "timestamp": started_at.to_rfc3339(),
                            "trace_id": thread_id,
                            "source": "runtime",
                            "span_id": "span-effect-receipt",
                            "parent_span_id": null,
                        },
                        "payload": {
                            "kind": "capability_effect_receipt",
                            "receipt_id": "receipt-1",
                            "execution_id": "execution-1",
                            "node_id": 7,
                            "invocation_id": "invocation-1",
                            "capability_binding": "calendar.write",
                            "dispatch_path": "ask_tool",
                            "implementation_kind": "typescript",
                            "implementation_ref": "package/calendar.write@1",
                            "request_digest": "sha256:request-1",
                            "admission_kind": "read_only",
                            "approval_status": "not_required",
                            "idempotency_proof": "transaction_verified",
                            "idempotency_key_digest": "sha256:idempotency-1",
                            "effect_ref": "effect-1",
                            "status": "committed",
                        },
                    }),
                }),
                PartialMeta::default(),
            )
            .await
            .expect("write undecodable receipt line");
        recorder.close().await.expect("close recorder");

        let error = rollout_replay_command(RolloutReplayOptions {
            thread_id: thread_id.to_string(),
            home: Some(home),
        })
        .await
        .expect_err("replay must fail closed on evidence it cannot decode");

        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("capability_effect_receipt"),
            "replay failure must name the undecodable event kind: {rendered}"
        );
        assert!(
            rendered.contains("ask_tool"),
            "replay failure must surface the undispatchable value that caused it: {rendered}"
        );
    }
}
