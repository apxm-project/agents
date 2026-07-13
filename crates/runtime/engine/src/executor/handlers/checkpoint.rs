//! CHECKPOINT operation - durable execution snapshot persistence.
//!
//! Serializes the current execution scope and drained input frontier, persists
//! it through the selected storage contract, and emits a manifest token only
//! after the selected backend acknowledges the payload.

use super::{
    ExecutionContext, Node, Result, Value, get_input, get_optional_string_attribute,
    get_optional_u64_attribute, get_string_attribute,
};
use crate::aam::TransitionLabel;
use crate::memory::MemorySpace;
use crate::scheduler::SchedulerSnapshot;
use crate::scheduler::snapshot::current_checkpoint_scheduler_snapshot;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::types::{OpStatus, values::Number};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

const SCOPE_FULL: &str = "full";
const SCOPE_LOCAL: &str = "local";

const STORAGE_FS: &str = "fs";
const STORAGE_MEMORY: &str = "memory";
const STORAGE_CUSTOM: &str = "custom";

const ON_FAIL_HALT: &str = "halt";
const ON_FAIL_CONTINUE: &str = "continue";

const CHECKPOINT_DIR_ENV: &str = "APXM_CHECKPOINT_DIR";
const CHECKPOINT_MANIFEST_PREFIX: &str = "_checkpoint_manifest:";
const CHECKPOINT_PAYLOAD_PREFIX: &str = "_checkpoint_payload:";

#[derive(Debug, Serialize)]
struct CheckpointSnapshotEnvelope {
    version: u32,
    checkpoint_id: String,
    execution_id: String,
    scope: String,
    scope_id: String,
    timestamp: String,
    ttl_seconds: Option<u64>,
    aam: crate::aam::AamCheckpoint,
    input_frontier: HashMap<String, Value>,
    scheduler: SchedulerSnapshot,
    notes: Vec<String>,
}

