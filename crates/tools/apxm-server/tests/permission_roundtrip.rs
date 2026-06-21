//! Permission round-trip fixture.

use std::collections::HashMap;
use std::time::Duration;

use apxm_core::events::kind as event_kind;
use apxm_core::events::payload::ApprovalRequestPayload;
use apxm_core::types::values::Value;
use apxm_server::permissions::{
    PermissionDecision, PermissionOutcome, PermissionRegistry, PermissionResponse,
    RecordingEmitter, apply_response,
};
use tokio::time::sleep;

#[tokio::test]
async fn sc005_permission_roundtrip_approve_and_session_grant() {
    let registry = PermissionRegistry::with_timeout(Duration::from_secs(5));
    let emitter = RecordingEmitter::new();
    let mut args = HashMap::new();
    args.insert("path".to_string(), Value::String("/tmp/sc005".to_string()));

    let registry_c = registry.clone();
    let emitter_c = emitter.clone();
    let args_c = args.clone();
    let waiter = tokio::spawn(async move {
        registry_c
            .block_for_permission(
                "exec-sc005",
                Some("sess-sc005"),
                "write_file",
                &args_c,
                &emitter_c,
            )
            .await
    });

    sleep(Duration::from_millis(25)).await;
    let approval_id = emitter
        .events()
        .into_iter()
        .find_map(|e| {
            e.payload
                .downcast_ref::<ApprovalRequestPayload>()
                .map(|p| p.approval_id.clone())
        })
        .expect("permission event on stream");
    assert!(
        emitter
            .events()
            .iter()
            .any(|e| { e.kind().name() == event_kind::APPROVAL_REQUEST.name() }),
        "stream emits permission event before proceed"
    );

    apply_response(
        &registry,
        &approval_id,
        PermissionResponse {
            decision: PermissionDecision::ApproveForSession,
        },
    )
    .expect("respond");
    assert_eq!(waiter.await.unwrap(), PermissionOutcome::Approved);

    let emitter2 = RecordingEmitter::new();
    let second = registry
        .block_for_permission(
            "exec-sc005",
            Some("sess-sc005"),
            "write_file",
            &args,
            &emitter2,
        )
        .await;
    assert_eq!(second, PermissionOutcome::Approved);
    assert!(
        emitter2.events().is_empty(),
        "session-grant suppresses second prompt"
    );
}
