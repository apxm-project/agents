//! Interaction Client shared by headless `apxm run` and the TUI.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use apxm_kernel::event_api::{CanonicalEventRef, EventApplication, EventOccurrence};
use apxm_runtime_protocol::{
    RUNTIME_PROTOCOL_VERSION, RuntimeHandshake, RuntimeOwnerClaim, RuntimeRequest, RuntimeResult,
};
use apxm_runtime_service::{
    MAX_FRAME_BYTES, RuntimeService, StdioFrame, decode_jsonl, encode_jsonl,
};
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
    owner_claim: Option<RuntimeOwnerClaim>,
    event_owner_claim: Option<RuntimeOwnerClaim>,
}

enum RuntimeInner {
    InProcess(Box<RuntimeService>),
    Stdio(StdioRuntime),
}

struct StdioRuntime {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    artifact_dir: PathBuf,
    last_output: Option<serde_json::Value>,
}

impl Default for InteractionClient {
    fn default() -> Self {
        Self {
            inner: RuntimeInner::InProcess(Box::default()),
            owner_claim: None,
            event_owner_claim: None,
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
            .env("APXM_ARTIFACT_DIR", &artifact_dir)
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
                artifact_dir,
                last_output: None,
            }),
            owner_claim: None,
            event_owner_claim: None,
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

    /// Bind the exact admission materials for an in-process runtime instance.
    ///
    /// Stdio runtimes currently expose no admission-binding protocol request;
    /// callers must provision their admission through the runtime's owning
    /// composition boundary before invoking.
    pub fn bind_admission(
        &mut self,
        program_instance_id: &str,
        materials: apxm_runtime_service::InvocationMaterials,
    ) -> Result<(), String> {
        match &mut self.inner {
            RuntimeInner::InProcess(service) => {
                service.bind_admission(program_instance_id, materials)
            }
            RuntimeInner::Stdio(_) => {
                Err("stdio runtime admission binding is not supported".to_owned())
            }
        }
    }

    /// Invoke an explicit artifact. Never contacts Compilation Service.
    pub fn run_artifact(&mut self, artifact_digest: &str) -> Result<String, String> {
        if artifact_digest.trim().is_empty() {
            return Err("empty artifact".to_owned());
        }
        let result = self.request(RuntimeRequest::ProgramInstanceCreate {
            request_id: "run".to_owned(),
            artifact_digest: artifact_digest.to_owned(),
        })?;
        match result {
            RuntimeResult::ProgramInstanceCreated {
                program_instance_id,
                owner_claim,
                ..
            } => {
                self.owner_claim = Some(owner_claim);
                Ok(program_instance_id)
            }
            other => Err(format!("{other:?}")),
        }
    }

    /// Start one invocation on an admitted instance.
    pub fn start_invocation(
        &mut self,
        program_instance_id: &str,
        input: serde_json::Value,
    ) -> Result<RuntimeResult, String> {
        let result = self.request(RuntimeRequest::ProgramInvocationStart {
            request_id: "invoke".to_owned(),
            program_instance_id: program_instance_id.to_owned(),
            owner_claim: self
                .owner_claim
                .clone()
                .ok_or_else(|| "missing runtime owner claim".to_owned())?,
            input,
        })?;
        if let RuntimeInner::Stdio(stdio) = &mut self.inner
            && let RuntimeResult::ProgramInvocationStarted {
                ref program_invocation_id,
                ..
            } = result
        {
            let path = stdio.artifact_dir.join(format!(
                "{}.output.json",
                program_invocation_id.replace(':', "-")
            ));
            if let Ok(bytes) = std::fs::read(&path) {
                stdio.last_output = serde_json::from_slice(&bytes).ok();
            }
        }
        Ok(result)
    }

    /// Last committed output from the in-process service or the shared artifact dir.
    #[must_use]
    pub fn last_output(&self) -> Option<&serde_json::Value> {
        match &self.inner {
            RuntimeInner::InProcess(service) => service.last_output(),
            RuntimeInner::Stdio(stdio) => stdio.last_output.as_ref(),
        }
    }

    /// Inspect one EventRef through the Runtime Service.
    pub fn inspect_event(
        &mut self,
        event_id: String,
        generation: u64,
    ) -> Result<RuntimeResult, String> {
        self.request(RuntimeRequest::EventInspect {
            request_id: "event.inspect".to_owned(),
            owner_claim: self
                .event_owner_claim
                .clone()
                .ok_or_else(|| "missing event owner claim".to_owned())?,
            event_ref: CanonicalEventRef {
                event_id,
                generation,
            },
        })
    }