#[derive(Debug)]
struct PersistedSnapshot {
    effective_storage: String,
    location: String,
}

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let checkpoint_id = get_string_attribute(node, graph_attrs::CHECKPOINT_ID)?;
    if checkpoint_id.trim().is_empty() {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "Attribute checkpoint_id cannot be empty".to_string(),
        });
    }

    let scope = get_optional_string_attribute(node, graph_attrs::SCOPE)?
        .unwrap_or_else(|| SCOPE_FULL.to_string());
    validate_scope(node, &scope)?;

    let requested_storage = get_optional_string_attribute(node, graph_attrs::STORAGE)?
        .unwrap_or_else(|| STORAGE_FS.to_string());
    validate_storage(node, &requested_storage)?;

    let on_fail = get_optional_string_attribute(node, graph_attrs::ON_FAIL)?
        .unwrap_or_else(|| ON_FAIL_HALT.to_string());
    validate_on_fail(node, &on_fail)?;

    let ttl_seconds = get_optional_u64_attribute(node, graph_attrs::TTL_SECONDS)?;

    // The scheduler invokes handlers only after upstream inputs are available,
    // so consuming the first input establishes the checkpoint barrier.
    let _barrier_input = get_input(node, &inputs, 0)?;
    let scheduler =
        current_checkpoint_scheduler_snapshot().ok_or_else(|| RuntimeError::Operation {
            op_type: node.op_type,
            message: "CHECKPOINT requires scheduler-owned replay frontier evidence".to_string(),
        })?;
    let running_nodes = scheduler
        .ops
        .iter()
        .filter(|operation| operation.node_id != node.id && operation.status == OpStatus::Running)
        .map(|operation| operation.node_id)
        .collect::<Vec<_>>();
    validate_replayable_scheduler_frontier(
        node,
        scheduler.replay_supported,
        &scheduler.replay_notes,
        &running_nodes,
    )?;

    let (captured_scope_id, aam_snapshot, mut notes) = resolve_scope_snapshot(ctx, &scope);
    notes.push(format!(
        "Captured scheduler replay frontier at the checkpoint barrier: {} live edges, {} tokens, and {} node statuses.",
        scheduler.edges.len(),
        scheduler.tokens.len(),
        scheduler.ops.len(),
    ));
    let effective_storage = requested_storage.clone();

    let timestamp = aam_snapshot.timestamp.to_rfc3339();
    let snapshot = CheckpointSnapshotEnvelope {
        version: 2,
        checkpoint_id: checkpoint_id.clone(),
        execution_id: ctx.execution_id.clone(),
        scope: scope.clone(),
        scope_id: captured_scope_id.clone(),
        timestamp: timestamp.clone(),
        ttl_seconds,
        aam: aam_snapshot,
        input_frontier: drained_input_frontier(&inputs),
        scheduler,
        notes: notes.clone(),
    };

    let payload_json = match serde_json::to_value(&snapshot).map_err(|e| RuntimeError::Operation {
        op_type: node.op_type,
        message: format!("Failed to serialize CHECKPOINT snapshot: {}", e),
    }) {
        Ok(payload_json) => payload_json,
        Err(err) => {
            return handle_checkpoint_failure(
                ctx,
                node,
                &checkpoint_id,
                &scope,
                &requested_storage,
                &effective_storage,
                ttl_seconds,
                &on_fail,
                &timestamp,
                0,
                &notes,
                err.to_string(),
            )
            .await;
        }
    };

    let payload_bytes =
        match serde_json::to_vec_pretty(&payload_json).map_err(|e| RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Failed to encode CHECKPOINT payload: {}", e),
        }) {
            Ok(payload_bytes) => payload_bytes,
            Err(err) => {
                return handle_checkpoint_failure(
                    ctx,
                    node,
                    &checkpoint_id,
                    &scope,
                    &requested_storage,
                    &effective_storage,
                    ttl_seconds,
                    &on_fail,
                    &timestamp,
                    0,
                    &notes,
                    err.to_string(),
                )
                .await;
            }
        };

    let byte_size = payload_bytes.len();
    let persisted = match persist_snapshot(
        ctx,
        node,
        &checkpoint_id,
        &captured_scope_id,
        &effective_storage,
        &payload_json,
        &payload_bytes,
    )
    .await
    {
        Ok(persisted) => persisted,
        Err(err) => {
            return handle_checkpoint_failure(
                ctx,
                node,
                &checkpoint_id,
                &scope,
                &requested_storage,
                &effective_storage,
                ttl_seconds,
                &on_fail,
                &timestamp,
                byte_size,
                &notes,
                err.to_string(),
            )
            .await;
        }
    };

    let manifest = checkpoint_manifest(
        &checkpoint_id,
        &scope,
        &requested_storage,
        &persisted.effective_storage,
        ttl_seconds,
        &on_fail,
        &timestamp,
        byte_size,
        true,
        Some(&persisted.location),
        &notes,
        None,
    );

    record_manifest(ctx, &checkpoint_id, &manifest).await;

    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_checkpoint_saved(&checkpoint_id);
    }

    tracing::info!(
        execution_id = %ctx.execution_id,
        checkpoint_id = %checkpoint_id,
        scope = %scope,
        requested_storage = %requested_storage,
        effective_storage = %persisted.effective_storage,
        location = %persisted.location,
        byte_size,
        "CHECKPOINT snapshot persisted"
    );

    Ok(Value::Object(manifest))
}

