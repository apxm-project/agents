//! Session management (list, inspect, diff, clean).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use apxm_core::paths::ApxmPaths;

use super::cli::*;
use super::implementations::parse_duration;

pub fn session_command(action: SessionAction, json: bool) -> Result<()> {
    match action {
        SessionAction::List {
            status,
            limit,
            session_root,
        } => session_list_command(status, limit, session_root, json),
        SessionAction::Inspect { session } => session_inspect_command(session, json),
        SessionAction::Diff { session1, session2 } => {
            session_diff_command(session1, session2, json)
        }
        SessionAction::Clean {
            older_than,
            all,
            dry_run,
            session_root,
        } => session_clean_command(older_than, all, dry_run, session_root),
    }
}

fn get_sessions_dir(explicit_root: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit_root {
        return Ok(path);
    }
    let paths = ApxmPaths::discover().context("Failed to discover APXM paths")?;
    // Honor the precedence apxm-core defines: a project-local `.apxm/sessions`
    // takes precedence over the state root. Fall back to the state root (which
    // may not exist yet) so the empty case still reports cleanly.
    Ok(paths
        .session_lookup_dirs()
        .into_iter()
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| paths.sessions_dir_for_read()))
}

pub fn session_list_command(
    status_filter: Option<String>,
    limit: usize,
    session_root: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    use apxm_core::constants;
    use apxm_core::types::SessionManifest;

    let sessions_dir = get_sessions_dir(session_root)?;
    if !sessions_dir.exists() {
        if json {
            println!("{{\"sessions\":[]}}");
        } else {
            println!("No sessions found");
        }
        return Ok(());
    }

    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(&sessions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join(constants::session::files::MANIFEST);
        if let Ok(text) = std::fs::read_to_string(&manifest_path) {
            if let Ok(manifest) = serde_json::from_str::<SessionManifest>(&text) {
                if let Some(ref filter) = status_filter {
                    if manifest.status.as_str() != filter.as_str() {
                        continue;
                    }
                }
                let size = dir_size(&path)?;
                sessions.push((manifest, path, size));
            }
        }
    }

    sessions.sort_by(|a, b| b.0.timestamp.cmp(&a.0.timestamp));
    let sessions: Vec<_> = sessions.into_iter().take(limit).collect();

    if json {
        let output: Vec<_> = sessions
            .iter()
            .map(|(m, p, s)| {
                serde_json::json!({
                    "execution_id": m.execution_id,
                    "workflow_name": m.workflow_name,
                    "timestamp": m.timestamp,
                    "status": m.status,
                    "duration_ms": m.duration_ms,
                    "node_count": m.node_count,
                    "success": m.success,
                    "path": p.display().to_string(),
                    "size_bytes": s,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        if sessions.is_empty() {
            println!("No sessions found");
            return Ok(());
        }
        println!("Sessions (most recent first):");
        println!();
        for (manifest, path, size) in &sessions {
            let duration_secs = manifest.duration_ms as f64 / 1000.0;
            let size_mb = *size as f64 / 1_000_000.0;
            let status_icon = match manifest.status {
                apxm_core::types::SessionStatus::Completed if manifest.success => {
                    constants::ui::icons::SUCCESS
                }
                apxm_core::types::SessionStatus::Failed => constants::ui::icons::FAILED,
                _ => constants::ui::icons::INFO,
            };
            println!(
                "{} {} | {} | {:.1}s | {} nodes | {:.1} MB",
                status_icon,
                manifest.execution_id,
                manifest.timestamp,
                duration_secs,
                manifest.node_count,
                size_mb
            );
            if let Some(ref name) = manifest.workflow_name {
                println!("   Workflow: {}", name);
            }
            println!("   Path: {}", path.display());
            println!();
        }
    }
    Ok(())
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0;
    for entry in walkdir::WalkDir::new(path) {
        let entry = entry?;
        if entry.file_type().is_file() {
            total += entry.metadata()?.len();
        }
    }
    Ok(total)
}

pub fn session_inspect_command(session_id: String, json: bool) -> Result<()> {
    use apxm_core::constants;
    use apxm_core::types::SessionManifest;

    let session_path = resolve_session_path(&session_id)?;
    let manifest_path = session_path.join(constants::session::files::MANIFEST);
    let manifest_text = std::fs::read_to_string(&manifest_path)?;
    let manifest: SessionManifest = serde_json::from_str(&manifest_text)?;

    let nodes_dir = session_path.join(constants::session::files::NODES_DIR);
    let mut node_info = Vec::new();
    let live = read_json_if_present(&session_path.join(constants::session::files::LIVE))?;
    let results = read_json_if_present(&session_path.join(constants::session::files::RESULTS))?;
    let metrics = read_json_if_present(&session_path.join(constants::session::files::METRICS))?;

    if nodes_dir.exists() {
        for entry in std::fs::read_dir(&nodes_dir)? {
            let entry = entry?;
            let path = entry.path();
            let node_json_path = path.join(constants::session::node::NODE_JSON);
            if let Ok(text) = std::fs::read_to_string(&node_json_path) {
                if let Ok(info) = serde_json::from_str::<serde_json::Value>(&text) {
                    let status_path = path.join(constants::session::node::STATUS_JSON);
                    let status = if let Ok(s) = std::fs::read_to_string(&status_path) {
                        serde_json::from_str::<serde_json::Value>(&s).ok()
                    } else {
                        None
                    };
                    node_info.push((info, status));
                }
            }
        }
    }

    if json {
        let output = serde_json::json!({
            "manifest": manifest,
            "nodes": node_info,
            "live": live,
            "results": results,
            "metrics": metrics,
            "path": session_path.display().to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Session: {}", manifest.execution_id);
        if let Some(ref name) = manifest.workflow_name {
            println!("Workflow: {}", name);
        }
        println!("Status: {}", manifest.status);
        println!("Duration: {:.2}s", manifest.duration_ms as f64 / 1000.0);
        println!("Nodes: {}", manifest.node_count);
        println!("Path: {}", session_path.display());
        println!();

        if !node_info.is_empty() {
            println!("Node details:");
            for (info, status) in &node_info {
                let id = info.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                let name = info
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let op = info.get("op").and_then(|v| v.as_str()).unwrap_or("unknown");
                println!("  {} | {} | {}", id, name, op);
                if let Some(s) = &status {
                    if let Some(duration) = s.get("duration_ms").and_then(|v| v.as_u64()) {
                        println!("    Duration: {:.2}s", duration as f64 / 1000.0);
                    }
                    if let Some(status_str) = s.get("status").and_then(|v| v.as_str()) {
                        println!("    Status: {}", status_str);
                    }
                }
            }
        }
        if let Some(results) = &results
            && let Some(step_results) = results.get("step_results").and_then(|v| v.as_object())
        {
            println!();
            println!("Workflow steps:");
            for (step_id, step) in step_results {
                let status = step
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let session_dir = step
                    .get("session_dir")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                println!("  {} | {}", step_id, status);
                if !session_dir.is_empty() {
                    println!("    Session: {}", session_dir);
                }
            }
        }
    }
    Ok(())
}

fn read_json_if_present(path: &Path) -> Result<Option<serde_json::Value>> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path)?;
    let value = serde_json::from_str(&text)
        .with_context(|| format!("Failed to parse JSON file {}", path.display()))?;
    Ok(Some(value))
}

fn resolve_session_path(session_id: &str) -> Result<PathBuf> {
    let path = PathBuf::from(session_id);
    if path.exists() && path.is_dir() {
        return Ok(path);
    }

    let paths = ApxmPaths::discover().context("Failed to discover APXM paths")?;
    for sessions_dir in paths.session_lookup_dirs() {
        let session_path = sessions_dir.join(session_id);
        if session_path.exists() && session_path.is_dir() {
            return Ok(session_path);
        }
    }

    Err(anyhow::anyhow!("Session not found: {}", session_id))
}

pub fn session_diff_command(session1_id: String, session2_id: String, json: bool) -> Result<()> {
    use apxm_core::constants;
    use apxm_core::types::SessionManifest;

    let path1 = resolve_session_path(&session1_id)?;
    let path2 = resolve_session_path(&session2_id)?;

    let manifest1: SessionManifest = {
        let text = std::fs::read_to_string(path1.join(constants::session::files::MANIFEST))?;
        serde_json::from_str(&text)?
    };
    let manifest2: SessionManifest = {
        let text = std::fs::read_to_string(path2.join(constants::session::files::MANIFEST))?;
        serde_json::from_str(&text)?
    };

    let results1 = load_session_results(&path1)?;
    let results2 = load_session_results(&path2)?;

    let mut changed_nodes = Vec::new();
    let mut timing_diffs = Vec::new();

    for (node_id, output1) in &results1 {
        if let Some(output2) = results2.get(node_id) {
            if output1 != output2 {
                changed_nodes.push(*node_id);
            }
        }
    }

    let nodes1 = load_node_timings(&path1)?;
    let nodes2 = load_node_timings(&path2)?;
    for (node_id, duration1) in &nodes1 {
        if let Some(duration2) = nodes2.get(node_id) {
            let diff = (*duration2 as i64) - (*duration1 as i64);
            if diff.abs() > 100 {
                timing_diffs.push((*node_id, *duration1, *duration2, diff));
            }
        }
    }

    if json {
        let output = serde_json::json!({
            "session1": {
                "id": manifest1.execution_id,
                "duration_ms": manifest1.duration_ms,
                "status": manifest1.status,
            },
            "session2": {
                "id": manifest2.execution_id,
                "duration_ms": manifest2.duration_ms,
                "status": manifest2.status,
            },
            "changed_nodes": changed_nodes,
            "timing_diffs": timing_diffs.iter().map(|(id, d1, d2, diff)| {
                serde_json::json!({
                    "node_id": id,
                    "duration1_ms": d1,
                    "duration2_ms": d2,
                    "diff_ms": diff,
                })
            }).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Comparing sessions:");
        println!(
            "  Session 1: {} ({:.2}s, {})",
            manifest1.execution_id,
            manifest1.duration_ms as f64 / 1000.0,
            manifest1.status
        );
        println!(
            "  Session 2: {} ({:.2}s, {})",
            manifest2.execution_id,
            manifest2.duration_ms as f64 / 1000.0,
            manifest2.status
        );
        println!();

        let duration_diff = (manifest2.duration_ms as i64) - (manifest1.duration_ms as i64);
        println!(
            "Total duration diff: {:+.2}s ({:+}ms)",
            duration_diff as f64 / 1000.0,
            duration_diff
        );
        println!();

        if !changed_nodes.is_empty() {
            println!("Nodes with changed output ({}):", changed_nodes.len());
            for node_id in &changed_nodes {
                println!("  - Node {}", node_id);
            }
            println!();
        }

        if !timing_diffs.is_empty() {
            println!("Nodes with significant timing changes:");
            for (node_id, d1, d2, diff) in &timing_diffs {
                println!(
                    "  Node {}: {:.2}s → {:.2}s ({:+.2}s)",
                    node_id,
                    *d1 as f64 / 1000.0,
                    *d2 as f64 / 1000.0,
                    *diff as f64 / 1000.0
                );
            }
        }
    }
    Ok(())
}

fn load_session_results(session_path: &Path) -> Result<HashMap<u64, serde_json::Value>> {
    use apxm_core::constants;
    let results_path = session_path.join(constants::session::files::RESULTS);
    if !results_path.exists() {
        return Ok(HashMap::new());
    }
    let text = std::fs::read_to_string(&results_path)?;
    let all: serde_json::Value = serde_json::from_str(&text)?;
    let mut results = HashMap::new();
    if let Some(token_values) = all.get("token_values").and_then(|v| v.as_object()) {
        for (k, v) in token_values {
            if let Ok(id) = k.parse::<u64>() {
                results.insert(id, v.clone());
            }
        }
    }
    Ok(results)
}

fn load_node_timings(session_path: &Path) -> Result<HashMap<u64, u64>> {
    use apxm_core::constants;
    let nodes_dir = session_path.join(constants::session::files::NODES_DIR);
    let mut timings = HashMap::new();
    if !nodes_dir.exists() {
        return Ok(timings);
    }
    for entry in std::fs::read_dir(&nodes_dir)? {
        let entry = entry?;
        let path = entry.path();
        let status_path = path.join(constants::session::node::STATUS_JSON);
        if let Ok(text) = std::fs::read_to_string(&status_path) {
            if let Ok(status) = serde_json::from_str::<serde_json::Value>(&text) {
                if let (Some(node_json), Some(duration)) = (
                    std::fs::read_to_string(path.join(constants::session::node::NODE_JSON)).ok(),
                    status.get("duration_ms").and_then(|v| v.as_u64()),
                ) {
                    if let Ok(node_info) = serde_json::from_str::<serde_json::Value>(&node_json) {
                        if let Some(id) = node_info.get("id").and_then(|v| v.as_u64()) {
                            timings.insert(id, duration);
                        }
                    }
                }
            }
        }
    }
    Ok(timings)
}

pub fn session_clean_command(
    older_than: Option<String>,
    all: bool,
    dry_run: bool,
    session_root: Option<PathBuf>,
) -> Result<()> {
    let sessions_dir = get_sessions_dir(session_root)?;
    if !sessions_dir.exists() {
        println!("No sessions directory found");
        return Ok(());
    }

    let cutoff = if all {
        None
    } else if let Some(ref duration_str) = older_than {
        Some(parse_duration(duration_str)?)
    } else {
        return Err(anyhow::anyhow!(
            "Must specify --older-than <duration> or --all"
        ));
    };

    let mut to_delete = Vec::new();
    let now = chrono::Utc::now();

    for entry in std::fs::read_dir(&sessions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        use apxm_core::constants;
        use apxm_core::types::SessionManifest;
        let manifest_path = path.join(constants::session::files::MANIFEST);
        if let Ok(text) = std::fs::read_to_string(&manifest_path) {
            if let Ok(manifest) = serde_json::from_str::<SessionManifest>(&text) {
                if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(&manifest.timestamp) {
                    let age = now.signed_duration_since(timestamp.with_timezone(&chrono::Utc));
                    if all || (cutoff.is_some() && age > cutoff.unwrap()) {
                        to_delete.push((path, manifest.execution_id.clone(), age));
                    }
                }
            }
        }
    }

    if to_delete.is_empty() {
        println!("No sessions to delete");
        return Ok(());
    }

    to_delete.sort_by_key(|(_, _, age)| *age);

    if dry_run {
        println!("Would delete {} sessions:", to_delete.len());
        for (_path, id, age) in &to_delete {
            println!("  {} (age: {}d)", id, age.num_days());
        }
    } else {
        println!("Deleting {} sessions...", to_delete.len());
        for (path, id, age) in &to_delete {
            println!("  {} (age: {}d)", id, age.num_days());
            std::fs::remove_dir_all(path)?;
        }
        println!("Done");
    }
    Ok(())
}
