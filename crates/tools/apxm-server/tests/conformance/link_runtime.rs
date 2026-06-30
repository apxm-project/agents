//! LINK-RUNTIME mode conformance tests.
//!
//! Spec vectors:
//! - HostTier::LinkRuntime is selected for hosts with Link AND local_agent=true on a local-exec op.
//! - Spawn offer includes an `attestation_nonce` field (confinement attestation).
//! - ACP frame routing works end-to-end through mock channels.

use apxm_core::types::host::{
    ConfinementFsScope, ConfinementMechanism, ConfinementNetwork, ConfinementProfile,
    HostDispatchGateway, HostTier, IngressConfig, OpKind, RuntimeConfig, SpawnOffer,
    TransportManifest, TierQuery, select_tier,
};

use super::mock_gateway::MockHostDispatchGateway;

// ── Tier selection ────────────────────────────────────────────────────────────

fn link_runtime_manifest() -> TransportManifest {
    TransportManifest {
        host_id: Some("ide-abc".into()),
        ingress: IngressConfig::default(),
        runtime: RuntimeConfig { local_agent: true },
        tier_hint: None,
        custody: None,
        push: None,
        confinement: Some(ConfinementProfile {
            mechanism: ConfinementMechanism::OsSandbox,
            network: ConfinementNetwork::EgressAllowlist,
            fs_scope: ConfinementFsScope::WorkdirOnly,
            writable: true,
        }),
        labels: vec![],
    }
}

#[test]
fn link_runtime_tier_selected_for_local_agent_spawn_op() {
    let manifest = link_runtime_manifest();
    let query = TierQuery {
        transport: "link",
        op_kind: Some(OpKind::Spawn),
    };

    let tier = select_tier(&manifest, &query).expect("tier selection must succeed");
    assert_eq!(
        tier,
        HostTier::LinkRuntime,
        "local_agent host with Spawn op must resolve to LINK-RUNTIME"
    );
}

#[test]
fn link_runtime_tier_selected_for_host_exec_op() {
    let manifest = link_runtime_manifest();
    let query = TierQuery {
        transport: "link",
        op_kind: Some(OpKind::HostExec),
    };

    let tier = select_tier(&manifest, &query).expect("tier selection must succeed");
    assert_eq!(tier, HostTier::LinkRuntime);
}

#[test]
fn link_runtime_tier_selected_for_host_fs_op() {
    let manifest = link_runtime_manifest();
    let query = TierQuery {
        transport: "link",
        op_kind: Some(OpKind::HostFs),
    };

    let tier = select_tier(&manifest, &query).expect("tier selection must succeed");
    assert_eq!(tier, HostTier::LinkRuntime);
}

#[test]
fn link_runtime_requires_confinement_block() {
    let mut manifest = link_runtime_manifest();
    manifest.confinement = None;
    let query = TierQuery {
        transport: "link",
        op_kind: Some(OpKind::Spawn),
    };

    let result = select_tier(&manifest, &query);
    assert!(
        result.is_err(),
        "LINK-RUNTIME host without [confinement] block must fail closed"
    );
    assert!(matches!(
        result.unwrap_err(),
        apxm_core::types::host::EnrollError::MissingConfinement
    ));
}

// ── Confinement attestation in spawn offer ────────────────────────────────────

#[tokio::test]
async fn spawn_offer_includes_attestation_nonce() {
    let gw = MockHostDispatchGateway::new();

    let offer = SpawnOffer {
        lease_id: "lease-lr-001".into(),
        channel_id: "chan-lr-001".into(),
        accept_by_ms: 30_000,
        signed_spawn_envelope: "signed-envelope-bytes".into(),
        profile: "os-sandbox".into(),
        mode: Some("link-runtime".into()),
        model: Some("apxm-default".into()),
        workdir_ref: Some("/tmp/workdir-abc".into()),
        // The attestation_nonce field carries the confinement attestation token.
        attestation_nonce: "confinement-nonce-xyz123".into(),
        extra_env: Default::default(),
    };

    gw.open_agent_channel("ide-abc", offer)
        .await
        .expect("open_agent_channel must succeed");

    let offers = gw.spawn_offers().await;
    assert_eq!(offers.len(), 1);

    let recorded_offer = &offers[0];
    assert!(
        !recorded_offer.attestation_nonce.is_empty(),
        "spawn offer must carry a non-empty attestation_nonce"
    );
    assert_eq!(recorded_offer.attestation_nonce, "confinement-nonce-xyz123");
}

#[tokio::test]
async fn spawn_offer_attestation_nonce_distinguishes_runtime_tier() {
    // LINK-TOOLS offers may have empty attestation_nonce; LINK-RUNTIME must not.
    let gw = MockHostDispatchGateway::new();

    // LINK-TOOLS offer: attestation_nonce is empty (no confinement).
    let lt_offer = SpawnOffer {
        lease_id: "lt-lease".into(),
        channel_id: "lt-chan".into(),
        accept_by_ms: 30_000,
        signed_spawn_envelope: "env".into(),
        profile: "default".into(),
        mode: Some("link-tools".into()),
        model: None,
        workdir_ref: None,
        attestation_nonce: String::new(), // empty for LINK-TOOLS
        extra_env: Default::default(),
    };

    // LINK-RUNTIME offer: attestation_nonce is non-empty (confinement attestation required).
    let lr_offer = SpawnOffer {
        lease_id: "lr-lease".into(),
        channel_id: "lr-chan".into(),
        accept_by_ms: 30_000,
        signed_spawn_envelope: "env".into(),
        profile: "os-sandbox".into(),
        mode: Some("link-runtime".into()),
        model: None,
        workdir_ref: None,
        attestation_nonce: "lr-nonce-abc".into(), // non-empty for LINK-RUNTIME
        extra_env: Default::default(),
    };

    gw.open_agent_channel("browser-x7f", lt_offer).await.unwrap();
    gw.open_agent_channel("ide-abc", lr_offer).await.unwrap();

    let offers = gw.spawn_offers().await;
    assert_eq!(offers.len(), 2);

    let link_tools_offer = &offers[0];
    let t2 = &offers[1];

    assert!(link_tools_offer.attestation_nonce.is_empty(), "LINK-TOOLS offer must have empty attestation_nonce");
    assert!(!t2.attestation_nonce.is_empty(), "LINK-RUNTIME offer must have non-empty attestation_nonce");
}

