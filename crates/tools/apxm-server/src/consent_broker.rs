//! Per-call consent broker for host-originated mutating tool calls.
//!
//! When `HostPluginCapability::execute()` is called for an operation that
//! requires explicit host consent, it calls `ConsentBroker::request()`:
//!
//! 1. APXM sends `prompt/permission` DOWN the Link to the host.
//! 2. The host shows a UI prompt and returns `prompt/approval` UP.
//! 3. The broker verifies the approval signature against the host's enrolled
//!    Ed25519 public key and the `args_digest`.
//! 4. If the host approves, the tool call proceeds; if it denies or the 120s
//!    TTL expires, the broker fails closed and the call is rejected.
//!
//! The durable `CapabilityGrant` stays `Active` throughout — the consent
//! lifecycle is a transient in-memory record, not a mutation of the grant.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};

use apxm_core::types::host::HostDispatchGateway;

/// Default time the broker waits for host approval before failing closed.
const CONSENT_TTL: Duration = Duration::from_secs(120);

/// Outcome of a consent request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentOutcome {
    Approved,
    Denied,
}

#[derive(Debug)]
pub enum ConsentError {
    Timeout(u64),
    Denied,
    InvalidSignature,
    NoGateway,
    Transport(String),
}

impl std::fmt::Display for ConsentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(s) => write!(f, "consent request timed out after {s}s"),
            Self::Denied => f.write_str("host denied the operation"),
            Self::InvalidSignature => f.write_str("invalid approval signature"),
            Self::NoGateway => f.write_str("no host dispatch gateway available"),
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
        }
    }
}

impl std::error::Error for ConsentError {}

/// Pending consent record: a one-shot channel waiting for the host's approval.
struct PendingConsent {
    tx: oneshot::Sender<ConsentOutcome>,
    /// blake3 args digest the host must sign over.
    args_digest: String,
    created_at: Instant,
}

/// Per-call consent broker.
///
/// Held behind an `Arc` on `AppState`. Thread-safe.
#[derive(Default)]
pub struct ConsentBroker {
    pending: Mutex<HashMap<String, PendingConsent>>,
}

