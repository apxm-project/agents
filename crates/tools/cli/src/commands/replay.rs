//! Replay a session as a timeline.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

#[cfg(feature = "driver")]
use super::compile::air_graph_from_source;

pub fn replay_command(session: PathBuf) -> Result<()> {
    use apxm_core::constants;
    use apxm_core::types::SessionManifest;

    // Read manifest
    let manifest_path = session.join(constants::session::files::MANIFEST);
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let manifest: SessionManifest =
        serde_json::from_str(&manifest_text).context("Failed to parse manifest")?;

    let duration_secs = manifest.duration_ms as f64 / 1000.0;
    let status_str = if manifest.success {
        "success"
    } else {
        "failed"
    };

    println!(
        "Session: {} ({} nodes, {:.1}s, {})",
        manifest.execution_id, manifest.node_count, duration_secs, status_str
    );
    println!();

    // Read trace (open directly, handle missing file)
    let trace_path = session.join(constants::session::files::TRACE);
    let trace_file = match std::fs::File::open(&trace_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("  (no trace file found)");
            return Ok(());
        }
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to open {}: {e}",
                trace_path.display()
            ));
        }
    };

    // Read node names from the input graph (best-effort)
    let input_path = session.join(constants::session::files::INPUT_GRAPH);
    let node_names: HashMap<u64, String> = load_session_input_graph(&input_path)
        .map(|graph| graph.nodes.iter().map(|n| (n.id, n.name.clone())).collect())
        .unwrap_or_default();

    // Parse trace events and build timeline
    use apxm_core::events::ApxmEvent;
    use apxm_core::events::payload::{OperationEndPayload, OperationStartPayload};
    use apxm_core::types::operations::AISOperationType;

    enum EventKind {
        Start,
        End { duration_ms: u64, success: bool },
    }

    struct TimelineEntry {
        timestamp_ms: f64,
        node_name: String,
        op_type: AISOperationType,
        kind: EventKind,
    }

    let mut entries: Vec<TimelineEntry> = Vec::new();
    let mut first_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;

    let reader = std::io::BufReader::new(trace_file);

    for line in std::io::BufRead::lines(reader) {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let event: ApxmEvent = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let ts = event.meta.timestamp;
        if first_timestamp.is_none() {
            first_timestamp = Some(ts);
        }
        let elapsed_ms = (ts - first_timestamp.unwrap()).num_milliseconds().max(0) as f64;

        let resolve_name = |node_id: u64| {
            node_names
                .get(&node_id)
                .cloned()
                .unwrap_or_else(|| format!("node_{}", node_id))
        };

        if let Some(p) = event.payload.downcast_ref::<OperationStartPayload>() {
            entries.push(TimelineEntry {
                timestamp_ms: elapsed_ms,
                node_name: resolve_name(p.node_id),
                op_type: p.op_type,
                kind: EventKind::Start,
            });
        } else if let Some(p) = event.payload.downcast_ref::<OperationEndPayload>() {
            entries.push(TimelineEntry {
                timestamp_ms: elapsed_ms,
                node_name: resolve_name(p.node_id),
                op_type: p.op_type,
                kind: EventKind::End {
                    duration_ms: p.duration_ms,
                    success: p.success,
                },
            });
        }
    }

    if entries.is_empty() {
        println!("  (no operation events in trace)");
        return Ok(());
    }

    // Sort by timestamp
    entries.sort_by(|a, b| a.timestamp_ms.partial_cmp(&b.timestamp_ms).unwrap());

    // Print timeline
    for entry in &entries {
        let t = entry.timestamp_ms / 1000.0;
        let (icon, detail) = match &entry.kind {
            EventKind::Start => (
                constants::ui::icons::STARTED,
                constants::ui::labels::STARTED.to_string(),
            ),
            EventKind::End {
                duration_ms,
                success,
            } => {
                let i = if *success {
                    constants::ui::icons::SUCCESS
                } else {
                    constants::ui::icons::FAILED
                };
                (i, format!("{:.1}s", *duration_ms as f64 / 1000.0))
            }
        };
        println!(
            "  {:>5.1}s  {:<20} {:<15} {} {}",
            t, entry.node_name, entry.op_type, icon, detail
        );
    }

    Ok(())
}

fn load_session_input_graph(input_path: &std::path::Path) -> Option<apxm_compiler::AirModule> {
    #[cfg(feature = "driver")]
    {
        air_graph_from_source(input_path).ok()
    }

    #[cfg(not(feature = "driver"))]
    {
        let _ = input_path;
        None
    }
}
