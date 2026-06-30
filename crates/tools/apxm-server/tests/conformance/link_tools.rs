//! LINK-TOOLS mode conformance tests (T1).
//!
//! Spec vectors:
//! - HostTier::LinkTools is selected for hosts with Link but no local runtime.
//! - `open_agent_channel` is called on the gateway for spawn operations.
//! - `take_relay_channel` returns the channel pair seeded by the mock.

use apxm_core::types::host::{
    HostDispatchGateway, HostTier, IngressConfig, OpKind, RuntimeConfig, SpawnOffer,
    TransportManifest, TierQuery, select_tier,
};

use super::mock_gateway::MockHostDispatchGateway;

// ── Tier selection ────────────────────────────────────────────────────────────

#[test]
fn link_tools_tier_selected_for_host_with_link_no_runtime() {
    let manifest = TransportManifest {
        host_id: Some("browser-x7f".into()),
        ingress: IngressConfig::default(),
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

    let tier = select_tier(&manifest, &query).expect("tier selection must succeed");
    assert_eq!(
        tier,
        HostTier::LinkTools,
        "enrolled host with no local_agent must resolve to LINK-TOOLS"
    );
}

#[test]
fn link_tools_not_selected_when_local_runtime_and_spawn_op() {
    // A host with local_agent=true AND a Spawn op must resolve to LINK-RUNTIME, not LINK-TOOLS.
    let manifest = TransportManifest {
        host_id: Some("ide-abc".into()),
        ingress: IngressConfig::default(),
        runtime: RuntimeConfig { local_agent: true },
        tier_hint: None,
        custody: None,
        push: None,
        confinement: Some(apxm_core::types::host::ConfinementProfile {
            mechanism: apxm_core::types::host::ConfinementMechanism::OsSandbox,
            network: apxm_core::types::host::ConfinementNetwork::EgressAllowlist,
            fs_scope: apxm_core::types::host::ConfinementFsScope::WorkdirOnly,
            writable: true,
        }),
        labels: vec![],
    };

    let query = TierQuery {
        transport: "link",
        op_kind: Some(OpKind::Spawn),
    };

    let tier = select_tier(&manifest, &query).expect("tier selection must succeed");
    assert_ne!(
        tier,
        HostTier::LinkTools,
        "local_agent host with Spawn op must not resolve to LINK-TOOLS"
    );
    assert_eq!(tier, HostTier::LinkRuntime);
}

#[test]
fn link_tools_selected_for_standard_op_even_with_local_runtime() {
    // A host with local_agent=true but a Standard (non-local-exec) op resolves to LINK-TOOLS.
    let manifest = TransportManifest {
        host_id: Some("ide-abc".into()),
        ingress: IngressConfig::default(),
        runtime: RuntimeConfig { local_agent: true },
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

    let tier = select_tier(&manifest, &query).expect("tier selection must succeed");
    assert_eq!(
        tier,
        HostTier::LinkTools,
        "non-local-exec op must resolve to LINK-TOOLS even when local_agent=true"
    );
}

// ── Gateway open_agent_channel ────────────────────────────────────────────────

#[tokio::test]
async fn open_agent_channel_called_on_gateway_for_spawn() {
    let gw = MockHostDispatchGateway::new();

    let offer = SpawnOffer {
        lease_id: "lease-001".into(),
        channel_id: "chan-lt-001".into(),
        accept_by_ms: 30_000,
        signed_spawn_envelope: "envelope-bytes".into(),
        profile: "default".into(),
        mode: Some("link-tools".into()),
        model: Some("apxm-default".into()),
        workdir_ref: None,
        attestation_nonce: "nonce-abc".into(),
        extra_env: Default::default(),
    };

    let handle = gw
        .open_agent_channel("browser-x7f", offer.clone())
        .await
        .expect("open_agent_channel must succeed on mock");

    // Verify the handle carries expected identifiers.
    assert_eq!(handle.channel_id, gw.channel_id, "returned channel_id must match mock config");
    assert_eq!(handle.host_id, "browser-x7f");

    // Verify the call was recorded.
    assert_eq!(gw.channel_open_count().await, 1);
    let opens = gw.channel_opens.lock().await;
    assert_eq!(opens[0].host_id, "browser-x7f");
    assert_eq!(opens[0].spawn_offer.lease_id, "lease-001");
}

#[tokio::test]
async fn multiple_channel_opens_are_all_recorded() {
    let gw = MockHostDispatchGateway::new();

    for i in 0..3u32 {
        let offer = SpawnOffer {
            lease_id: format!("lease-{i}"),
            channel_id: format!("chan-{i}"),
            accept_by_ms: 30_000,
            signed_spawn_envelope: "env".into(),
            profile: "default".into(),
            mode: None,
            model: None,
            workdir_ref: None,
            attestation_nonce: "nonce".into(),
            extra_env: Default::default(),
        };
        gw.open_agent_channel("host-multi", offer).await.unwrap();
    }

    assert_eq!(
        gw.channel_open_count().await,
        3,
        "all three channel opens must be recorded"
    );
}

// ── take_relay_channel ────────────────────────────────────────────────────────

#[tokio::test]
async fn take_relay_channel_returns_seeded_channel_pair() {
    let gw = MockHostDispatchGateway::new();

    // Seed a relay channel into the mock before the test consumes it.
    let (tx_to_apxm, mut rx_from_host) = gw.seed_relay_channel("chan-lt-001").await;

    // open_agent_channel would normally be called first to create the handle;
    // here we test take_relay_channel in isolation.
    let taken = gw.take_relay_channel("chan-lt-001").await;
    assert!(taken.is_some(), "take_relay_channel must return the seeded pair");

    let (tx_to_host, mut rx_from_apxm) = taken.unwrap();

    // Verify the channel is functional end-to-end.
    tx_to_host.send(serde_json::json!({ "type": "ping" })).await.unwrap();
    let msg = rx_from_host.recv().await.expect("must receive ping");
    assert_eq!(msg["type"], "ping");

    tx_to_apxm.send(serde_json::json!({ "type": "pong" })).await.unwrap();
    let reply = rx_from_apxm.recv().await.expect("must receive pong");
    assert_eq!(reply["type"], "pong");
}

#[tokio::test]
async fn take_relay_channel_is_one_shot() {
    let gw = MockHostDispatchGateway::new();
    gw.seed_relay_channel("chan-one-shot").await;

    let first = gw.take_relay_channel("chan-one-shot").await;
    let second = gw.take_relay_channel("chan-one-shot").await;

    assert!(first.is_some(), "first take must succeed");
    assert!(second.is_none(), "second take must return None (already consumed)");
}

#[tokio::test]
async fn take_relay_channel_unknown_id_returns_none() {
    let gw = MockHostDispatchGateway::new();
    let result = gw.take_relay_channel("nonexistent-channel").await;
    assert!(result.is_none(), "unknown channel_id must return None");
}
