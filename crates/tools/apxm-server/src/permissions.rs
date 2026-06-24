//! Server-driven permission prompts.
//!
//! Emits permission events on the run stream, blocks tool execution until the
//! client responds on `POST /v1/permissions/{id}/respond`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use apxm_core::events::payload::{ApprovalRequestPayload, ApprovalResolvedPayload};
use apxm_core::events::{ApxmEvent, EventEmitter, EventSource};
use apxm_core::types::values::Value;
use axum::Json;
use axum::extract::Path;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify};
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::helpers::now_ms;
use crate::types::responses::OkAck;

/// Default wait bound when the client does not answer a permission prompt.
pub const DEFAULT_PERMISSION_TIMEOUT: Duration = Duration::from_secs(120);

/// Client reply to a server permission prompt (OpenAPI `PermissionResponse`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PermissionResponse {
    pub decision: PermissionDecision,
}

/// Wire decision enum for permission responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Approve,
    Deny,
}

/// Outcome of a resolved permission wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionOutcome {
    Approved,
    Denied,
    TimedOut,
}

/// Permission event surfaced on the run/event stream (data-model § Permission Event).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionEvent {
    pub permission_id: String,
    pub execution_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub capability_id: String,
    pub args_summary: String,
    pub timeout_at_ms: u64,
}

#[derive(Debug)]
struct PendingPermission {
    event: PermissionEvent,
    resolved: Arc<Notify>,
    outcome: Arc<Mutex<Option<PermissionOutcome>>>,
}

/// Registry of in-flight permission prompts and their resolution channels.
#[derive(Debug, Clone, Default)]
pub struct PermissionRegistry {
    pending: Arc<DashMap<String, PendingPermission>>,
    timeout: Duration,
    next_id: Arc<AtomicU64>,
}

impl PermissionRegistry {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(DashMap::new()),
            timeout: DEFAULT_PERMISSION_TIMEOUT,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            ..Self::new()
        }
    }

    fn mint_permission_id(&self) -> String {
        let n = self.next_id.fetch_add(1, Ordering::Relaxed);
        format!("perm-{n:016x}")
    }

    /// Human-visible digest of invocation args (keys only, redaction-safe).
    pub fn summarize_args(args: &HashMap<String, Value>) -> String {
        let mut keys: Vec<_> = args.keys().collect();
        keys.sort();
        if keys.is_empty() {
            return "(no args)".to_string();
        }
        keys.into_iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Emit a permission event and block until the client responds or times out.
    ///
    pub async fn block_for_permission(
        &self,
        execution_id: &str,
        session_id: Option<&str>,
        capability_id: &str,
        args: &HashMap<String, Value>,
        emitter: &dyn EventEmitter,
    ) -> PermissionOutcome {
        let permission_id = self.mint_permission_id();
        let timeout_at_ms = now_ms().saturating_add(self.timeout.as_millis() as u64);
        let event = PermissionEvent {
            permission_id: permission_id.clone(),
            execution_id: execution_id.to_string(),
            session_id: session_id.map(str::to_string),
            capability_id: capability_id.to_string(),
            args_summary: Self::summarize_args(args),
            timeout_at_ms,
        };

        emit_permission_request(emitter, execution_id, session_id, &event);

        let resolved = Arc::new(Notify::new());
        let outcome = Arc::new(Mutex::new(None));
        self.pending.insert(
            permission_id.clone(),
            PendingPermission {
                event: event.clone(),
                resolved: resolved.clone(),
                outcome: outcome.clone(),
            },
        );

        let wait = async {
            resolved.notified().await;
            outcome
                .lock()
                .await
                .clone()
                .unwrap_or(PermissionOutcome::Denied)
        };

        let result = match tokio::time::timeout(self.timeout, wait).await {
            Ok(outcome) => outcome,
            Err(_) => {
                self.resolve(
                    &permission_id,
                    PermissionDecision::Deny,
                    true,
                    Some(emitter),
                    execution_id,
                    "expired",
                );
                PermissionOutcome::TimedOut
            }
        };

        self.pending.remove(&permission_id);
        result
    }

    /// Record a client reply. Idempotent: concurrent duplicate responses return OK.
    pub(crate) fn respond(
        &self,
        permission_id: &str,
        response: PermissionResponse,
        emitter: Option<&dyn EventEmitter>,
    ) -> Result<PermissionOutcome, ApiError> {
        let Some(entry) = self.pending.get(permission_id) else {
            return Err(ApiError::not_found(format!(
                "unknown or already resolved permission '{permission_id}'"
            )));
        };

        if let Ok(guard) = entry.outcome.try_lock()
            && let Some(existing) = guard.clone()
        {
            return Ok(existing);
        }

        let execution_id = entry.event.execution_id.clone();
        let decision_label = match response.decision {
            PermissionDecision::Approve => "approved",
            PermissionDecision::Deny => "denied",
        };

        let outcome = self.resolve(
            permission_id,
            response.decision,
            true,
            emitter,
            &execution_id,
            decision_label,
        );

        Ok(outcome)
    }

    fn resolve(
        &self,
        permission_id: &str,
        decision: PermissionDecision,
        notify_waiter: bool,
        emitter: Option<&dyn EventEmitter>,
        execution_id: &str,
        decision_label: &str,
    ) -> PermissionOutcome {
        let Some(entry) = self.pending.get(permission_id) else {
            return PermissionOutcome::Denied;
        };

        if let Ok(guard) = entry.outcome.try_lock()
            && guard.is_some()
        {
            return guard.clone().unwrap_or(PermissionOutcome::Denied);
        }

        let outcome = match decision {
            PermissionDecision::Approve => PermissionOutcome::Approved,
            PermissionDecision::Deny => PermissionOutcome::Denied,
        };

        if let Ok(mut guard) = entry.outcome.try_lock() {
            *guard = Some(outcome.clone());
        }

        if let Some(emitter) = emitter {
            emit_permission_resolved(emitter, execution_id, permission_id, decision_label);
        }

        if notify_waiter {
            entry.resolved.notify_waiters();
        }

        outcome
    }

    /// Snapshot of a pending permission event (for stream replay / tests).
    pub fn pending_event(&self, permission_id: &str) -> Option<PermissionEvent> {
        self.pending
            .get(permission_id)
            .map(|entry| entry.event.clone())
    }
}

