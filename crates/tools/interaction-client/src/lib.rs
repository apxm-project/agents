//! Interaction Client shared by headless `apxm run` and the TUI.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use apxm_kernel::event_api::{CanonicalEventRef, EventApplication, EventOccurrence};
use apxm_runtime_protocol::{
    RUNTIME_PROTOCOL_VERSION, RuntimeHandshake, RuntimeRequest, RuntimeResult,
};
use apxm_runtime_service::{RuntimeService, StdioFrame, decode_jsonl, encode_jsonl};
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
pub struct InteractionClient {
    inner: RuntimeInner,
}

enum RuntimeInner {
    InProcess(RuntimeService),
    Stdio(StdioRuntime),
}

struct StdioRuntime {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Default for InteractionClient {
    fn default() -> Self {
        Self {
            inner: RuntimeInner::InProcess(RuntimeService::default()),
        }
    }
}

impl Drop for InteractionClient {
    fn drop(&mut self) {
        if let RuntimeInner::Stdio(stdio) = &mut self.inner {
            let _ = stdio.child.kill();
            let _ = stdio.child.wait();
        }
    }
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
    /// Speak JSONL to a Runtime Service child. The parent does not construct
    /// the service handler.
    pub fn spawn_stdio(
        program: impl AsRef<Path>,
        args: &[&str],
        artifact_dir: impl AsRef<Path>,
    ) -> Result<Self, String> {
        let artifact_dir = artifact_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&artifact_dir).map_err(|error| error.to_string())?;
        let mut child = Command::new(program.as_ref())
            .args(args)
            .env("APXM_ARTIFACT_DIR", artifact_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| error.to_string())?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "runtime stdin".to_owned())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "runtime stdout".to_owned())?;
        Ok(Self {
            inner: RuntimeInner::Stdio(StdioRuntime {
                child,
                stdin,
                stdout: BufReader::new(stdout),
            }),
        })
    }

    /// Admit bytes into an in-process service. Stdio children load from the
    /// shared artifact directory instead.
    pub fn admit_artifact(&mut self, bytes: Vec<u8>) -> Option<String> {
        match &mut self.inner {
            RuntimeInner::InProcess(service) => Some(service.admit_artifact(bytes)),
            RuntimeInner::Stdio(_) => None,
        }
    }

    /// Invoke an explicit artifact. Never contacts Compilation Service.
    pub fn run_artifact(&mut self, artifact_digest: &str) -> Result<String, String> {
        if artifact_digest.trim().is_empty() {
            return Err("empty artifact".to_owned());
        }
        match self.request(RuntimeRequest::ProgramInstanceCreate {
            request_id: "run".to_owned(),
            artifact_digest: artifact_digest.to_owned(),
        })? {
            RuntimeResult::ProgramInstanceCreated {
                program_instance_id,
                ..
            } => Ok(program_instance_id),
            other => Err(format!("{other:?}")),
        }
    }

    /// Start one invocation on an admitted instance.
    pub fn start_invocation(
        &mut self,
        program_instance_id: &str,
        input: serde_json::Value,
    ) -> Result<RuntimeResult, String> {
        self.request(RuntimeRequest::ProgramInvocationStart {
            request_id: "invoke".to_owned(),
            program_instance_id: program_instance_id.to_owned(),
            input,
        })
    }

    /// Last committed output when the client owns an in-process service.
    #[must_use]
    pub fn last_output(&self) -> Option<&serde_json::Value> {
        match &self.inner {
            RuntimeInner::InProcess(service) => service.last_output(),
            RuntimeInner::Stdio(_) => None,
        }
    }

    /// Fulfill one exact EventRef. Invocation input is a different method.
    pub fn fulfill_event(
        &mut self,
        event_id: String,
        generation: u64,
        payload: serde_json::Value,
        idempotency_key: String,
    ) -> Result<RuntimeResult, String> {
        self.request(RuntimeRequest::EventFulfill {
            request_id: "event.fulfill".to_owned(),
            application: EventApplication {
                event_ref: CanonicalEventRef {
                    event_id,
                    generation,
                },
                occurrence: EventOccurrence {
                    occurrence_id: idempotency_key.clone(),
                    source_kind: "human.terminal".to_owned(),
                    mapping_digest: "client".to_owned(),
                    source_record: idempotency_key.clone(),
                    payload,
                },
                idempotency_key,
            },
        })
    }

    /// Cancel one invocation.
    pub fn cancel_invocation(
        &mut self,
        program_invocation_id: &str,
    ) -> Result<RuntimeResult, String> {
        self.request(RuntimeRequest::ProgramInvocationCancel {
            request_id: "cancel".to_owned(),
            program_invocation_id: program_invocation_id.to_owned(),
        })
    }

    /// Record disconnect without fabricating a successful stop.
    pub fn disconnect(&mut self) {
        if let RuntimeInner::InProcess(service) = &mut self.inner {
            service.disconnect();
        }
    }

    fn request(&mut self, request: RuntimeRequest) -> Result<RuntimeResult, String> {
        match &mut self.inner {
            RuntimeInner::InProcess(service) => service
                .handle(
                    &RuntimeHandshake {
                        protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
                    },
                    request,
                )
                .map_err(|error| format!("{error:?}")),
            RuntimeInner::Stdio(stdio) => {
                let envelope = serde_json::json!({
                    "handshake": {
                        "protocol_version": RUNTIME_PROTOCOL_VERSION,
                    },
                    "request": request,
                });
                let frame = StdioFrame {
                    channel: "runtime".to_owned(),
                    payload: envelope.to_string(),
                };
                stdio
                    .stdin
                    .write_all(encode_jsonl(&frame).as_bytes())
                    .map_err(|error| error.to_string())?;
                stdio.stdin.flush().map_err(|error| error.to_string())?;
                let mut line = String::new();
                stdio
                    .stdout
                    .read_line(&mut line)
                    .map_err(|error| error.to_string())?;
                let reply = decode_jsonl(&line)?;
                serde_json::from_str(&reply.payload).map_err(|error| error.to_string())
            }
        }
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
        let digest = client
            .admit_artifact(br#"{"schema_version":"apxm.air","semantic_operations":[],"structural_ir":[],"context_flow":[],"source_map":{"schema_version":"apxm.source-map","source_language":"python","node_spans":[],"region_annotations":[]}}"#.to_vec())
            .expect("in-process admit");
        assert!(client.run_artifact(&digest).is_ok());
        assert!(client.run_artifact("sha256:missing").is_err());
    }

    #[test]
    fn headless_and_tui_share_outcome_labels() {
        assert_eq!(
            render_outcome(HeadlessOutcome::WaitingEvent),
            "waiting_event"
        );
    }
}