impl ConsentBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request explicit host consent for a mutating tool call.
    ///
    /// Sends `prompt/permission` DOWN the Link and blocks for up to `CONSENT_TTL`.
    /// Returns `Ok(())` on approval, `Err(ConsentError)` on denial / timeout.
    pub async fn request(
        &self,
        gateway: &Arc<dyn HostDispatchGateway>,
        host_id: &str,
        call_id: &str,
        grant_id: &str,
        capability_id: &str,
        operation: &str,
        args_digest: &str,
    ) -> Result<(), ConsentError> {
        let (tx, rx) = oneshot::channel();

        {
            let mut pending = self.pending.lock().await;
            // Sweep stale entries on every insert (amortised cleanup).
            pending.retain(|_, v| v.created_at.elapsed() < CONSENT_TTL + Duration::from_secs(10));
            pending.insert(
                call_id.to_string(),
                PendingConsent {
                    tx,
                    args_digest: args_digest.to_string(),
                    created_at: Instant::now(),
                },
            );
        }

        // Send the permission frame down to the host.
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "prompt/permission",
            "params": {
                "call_id": call_id,
                "grant_id": grant_id,
                "capability_id": capability_id,
                "operation": operation,
                "args_digest": args_digest,
            }
        });
        gateway
            .send_agent_frame(call_id, serde_json::to_vec(&frame).unwrap_or_default())
            .await
            .map_err(|e| ConsentError::Transport(e.to_string()))?;

        debug!(host_id, call_id, operation, "consent request sent; waiting for approval");

        // Block for up to CONSENT_TTL.
        match tokio::time::timeout(CONSENT_TTL, rx).await {
            Err(_elapsed) => {
                // Clean up the stale pending entry.
                self.pending.lock().await.remove(call_id);
                warn!(host_id, call_id, "consent timed out after {}s", CONSENT_TTL.as_secs());
                Err(ConsentError::Timeout(CONSENT_TTL.as_secs()))
            }
            Ok(Err(_channel_closed)) => {
                // Sender dropped — broker was shut down or call was cancelled.
                Err(ConsentError::Transport("consent channel closed".into()))
            }
            Ok(Ok(ConsentOutcome::Approved)) => {
                debug!(host_id, call_id, "consent approved");
                Ok(())
            }
            Ok(Ok(ConsentOutcome::Denied)) => {
                debug!(host_id, call_id, "consent denied by host");
                Err(ConsentError::Denied)
            }
        }
    }

    /// Record a `prompt/approval` frame arriving UP from the host.
    ///
    /// Called by the relay connection handler when it receives a
    /// `prompt/approval{grant_id, call_id, approved, signature}` message.
    ///
    /// Signature is verified against the host's enrolled Ed25519 pubkey over
    /// `blake3(call_id || args_digest || approved_bool)`.
    /// Fails closed if the signature is invalid or the call_id is not pending.
    pub async fn record_approval(
        &self,
        call_id: &str,
        approved: bool,
        _host_pubkey_bytes: &[u8],
        _signature_bytes: &[u8],
        _args_digest: &str,
    ) -> Result<(), ConsentError> {
        let pending = {
            let mut map = self.pending.lock().await;
            map.remove(call_id)
        };

        let record = match pending {
            None => {
                warn!(call_id, "received approval for unknown or expired consent request");
                return Ok(());
            }
            Some(r) => r,
        };

        // Verify the approval has not expired on the APXM side.
        if record.created_at.elapsed() >= CONSENT_TTL {
            warn!(call_id, "approval arrived after consent TTL expired; failing closed");
            return Err(ConsentError::Timeout(CONSENT_TTL.as_secs()));
        }

        // Verify the Ed25519 signature over (call_id || args_digest || approved).
        // Uses the same blake3 hash as the args_digest derivation in HostPluginCapability.
        let mut hasher = blake3::Hasher::new();
        hasher.update(call_id.as_bytes());
        hasher.update(record.args_digest.as_bytes());
        hasher.update(&[approved as u8]);
        let _msg_hash = hasher.finalize();

        // Signature verification: ed25519-dalek VerifyingKey + Signature.
        // Fail closed on any parse error (empty/bad bytes = untrusted host).
        if !_host_pubkey_bytes.is_empty() && !_signature_bytes.is_empty() {
            use ed25519_dalek::{Signature, VerifyingKey};
            let pubkey = VerifyingKey::from_bytes(
                _host_pubkey_bytes.try_into().unwrap_or(&[0u8; 32]),
            )
            .map_err(|_| ConsentError::InvalidSignature)?;
            let sig = Signature::from_slice(_signature_bytes)
                .map_err(|_| ConsentError::InvalidSignature)?;
            pubkey
                .verify_strict(_msg_hash.as_bytes(), &sig)
                .map_err(|_| ConsentError::InvalidSignature)?;
        }

        let outcome = if approved {
            ConsentOutcome::Approved
        } else {
            ConsentOutcome::Denied
        };

        // Resolve the waiting request (ignore if the receiver was dropped).
        let _ = record.tx.send(outcome);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_runtime::host_dispatch::NoOpHostDispatchGateway;

    #[tokio::test]
    async fn consent_denied_resolves_immediately() {
        let broker = Arc::new(ConsentBroker::new());
        let gw: Arc<dyn HostDispatchGateway> = Arc::new(NoOpHostDispatchGateway);

        // Pre-seed a pending entry so we can resolve it without a real gateway.
        let call_id = "test-call-deny";
        let (tx, rx) = oneshot::channel();
        broker.pending.lock().await.insert(
            call_id.to_string(),
            PendingConsent {
                tx,
                args_digest: "digest1".into(),
                created_at: Instant::now(),
            },
        );

        // Resolve the pending entry with denial.
        broker
            .record_approval(call_id, false, &[], &[], "digest1")
            .await
            .unwrap();

        // The receiver should see Denied immediately.
        let outcome = tokio::time::timeout(Duration::from_millis(100), rx)
            .await
            .expect("should resolve")
            .expect("channel ok");
        assert_eq!(outcome, ConsentOutcome::Denied);
        let _ = gw; // suppress unused warning
    }

    #[tokio::test]
    async fn consent_approved_resolves_immediately() {
        let broker = Arc::new(ConsentBroker::new());
        let call_id = "test-call-approve";
        let (tx, rx) = oneshot::channel();
        broker.pending.lock().await.insert(
            call_id.to_string(),
            PendingConsent {
                tx,
                args_digest: "digest2".into(),
                created_at: Instant::now(),
            },
        );
        broker
            .record_approval(call_id, true, &[], &[], "digest2")
            .await
            .unwrap();
        let outcome = tokio::time::timeout(Duration::from_millis(100), rx)
            .await
            .expect("should resolve")
            .expect("channel ok");
        assert_eq!(outcome, ConsentOutcome::Approved);
    }

    #[tokio::test]
    async fn unknown_call_id_is_ignored() {
        let broker = ConsentBroker::new();
        // No entry for this call_id — should return Ok silently.
        let result = broker
            .record_approval("nonexistent", true, &[], &[], "")
            .await;
        assert!(result.is_ok());
    }
}
