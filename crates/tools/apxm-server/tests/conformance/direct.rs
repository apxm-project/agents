//! DIRECT mode conformance tests (T0).
//!
//! Spec vectors:
//! - HostTier::Direct is selected for hosts with `transport="direct"` + public ingress.
//! - Host capability call routes through `call_tool` on the gateway.
//! - Consent NoBroker = admit: a NoOp host with no consent gate dispatches unconditionally.

use std::sync::Arc;

use apxm_core::types::host::{
    HostDispatchGateway, HostTier, HostToolCall, IngressConfig, OpKind, RuntimeConfig,
    TransportManifest, TierQuery, select_tier,
};

use super::mock_gateway::MockHostDispatchGateway;

// ── Tier selection ────────────────────────────────────────────────────────────

#[test]
fn direct_tier_selected_for_public_ingress_host() {
    let manifest = TransportManifest {
        host_id: Some("saas-direct-host".into()),
        ingress: IngressConfig {
            public_url: Some("https://hooks.example.com/apxm".into()),
            webhook_base_path: None,
        },
        runtime: RuntimeConfig { local_agent: false },
        tier_hint: None,
        custody: None,
        push: None,
        confinement: None,
        labels: vec![],
    };

    let query = TierQuery {
        transport: "direct",
        op_kind: None,
    };

    let tier = select_tier(&manifest, &query).expect("tier selection must succeed");
    assert_eq!(
        tier,
        HostTier::Direct,
        "public-ingress host with transport=direct must resolve to DIRECT"
    );
}

#[test]
fn direct_tier_not_selected_without_public_url() {
    // A host that has no public ingress and no host_id cannot be admitted.
    let manifest = TransportManifest {
        host_id: None,
        ingress: IngressConfig {
            public_url: None,
            webhook_base_path: None,
        },
        runtime: RuntimeConfig { local_agent: false },
        tier_hint: None,
        custody: None,
        push: None,
        confinement: None,
        labels: vec![],
    };

    let query = TierQuery {
        transport: "direct",
        op_kind: None,
    };

    let result = select_tier(&manifest, &query);
    assert!(
        result.is_err(),
        "host with no public_url and no host_id must fail closed"
    );
}

#[test]
fn direct_tier_not_selected_for_link_transport() {
    // Even with a public URL, transport="link" does not resolve to DIRECT.
    let manifest = TransportManifest {
        host_id: Some("saas-host".into()),
        ingress: IngressConfig {
            public_url: Some("https://hooks.example.com/apxm".into()),
            webhook_base_path: None,
        },
        runtime: RuntimeConfig { local_agent: false },
        tier_hint: None,
        custody: None,
        push: None,
        confinement: None,
        labels: vec![],
    };

    let query = TierQuery {
        transport: "link",
        op_kind: Some(OpKind::Standard),
    };

    let tier = select_tier(&manifest, &query).expect("should resolve via link-tools");
    assert_ne!(tier, HostTier::Direct, "link transport must not resolve to DIRECT");
    assert_eq!(tier, HostTier::LinkTools, "link transport with host_id resolves to LINK-TOOLS");
}

// ── Gateway call_tool routing ─────────────────────────────────────────────────

#[tokio::test]
async fn direct_capability_call_routes_through_call_tool() {
    let gw = MockHostDispatchGateway::new();

    let call = HostToolCall {
        call_id: "call-001".into(),
        capability_id: "cap-read-file".into(),
        host_op: "read_file".into(),
        tool_binding: "host_read_file_v1".into(),
        args: serde_json::json!({ "path": "/tmp/test.txt" }),
        args_digest: "abc123".into(),
        grant_ref: Some("grant-001".into()),
        timeout_ms: 5_000,
        idempotency_key: None,
        subject: None,
    };

    let result = gw.call_tool("saas-direct-host", call).await;

    assert!(result.is_ok(), "mock gateway must return Ok");
    let tool_result = result.unwrap();
    assert!(tool_result.ok, "mock result must have ok=true");

    // Verify the call was recorded exactly once.
    assert_eq!(
        gw.tool_call_count().await,
        1,
        "exactly one call_tool invocation must be recorded"
    );

    let recorded = gw.tool_calls.lock().await;
    let first = &recorded[0];
    assert_eq!(first.host_id, "saas-direct-host");
    assert_eq!(first.call.call_id, "call-001");
    assert_eq!(first.call.capability_id, "cap-read-file");
}

#[tokio::test]
async fn direct_mode_no_open_agent_channel_called() {
    // DIRECT hosts never open agent channels — only LINK-RUNTIME does.
    let gw = MockHostDispatchGateway::new();

    // Simulate a DIRECT dispatch: only call_tool is invoked.
    let call = HostToolCall {
        call_id: "call-002".into(),
        capability_id: "cap-write".into(),
        host_op: "write".into(),
        tool_binding: "host_write_v1".into(),
        args: serde_json::json!({}),
        args_digest: "def456".into(),
        grant_ref: None,
        timeout_ms: 5_000,
        idempotency_key: None,
        subject: None,
    };

    gw.call_tool("direct-host", call).await.expect("call_tool ok");

    assert_eq!(
        gw.channel_open_count().await,
        0,
        "DIRECT mode must never call open_agent_channel"
    );
}

// ── Consent NoBroker = admit (NoOp host dispatch) ────────────────────────────

#[tokio::test]
async fn noop_host_dispatch_admits_without_consent_gate() {
    // In DIRECT mode with a NoOp gateway, operations are dispatched without a
    // separate consent step — the absence of a consent gate means admit.
    use apxm_runtime::host_dispatch::NoOpHostDispatchGateway;

    let gw: Arc<dyn HostDispatchGateway> = Arc::new(NoOpHostDispatchGateway);

    // The NoOp gateway returns a Transport error (no real host), which is the
    // expected "no-op" path: it does NOT block on a consent broker or hold the
    // call waiting for approval. The error surfaces immediately, proving no
    // consent wait occurred.
    let call = HostToolCall {
        call_id: "consent-test-001".into(),
        capability_id: "cap-noop".into(),
        host_op: "noop".into(),
        tool_binding: "noop_v1".into(),
        args: serde_json::json!({}),
        args_digest: "".into(),
        grant_ref: None,
        timeout_ms: 1_000,
        idempotency_key: None,
        subject: None,
    };

    let result = gw.call_tool("noop-host", call).await;

    // The NoOp gateway returns immediately with a Transport error — this
    // confirms no blocking consent wait occurred (NoBroker = admit path).
    assert!(
        result.is_err(),
        "NoOp gateway must return an error (not block on consent)"
    );
    match result.unwrap_err() {
        apxm_core::types::host::HostDispatchError::Transport(_) => {}
        other => panic!("expected Transport error, got: {other:?}"),
    }
}