/// Emit the server→client permission prompt on the run/event stream.
pub fn emit_permission_request(
    emitter: &dyn EventEmitter,
    execution_id: &str,
    session_id: Option<&str>,
    event: &PermissionEvent,
) {
    let payload = ApprovalRequestPayload {
        agent_code: session_id.unwrap_or("server").to_string(),
        tool_name: event.capability_id.clone(),
        approval_id: event.permission_id.clone(),
        risk_level: "high".to_string(),
    };
    emitter.emit(
        ApxmEvent::root(payload, EventSource::Server, execution_id)
            .with_scope_id(session_id.map(str::to_string)),
    );
}

/// Emit resolution after the client answers.
pub fn emit_permission_resolved(
    emitter: &dyn EventEmitter,
    execution_id: &str,
    permission_id: &str,
    decision: &str,
) {
    let payload = ApprovalResolvedPayload {
        approval_id: permission_id.to_string(),
        decision: decision.to_string(),
    };
    emitter.emit(ApxmEvent::root(payload, EventSource::Server, execution_id));
}

/// Axum handler for `POST /v1/permissions/{permission_id}/respond`.
pub(crate) async fn respond_permission(
    Path(permission_id): Path<String>,
    Json(body): Json<PermissionResponse>,
) -> Result<Json<OkAck>, ApiError> {
    respond_permission_with_registry(&permission_id, body, PermissionRegistry::global())
        .map(|()| Json(OkAck::new()))
}

/// Apply a client reply without exposing internal HTTP error types (tests/clients).
pub fn apply_response(
    registry: &PermissionRegistry,
    permission_id: &str,
    body: PermissionResponse,
) -> Result<PermissionOutcome, String> {
    registry
        .respond(permission_id, body, None)
        .map_err(|_| format!("failed to record permission response for '{permission_id}'"))
}

/// Testable respond path without axum extractors.
pub(crate) fn respond_permission_with_registry(
    permission_id: &str,
    body: PermissionResponse,
    registry: &PermissionRegistry,
) -> Result<(), ApiError> {
    registry.respond(permission_id, body, None)?;
    Ok(())
}

static GLOBAL_REGISTRY: std::sync::OnceLock<PermissionRegistry> = std::sync::OnceLock::new();

impl PermissionRegistry {
    /// Process-wide registry for HTTP respond handler.
    pub fn global() -> &'static PermissionRegistry {
        GLOBAL_REGISTRY.get_or_init(PermissionRegistry::new)
    }
}

