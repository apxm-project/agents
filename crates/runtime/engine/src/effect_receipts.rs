//! Durable capability-effect replay evidence lookup and verification.

use std::collections::HashMap;

use apxm_capability_iface::{
    CapabilityEffectReplayEvidence, CapabilityEffectReplayEvidenceEnvelope,
    HostEffectPrepareEvidence, capability_effect_idempotency_key_digest,
};
use apxm_core::events::payload::{
    CapabilityEffectAdmissionKind, CapabilityEffectApprovalStatus, CapabilityEffectDispatchPath,
    CapabilityEffectIdempotencyProof, CapabilityEffectImplementationKind,
};
use apxm_core::types::host::{HostEffectCommit, HostEffectOutcome};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

use crate::metadata_keys;

/// Runtime-minted opaque identifier for a future persisted capability receipt.
pub fn mint_capability_effect_receipt_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// Facts the replayer knows about a skipped capability node in the recompiled
/// graph. Verification binds host evidence to these graph facts and then
/// proves the evidence is internally consistent.
pub struct ExpectedCapabilityEffect<'a> {
    pub node_id: u64,
    pub capability_binding: &'a str,
    pub dispatch_path: CapabilityEffectDispatchPath,
}

/// Resolve durable replay evidence supplied inline by the Server-owned store.
pub fn lookup_replayable_effect(
    metadata: &HashMap<String, String>,
    expected: &ExpectedCapabilityEffect<'_>,
) -> Result<Option<CapabilityEffectReplayEvidence>, String> {
    let Some(raw) = metadata.get(metadata_keys::CAPABILITY_EFFECT_REPLAY_EVIDENCE) else {
        return Ok(None);
    };
    lookup_replayable_effect_in_metadata(raw, expected)
}

fn lookup_replayable_effect_in_metadata(
    raw: &str,
    expected: &ExpectedCapabilityEffect<'_>,
) -> Result<Option<CapabilityEffectReplayEvidence>, String> {
    let envelope: CapabilityEffectReplayEvidenceEnvelope = serde_json::from_str(raw)
        .map_err(|error| format!("invalid capability effect replay evidence metadata: {error}"))?;
    let mut matches = envelope
        .records
        .into_iter()
        .filter(|record| {
            record.receipt.node_id == expected.node_id
                && record.receipt.capability_binding == expected.capability_binding
                && record.receipt.dispatch_path == expected.dispatch_path
        })
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err(format!(
            "multiple replay evidence records matched node_id={} capability_binding={} dispatch_path={:?}",
            expected.node_id, expected.capability_binding, expected.dispatch_path
        ));
    }
    Ok(matches.pop())
}

/// Verify a replayable capability effect.
///
/// The runtime remains fail-closed: host replay is allowed only when the
/// public receipt, stored prepare, stored commit, and enrolled host key all
/// exist and bind exactly.
pub fn verify_replayable_effect(
    evidence: &CapabilityEffectReplayEvidence,
    expected: &ExpectedCapabilityEffect<'_>,
) -> Result<(), String> {
    evidence.receipt.validate()?;

    if evidence.receipt.node_id != expected.node_id {
        return Err(format!(
            "receipt node_id {} does not match replay node {}",
            evidence.receipt.node_id, expected.node_id
        ));
    }
    if evidence.receipt.capability_binding != expected.capability_binding {
        return Err(format!(
            "receipt capability_binding {} does not match replay binding {}",
            evidence.receipt.capability_binding, expected.capability_binding
        ));
    }
    if evidence.receipt.dispatch_path != expected.dispatch_path {
        return Err(format!(
            "receipt dispatch_path {:?} does not match replay dispatch path {:?}",
            evidence.receipt.dispatch_path, expected.dispatch_path
        ));
    }
    if evidence.receipt.implementation_kind != CapabilityEffectImplementationKind::Host {
        return Err(format!(
            "receipt implementation_kind {:?} is replay-ineligible",
            evidence.receipt.implementation_kind
        ));
    }

    let prepare = evidence
        .host_prepare
        .as_ref()
        .ok_or_else(|| "host replay evidence is missing HostEffectPrepareEvidence".to_string())?;
    let commit = evidence
        .host_commit
        .as_ref()
        .ok_or_else(|| "host replay evidence is missing HostEffectCommit".to_string())?;
    let host_pubkey_hex = evidence
        .host_pubkey_hex
        .as_deref()
        .ok_or_else(|| "host replay evidence is missing host_pubkey_hex".to_string())?;
    if host_pubkey_hex.is_empty() {
        return Err("host replay evidence host_pubkey_hex must not be empty".to_string());
    }

    verify_prepare_against_receipt(prepare, &evidence.receipt)?;
    verify_prepare_against_commit(prepare, commit)?;
    verify_commit_against_receipt(commit, &evidence.receipt)?;
    verify_host_commit_signature(commit, host_pubkey_hex)?;
    Ok(())
}

