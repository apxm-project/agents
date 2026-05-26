use std::path::Path;
use std::time::Instant;

use crate::execution_index::ExecutionIndex;
use crate::executions::{ExecutionRecord, ExecutionStatus};

const SNAPSHOT_COUNT: usize = 1_000;
const LOOKUP_ITERATIONS: usize = 200;

fn make_record(execution_id: &str, session_dir: &Path) -> ExecutionRecord {
    ExecutionRecord {
        execution_id: execution_id.to_string(),
        skill_id: "bench-skill".to_string(),
        skill_version: "0.0.0".to_string(),
        entry_flow: None,
        source_hash: None,
        air_hash: None,
        artifact_hash: None,
        parent_execution_id: None,
        parent_skill_id: None,
        parent_skill_version: None,
        session_id: "session".to_string(),
        session_dir: session_dir.to_string_lossy().into_owned(),
        status: ExecutionStatus::Succeeded,
        started_at_ms: 1_000,
        completed_at_ms: Some(2_000),
        result: None,
        error: None,
        node_outputs: Vec::new(),
        node_metrics: Vec::new(),
    }
}

fn write_snapshot(record: &ExecutionRecord) {
    let dir = Path::new(&record.session_dir).join("executions");
    std::fs::create_dir_all(&dir).expect("executions dir");
    let path = dir.join(format!("{}.json", record.execution_id));
    let bytes = serde_json::to_vec_pretty(record).expect("serialize record");
    std::fs::write(path, bytes).expect("write snapshot");
}

/// Scan the executions directory and decode every JSON file. This is the
/// pre-index baseline: O(n) work per lookup, with full record deserialization
/// rather than the lightweight metadata stored in the sidecar.
fn directory_scan_lookup(executions_dir: &Path, execution_id: &str) -> Option<ExecutionRecord> {
    let entries = std::fs::read_dir(executions_dir).ok()?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = path.file_name().and_then(|name| name.to_str())?;
        if !file_name.ends_with(".json") {
            continue;
        }
        let bytes = std::fs::read(&path).ok()?;
        let record: ExecutionRecord = match serde_json::from_slice(&bytes) {
            Ok(record) => record,
            Err(_) => continue,
        };
        if record.execution_id == execution_id {
            return Some(record);
        }
    }
    None
}

#[test]
fn execution_index_lookup_beats_directory_scan_for_1000_snapshots() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_dir = temp.path().join("sessions/skills/bench/session-1");
    let executions_dir = session_dir.join("executions");

    let ids: Vec<String> = (0..SNAPSHOT_COUNT)
        .map(|i| format!("exec-{i:04}"))
        .collect();
    for id in &ids {
        let record = make_record(id, &session_dir);
        write_snapshot(&record);
    }

    let index = ExecutionIndex::new();
    let loaded = index.reload_from_session_roots([temp.path().join("sessions")]);
    assert_eq!(loaded, SNAPSHOT_COUNT);

    // Warm both paths once so the kernel cache doesn't favor the second run.
    let _ = directory_scan_lookup(&executions_dir, &ids[SNAPSHOT_COUNT / 2]);
    let _ = index.get(&ids[SNAPSHOT_COUNT / 2]);

    let scan_start = Instant::now();
    for i in 0..LOOKUP_ITERATIONS {
        let id = &ids[i * (SNAPSHOT_COUNT / LOOKUP_ITERATIONS)];
        let hit = directory_scan_lookup(&executions_dir, id);
        assert!(hit.is_some(), "scan lookup must find {id}");
    }
    let scan_elapsed = scan_start.elapsed();

    let index_start = Instant::now();
    for i in 0..LOOKUP_ITERATIONS {
        let id = &ids[i * (SNAPSHOT_COUNT / LOOKUP_ITERATIONS)];
        let hit = index.get(id);
        assert!(hit.is_some(), "indexed lookup must find {id}");
    }
    let index_elapsed = index_start.elapsed();

    let ratio = scan_elapsed.as_secs_f64() / index_elapsed.as_secs_f64().max(1e-9);
    eprintln!(
        "execution_index bench: scan={:?} index={:?} ratio={:.1}x ({} snapshots, {} lookups)",
        scan_elapsed, index_elapsed, ratio, SNAPSHOT_COUNT, LOOKUP_ITERATIONS,
    );

    assert!(
        ratio >= 10.0,
        "indexed lookup should be >=10x faster than directory scan; got scan={scan_elapsed:?}, index={index_elapsed:?} ({ratio:.1}x)",
    );
}