async fn handle_checkpoint_failure(
    ctx: &ExecutionContext,
    node: &Node,
    checkpoint_id: &str,
    scope: &str,
    requested_storage: &str,
    effective_storage: &str,
    ttl_seconds: Option<u64>,
    on_fail: &str,
    timestamp: &str,
    byte_size: usize,
    notes: &[String],
    error_message: String,
) -> Result<Value> {
    if on_fail == ON_FAIL_CONTINUE {
        let mut failure_notes = notes.to_vec();
        failure_notes.push(format!(
            "Checkpoint persistence failed and execution continued: {}",
            error_message
        ));

        let manifest = checkpoint_manifest(
            checkpoint_id,
            scope,
            requested_storage,
            effective_storage,
            ttl_seconds,
            on_fail,
            timestamp,
            byte_size,
            false,
            None,
            &failure_notes,
            Some(&error_message),
        );
        record_manifest(ctx, checkpoint_id, &manifest).await;

        tracing::warn!(
            execution_id = %ctx.execution_id,
            checkpoint_id = %checkpoint_id,
            scope = %scope,
            requested_storage = %requested_storage,
            effective_storage = %effective_storage,
            error = %error_message,
            "CHECKPOINT failed; continuing because on_fail=continue"
        );

        return Ok(Value::Object(manifest));
    }

    Err(RuntimeError::Operation {
        op_type: node.op_type,
        message: error_message,
    })
}

async fn persist_snapshot(
    ctx: &ExecutionContext,
    node: &Node,
    checkpoint_id: &str,
    scope_id: &str,
    effective_storage: &str,
    payload_json: &serde_json::Value,
    payload_bytes: &[u8],
) -> Result<PersistedSnapshot> {
    match effective_storage {
        STORAGE_FS => {
            let path = persist_snapshot_to_fs(checkpoint_id, payload_bytes).map_err(|e| {
                RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!("Failed to write CHECKPOINT snapshot to filesystem: {}", e),
                }
            })?;

            Ok(PersistedSnapshot {
                effective_storage: STORAGE_FS.to_string(),
                location: path.display().to_string(),
            })
        }
        STORAGE_MEMORY => {
            let key = memory_payload_key(checkpoint_id);
            let payload_value =
                Value::try_from(payload_json.clone()).map_err(|e| RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!("Failed to encode CHECKPOINT payload for memory: {}", e),
                })?;

            ctx.memory
                .write_scoped(MemorySpace::Stm, scope_id, key.clone(), payload_value)
                .await
                .map_err(|e| RuntimeError::Operation {
                    op_type: node.op_type,
                    message: format!("Failed to persist CHECKPOINT payload in memory: {}", e),
                })?;

            Ok(PersistedSnapshot {
                effective_storage: STORAGE_MEMORY.to_string(),
                location: key,
            })
        }
        _ => unreachable!("validated storage backend"),
    }
}

async fn record_manifest(
    ctx: &ExecutionContext,
    checkpoint_id: &str,
    manifest: &HashMap<String, Value>,
) {
    let manifest_value = Value::Object(manifest.clone());
    let manifest_key = manifest_key(checkpoint_id);

    ctx.aam.set_belief(
        manifest_key.clone(),
        manifest_value.clone(),
        TransitionLabel::Custom(format!("checkpoint:{}", checkpoint_id)),
    );

    let _ = ctx
        .memory
        .write_scoped(
            MemorySpace::Stm,
            ctx.scope_id(),
            manifest_key,
            manifest_value,
        )
        .await;
}

fn resolve_scope_snapshot(
    ctx: &ExecutionContext,
    scope: &str,
) -> (String, crate::aam::AamCheckpoint, Vec<String>) {
    let mut notes = Vec::new();

    match scope {
        SCOPE_FULL => {
            let root_scope_id = root_scope_id(ctx);
            if let Some(entry) = ctx.scope_registry.get(&root_scope_id) {
                return (root_scope_id, entry.aam.checkpoint(), notes);
            }

            notes.push(
                "Root scope entry was unavailable; falling back to the current execution scope."
                    .to_string(),
            );
            (ctx.scope_id().to_string(), ctx.aam.checkpoint(), notes)
        }
        SCOPE_LOCAL => {
            if let Some(entry) = ctx.scope_registry.get(ctx.scope_id()) {
                return (entry.scope_id, entry.aam.checkpoint(), notes);
            }

            notes.push(
                "Current scope entry was unavailable; falling back to the handler-local AAM snapshot."
                    .to_string(),
            );
            (ctx.scope_id().to_string(), ctx.aam.checkpoint(), notes)
        }
        _ => unreachable!("validated checkpoint scope"),
    }
}