fn verify_prepare_against_receipt(
    prepare: &HostEffectPrepareEvidence,
    receipt: &apxm_core::events::payload::CapabilityEffectReceiptPayload,
) -> Result<(), String> {
    if prepare.execution_id != receipt.execution_id {
        return Err("host prepare execution_id does not match receipt".to_string());
    }
    if prepare.node_id != receipt.node_id {
        return Err("host prepare node_id does not match receipt".to_string());
    }
    if prepare.invocation_id != receipt.invocation_id {
        return Err("host prepare invocation_id does not match receipt".to_string());
    }
    if prepare.capability_binding != receipt.capability_binding {
        return Err("host prepare capability_binding does not match receipt".to_string());
    }
    if prepare.implementation_ref != receipt.implementation_ref {
        return Err("host prepare implementation_ref does not match receipt".to_string());
    }
    if prepare.request_digest != receipt.request_digest {
        return Err("host prepare request_digest does not match receipt".to_string());
    }
    let expected_idempotency = capability_effect_idempotency_key_digest(&prepare.idempotency_key);
    if expected_idempotency != receipt.idempotency_key_digest {
        return Err("host prepare idempotency key digest does not match receipt".to_string());
    }
    match receipt.admission_kind {
        CapabilityEffectAdmissionKind::Grant => {
            let grant_id = receipt.grant_id.as_deref().ok_or_else(|| {
                "receipt grant admission is missing grant_id after validation".to_string()
            })?;
            if !prepare
                .grant_refs
                .iter()
                .any(|candidate| candidate == grant_id)
            {
                return Err("receipt grant_id is not bound by host prepare grant_refs".to_string());
            }
        }
        CapabilityEffectAdmissionKind::ReadOnly | CapabilityEffectAdmissionKind::Sandbox => {}
    }
    match receipt.approval_status {
        Some(CapabilityEffectApprovalStatus::Approved) => {
            let approval_id = receipt.approval_id.as_deref().ok_or_else(|| {
                "receipt approved status is missing approval_id after validation".to_string()
            })?;
            if !prepare
                .approval_refs
                .iter()
                .any(|candidate| candidate == approval_id)
            {
                return Err(
                    "receipt approval_id is not bound by host prepare approval_refs".to_string(),
                );
            }
        }
        Some(CapabilityEffectApprovalStatus::NotRequired) | None => {
            if receipt.approval_id.is_some() {
                return Err(
                    "receipt approval_id must be absent when approval was not required".to_string(),
                );
            }
        }
    }
    Ok(())
}

fn verify_prepare_against_commit(
    prepare: &HostEffectPrepareEvidence,
    commit: &HostEffectCommit,
) -> Result<(), String> {
    if prepare.execution_id != commit.execution_id {
        return Err("host prepare execution_id does not match commit".to_string());
    }
    if prepare.graph_id != commit.graph_id {
        return Err("host prepare graph_id does not match commit".to_string());
    }
    if prepare.node_id != commit.node_id {
        return Err("host prepare node_id does not match commit".to_string());
    }
    if prepare.invocation_id != commit.invocation_id {
        return Err("host prepare invocation_id does not match commit".to_string());
    }
    if prepare.call_id != commit.call_id {
        return Err("host prepare call_id does not match commit".to_string());
    }
    if prepare.request_digest != commit.request_digest {
        return Err("host prepare request_digest does not match commit".to_string());
    }
    if prepare.idempotency_key != commit.idempotency_key {
        return Err("host prepare idempotency_key does not match commit".to_string());
    }
    Ok(())
}

