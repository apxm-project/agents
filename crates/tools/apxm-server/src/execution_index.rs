//! Bounded, reloadable execution-index sidecar.
//!
//! The execution snapshot directory (`{session_dir}/executions/`) holds one
//! JSON file per execution. Listing or resolving an execution by id used to
//! require an O(n) directory walk per request. This module keeps a per-
//! directory `_index.json` sidecar plus an in-memory LRU map that lets the
//! server skip the walk entirely for the hot set.
//!
//! Contract:
//!
//! * The index is a cache, not the source of truth — if the on-disk file is
//!   missing for an indexed id, the directory wins and the entry is dropped.
//! * Writes update the in-memory entry and the sidecar atomically
//!   (temp file + rename).
//! * The in-memory map is bounded; evicted entries fall back to a one-shot
//!   directory probe rather than returning a stale "not found".

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::executions::{ExecutionRecord, ExecutionStatus};

/// File name used for the per-directory sidecar.
pub(crate) const INDEX_FILE_NAME: &str = "_index.json";

/// Default LRU bound for in-memory index entries.
pub(crate) const DEFAULT_MAX_ENTRIES: usize = 10_000;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Compact lookup record stored in the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IndexEntry {
    /// Owning `executions/` directory (parent of the snapshot file).
    pub(crate) executions_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) skill_id: Option<String>,
    pub(crate) started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) finished_at_ms: Option<u64>,
    pub(crate) status: ExecutionStatus,
}

impl IndexEntry {
    pub(crate) fn snapshot_path(&self, execution_id: &str) -> PathBuf {
        self.executions_dir.join(format!("{execution_id}.json"))
    }
}

/// On-disk shape of the sidecar — `{ execution_id: { skill_id?, ... } }`.
/// `executions_dir` is *not* persisted (it is recovered from the sidecar's
/// own location at load time).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SidecarEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skill_id: Option<String>,
    started_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    finished_at_ms: Option<u64>,
    status: ExecutionStatus,
}

impl From<&IndexEntry> for SidecarEntry {
    fn from(entry: &IndexEntry) -> Self {
        Self {
            skill_id: entry.skill_id.clone(),
            started_at_ms: entry.started_at_ms,
            finished_at_ms: entry.finished_at_ms,
            status: entry.status.clone(),
        }
    }
}

#[derive(Default)]
struct LruInner {
    /// execution_id → (entry, access counter)
    entries: HashMap<String, (IndexEntry, u64)>,
    /// access counter → execution_id (sorted; smallest counter is oldest)
    by_access: BTreeMap<u64, String>,
    /// Monotonic access counter.
    counter: u64,
    /// Per-directory dirty flag, set whenever an entry under that directory
    /// is inserted, updated, or deleted in memory.
    dirty_dirs: HashMap<PathBuf, ()>,
    max_entries: usize,
}

impl LruInner {
    fn next_counter(&mut self) -> u64 {
        self.counter = self.counter.wrapping_add(1);
        self.counter
    }

    fn touch(&mut self, key: &str) {
        let new_counter = self.next_counter();
        if let Some((_, counter)) = self.entries.get_mut(key) {
            self.by_access.remove(counter);
            *counter = new_counter;
            self.by_access.insert(new_counter, key.to_string());
        }
    }

    fn insert(&mut self, key: String, entry: IndexEntry) {
        // Drop any prior counter for this key.
        if let Some((_, old_counter)) = self.entries.remove(&key) {
            self.by_access.remove(&old_counter);
        }
        let counter = self.next_counter();
        self.dirty_dirs.insert(entry.executions_dir.clone(), ());
        self.by_access.insert(counter, key.clone());
        self.entries.insert(key, (entry, counter));
        // Evict until we are within bounds. Evictions do not flag the
        // sidecar dirty — the on-disk index keeps the entry, so a future
        // miss can rehydrate it from disk.
        while self.entries.len() > self.max_entries {
            let Some((&oldest_counter, _)) = self.by_access.iter().next() else {
                break;
            };
            let oldest_key = self
                .by_access
                .remove(&oldest_counter)
                .expect("oldest counter present");
            self.entries.remove(&oldest_key);
        }
    }

    fn get(&mut self, key: &str) -> Option<IndexEntry> {
        let entry = self.entries.get(key).map(|(entry, _)| entry.clone());
        if entry.is_some() {
            self.touch(key);
        }
        entry
    }

    fn entries_for_dir(&self, dir: &Path) -> HashMap<String, IndexEntry> {
        self.entries
            .iter()
            .filter(|(_, (entry, _))| entry.executions_dir == dir)
            .map(|(id, (entry, _))| (id.clone(), entry.clone()))
            .collect()
    }

