//! Permanent AIS operation-usage counter (RT-9).
//!
//! RT-1 justified pruning `NEGOTIATE`/`SPAWN_TEAM`/`GUARD`/`CLAIM` with a
//! one-off measurement of op usage against the example/test/studio-lowering
//! corpus; that instrumentation was never committed, so the next drift
//! between "operations defined" (`AIS_OPERATIONS`) and "operations actually
//! dispatched" would again require a bespoke, throwaway pass. This module
//! promotes that idea into a durable, always-on counter: every dispatch goes
//! through [`record`], and `apxm ops usage` reads the persisted snapshot
//! (see `crates/tools/cli/src/commands/ops.rs`).
//!
//! Counts are process-local until [`flush_to_disk`] merges them into a
//! cumulative JSON file under `<cache_dir>/op-usage/op-usage.json` (see
//! `apxm_core::paths::ApxmPaths::cache_component_dir`). CLI entry points that run
//! a graph to completion (`execute`, `run`, `workflow`) call
//! [`flush_to_disk`] once after the run finishes.

use apxm_core::types::operations::AISOperationType;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Filename for the persisted cumulative counter, relative to its cache
/// component directory.
pub const OP_USAGE_FILE_NAME: &str = "op-usage.json";

fn counters() -> &'static Mutex<HashMap<AISOperationType, u64>> {
    static COUNTERS: OnceLock<Mutex<HashMap<AISOperationType, u64>>> = OnceLock::new();
    COUNTERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record one dispatch of `op`. Called from
/// `OperationDispatcher::dispatch_inner` for every node — including
/// pseudo-ops (`AGENT`/`YIELD`), which still flow through dispatch even
/// though their handler is a no-op.
pub fn record(op: AISOperationType) {
    let mut map = counters()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *map.entry(op).or_insert(0) += 1;
}

/// In-process snapshot of dispatch counts accumulated since the process
/// started (or since the last [`reset`]).
pub fn snapshot() -> HashMap<AISOperationType, u64> {
    counters()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// Clear the in-process counters. Called by [`flush_to_disk`] after a
/// successful merge so a long-lived process doesn't double-count across
/// flushes.
pub fn reset() {
    counters()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

fn load_cumulative(path: &Path) -> HashMap<String, u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Merge the in-process snapshot into the persisted cumulative counter file
/// under `cache_dir` and reset in-process counters. Returns the path written
/// to. Pure I/O helper — [`flush_to_disk`] resolves `cache_dir` from
/// `ApxmPaths` for real callers; tests call this directly against a temp
/// directory so they don't need to mutate process environment.
///
/// Best-effort by design: usage stats are diagnostic, not part of the
/// execution contract, so a caller may choose to ignore I/O errors here
/// rather than fail a run over stats bookkeeping.
fn flush_to_dir(cache_dir: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(cache_dir)?;
    let path = cache_dir.join(OP_USAGE_FILE_NAME);
    let mut cumulative = load_cumulative(&path);
    for (op, count) in snapshot() {
        *cumulative.entry(op.to_string()).or_insert(0) += count;
    }
    let json = serde_json::to_string_pretty(&cumulative)?;
    std::fs::write(&path, json)?;
    reset();
    Ok(path)
}

/// Merge the in-process snapshot into the persisted cumulative counter file
/// under `<cache_dir>/op-usage/op-usage.json` and reset in-process counters.
/// Returns the path written to.
pub fn flush_to_disk(paths: &apxm_core::paths::ApxmPaths) -> io::Result<PathBuf> {
    let dir = paths.cache_component_dir("op-usage")?;
    flush_to_dir(&dir)
}

/// Read the persisted cumulative counters without touching in-process state.
/// Returns an empty map if the file does not exist yet.
pub fn read_persisted(paths: &apxm_core::paths::ApxmPaths) -> io::Result<HashMap<String, u64>> {
    let dir = paths.cache_component_dir("op-usage")?;
    Ok(load_cumulative(&dir.join(OP_USAGE_FILE_NAME)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // Global counters are process-wide state; serialize tests that touch
    // them so they don't observe each other's increments.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    #[test]
    fn record_and_snapshot_round_trip() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        record(AISOperationType::Ask);
        record(AISOperationType::Ask);
        record(AISOperationType::QMem);
        let snap = snapshot();
        assert_eq!(snap.get(&AISOperationType::Ask), Some(&2));
        assert_eq!(snap.get(&AISOperationType::QMem), Some(&1));
        reset();
        assert!(snapshot().is_empty());
    }

    #[test]
    fn flush_to_disk_accumulates_across_flushes() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset();
        let tmp = tempfile::tempdir().expect("tempdir");

        record(AISOperationType::Print);
        let path1 = flush_to_dir(tmp.path()).expect("first flush");
        let after_first = load_cumulative(&path1);
        assert_eq!(after_first.get("PRINT"), Some(&1));

        record(AISOperationType::Print);
        record(AISOperationType::Print);
        let path2 = flush_to_dir(tmp.path()).expect("second flush");
        let after_second = load_cumulative(&path2);
        assert_eq!(after_second.get("PRINT"), Some(&3));
    }
}