    /// Reserve one EventRef through the Runtime Service.
    pub fn reserve_event(&mut self, type_id: &str) -> Result<RuntimeResult, String> {
        let result = self.request(RuntimeRequest::EventReserve {
            request_id: "event.reserve".to_owned(),
            type_id: type_id.to_owned(),
        })?;
        if let RuntimeResult::EventReserved { owner_claim, .. } = &result {
            self.event_owner_claim = Some(owner_claim.clone());
        }
        Ok(result)
    }

    /// Classify a start result into the closed headless outcome set.
    #[must_use]
    pub fn classify_outcome(
        started: &RuntimeResult,
        last_output: Option<&serde_json::Value>,
    ) -> HeadlessOutcome {
        match started {
            RuntimeResult::Failed { .. } | RuntimeResult::Cancelled { .. } => {
                HeadlessOutcome::Failed
            }
            RuntimeResult::ProgramInvocationStarted { .. } => {
                if last_output.is_some_and(output_is_waiting_event) {
                    HeadlessOutcome::WaitingEvent
                } else {
                    HeadlessOutcome::Returned
                }
            }
            _ => HeadlessOutcome::Failed,
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
            owner_claim: self
                .event_owner_claim
                .clone()
                .ok_or_else(|| "missing event owner claim".to_owned())?,
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
            owner_claim: self
                .owner_claim
                .clone()
                .ok_or_else(|| "missing runtime owner claim".to_owned())?,
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
                let line = read_limited_line(&mut stdio.stdout)?;
                let reply = decode_jsonl(&line)?;
                serde_json::from_str(&reply.payload).map_err(|error| error.to_string())
            }
        }
    }
}

fn read_limited_line(reader: &mut BufReader<ChildStdout>) -> Result<String, String> {
    let mut bytes = Vec::new();
    loop {
        let (take, newline) = {
            let available = reader.fill_buf().map_err(|error| error.to_string())?;
            if available.is_empty() {
                return if bytes.is_empty() {
                    Err("runtime service closed stdout before replying".to_owned())
                } else {
                    String::from_utf8(bytes)
                        .map_err(|error| format!("runtime reply is not UTF-8: {error}"))
                };
            }
            let newline = available.iter().position(|byte| *byte == b'\n');
            let take = newline.map_or(available.len(), |index| index + 1);
            if bytes.len().saturating_add(take) > MAX_FRAME_BYTES {
                return Err(format!("JSONL frame exceeds {MAX_FRAME_BYTES} bytes"));
            }
            (take, newline.is_some())
        };
        let available = reader.fill_buf().map_err(|error| error.to_string())?;
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if newline {
            return String::from_utf8(bytes)
                .map_err(|error| format!("runtime reply is not UTF-8: {error}"));
        }
    }
}

fn output_is_waiting_event(output: &serde_json::Value) -> bool {
    output["results"]["node_outcomes"]
        .as_array()
        .is_some_and(|nodes| {
            nodes
                .iter()
                .any(|node| node["kind"] == "await.event" && node["outcome"]["status"] == "parked")
        })
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
        assert_eq!(render_outcome(HeadlessOutcome::Returned), "returned");
        assert_eq!(render_outcome(HeadlessOutcome::Failed), "failed");
    }

    #[test]
    fn classify_outcome_distinguishes_returned_waiting_and_failed() {
        let started = RuntimeResult::ProgramInvocationStarted {
            request_id: "s".to_owned(),
            program_invocation_id: "pi-1:inv-1".to_owned(),
        };
        assert_eq!(
            InteractionClient::classify_outcome(&started, None),
            HeadlessOutcome::Returned
        );
        let waiting = serde_json::json!({
            "results": {
                "node_outcomes": [{
                    "kind": "await.event",
                    "outcome": { "status": "parked" }
                }]
            }
        });
        assert_eq!(
            InteractionClient::classify_outcome(&started, Some(&waiting)),
            HeadlessOutcome::WaitingEvent
        );
        let failed = RuntimeResult::Failed {
            request_id: "s".to_owned(),
            code: "unknown_artifact".to_owned(),
        };
        assert_eq!(
            InteractionClient::classify_outcome(&failed, None),
            HeadlessOutcome::Failed
        );
    }

    #[test]
    fn compile_failure_does_not_create_an_instance() {
        let mut client = InteractionClient::default();
        assert!(client.run_artifact("sha256:not-admitted").is_err());
    }
}