// ── ACP frame routing end-to-end ──────────────────────────────────────────────

#[tokio::test]
async fn acp_frame_routing_works_end_to_end() {
    let gw = MockHostDispatchGateway::new();

    // First open a channel to get a handle.
    let offer = SpawnOffer {
        lease_id: "lr-frame-lease".into(),
        channel_id: "lr-frame-chan".into(),
        accept_by_ms: 30_000,
        signed_spawn_envelope: "env".into(),
        profile: "os-sandbox".into(),
        mode: Some("link-runtime".into()),
        model: None,
        workdir_ref: None,
        attestation_nonce: "frame-nonce".into(),
        extra_env: Default::default(),
    };

    let handle = gw
        .open_agent_channel("ide-abc", offer)
        .await
        .expect("open_agent_channel");

    // Seed a relay channel so take_relay_channel works.
    let (tx_to_apxm, mut rx_from_host) = gw.seed_relay_channel(&handle.channel_id).await;

    // Send an ACP frame through the gateway to the host.
    let acp_frame = serde_json::json!({
        "type": "acp/request",
        "channel_id": handle.channel_id,
        "payload": { "op": "spawn", "profile": "os-sandbox" }
    });
    let frame_bytes = serde_json::to_vec(&acp_frame).unwrap();

    gw.send_agent_frame(&handle.channel_id, frame_bytes.clone())
        .await
        .expect("send_agent_frame");

    // Verify the frame was recorded by the mock.
    let frames = gw.frames_sent.lock().await;
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].channel_id, handle.channel_id);
    assert_eq!(frames[0].frame, frame_bytes);
    drop(frames);

    // Simulate the host-side: take the relay channel and send a response back.
    let taken = gw.take_relay_channel(&handle.channel_id).await;
    assert!(taken.is_some(), "relay channel must be available after seeding");
    let (tx_to_host, mut rx_from_apxm) = taken.unwrap();

    // Host sends an ACP response back to APXM-side.
    tx_to_apxm
        .send(serde_json::json!({ "type": "acp/response", "status": "spawned" }))
        .await
        .unwrap();

    // APXM-side receives the response from the host.
    let response = rx_from_host.recv().await.expect("must receive acp/response");
    assert_eq!(response["type"], "acp/response");
    assert_eq!(response["status"], "spawned");

    // APXM-side sends a follow-up frame to the host.
    tx_to_host
        .send(serde_json::json!({ "type": "acp/ack", "channel_id": handle.channel_id }))
        .await
        .unwrap();

    let ack = rx_from_apxm.recv().await.expect("must receive acp/ack");
    assert_eq!(ack["type"], "acp/ack");
}

#[tokio::test]
async fn close_agent_channel_recorded_after_frame_exchange() {
    let gw = MockHostDispatchGateway::new();

    let offer = SpawnOffer {
        lease_id: "close-lease".into(),
        channel_id: "close-chan".into(),
        accept_by_ms: 30_000,
        signed_spawn_envelope: "env".into(),
        profile: "os-sandbox".into(),
        mode: Some("link-runtime".into()),
        model: None,
        workdir_ref: None,
        attestation_nonce: "close-nonce".into(),
        extra_env: Default::default(),
    };

    let handle = gw.open_agent_channel("ide-abc", offer).await.unwrap();

    // Exchange one frame then close.
    gw.send_agent_frame(&handle.channel_id, b"frame".to_vec())
        .await
        .unwrap();
    gw.close_agent_channel(&handle.channel_id, "session_complete")
        .await
        .unwrap();

    let closes = gw.channel_closes.lock().await;
    assert_eq!(closes.len(), 1);
    assert_eq!(closes[0].channel_id, handle.channel_id);
    assert_eq!(closes[0].reason, "session_complete");
}

// ── Confinement profile strength ordering ────────────────────────────────────

#[test]
fn link_runtime_confinement_profile_strength_ordering() {
    // Verify the confinement profile used by LINK-RUNTIME hosts satisfies
    // itself and weaker profiles, but not stronger ones.
    let os_sandbox = ConfinementProfile {
        mechanism: ConfinementMechanism::OsSandbox,
        network: ConfinementNetwork::EgressAllowlist,
        fs_scope: ConfinementFsScope::WorkdirOnly,
        writable: true,
    };

    let container = ConfinementProfile {
        mechanism: ConfinementMechanism::Container,
        network: ConfinementNetwork::Isolated,
        fs_scope: ConfinementFsScope::WorkdirOnly,
        writable: false,
    };

    let vm = ConfinementProfile {
        mechanism: ConfinementMechanism::Vm,
        network: ConfinementNetwork::Isolated,
        fs_scope: ConfinementFsScope::WorkdirOnly,
        writable: false,
    };

    // A Container satisfies OsSandbox requirements.
    assert!(container.satisfies(&os_sandbox));
    // An OsSandbox does NOT satisfy Container requirements.
    assert!(!os_sandbox.satisfies(&container));
    // A VM satisfies Container requirements.
    assert!(vm.satisfies(&container));
    // A VM satisfies itself.
    assert!(vm.satisfies(&vm));
}