fn root_scope_id(ctx: &ExecutionContext) -> String {
    let mut current_scope_id = ctx.scope_id().to_string();

    loop {
        let Some(entry) = ctx.scope_registry.get(&current_scope_id) else {
            return current_scope_id;
        };

        match entry.parent_id {
            Some(parent_id) => current_scope_id = parent_id,
            None => return current_scope_id,
        }
    }
}

fn drained_input_frontier(inputs: &[Value]) -> HashMap<String, Value> {
    inputs
        .iter()
        .enumerate()
        .map(|(index, value)| (format!("input_{}", index), value.clone()))
        .collect()
}

fn validate_scope(node: &Node, scope: &str) -> Result<()> {
    if matches!(scope, SCOPE_FULL | SCOPE_LOCAL) {
        return Ok(());
    }

    Err(RuntimeError::Operation {
        op_type: node.op_type,
        message: format!(
            "Unsupported checkpoint scope '{}'; expected '{}' or '{}'",
            scope, SCOPE_FULL, SCOPE_LOCAL
        ),
    })
}

fn validate_storage(node: &Node, storage: &str) -> Result<()> {
    if matches!(storage, STORAGE_FS | STORAGE_MEMORY) {
        return Ok(());
    }

    if storage == STORAGE_CUSTOM {
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: "Checkpoint storage 'custom' is unavailable because no typed persistence backend is registered"
                .to_string(),
        });
    }

    Err(RuntimeError::Operation {
        op_type: node.op_type,
        message: format!(
            "Unsupported checkpoint storage '{}'; expected '{}' or '{}'",
            storage, STORAGE_FS, STORAGE_MEMORY
        ),
    })
}

fn validate_on_fail(node: &Node, on_fail: &str) -> Result<()> {
    if matches!(on_fail, ON_FAIL_HALT | ON_FAIL_CONTINUE) {
        return Ok(());
    }

    Err(RuntimeError::Operation {
        op_type: node.op_type,
        message: format!(
            "Unsupported checkpoint on_fail '{}'; expected '{}' or '{}'",
            on_fail, ON_FAIL_HALT, ON_FAIL_CONTINUE
        ),
    })
}

/// Require the captured frontier to authorize replay with no concurrent operations.
fn validate_replayable_scheduler_frontier(
    node: &Node,
    replay_supported: bool,
    replay_notes: &[String],
    running_nodes: &[u64],
) -> Result<()> {
    if !replay_supported {
        let reason = if replay_notes.is_empty() {
            "the scheduler did not provide replay validation details".to_string()
        } else {
            replay_notes.join("; ")
        };
        return Err(RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("CHECKPOINT rejects a non-replayable scheduler frontier: {reason}"),
        });
    }

    if running_nodes.is_empty() {
        return Ok(());
    }

    Err(RuntimeError::Operation {
        op_type: node.op_type,
        message: format!(
            "CHECKPOINT requires a quiescent scheduler frontier; node(s) {} are still running",
            running_nodes
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    })
}

fn persist_snapshot_to_fs(checkpoint_id: &str, payload: &[u8]) -> std::io::Result<PathBuf> {
    let checkpoint_dir = checkpoint_root_dir()?;
    persist_snapshot_to_dir(&checkpoint_dir, checkpoint_id, payload)
}

