//! Interaction Client shared by headless `apxm run` and the TUI.

use apxm_kernel::event_api::{CanonicalEventRef, EventApplication, EventOccurrence};
use apxm_runtime_protocol::{
    RUNTIME_PROTOCOL_VERSION, RuntimeHandshake, RuntimeRequest, RuntimeResult,
};
use apxm_runtime_service::RuntimeService;
use serde::{Deserialize, Serialize};

/// Versioned client interaction record under `.apxm/client/`.
pub const CLIENT_RECORD_CONTRACT: &str = "apxm.client-interaction/1";

/// Client-side references used to reopen a UI. Not Program Context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientInteractionRecord {
    /// Frozen client record contract.
    pub contract: String,
    /// Admitted artifact digest.
    pub artifact_digest: String,
    /// Program Instance id when one exists.
    pub program_instance_id: Option<String>,
}

impl ClientInteractionRecord {
    /// Reject old chat/rollout/session payloads.
    pub fn decode(value: &serde_json::Value) -> Result<Self, String> {
        let record: Self = match serde_json::from_value(value.clone()) {
            Ok(record) => record,
            Err(_) => return Err("unsupported-format".to_owned()),
        };
        if record.contract != CLIENT_RECORD_CONTRACT {
            return Err("unsupported-format".to_owned());
        }
        Ok(record)
    }
}

/// Headless and TUI runtime client.
#[derive(Default)]
pub struct InteractionClient {
    service: RuntimeService,
}

/// Distinct headless exit classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadlessOutcome {
    /// Committed return.
    Returned,
    /// Waiting on an EventRef.
    WaitingEvent,
    /// Protocol or admission failure.
    Failed,
}

impl InteractionClient {
    /// Invoke an explicit artifact. Never contacts Compilation Service.
    pub fn run_artifact(&mut self, artifact_digest: &str) -> Result<String, String> {
        if artifact_digest.trim().is_empty() {
            return Err("empty artifact".to_owned());
        }
        match self
            .service
            .handle(
                &RuntimeHandshake {
                    protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
                },
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "run".to_owned(),
                    artifact_digest: artifact_digest.to_owned(),
                },
            )
            .map_err(|error| format!("{error:?}"))?
        {
            RuntimeResult::ProgramInstanceCreated {
                program_instance_id,
                ..
            } => Ok(program_instance_id),
            other => Err(format!("{other:?}")),
        }
    }

    /// Fulfill one exact EventRef. Invocation input is a different method.
    pub fn fulfill_event(
        &mut self,
        event_ref: CanonicalEventRef,
        payload: serde_json::Value,
        idempotency_key: String,
    ) -> Result<RuntimeResult, String> {
        self.service
            .handle(
                &RuntimeHandshake {
                    protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
                },
                RuntimeRequest::EventFulfill {
                    request_id: "event.fulfill".to_owned(),
                    application: EventApplication {
                        event_ref,
                        occurrence: EventOccurrence {
                            occurrence_id: idempotency_key.clone(),
                            source_kind: "human.terminal".to_owned(),
                            mapping_digest: "client".to_owned(),
                            source_record: idempotency_key.clone(),
                            payload,
                        },
                        idempotency_key,
                    },
                },
            )
            .map_err(|error| format!("{error:?}"))
    }
}

/// Render one TUI/headless protocol fixture the same way.
#[must_use]
pub fn render_outcome(kind: HeadlessOutcome) -> &'static str {
    match kind {
        HeadlessOutcome::Returned => "returned",
        HeadlessOutcome::WaitingEvent => "waiting_event",
        HeadlessOutcome::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_session_payloads_are_unsupported() {
        let old = serde_json::json!({"contract": "apxm.session/0", "messages": []});
        assert_eq!(
            ClientInteractionRecord::decode(&old).unwrap_err(),
            "unsupported-format"
        );
    }

    #[test]
    fn run_artifact_does_not_accept_empty_digest() {
        let mut client = InteractionClient::default();
        assert!(client.run_artifact(" ").is_err());
        assert!(client.run_artifact("artifact:abc").is_ok());
    }

    #[test]
    fn headless_and_tui_share_outcome_labels() {
        assert_eq!(
            render_outcome(HeadlessOutcome::WaitingEvent),
            "waiting_event"
        );
    }
}