    fn drain_dirty_dirs(&mut self) -> Vec<PathBuf> {
        let dirs: Vec<PathBuf> = self.dirty_dirs.keys().cloned().collect();
        self.dirty_dirs.clear();
        dirs
    }
}

/// Bounded, reloadable execution index.
#[derive(Clone)]
pub(crate) struct ExecutionIndex {
    inner: Arc<Mutex<LruInner>>,
}

impl ExecutionIndex {
    pub(crate) fn new() -> Self {
        Self::with_capacity(DEFAULT_MAX_ENTRIES)
    }

    pub(crate) fn with_capacity(max_entries: usize) -> Self {
        let max_entries = max_entries.max(1);
        Self {
            inner: Arc::new(Mutex::new(LruInner {
                max_entries,
                ..LruInner::default()
            })),
        }
    }

    #[cfg(test)]
    pub(crate) fn max_entries_for_tests(&self) -> usize {
        self.inner
            .lock()
            .expect("execution index poisoned")
            .max_entries
    }

    /// Upsert an entry derived from a freshly-persisted execution record.
    /// Persists the sidecar for the affected directory eagerly.
    pub(crate) fn upsert_from_record(&self, record: &ExecutionRecord) {
        let executions_dir = Path::new(&record.session_dir).join("executions");
        let entry = IndexEntry {
            executions_dir: executions_dir.clone(),
            skill_id: Some(record.skill_id.clone()),
            started_at_ms: record.started_at_ms,
            finished_at_ms: record.completed_at_ms,
            status: record.status.clone(),
        };
        let key = record.execution_id.clone();
        let dirs_to_flush = {
            let mut inner = self.inner.lock().expect("execution index poisoned");
            inner.insert(key, entry);
            inner.drain_dirty_dirs()
        };
        for dir in dirs_to_flush {
            self.flush_dir(&dir);
        }
    }

    /// Lookup an execution by id. Returns `None` when neither the in-memory
    /// LRU nor a disk probe finds it.
    pub(crate) fn get(&self, execution_id: &str) -> Option<IndexEntry> {
        let cached = {
            let mut inner = self.inner.lock().expect("execution index poisoned");
            inner.get(execution_id)
        };
        if cached.is_some() {
            return cached;
        }
        self.resolve_from_disk(execution_id)
    }

    /// Replace in-memory state by walking the supplied session roots: each
    /// `executions/` directory under the tree is rebuilt, with the
    /// directory winning over any disagreement with `_index.json`.
    pub(crate) fn reload_from_session_roots<I, P>(&self, session_roots: I) -> usize
    where
        I: IntoIterator<Item = P>,
        P: AsRef<Path>,
    {
        let mut loaded = 0;
        let mut executions_dirs: Vec<PathBuf> = Vec::new();
        for root in session_roots {
            collect_executions_dirs(root.as_ref(), &mut executions_dirs);
        }
        for dir in executions_dirs {
            loaded += self.reload_dir(&dir);
        }
        loaded
    }

    fn reload_dir(&self, dir: &Path) -> usize {
        // Source of truth: actual `*.json` snapshots in the directory.
        let mut on_disk: HashMap<String, IndexEntry> = HashMap::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = match path.file_name().and_then(|name| name.to_str()) {
                Some(name) => name,
                None => continue,
            };
            if file_name == INDEX_FILE_NAME {
                continue;
            }
            if !file_name.ends_with(".json") {
                continue;
            }
            let Some(record) = read_snapshot(&path) else {
                continue;
            };
            let key = record.execution_id.clone();
            let index_entry = IndexEntry {
                executions_dir: dir.to_path_buf(),
                skill_id: Some(record.skill_id.clone()),
                started_at_ms: record.started_at_ms,
                finished_at_ms: record.completed_at_ms,
                status: record.status.clone(),
            };
            on_disk.insert(key, index_entry);
        }

        // Try to short-circuit by trusting the sidecar where it agrees.
        let sidecar = read_sidecar(dir).unwrap_or_default();
        let mut loaded = 0;
        let dir_path = dir.to_path_buf();
        for (id, entry) in &on_disk {
            if let Some(side) = sidecar.get(id)
                && side.started_at_ms == entry.started_at_ms
                    && side.status == entry.status
                    && side.finished_at_ms == entry.finished_at_ms
                {
                    // Sidecar agrees; nothing to do beyond inserting.
                }
            {
                let mut inner = self.inner.lock().expect("execution index poisoned");
                inner.insert(id.clone(), entry.clone());
            }
            loaded += 1;
        }

