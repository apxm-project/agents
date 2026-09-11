//! Interaction Client shared by headless `apxm run` and the TUI.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub use apxm_kernel::event_api::EventOccurrence;
use apxm_kernel::event_api::{CanonicalEventRef, EventApplication};
pub use apxm_runtime_protocol::{
    ContentReadResult, CorrelationId, EvidenceRecord, ExecutionCursor, ExecutionObservation,
    ExecutionPage, GrantRef, OutputRef, PrincipalRef, ProgramInvocationId,
    ProgramInvocationInspection, ProgramInvocationStatus, RUNTIME_PROTOCOL_VERSION, ReadContext,
    ReadPurpose, RequestId, RuntimeAdmissionProfileDescriptor, RuntimeExecutionAdmissionHandshake,
    RuntimeExecutionAdmissionRequest, RuntimeHandshake, RuntimeHandshakeV2, RuntimeOwnerClaim,
    RuntimeRequest, RuntimeRequestV2, RuntimeResult, RuntimeResultV2, ScopeRef,
};
use apxm_runtime_service::{
    MAX_FRAME_BYTES, RuntimeService, StdioFrame, decode_jsonl, encode_jsonl,
};
use serde::{Deserialize, Serialize};

/// Versioned client interaction record under `.apxm/client/`.
pub const CLIENT_RECORD_CONTRACT: &str = "apxm.client-interaction/1";

/// Caller-owned identity binding transported to an owner-composed stdio
/// Runtime Service.  The service compares every V2 read context to these
/// typed refs; it does not interpret the grant or implement product policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadAuthorizationTransport {
    pub scope_ref: ScopeRef,
    pub principal_ref: PrincipalRef,
    pub grant_ref: GrantRef,
    pub correlation_id: Option<CorrelationId>,
}

impl ReadAuthorizationTransport {
    pub fn new(
        scope_ref: impl Into<String>,
        principal_ref: impl Into<String>,
        grant_ref: impl Into<String>,
        correlation_id: Option<String>,
    ) -> Result<Self, String> {
        Ok(Self {
            scope_ref: ScopeRef::new(scope_ref.into()).map_err(|error| error.to_string())?,
            principal_ref: PrincipalRef::new(principal_ref.into())
                .map_err(|error| error.to_string())?,
            grant_ref: GrantRef::new(grant_ref.into()).map_err(|error| error.to_string())?,
            correlation_id: correlation_id
                .map(CorrelationId::new)
                .transpose()
                .map_err(|error| error.to_string())?,
        })
    }

    /// Read the explicit owner-composition binding used for a stdio child.
    /// Missing fields are an error; callers must opt into the binding rather
    /// than accidentally falling back to an allow-all identity.
    pub fn from_env() -> Result<Self, String> {
        let scope = std::env::var("APXM_RUNTIME_READ_SCOPE_REF")
            .map_err(|_| "APXM_RUNTIME_READ_SCOPE_REF is required".to_owned())?;
        let principal = std::env::var("APXM_RUNTIME_READ_PRINCIPAL_REF")
            .map_err(|_| "APXM_RUNTIME_READ_PRINCIPAL_REF is required".to_owned())?;
        let grant = std::env::var("APXM_RUNTIME_READ_GRANT_REF")
            .map_err(|_| "APXM_RUNTIME_READ_GRANT_REF is required".to_owned())?;
        Self::new(
            scope,
            principal,
            grant,
            std::env::var("APXM_RUNTIME_READ_CORRELATION_REF").ok(),
        )
    }
}

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
    /// Program Invocation id used for canonical read/resume requests.
    pub program_invocation_id: String,
}

