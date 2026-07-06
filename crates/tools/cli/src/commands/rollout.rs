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
    /// Optional explicit skill source directory (skill.toml/SKILL.md/skill.air).
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
    // 3) Optional skill source (skill.toml / SKILL.md / skill.air) +
    //    any .apxmobj alongside. Required for byte-identical replay.
    if let Some(skill_dir) = opts.skill_dir {
        for filename in ["skill.toml", "SKILL.md", "skill.air", "skill.apxmobj"] {
            let path = skill_dir.join(filename);
            if path.is_file() {
                append_file(&mut builder, &path, &format!("skill/{filename}"))?;
            }
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
