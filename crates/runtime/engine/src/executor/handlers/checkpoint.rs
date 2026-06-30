//! CHECKPOINT operation - Durable execution snapshot stub
//!
//! Serializes a checkpoint payload for the current execution scope and
//! continues immediately by emitting a manifest token.
//!
//! This stub snapshots AAM state plus the drained input frontier. Full
//! scheduler frontier and node-status capture will be added once that state is
//! exposed to handlers.

use super::{
    ExecutionContext, Node, Result, Value, get_input, get_optional_string_attribute,
    get_optional_u64_attribute, get_string_attribute,
};
use crate::aam::TransitionLabel;
use crate::memory::MemorySpace;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::types::values::Number;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

const SCOPE_FULL: &str = "full";
const SCOPE_LOCAL: &str = "local";

const STORAGE_FS: &str = "fs";
const STORAGE_MEMORY: &str = "memory";
const STORAGE_CUSTOM: &str = "custom";

const ON_FAIL_HALT: &str = "halt";
const ON_FAIL_CONTINUE: &str = "continue";

const CHECKPOINT_DIR_ENV: &str = "APXM_CHECKPOINT_DIR";
const DEFAULT_CHECKPOINT_DIR: &str = "apxm-checkpoints";
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
    token_map: HashMap<String, Value>,
    node_status_vector: Vec<String>,
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

    // The scheduler only invokes handlers once upstream inputs are available,
    // so consuming the first input gives this stub barrier semantics.
    let _barrier_input = get_input(node, &inputs, 0)?;

    let (captured_scope_id, aam_snapshot, mut notes) = resolve_scope_snapshot(ctx, &scope);
    notes.push(
        "Stub captures the drained input frontier only; full live-edge token capture requires scheduler integration."
            .to_string(),
    );
    notes.push(
        "Scheduler node-status vector is not exposed to handlers yet; the snapshot stores an empty placeholder."
            .to_string(),
    );

    let effective_storage = if requested_storage == STORAGE_CUSTOM {
        notes.push(
            "storage=\"custom\" is not wired to a host backend yet; falling back to filesystem persistence."
                .to_string(),
        );
        STORAGE_FS.to_string()
    } else {
        requested_storage.clone()
    };

    let timestamp = aam_snapshot.timestamp.to_rfc3339();
    let snapshot = CheckpointSnapshotEnvelope {
        version: 1,
        checkpoint_id: checkpoint_id.clone(),
        execution_id: ctx.execution_id.clone(),
        scope: scope.clone(),
        scope_id: captured_scope_id.clone(),
        timestamp: timestamp.clone(),
        ttl_seconds,
        aam: aam_snapshot,
        token_map: drained_token_map(&inputs),
        node_status_vector: Vec::new(),
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
        "CHECKPOINT snapshot persisted (stub)"
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

fn drained_token_map(inputs: &[Value]) -> HashMap<String, Value> {
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
    if matches!(storage, STORAGE_FS | STORAGE_MEMORY | STORAGE_CUSTOM) {
        return Ok(());
    }

    Err(RuntimeError::Operation {
        op_type: node.op_type,
        message: format!(
            "Unsupported checkpoint storage '{}'; expected '{}', '{}', or '{}'",
            storage, STORAGE_FS, STORAGE_MEMORY, STORAGE_CUSTOM
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

fn persist_snapshot_to_fs(checkpoint_id: &str, payload: &[u8]) -> std::io::Result<PathBuf> {
    let checkpoint_dir = checkpoint_root_dir();
    std::fs::create_dir_all(&checkpoint_dir)?;

    let file_name = format!("{}.json", sanitize_filename_component(checkpoint_id));
    let path = checkpoint_dir.join(file_name);
    std::fs::write(&path, payload)?;
    Ok(path)
}

fn checkpoint_root_dir() -> PathBuf {
    std::env::var_os(CHECKPOINT_DIR_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(DEFAULT_CHECKPOINT_DIR))
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
    manifest.insert("stub".to_string(), Value::Bool(true));
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