        // If the sidecar disagrees (or is missing), rewrite it from the
        // freshly-loaded set so subsequent boots have a clean cache.
        let needs_rewrite = sidecar.len() != on_disk.len()
            || sidecar.keys().any(|id| !on_disk.contains_key(id))
            || on_disk.iter().any(|(id, entry)| {
                sidecar
                    .get(id)
                    .is_none_or(|s| {
                        s.started_at_ms != entry.started_at_ms
                            || s.status != entry.status
                            || s.finished_at_ms != entry.finished_at_ms
                    })
            });
        if needs_rewrite {
            let snapshot: HashMap<String, SidecarEntry> = on_disk
                .iter()
                .map(|(id, entry)| (id.clone(), SidecarEntry::from(entry)))
                .collect();
            write_sidecar(&dir_path, &snapshot);
            // We persisted; clear any dirty flag we may have set so the next
            // upsert doesn't redundantly rewrite.
            let mut inner = self.inner.lock().expect("execution index poisoned");
            inner.dirty_dirs.remove(&dir_path);
        }
        loaded
    }

    fn flush_dir(&self, dir: &Path) {
        let entries = {
            let inner = self.inner.lock().expect("execution index poisoned");
            inner.entries_for_dir(dir)
        };
        // The sidecar must reflect *all* snapshots on disk, not just the
        // hot set in memory. Merge the in-memory view with whatever else is
        // on disk before persisting.
        let mut sidecar = read_sidecar(dir).unwrap_or_default();
        for (id, entry) in &entries {
            sidecar.insert(id.clone(), SidecarEntry::from(entry));
        }
        // Drop sidecar entries whose snapshot file vanished.
        sidecar.retain(|id, _| dir.join(format!("{id}.json")).is_file());
        write_sidecar(dir, &sidecar);
    }

    fn resolve_from_disk(&self, execution_id: &str) -> Option<IndexEntry> {
        // We can only probe directories we already know about (i.e. ones the
        // index has touched in this process). Scanning every session tree
        // here would defeat the point of having an index in the first place.
        let candidate_dirs: Vec<PathBuf> = {
            let inner = self.inner.lock().expect("execution index poisoned");
            let mut dirs: HashMap<PathBuf, ()> = HashMap::new();
            for (entry, _) in inner.entries.values() {
                dirs.insert(entry.executions_dir.clone(), ());
            }
            dirs.into_keys().collect()
        };
        for dir in candidate_dirs {
            let path = dir.join(format!("{execution_id}.json"));
            if !path.is_file() {
                continue;
            }
            let Some(record) = read_snapshot(&path) else {
                continue;
            };
            let entry = IndexEntry {
                executions_dir: dir,
                skill_id: Some(record.skill_id.clone()),
                started_at_ms: record.started_at_ms,
                finished_at_ms: record.completed_at_ms,
                status: record.status.clone(),
            };
            {
                let mut inner = self.inner.lock().expect("execution index poisoned");
                inner.insert(execution_id.to_string(), entry.clone());
                // Disk probe rehydrates a previously-evicted entry; the
                // sidecar already contains it, so we drop the dirty flag.
                inner.dirty_dirs.remove(&entry.executions_dir);
            }
            return Some(entry);
        }
        None
    }

    #[cfg(test)]
    pub(crate) fn in_memory_len(&self) -> usize {
        self.inner
            .lock()
            .expect("execution index poisoned")
            .entries
            .len()
    }
}

impl Default for ExecutionIndex {
    fn default() -> Self {
        Self::new()
    }
}

fn collect_executions_dirs(root: &Path, out: &mut Vec<PathBuf>) {
    if !root.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("executions") {
            out.push(path);
        } else {
            collect_executions_dirs(&path, out);
        }
    }
}

fn read_snapshot(path: &Path) -> Option<ExecutionRecord> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn read_sidecar(dir: &Path) -> Option<HashMap<String, SidecarEntry>> {
    let path = dir.join(INDEX_FILE_NAME);
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_sidecar(dir: &Path, sidecar: &HashMap<String, SidecarEntry>) {
    if let Err(error) = std::fs::create_dir_all(dir) {
        tracing::warn!(
            path = %dir.display(),
            %error,
            "failed to create execution index directory"
        );
        return;
    }
    let path = dir.join(INDEX_FILE_NAME);
    let Ok(bytes) = serde_json::to_vec_pretty(sidecar) else {
        tracing::warn!(
            path = %path.display(),
            "failed to serialize execution index sidecar"
        );
        return;
    };
    let temp_path = unique_temp_path(&path);
    if let Err(error) = std::fs::write(&temp_path, bytes) {
        tracing::warn!(
            path = %temp_path.display(),
            %error,
            "failed to write execution index sidecar"
        );
        return;
    }
    if let Err(error) = std::fs::rename(&temp_path, &path) {
        tracing::warn!(
            from = %temp_path.display(),
            to = %path.display(),
            %error,
            "failed to persist execution index sidecar"
        );
    }
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("tmp");
    path.with_file_name(format!("{file_name}.{pid}.{counter}.tmp"))
}