/// Persist a snapshot atomically and synchronously within a configured root.
fn persist_snapshot_to_dir(
    checkpoint_dir: &Path,
    checkpoint_id: &str,
    payload: &[u8],
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(checkpoint_dir)?;

    let file_name = format!("{}.json", sanitize_filename_component(checkpoint_id));
    let path = checkpoint_dir.join(file_name);
    let temporary_path = checkpoint_dir.join(format!(
        ".{}.{}.tmp",
        sanitize_filename_component(checkpoint_id),
        uuid::Uuid::now_v7()
    ));
    let write_result: std::io::Result<()> = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)?;
        file.write_all(payload)?;
        file.sync_all()?;
        std::fs::rename(&temporary_path, &path)?;
        std::fs::File::open(checkpoint_dir)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    write_result?;
    Ok(path)
}

/// Resolve the explicitly configured filesystem checkpoint root.
fn checkpoint_root_dir() -> std::io::Result<PathBuf> {
    checkpoint_root_dir_from(std::env::var_os(CHECKPOINT_DIR_ENV))
}

fn checkpoint_root_dir_from(
    configured_root: Option<std::ffi::OsString>,
) -> std::io::Result<PathBuf> {
    configured_root
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "{} must be configured for filesystem checkpoints",
                    CHECKPOINT_DIR_ENV
                ),
            )
        })
}

fn sanitize_filename_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();

    if sanitized.is_empty() {
        "checkpoint".to_string()
    } else {
        sanitized
    }
}

fn manifest_key(checkpoint_id: &str) -> String {
    format!("{}{}", CHECKPOINT_MANIFEST_PREFIX, checkpoint_id)
}

fn memory_payload_key(checkpoint_id: &str) -> String {
    format!("{}{}", CHECKPOINT_PAYLOAD_PREFIX, checkpoint_id)
}