fn verify_commit_against_receipt(
    commit: &HostEffectCommit,
    receipt: &apxm_core::events::payload::CapabilityEffectReceiptPayload,
) -> Result<(), String> {
    if commit.execution_id != receipt.execution_id {
        return Err("host commit execution_id does not match receipt".to_string());
    }
    if commit.node_id != receipt.node_id {
        return Err("host commit node_id does not match receipt".to_string());
    }
    if commit.invocation_id != receipt.invocation_id {
        return Err("host commit invocation_id does not match receipt".to_string());
    }
    if commit.request_digest != receipt.request_digest {
        return Err("host commit request_digest does not match receipt".to_string());
    }
    let expected_idempotency = capability_effect_idempotency_key_digest(&commit.idempotency_key);
    if expected_idempotency != receipt.idempotency_key_digest {
        return Err("host commit idempotency key digest does not match receipt".to_string());
    }
    let expected_proof = match commit.effect_outcome {
        HostEffectOutcome::Committed => CapabilityEffectIdempotencyProof::TransactionVerified,
        HostEffectOutcome::Deduplicated => CapabilityEffectIdempotencyProof::RemoteDeduplicated,
    };
    if receipt.idempotency_proof != expected_proof {
        return Err(format!(
            "receipt idempotency proof {:?} does not match host outcome {:?}",
            receipt.idempotency_proof, commit.effect_outcome
        ));
    }
    if commit.host_key_id.is_empty() {
        return Err("host commit host_key_id must not be empty".to_string());
    }
    if commit.signature.is_empty() {
        return Err("host commit signature must not be empty".to_string());
    }
    Ok(())
}

fn verify_host_commit_signature(
    commit: &HostEffectCommit,
    host_pubkey_hex: &str,
) -> Result<(), String> {
    let key_bytes = decode_hex(host_pubkey_hex)?;
    let signature_bytes = decode_binary_value(&commit.signature)?;
    let key = VerifyingKey::from_bytes(
        &key_bytes
            .try_into()
            .map_err(|_| "host_pubkey_hex must decode to 32 bytes".to_string())?,
    )
    .map_err(|error| format!("invalid host_pubkey_hex: {error}"))?;
    let signature = Signature::from_slice(&signature_bytes)
        .map_err(|error| format!("invalid host effect signature: {error}"))?;
    key.verify(commit.effect_digest.as_bytes(), &signature)
        .map_err(|error| format!("host effect signature verification failed: {error}"))
}