/// Collecting emitter for unit/integration tests.
#[derive(Debug, Default, Clone)]
pub struct RecordingEmitter {
    events: Arc<std::sync::Mutex<Vec<ApxmEvent>>>,
}

impl RecordingEmitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<ApxmEvent> {
        self.events
            .lock()
            .expect("recording emitter poisoned")
            .clone()
    }
}

impl EventEmitter for RecordingEmitter {
    fn emit(&self, event: ApxmEvent) {
        self.events
            .lock()
            .expect("recording emitter poisoned")
            .push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::events::kind as event_kind;
    use tokio::time::sleep;

    #[tokio::test]
    async fn approve_roundtrip_unblocks_waiter() {
        let registry = PermissionRegistry::with_timeout(Duration::from_secs(5));
        let emitter = RecordingEmitter::new();
        let registry_c = registry.clone();
        let emitter_c = emitter.clone();

        let waiter = tokio::spawn(async move {
            registry_c
                .block_for_permission(
                    "exec-1",
                    Some("sess-1"),
                    "write_file",
                    &HashMap::new(),
                    &emitter_c,
                )
                .await
        });

        sleep(Duration::from_millis(20)).await;
        let events = emitter.events();
        assert!(
            events
                .iter()
                .any(|e| e.kind().name() == event_kind::APPROVAL_REQUEST.name()),
            "permission request event must be emitted"
        );
        let permission_id = events
            .iter()
            .find_map(|e| e.payload.downcast_ref::<ApprovalRequestPayload>())
            .map(|p| p.approval_id.clone())
            .expect("approval id");

        registry
            .respond(
                &permission_id,
                PermissionResponse {
                    decision: PermissionDecision::Approve,
                },
                Some(&emitter),
            )
            .expect("respond");

        assert_eq!(waiter.await.unwrap(), PermissionOutcome::Approved);
        let resolved = emitter.events();
        assert!(
            resolved
                .iter()
                .any(|e| e.kind().name() == event_kind::APPROVAL_RESOLVED.name())
        );
    }

    #[tokio::test]
    async fn deny_roundtrip_returns_denied() {
        let registry = PermissionRegistry::with_timeout(Duration::from_secs(5));
        let emitter = RecordingEmitter::new();
        let registry_c = registry.clone();
        let emitter_c = emitter.clone();

        let waiter = tokio::spawn(async move {
            registry_c
                .block_for_permission("exec-deny", None, "bash", &HashMap::new(), &emitter_c)
                .await
        });

        sleep(Duration::from_millis(20)).await;
        let payload = emitter
            .events()
            .into_iter()
            .find_map(|e| e.payload.downcast_ref::<ApprovalRequestPayload>().cloned())
            .expect("approval request");
        registry
            .respond(
                &payload.approval_id,
                PermissionResponse {
                    decision: PermissionDecision::Deny,
                },
                Some(&emitter),
            )
            .expect("respond");

        assert_eq!(waiter.await.unwrap(), PermissionOutcome::Denied);
    }

    #[tokio::test]
    async fn concurrent_respond_is_idempotent() {
        let registry = PermissionRegistry::with_timeout(Duration::from_secs(5));
        let emitter = RecordingEmitter::new();
        let registry_c = registry.clone();
        let emitter_c = emitter.clone();

        let waiter = tokio::spawn(async move {
            registry_c
                .block_for_permission("exec-idem", None, "bash", &HashMap::new(), &emitter_c)
                .await
        });

        sleep(Duration::from_millis(20)).await;
        let approval_id = emitter
            .events()
            .into_iter()
            .find_map(|e| {
                e.payload
                    .downcast_ref::<ApprovalRequestPayload>()
                    .map(|p| p.approval_id.clone())
            })
            .expect("approval id");

        let first = registry.respond(
            &approval_id,
            PermissionResponse {
                decision: PermissionDecision::Approve,
            },
            None,
        );
        let second = registry.respond(
            &approval_id,
            PermissionResponse {
                decision: PermissionDecision::Approve,
            },
            None,
        );
        assert_eq!(first.unwrap(), PermissionOutcome::Approved);
        assert_eq!(
            second.unwrap(),
            PermissionOutcome::Approved,
            "duplicate respond is idempotent"
        );
        assert_eq!(waiter.await.unwrap(), PermissionOutcome::Approved);
    }
}