impl ClientInteractionRecord {
    /// Reject unsupported client-record formats.
    pub fn decode(value: &serde_json::Value) -> Result<Self, String> {
        let record: Self = match serde_json::from_value(value.clone()) {
            Ok(record) => record,
            Err(_) => return Err("unsupported-format".to_owned()),
        };
        if record.contract != CLIENT_RECORD_CONTRACT {
            return Err("unsupported-format".to_owned());
        }
        if ProgramInvocationId::new(record.program_invocation_id.clone()).is_err() {
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
    admission_profile: Option<RuntimeAdmissionProfileDescriptor>,
}

enum RuntimeInner {
    InProcess(Box<RuntimeService>),
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
            inner: RuntimeInner::InProcess(Box::new(RuntimeService::in_memory())),
            owner_claim: None,
            event_owner_claim: None,
            admission_profile: None,
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

impl InteractionClient {
    /// Speak JSONL to a Runtime Service child. The parent does not construct
    /// the service handler.
    pub fn spawn_stdio(
        program: impl AsRef<Path>,
        args: &[&str],
        artifact_dir: impl AsRef<Path>,
    ) -> Result<Self, String> {
        Self::spawn_stdio_with_read_authorization(program, args, artifact_dir, None)
    }

    /// Speak JSONL to a Runtime Service with an explicit caller read binding.
    /// Omitting the binding preserves the service's deny-by-default behavior.
    pub fn spawn_stdio_with_read_authorization(
        program: impl AsRef<Path>,
        args: &[&str],
        artifact_dir: impl AsRef<Path>,
        authorization: Option<ReadAuthorizationTransport>,
    ) -> Result<Self, String> {
        let artifact_dir = artifact_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&artifact_dir).map_err(|error| error.to_string())?;
        // The child process must receive an explicit stable owner-local state
        // directory. It must never invent ephemeral commit state on startup.
        let runtime_state_dir = artifact_dir.join(".apxm-runtime-state");
        std::fs::create_dir_all(&runtime_state_dir).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        std::fs::set_permissions(
            &runtime_state_dir,
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .map_err(|error| error.to_string())?;
        let mut command = Command::new(program.as_ref());
        command
            .args(args)
            .env("APXM_ARTIFACT_DIR", &artifact_dir)
            .env("APXM_RUNTIME_STATE_DIR", &runtime_state_dir);
        if let Some(authorization) = authorization {
            command
                .env(
                    "APXM_RUNTIME_READ_SCOPE_REF",
                    authorization.scope_ref.as_str(),
                )
                .env(
                    "APXM_RUNTIME_READ_PRINCIPAL_REF",
                    authorization.principal_ref.as_str(),
                )
                .env(
                    "APXM_RUNTIME_READ_GRANT_REF",
                    authorization.grant_ref.as_str(),
                );
            if let Some(correlation_id) = authorization.correlation_id {
                command.env("APXM_RUNTIME_READ_CORRELATION_REF", correlation_id.as_str());
            }
        }
        let mut child = command
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
            owner_claim: None,
            event_owner_claim: None,
            admission_profile: None,
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

    /// Bind the exact admission materials for a runtime instance. The same
    /// typed handoff is used for in-process and stdio runtimes; omitting it
    /// keeps the Runtime Service fail-closed at invocation start.
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
                let _ = (program_instance_id, materials);
                Err("standalone Runtime requires bind_admission_profile with its image-owned profile_ref".to_owned())
            }
        }
    }

    /// Bind the image-owned admission profile through the standalone Runtime
    /// protocol. Release/provenance carriers never cross this boundary.
    pub fn bind_admission_profile(
        &mut self,
        program_instance_id: &str,
        admission_profile_ref: &str,
    ) -> Result<(), String> {
        let result = self.request_execution_admission(
            RuntimeExecutionAdmissionRequest::ProgramInstanceBindAdmission {
                request_id: "bind".to_owned(),
                program_instance_id: program_instance_id.to_owned(),
                owner_claim: self
                    .owner_claim
                    .clone()
                    .ok_or_else(|| "missing runtime owner claim".to_owned())?,
                admission_profile_ref: admission_profile_ref.to_owned(),
            },
        )?;
        match result {
            RuntimeResult::ProgramInstanceAdmissionBound { .. } => Ok(()),
            other => Err(format!("{other:?}")),
        }
    }

    /// Bind the exact profile advertised by the most recent instance
    /// creation. This keeps callers from hardcoding Runtime descriptor
    /// digests or profile references.
    pub fn bind_admission_profile_from_creation(
        &mut self,
        program_instance_id: &str,
    ) -> Result<(), String> {
        let profile_ref = self
            .admission_profile
            .as_ref()
            .map(|profile| profile.profile_ref.clone())
            .ok_or_else(|| "runtime did not advertise an admission profile".to_owned())?;
        self.bind_admission_profile(program_instance_id, &profile_ref)
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
                admission_profile,
                ..
            } => {
                self.owner_claim = Some(owner_claim);
                self.admission_profile = admission_profile;
                Ok(program_instance_id)
            }
            other => Err(format!("{other:?}")),
        }
    }

    /// Profile descriptor returned by the last successful instance creation.
    /// The carrier bytes remain a host/image composition responsibility.
    #[must_use]
    pub fn admission_profile(&self) -> Option<&RuntimeAdmissionProfileDescriptor> {
        self.admission_profile.as_ref()
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
        Ok(result)
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

    /// List pending EventRefs authorized by the exact reservation claim.
    pub fn list_events(&mut self, owner_claim: RuntimeOwnerClaim) -> Result<RuntimeResult, String> {
        self.request(RuntimeRequest::EventList {
            request_id: "event.list".to_owned(),
            owner_claim,
        })
    }

    /// Bind the exact reservation claim supplied by the Event authority.
    pub fn set_event_owner_claim(&mut self, owner_claim: RuntimeOwnerClaim) -> Result<(), String> {
        owner_claim
            .validate()
            .map_err(|error| format!("invalid event owner claim: {error:?}"))?;
        self.event_owner_claim = Some(owner_claim);
        Ok(())
    }

    /// Reserve one typed EventRef for the exact admitted instance. The caller
    /// retains its request identity to retry without minting another target.
    pub fn reserve_event(
        &mut self,
        request_id: &str,
        program_instance_id: &str,
        type_id: &str,
    ) -> Result<RuntimeResult, String> {
        let result = self.request(RuntimeRequest::EventReserve {
            request_id: request_id.to_owned(),
            program_instance_id: program_instance_id.to_owned(),
            owner_claim: self
                .owner_claim
                .clone()
                .ok_or("instance owner claim is unavailable")?,
            type_id: type_id.to_owned(),
        })?;
        if let RuntimeResult::EventReserved { owner_claim, .. } = &result {
            self.event_owner_claim = Some(owner_claim.clone());
        }
        Ok(result)
    }

    /// Construct caller-supplied read authorization context.
    pub fn read_context(
        request_id: &str,
        scope_ref: &str,
        principal_ref: &str,
        grant_ref: &str,
        purpose: ReadPurpose,
    ) -> Result<ReadContext, String> {
        Self::read_context_with_correlation(
            request_id,
            scope_ref,
            principal_ref,
            grant_ref,
            None,
            purpose,
        )
    }

    /// Construct a caller-supplied read context including its optional
    /// composition correlation reference.
    pub fn read_context_with_correlation(
        request_id: &str,
        scope_ref: &str,
        principal_ref: &str,
        grant_ref: &str,
        correlation_id: Option<&str>,
        purpose: ReadPurpose,
    ) -> Result<ReadContext, String> {
        Ok(ReadContext {
            request_id: RequestId::new(request_id).map_err(|error| error.to_string())?,
            scope_ref: ScopeRef::new(scope_ref).map_err(|error| error.to_string())?,
            principal_ref: PrincipalRef::new(principal_ref).map_err(|error| error.to_string())?,
            grant_ref: GrantRef::new(grant_ref).map_err(|error| error.to_string())?,
            correlation_id: correlation_id
                .map(CorrelationId::new)
                .transpose()
                .map_err(|error| error.to_string())?,
            purpose,
        })
    }

    /// Read committed invocation inspection, whose status is authoritative.
    pub fn inspect_invocation(
        &mut self,
        context: ReadContext,
        program_invocation_id: ProgramInvocationId,
    ) -> Result<ProgramInvocationInspection, String> {
        match self.request_v2(RuntimeRequestV2::ProgramInvocationInspect {
            context,
            program_invocation_id,
            node_execution_id: None,
        })? {
            RuntimeResultV2::ProgramInvocationInspection { inspection, .. } => Ok(inspection),
            RuntimeResultV2::Failed { code, .. } => Err(format!("{code:?}")),
            other => Err(format!("unexpected inspection result: {other:?}")),
        }
    }

    /// Read the exact durable inspection record for one dynamic node
    /// execution. The service validates that the node belongs to the
    /// invocation before returning its typed details.
    pub fn inspect_node_execution(
        &mut self,
        context: ReadContext,
        program_invocation_id: ProgramInvocationId,
        node_execution_id: apxm_runtime_protocol::NodeExecutionId,
    ) -> Result<apxm_runtime_protocol::NodeExecutionInspection, String> {
        match self.request_v2(RuntimeRequestV2::ProgramInvocationInspect {
            context,
            program_invocation_id,
            node_execution_id: Some(node_execution_id),
        })? {
            RuntimeResultV2::NodeExecutionInspection { inspection, .. } => Ok(inspection),
            RuntimeResultV2::Failed { code, .. } => Err(format!("{code:?}")),
            other => Err(format!("unexpected node inspection result: {other:?}")),
        }
    }

    /// Read a bounded, reconnectable observation page. This stream is
    /// non-authoritative and never manufactures a terminal state.
    pub fn subscribe_observations(
        &mut self,
        context: ReadContext,
        program_invocation_id: ProgramInvocationId,
        after_cursor: Option<ExecutionCursor>,
        limit: u32,
    ) -> Result<ExecutionPage<ExecutionObservation>, String> {
        match self.request_v2(RuntimeRequestV2::ObservationSubscribe {
            context,
            program_invocation_id,
            after_cursor,
            limit,
        })? {
            RuntimeResultV2::ObservationPage { page, .. } => Ok(page),
            RuntimeResultV2::Failed { code, .. } => Err(format!("{code:?}")),
            other => Err(format!("unexpected observation result: {other:?}")),
        }
    }

    /// Read committed content only through its opaque reference.
    pub fn read_content(
        &mut self,
        context: ReadContext,
        content_ref: apxm_runtime_protocol::ContentRef,
    ) -> Result<ContentReadResult, String> {
        match self.request_v2(RuntimeRequestV2::ContentRead {
            context,
            content_ref,
        })? {
            RuntimeResultV2::Content { content, .. } => Ok(content),
            RuntimeResultV2::Failed { code, .. } => Err(format!("{code:?}")),
            other => Err(format!("unexpected content result: {other:?}")),
        }
    }

    /// Read one committed Session Output through its typed opaque reference.
    pub fn read_output(
        &mut self,
        context: ReadContext,
        output_ref: OutputRef,
    ) -> Result<ContentReadResult, String> {
        match self.request_v2(RuntimeRequestV2::OutputRead {
            context,
            output_ref,
        })? {
            RuntimeResultV2::Output { output, .. } => Ok(output),
            RuntimeResultV2::Failed { code, .. } => Err(format!("{code:?}")),
            other => Err(format!("unexpected output result: {other:?}")),
        }
    }

    /// Read committed evidence pages by cursor.
    pub fn read_evidence(
        &mut self,
        context: ReadContext,
        program_invocation_id: ProgramInvocationId,
        after_cursor: Option<ExecutionCursor>,
        limit: u32,
    ) -> Result<ExecutionPage<EvidenceRecord>, String> {
        match self.request_v2(RuntimeRequestV2::EvidenceRead {
            context,
            program_invocation_id,
            after_cursor,
            limit,
        })? {
            RuntimeResultV2::EvidencePage { page, .. } => Ok(page),
            RuntimeResultV2::Failed { code, .. } => Err(format!("{code:?}")),
            other => Err(format!("unexpected evidence result: {other:?}")),
        }
    }

    /// Fulfill one exact EventRef. Invocation input is a different method.
    pub fn fulfill_event(
        &mut self,
        event_id: String,
        generation: u64,
        occurrence: EventOccurrence<serde_json::Value>,
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
                occurrence,
                idempotency_key,
            },
        })
    }

    /// Expire one exact pending EventRef.
    pub fn expire_event(
        &mut self,
        event_id: String,
        generation: u64,
    ) -> Result<RuntimeResult, String> {
        self.request(RuntimeRequest::EventExpire {
            request_id: "event.expire".to_owned(),
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

    /// Cancel one exact pending EventRef.
    pub fn cancel_event(
        &mut self,
        event_id: String,
        generation: u64,
    ) -> Result<RuntimeResult, String> {
        self.request(RuntimeRequest::EventCancel {
            request_id: "event.cancel".to_owned(),
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

    fn request_execution_admission(
        &mut self,
        request: RuntimeExecutionAdmissionRequest,
    ) -> Result<RuntimeResult, String> {
        match &mut self.inner {
            RuntimeInner::InProcess(service) => Ok(service.handle_execution_admission(
                &RuntimeExecutionAdmissionHandshake::server(),
                request,
            )),
            RuntimeInner::Stdio(stdio) => {
                let envelope = serde_json::json!({
                    "handshake": RuntimeExecutionAdmissionHandshake::server(),
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

    fn request_v2(&mut self, request: RuntimeRequestV2) -> Result<RuntimeResultV2, String> {
        match &mut self.inner {
            RuntimeInner::InProcess(service) => service
                .handle_v2(&RuntimeHandshakeV2::server(), request)
                .map_err(|error| format!("{error:?}")),
            RuntimeInner::Stdio(stdio) => {
                let envelope = serde_json::json!({
                    "handshake": RuntimeHandshakeV2::server(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_client_records_are_unsupported() {
        let old = serde_json::json!({"contract": CLIENT_RECORD_CONTRACT});
        assert_eq!(
            ClientInteractionRecord::decode(&old).unwrap_err(),
            "unsupported-format"
        );
    }

    #[test]
    fn run_artifact_does_not_accept_empty_digest() {
        let mut client = InteractionClient::default();
        assert!(client.run_artifact(" ").is_err());
        let air: apxm_program::air::AirModule = serde_json::from_slice(br#"{"schema_version":"apxm.air","semantic_operations":[],"structural_ir":[],"context_flow":[],"source_map":{"schema_version":"apxm.source-map","source_language":"python","node_spans":[],"region_annotations":[]}}"#)
            .expect("minimal canonical AIR");
        let artifact = apxm_program::ExecutableArtifact::from_air(&air)
            .expect("seal canonical executable artifact");
        let digest = client
            .admit_artifact(artifact.encode().expect("canonical artifact encoding"))
            .expect("in-process admit");
        assert!(client.run_artifact(&digest).is_ok());
        assert!(client.run_artifact("sha256:missing").is_err());
    }

    #[test]
    fn compile_failure_does_not_create_an_instance() {
        let mut client = InteractionClient::default();
        assert!(client.run_artifact("sha256:not-admitted").is_err());
    }
}