fn decode_binary_value(value: &str) -> Result<Vec<u8>, String> {
    if let Ok(bytes) = decode_hex(value) {
        return Ok(bytes);
    }
    base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, value)
        .or_else(|_| base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE, value))
        .or_else(|_| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, value))
        .map_err(|error| format!("failed to decode signature: {error}"))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("hex value must have even length".to_string());
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    let mut chars = value.chars();
    while let (Some(high), Some(low)) = (chars.next(), chars.next()) {
        let pair = [high, low].iter().collect::<String>();
        let byte =
            u8::from_str_radix(&pair, 16).map_err(|error| format!("invalid hex value: {error}"))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::events::payload::{
        CapabilityEffectReceiptPayload, CapabilityEffectReceiptStatus,
    };
    use ed25519_dalek::{Signer, SigningKey};

    fn encode_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn fixture_prepare() -> HostEffectPrepareEvidence {
        HostEffectPrepareEvidence {
            execution_id: "execution-1".to_string(),
            graph_id: "graph-1".to_string(),
            node_id: 7,
            invocation_id: "invocation-1".to_string(),
            call_id: "call-1".to_string(),
            capability_id: "calendar.write".to_string(),
            host_op: "calendar.events.create".to_string(),
            capability_binding: "calendar.write".to_string(),
            implementation_ref: "host/calendar.write@1".to_string(),
            request_digest: "sha256:request-1".to_string(),
            idempotency_key: "idempotency-1".to_string(),
            grant_refs: vec!["grant-1".to_string()],
            approval_refs: vec!["approval-1".to_string()],
        }
    }

    fn signed_commit(effect_digest: &str) -> (HostEffectCommit, String) {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let verifying_key = signing_key.verifying_key();
        let mut commit = HostEffectCommit {
            execution_id: "execution-1".to_string(),
            graph_id: "graph-1".to_string(),
            node_id: 7,
            invocation_id: "invocation-1".to_string(),
            call_id: "call-1".to_string(),
            request_digest: "sha256:request-1".to_string(),
            idempotency_key: "idempotency-1".to_string(),
            effect_digest: effect_digest.to_string(),
            effect_outcome: HostEffectOutcome::Committed,
            host_key_id: "host-key-1".to_string(),
            signature: String::new(),
        };
        let signature = signing_key.sign(effect_digest.as_bytes());
        commit.signature = encode_hex(&signature.to_bytes());
        (commit, encode_hex(&verifying_key.to_bytes()))
    }

    fn fixture_receipt(
        idempotency_proof: CapabilityEffectIdempotencyProof,
    ) -> CapabilityEffectReceiptPayload {
        CapabilityEffectReceiptPayload {
            receipt_id: "receipt-1".to_string(),
            execution_id: "execution-1".to_string(),
            node_id: 7,
            invocation_id: "invocation-1".to_string(),
            capability_binding: "calendar.write".to_string(),
            dispatch_path: CapabilityEffectDispatchPath::AskTool,
            implementation_kind: CapabilityEffectImplementationKind::Host,
            implementation_ref: "host/calendar.write@1".to_string(),
            request_digest: "sha256:request-1".to_string(),
            admission_kind: CapabilityEffectAdmissionKind::Grant,
            grant_id: Some("grant-1".to_string()),
            approval_status: Some(CapabilityEffectApprovalStatus::Approved),
            approval_id: Some("approval-1".to_string()),
            idempotency_proof,
            idempotency_key_digest: capability_effect_idempotency_key_digest("idempotency-1"),
            effect_ref: "effect-ref-1".to_string(),
            status: CapabilityEffectReceiptStatus::Committed,
        }
    }

    #[test]
    fn verifies_signed_host_effect_evidence() {
        let prepare = fixture_prepare();
        let (commit, host_pubkey_hex) = signed_commit("sha256:effect-1");
        let evidence = CapabilityEffectReplayEvidence {
            receipt: fixture_receipt(CapabilityEffectIdempotencyProof::TransactionVerified),
            host_prepare: Some(prepare),
            host_commit: Some(commit),
            host_pubkey_hex: Some(host_pubkey_hex),
        };

        verify_replayable_effect(
            &evidence,
            &ExpectedCapabilityEffect {
                node_id: 7,
                capability_binding: "calendar.write",
                dispatch_path: CapabilityEffectDispatchPath::AskTool,
            },
        )
        .expect("host replay evidence must verify");
    }

    #[test]
    fn rejects_missing_prepare_evidence() {
        let (commit, host_pubkey_hex) = signed_commit("sha256:effect-1");
        let evidence = CapabilityEffectReplayEvidence {
            receipt: fixture_receipt(CapabilityEffectIdempotencyProof::TransactionVerified),
            host_prepare: None,
            host_commit: Some(commit),
            host_pubkey_hex: Some(host_pubkey_hex),
        };

        let error = verify_replayable_effect(
            &evidence,
            &ExpectedCapabilityEffect {
                node_id: 7,
                capability_binding: "calendar.write",
                dispatch_path: CapabilityEffectDispatchPath::AskTool,
            },
        )
        .expect_err("missing replay preparation evidence must fail closed");
        assert!(error.contains("HostEffectPrepareEvidence"));
    }

    #[test]
    fn metadata_lookup_matches_node_binding_and_dispatch_path() {
        let prepare = fixture_prepare();
        let (commit, host_pubkey_hex) = signed_commit("sha256:effect-1");
        let metadata = HashMap::from([(
            metadata_keys::CAPABILITY_EFFECT_REPLAY_EVIDENCE.to_string(),
            serde_json::to_string(&CapabilityEffectReplayEvidenceEnvelope {
                records: vec![CapabilityEffectReplayEvidence {
                    receipt: fixture_receipt(CapabilityEffectIdempotencyProof::TransactionVerified),
                    host_prepare: Some(prepare),
                    host_commit: Some(commit),
                    host_pubkey_hex: Some(host_pubkey_hex),
                }],
            })
            .unwrap(),
        )]);

        let evidence = lookup_replayable_effect(
            &metadata,
            &ExpectedCapabilityEffect {
                node_id: 7,
                capability_binding: "calendar.write",
                dispatch_path: CapabilityEffectDispatchPath::AskTool,
            },
        )
        .expect("lookup")
        .expect("matching evidence");

        assert_eq!(evidence.receipt.capability_binding, "calendar.write");
        assert_eq!(
            evidence.receipt.dispatch_path,
            CapabilityEffectDispatchPath::AskTool
        );
    }
}