#[allow(clippy::too_many_arguments)]
fn checkpoint_manifest(
    checkpoint_id: &str,
    scope: &str,
    requested_storage: &str,
    effective_storage: &str,
    ttl_seconds: Option<u64>,
    on_fail: &str,
    timestamp: &str,
    byte_size: usize,
    saved: bool,
    location: Option<&str>,
    notes: &[String],
    error: Option<&str>,
) -> HashMap<String, Value> {
    let mut manifest = HashMap::new();
    manifest.insert("id".to_string(), Value::String(checkpoint_id.to_string()));
    manifest.insert(
        graph_attrs::CHECKPOINT_ID.to_string(),
        Value::String(checkpoint_id.to_string()),
    );
    manifest.insert(
        "timestamp".to_string(),
        Value::String(timestamp.to_string()),
    );
    manifest.insert(
        "byte_size".to_string(),
        unsigned_number_value(byte_size as u64),
    );
    manifest.insert(
        graph_attrs::SCOPE.to_string(),
        Value::String(scope.to_string()),
    );
    manifest.insert(
        graph_attrs::STORAGE.to_string(),
        Value::String(effective_storage.to_string()),
    );
    manifest.insert(
        "requested_storage".to_string(),
        Value::String(requested_storage.to_string()),
    );
    manifest.insert(
        graph_attrs::TTL_SECONDS.to_string(),
        optional_unsigned_number_value(ttl_seconds),
    );
    manifest.insert(
        graph_attrs::ON_FAIL.to_string(),
        Value::String(on_fail.to_string()),
    );
    manifest.insert("saved".to_string(), Value::Bool(saved));
    manifest.insert(
        "location".to_string(),
        location
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    manifest.insert(
        "error".to_string(),
        error
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    manifest.insert(
        "notes".to_string(),
        Value::Array(notes.iter().cloned().map(Value::String).collect()),
    );
    manifest
}

fn optional_unsigned_number_value(value: Option<u64>) -> Value {
    value.map(unsigned_number_value).unwrap_or(Value::Null)
}

fn unsigned_number_value(value: u64) -> Value {
    match i64::try_from(value) {
        Ok(value) => Value::Number(Number::Integer(value)),
        Err(_) => Value::Number(Number::Float(value as f64)),
    }
}

#[cfg(test)]
mod tests {
    //! Checkpoint persistence tests cover explicit storage and durable writes.

    use super::*;
    use apxm_core::types::operations::AISOperationType;

    #[test]
    fn rejects_unregistered_custom_storage() {
        let node = Node::new(1, AISOperationType::Checkpoint);
        let error = validate_storage(&node, STORAGE_CUSTOM).expect_err("custom storage rejects");

        assert!(
            error
                .to_string()
                .contains("no typed persistence backend is registered")
        );
    }

    #[test]
    fn filesystem_checkpoint_requires_an_explicit_root() {
        let error = checkpoint_root_dir_from(None).expect_err("missing root rejects");

        assert!(
            error
                .to_string()
                .contains("APXM_CHECKPOINT_DIR must be configured")
        );
    }

    #[test]
    fn filesystem_checkpoint_uses_the_configured_root() {
        let root = PathBuf::from("fixture-checkpoints");

        assert_eq!(
            checkpoint_root_dir_from(Some(root.as_os_str().to_os_string()))
                .expect("configured root"),
            root
        );
    }

    #[test]
    fn checkpoint_rejects_a_non_replayable_scheduler_frontier() {
        let node = Node::new(1, AISOperationType::Checkpoint);
        let error = validate_replayable_scheduler_frontier(
            &node,
            false,
            &["completed capability effect is not replay-verifiable".to_string()],
            &[],
        )
        .expect_err("non-replayable frontiers reject");

        assert!(
            error
                .to_string()
                .contains("completed capability effect is not replay-verifiable")
        );
    }

    #[test]
    fn checkpoint_rejects_an_in_flight_scheduler_frontier() {
        let node = Node::new(1, AISOperationType::Checkpoint);
        let error = validate_replayable_scheduler_frontier(&node, true, &[], &[2, 3])
            .expect_err("in-flight work rejects");

        assert!(
            error.to_string().contains(
                "requires a quiescent scheduler frontier; node(s) 2, 3 are still running"
            )
        );
    }

    #[test]
    fn persists_a_complete_payload_at_the_configured_root() {
        let root = std::env::temp_dir().join(format!("apxm-checkpoint-{}", uuid::Uuid::now_v7()));
        let payload = b"checkpoint payload";
        let path = persist_snapshot_to_dir(&root, "checkpoint", payload)
            .expect("configured checkpoint write");

        assert_eq!(std::fs::read(path).expect("checkpoint payload"), payload);
        std::fs::remove_dir_all(root).expect("checkpoint cleanup");
    }

    #[test]
    fn persists_scheduler_frontier_payload_at_the_configured_root() {
        let root = std::env::temp_dir().join(format!("apxm-checkpoint-{}", uuid::Uuid::now_v7()));
        let payload = serde_json::json!({
            "version": 2,
            "scheduler": {
                "edges": [{
                    "from_node_id": 1,
                    "to_node_id": 2,
                    "token_id": 10,
                    "dependency_type": "Data"
                }],
                "tokens": [{
                    "token_id": 10,
                    "ready": true,
                    "value": "ready-value",
                    "consumers": [2]
                }],
                "ops": [{
                    "node_id": 2,
                    "status": "Running"
                }]
            }
        });
        let path = persist_snapshot_to_dir(
            &root,
            "checkpoint-frontier",
            &serde_json::to_vec(&payload).expect("checkpoint payload encodes"),
        )
        .expect("configured checkpoint write");

        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).expect("checkpoint payload"))
                .expect("checkpoint payload decodes");
        assert_eq!(
            persisted["scheduler"]["edges"][0]["token_id"],
            serde_json::json!(10)
        );
        assert_eq!(
            persisted["scheduler"]["tokens"][0]["ready"],
            serde_json::json!(true)
        );
        assert_eq!(
            persisted["scheduler"]["ops"][0]["status"],
            serde_json::json!("Running")
        );
        std::fs::remove_dir_all(root).expect("checkpoint cleanup");
    }
}
