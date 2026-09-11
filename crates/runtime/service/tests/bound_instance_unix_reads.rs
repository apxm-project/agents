//! The Unix transport answers every accepted connection with a typed frame.
//!
//! An embedding host answers `capability_fulfill` for a bound Program
//! Instance and then reads the invocation on fresh connections: the
//! observation stream until the terminal observation arrives, then the
//! inspection and output that fix the program boundary. Whatever the service
//! is doing for that invocation, and whatever the host sent, a connection
//! closed without a line is a transport bug: the host can only report an
//! empty reply, never the reason.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use apxm_core::types::host_capability::HostCapabilityOutcomeKind;
use apxm_program::artifact::ExecutableArtifact;
use apxm_runtime_protocol::{
    ExecutionCursor, ExecutionObservation, GrantRef, ObservationKind, PrincipalRef,
    ProgramInvocationId, ProgramInvocationStatus, RUNTIME_PROTOCOL_VERSION, ReadContext,
    ReadPurpose, RequestId, RuntimeHandshake, RuntimeHandshakeV2, RuntimeOwnerClaim,
    RuntimeRequest, RuntimeRequestV2, RuntimeResult, RuntimeResultV2, ScopeRef,
};
use apxm_runtime_service::{
    RUNTIME_CHANNEL, RuntimeService, StdioFrame, decode_jsonl, encode_jsonl,
    materials_for_artifact, serve_unix,
};
use apxm_source_port::{
    Frontend, FrontendDrivers, FrontendRoots, SourceBundleRequest, compile_source_bundle,
};

const READ_SCOPE: &str = "scope.host-capability";
const PUMP_LIMIT: u32 = 100;
const TURN_DEADLINE: Duration = Duration::from_secs(60);

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("the runtime service sits three levels under the repository root")
        .to_path_buf()
}

/// A Session-shaped agent: every message asks the host before it yields the
/// reply, so each invocation of the bound instance parks on a host request and
/// settles as a committed yield once the host answers.
fn session_agent_artifact() -> Vec<u8> {
    let root = repository_root();
    let roots = FrontendRoots::new(
        root.join("crates/compiler/frontend/python"),
        root.join("crates/compiler/frontend/typescript"),
    );
    let drivers = FrontendDrivers::new(
        root.join(".dekk/env/bin/python"),
        root.join(".dekk/env/bin/node"),
    );
    let source = r#"import { Workflow, Capability } from "@apxm/frontend";
type Input = {message: string};
type Output = {reply: string};
const Notes = Capability<unknown, unknown>("host:notes.search");
export const Assistant = Workflow<Input, Output>({name: "Assistant", async run(agent, input) {
  while (input.message !== "") {
    await Notes({reference: input.message});
    input = await agent.yield_({reply: input.message});
  }
  return {reply: ""};
}});
"#;
    let compiled = compile_source_bundle(
        &SourceBundleRequest::new(Frontend::Typescript, "Assistant", source)
            .with_host_capabilities(["notes.search"]),
        &roots,
        &drivers,
    )
    .expect("the Session-shaped agent compiles");
    ExecutableArtifact::from_graph_and_air(&compiled.frontend_graph, &compiled.air)
        .expect("the compiled agent seals into an artifact")
        .encode()
        .expect("the artifact encodes")
}

/// One owner-published Unix endpoint serving the production dispatcher.
struct Served {
    socket: PathBuf,
    _state: tempfile::TempDir,
}

/// Create and bind one Program Instance the way an embedding host does, then
/// publish the service on its Unix endpoint. Instance creation and admission
/// binding are owner-side composition; every request below crosses the wire.
fn serve_bound_instance(artifact: &[u8]) -> (Served, String, RuntimeOwnerClaim) {
    let state = tempfile::tempdir().expect("runtime state directory");
    let mut service = RuntimeService::in_memory()
        .with_runtime_state_dir(state.path().join("state"))
        .with_embedded_read_access()
        .with_output_access_scope_ref(READ_SCOPE.to_owned());
    assert!(
        service.startup_error().is_none(),
        "{:?}",
        service.startup_error()
    );
    let digest = service
        .try_admit_artifact(artifact.to_vec())
        .expect("the artifact is admitted");
    let created = service
        .handle(
            &RuntimeHandshake {
                protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
            },
            RuntimeRequest::ProgramInstanceCreate {
                request_id: "session:instance".to_owned(),
                artifact_digest: digest,
            },
        )
        .expect("instance creation is admitted");
    let RuntimeResult::ProgramInstanceCreated {
        program_instance_id,
        owner_claim,
        ..
    } = created
    else {
        panic!("instance creation failed: {created:?}");
    };
    let release = std::fs::read(
        repository_root().join("tools/tests/fixtures/canonical-execute.release.json"),
    )
    .expect("release");
    let provenance = std::fs::read(
        repository_root().join("tools/tests/fixtures/canonical-execute.provenance.json"),
    )
    .expect("provenance");
    service
        .bind_admission(
            &program_instance_id,
            materials_for_artifact(
                artifact,
                format!("{program_instance_id}:admission"),
                release,
                provenance,
            ),
        )
        .expect("the bound admission is accepted");
    let socket = state.path().join("runtime.sock");
    let path = socket.to_string_lossy().into_owned();
    thread::spawn(move || {
        if let Err(error) = serve_unix(&path, service) {
            eprintln!("runtime Unix endpoint stopped: {error}");
        }
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while UnixStream::connect(&socket).is_err() {
        assert!(
            Instant::now() < deadline,
            "the Unix endpoint did not come up"
        );
        thread::sleep(Duration::from_millis(10));
    }
    (
        Served {
            socket,
            _state: state,
        },
        program_instance_id,
        owner_claim,
    )
}

/// What one accepted connection produced. The host's transport treats an
/// accepted-then-closed connection as an empty response.
#[derive(Debug)]
enum Reply {
    Typed(String),
    Empty,
}

fn round_trip(socket: &Path, payload: serde_json::Value) -> Reply {
    let mut stream = UnixStream::connect(socket).expect("connect to the runtime endpoint");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("client read deadline");
    let frame = StdioFrame {
        channel: RUNTIME_CHANNEL.to_owned(),
        payload: payload.to_string(),
    };
    stream
        .write_all(encode_jsonl(&frame).as_bytes())
        .expect("write request frame");
    stream.flush().expect("flush request frame");
    let mut line = String::new();
    let read = BufReader::new(stream)
        .read_line(&mut line)
        .expect("read the reply line");
    if read == 0 || line.trim().is_empty() {
        return Reply::Empty;
    }
    let frame = decode_jsonl(&line).expect("the reply is a runtime frame");
    assert_eq!(frame.channel, RUNTIME_CHANNEL);
    Reply::Typed(frame.payload)
}

fn runtime_call(socket: &Path, request: RuntimeRequest) -> RuntimeResult {
    let payload = serde_json::json!({
        "handshake": RuntimeHandshake { protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned() },
        "request": request,
    });
    match round_trip(socket, payload) {
        Reply::Typed(body) => serde_json::from_str(&body).expect("a Runtime/1 result"),
        Reply::Empty => panic!("Runtime/1 request was closed without a reply"),
    }
}

fn read_context(request_id: &str, purpose: ReadPurpose) -> ReadContext {
    ReadContext {
        request_id: RequestId::new(request_id).expect("request id"),
        scope_ref: ScopeRef::new(READ_SCOPE).expect("scope"),
        principal_ref: PrincipalRef::new("principal.host-capability").expect("principal"),
        grant_ref: GrantRef::new("grant.host-capability").expect("grant"),
        correlation_id: None,
        purpose,
    }
}

fn read_call(socket: &Path, request: RuntimeRequestV2) -> Result<RuntimeResultV2, Reply> {
    let payload = serde_json::json!({
        "handshake": RuntimeHandshakeV2::server(),
        "request": request,
    });
    match round_trip(socket, payload) {
        Reply::Typed(body) => Ok(serde_json::from_str(&body).expect("a Runtime/2 result")),
        Reply::Empty => Err(Reply::Empty),
    }
}

fn observation_page(
    socket: &Path,
    request_id: &str,
    invocation: &str,
    after_cursor: Option<ExecutionCursor>,
) -> Result<RuntimeResultV2, Reply> {
    read_call(
        socket,
        RuntimeRequestV2::ObservationSubscribe {
            context: read_context(request_id, ReadPurpose::Observation),
            program_invocation_id: ProgramInvocationId::new(invocation.to_owned())
                .expect("invocation id"),
            after_cursor,
            limit: PUMP_LIMIT,
        },
    )
}

fn invocation_status(socket: &Path, invocation: &str) -> ProgramInvocationStatus {
    let result = read_call(
        socket,
        RuntimeRequestV2::ProgramInvocationInspect {
            context: read_context("inspect", ReadPurpose::Inspection),
            program_invocation_id: ProgramInvocationId::new(invocation.to_owned())
                .expect("invocation id"),
            node_execution_id: None,
        },
    );
    match result {
        Ok(RuntimeResultV2::ProgramInvocationInspection { inspection, .. }) => inspection.status,
        other => panic!("invocation inspection returned {other:?}"),
    }
}

/// Read the complete stream from the beginning on one fresh connection.
fn all_observations(socket: &Path, invocation: &str) -> Vec<ExecutionObservation> {
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let page = match observation_page(socket, "read.all", invocation, cursor.take()) {
            Ok(RuntimeResultV2::ObservationPage { page, .. }) => page,
            other => panic!("observation read returned {other:?}"),
        };
        let has_more = page.has_more;
        cursor = page.next_cursor.clone();
        items.extend(page.items);
        if !has_more {
            return items;
        }
    }
}

fn of_kind(items: &[ExecutionObservation], kind: ObservationKind) -> Vec<&ExecutionObservation> {
    items
        .iter()
        .filter(|item| item.observation_kind == kind)
        .collect()
}

/// The host's observation pump: fresh connection per read, the page cursor
/// carried forward, polling until a committed terminal observation appears.
/// It records every read that came back without a typed frame.
struct Pump {
    stop: Arc<AtomicBool>,
    empty_replies: Arc<Mutex<Vec<String>>>,
    terminal: Arc<Mutex<Option<ObservationKind>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Pump {
    fn start(socket: PathBuf, invocation: String) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let empty_replies = Arc::new(Mutex::new(Vec::new()));
        let terminal = Arc::new(Mutex::new(None));
        let handle = {
            let stop = stop.clone();
            let empty_replies = empty_replies.clone();
            let terminal = terminal.clone();
            thread::spawn(move || {
                let mut cursor: Option<ExecutionCursor> = None;
                let mut reads = 0_u64;
                while !stop.load(Ordering::SeqCst) {
                    reads += 1;
                    let request_id = format!("pump.{reads}");
                    match observation_page(&socket, &request_id, &invocation, cursor.clone()) {
                        Ok(RuntimeResultV2::ObservationPage { page, .. }) => {
                            for item in &page.items {
                                if item.commitment == apxm_runtime_protocol::Commitment::Committed
                                    && matches!(
                                        item.observation_kind,
                                        ObservationKind::TerminalCommitted
                                            | ObservationKind::InvocationFailed
                                            | ObservationKind::InvocationCancelled
                                            | ObservationKind::OutcomeUnknown
                                    )
                                {
                                    *terminal.lock().unwrap() = Some(item.observation_kind);
                                    return;
                                }
                            }
                            if page.next_cursor.is_some() {
                                cursor = page.next_cursor;
                            }
                        }
                        Ok(RuntimeResultV2::Failed { code, .. }) => {
                            panic!("observation read {request_id} failed with {code:?}");
                        }
                        Ok(other) => panic!("observation read returned {other:?}"),
                        Err(_) => empty_replies.lock().unwrap().push(request_id),
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })
        };
        Self {
            stop,
            empty_replies,
            terminal,
            handle: Some(handle),
        }
    }

    fn wait_for_terminal(&mut self) -> Option<ObservationKind> {
        let deadline = Instant::now() + TURN_DEADLINE;
        while self.terminal.lock().unwrap().is_none() && Instant::now() < deadline {
            if self
                .handle
                .as_ref()
                .is_some_and(|handle| handle.is_finished())
            {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .expect("the observation pump thread did not panic");
        }
        *self.terminal.lock().unwrap()
    }

    fn empty_replies(&self) -> Vec<String> {
        self.empty_replies.lock().unwrap().clone()
    }
}

fn parked_request(socket: &Path, invocation: &str) -> String {
    let deadline = Instant::now() + TURN_DEADLINE;
    loop {
        let stream = all_observations(socket, invocation);
        if let Some(requested) = of_kind(&stream, ObservationKind::CapabilityRequested).last() {
            return requested
                .host_capability
                .as_ref()
                .expect("a capability_requested observation carries its request")
                .capability_request_id
                .clone();
        }
        assert!(
            Instant::now() < deadline,
            "the invocation did not publish its host request"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// One Session message: start the invocation on the bound instance, let it
/// park on its host request, settle the request over the wire while the pump
/// is reading, and require the terminal observation set to arrive.
fn drive_turn(
    served: &Served,
    instance: &str,
    owner_claim: &RuntimeOwnerClaim,
    turn: usize,
    message: &str,
) -> String {
    let started = runtime_call(
        &served.socket,
        RuntimeRequest::ProgramInvocationStart {
            request_id: format!("op-{turn}:invocation"),
            program_instance_id: instance.to_owned(),
            owner_claim: owner_claim.clone(),
            input: serde_json::json!({"message": message}),
        },
    );
    let RuntimeResult::ProgramInvocationStarted {
        program_invocation_id,
        ..
    } = started
    else {
        panic!("turn {turn} did not start: {started:?}");
    };
    let request_id = parked_request(&served.socket, &program_invocation_id);

    let mut pump = Pump::start(served.socket.clone(), program_invocation_id.clone());
    thread::sleep(Duration::from_millis(30));
    let settled = runtime_call(
        &served.socket,
        RuntimeRequest::CapabilityFulfill {
            request_id: format!("approval-{turn}"),
            owner_claim: owner_claim.clone(),
            capability_request_id: request_id.clone(),
            outcome: HostCapabilityOutcomeKind::Ok,
            output: Some("{\"matches\":1}".to_owned()),
            receipt_ref: Some(format!("receipt.turn.{turn}")),
            message: None,
        },
    );
    assert!(
        matches!(settled, RuntimeResult::CapabilitySettled { .. }),
        "turn {turn} settlement: {settled:?}"
    );
    // Reads racing the resume worker: fresh connections right behind the
    // settlement acknowledgement must all be answered.
    for burst in 0..20 {
        let result = observation_page(
            &served.socket,
            &format!("burst.{turn}.{burst}"),
            &program_invocation_id,
            None,
        );
        assert!(
            matches!(result, Ok(RuntimeResultV2::ObservationPage { .. })),
            "turn {turn} read {burst} racing the resume was not answered with a page: {result:?}"
        );
    }
    let terminal = pump.wait_for_terminal();
    let empty = pump.empty_replies();
    assert!(
        empty.is_empty(),
        "turn {turn}: {} observation reads were closed without a reply after the settlement (first: {:?})",
        empty.len(),
        empty.first()
    );
    assert_eq!(
        terminal,
        Some(ObservationKind::TerminalCommitted),
        "turn {turn}: the pump never saw the terminal observation"
    );

    let stream = all_observations(&served.socket, &program_invocation_id);
    assert_eq!(
        of_kind(&stream, ObservationKind::CapabilityRequested).len(),
        1
    );
    assert_eq!(
        of_kind(&stream, ObservationKind::CapabilitySettled).len(),
        1
    );
    assert_eq!(
        of_kind(&stream, ObservationKind::TerminalCommitted).len(),
        1
    );
    assert_eq!(
        invocation_status(&served.socket, &program_invocation_id),
        ProgramInvocationStatus::CommittedYield
    );
    program_invocation_id
}

#[test]
fn bound_instance_observation_reads_are_answered_across_host_settlement() {
    let artifact = session_agent_artifact();
    let (served, instance, owner_claim) = serve_bound_instance(&artifact);
    let first = drive_turn(&served, &instance, &owner_claim, 1, "first");
    let second = drive_turn(&served, &instance, &owner_claim, 2, "second");
    assert_ne!(first, second, "each message is its own invocation");
    // The first invocation's stream stays readable after the instance moved on.
    let stream = all_observations(&served.socket, &first);
    assert_eq!(
        of_kind(&stream, ObservationKind::TerminalCommitted).len(),
        1
    );
}

/// Send one raw payload the typed decoders cannot accept and require the
/// typed refusal the framing layer owes every accepted connection.
fn refused(socket: &Path, payload: serde_json::Value) -> serde_json::Value {
    match round_trip(socket, payload) {
        Reply::Typed(body) => serde_json::from_str(&body).expect("the refusal is JSON"),
        Reply::Empty => panic!("the runtime closed the connection without a reply"),
    }
}

/// The host's program-boundary reads after a terminal observation carry a
/// request id the Protocol/2 grammar refuses (it begins with a separator).
/// The refusal must be a typed `failed` frame that names the request, not a
/// connection closed before any line is written.
#[test]
fn a_read_with_a_refused_request_id_is_answered_with_a_typed_failure() {
    let artifact = session_agent_artifact();
    let (served, instance, _owner_claim) = serve_bound_instance(&artifact);
    let invocation = format!("{instance}:inv-00000000-0000-4000-8000-000000000000");
    for (request_id, method, purpose) in [
        (":inspection", "program_invocation.inspect", "inspection"),
        (":output", "output.read", "output"),
        (":evidence", "evidence.read", "evidence"),
        ("", "observation.subscribe", "observation"),
    ] {
        let mut request = serde_json::json!({
            "method": method,
            "context": {
                "request_id": request_id,
                "scope_ref": READ_SCOPE,
                "principal_ref": "principal.host-capability",
                "grant_ref": "grant.host-capability",
                "purpose": purpose,
            },
        });
        match method {
            "output.read" => request["output_ref"] = serde_json::json!("output.missing"),
            "program_invocation.inspect" => {
                request["program_invocation_id"] = serde_json::json!(invocation);
            }
            _ => {
                request["program_invocation_id"] = serde_json::json!(invocation);
                request["limit"] = serde_json::json!(100);
            }
        }
        let reply = refused(
            &served.socket,
            serde_json::json!({"handshake": RuntimeHandshakeV2::server(), "request": request}),
        );
        let expected_id = if request_id.is_empty() {
            "unknown"
        } else {
            request_id
        };
        assert_eq!(
            reply,
            serde_json::json!({"kind": "failed", "request_id": expected_id, "code": "invalid_request"}),
            "{method} with request id {request_id:?}"
        );
    }
}

#[test]
fn a_refused_protocol_2_handshake_is_answered_with_protocol_skew() {
    let artifact = session_agent_artifact();
    let (served, instance, _owner_claim) = serve_bound_instance(&artifact);
    let mut handshake = serde_json::to_value(RuntimeHandshakeV2::server()).unwrap();
    handshake["schema_digest"] = serde_json::json!(
        "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    );
    let reply = refused(
        &served.socket,
        serde_json::json!({
            "handshake": handshake,
            "request": {
                "method": "program_invocation.inspect",
                "context": {
                    "request_id": "skewed.read",
                    "scope_ref": READ_SCOPE,
                    "principal_ref": "principal.host-capability",
                    "grant_ref": "grant.host-capability",
                    "purpose": "inspection",
                },
                "program_invocation_id": format!("{instance}:inv-1"),
            },
        }),
    );
    assert_eq!(
        reply,
        serde_json::json!({"kind": "failed", "request_id": "skewed.read", "code": "protocol_skew"})
    );
}

#[test]
fn a_malformed_protocol_1_request_is_answered_with_a_typed_failure() {
    let artifact = session_agent_artifact();
    let (served, instance, owner_claim) = serve_bound_instance(&artifact);
    // A method the closed request set does not know.
    let reply = refused(
        &served.socket,
        serde_json::json!({
            "handshake": {"protocol_version": RUNTIME_PROTOCOL_VERSION},
            "request": {"method": "program_instance_forget", "request_id": "forget.1"},
        }),
    );
    assert_eq!(
        reply,
        serde_json::json!({"kind": "failed", "request_id": "forget.1", "code": "invalid_request"})
    );
    // A handshake the service does not admit, on a shared-path start request.
    let reply = refused(
        &served.socket,
        serde_json::json!({
            "handshake": {"protocol_version": "apxm.runtime.protocol/0"},
            "request": {
                "method": "program_invocation_start",
                "request_id": "start.skewed",
                "program_instance_id": instance,
                "owner_claim": owner_claim,
                "input": {"message": "first"},
            },
        }),
    );
    assert_eq!(
        reply,
        serde_json::json!({"kind": "failed", "request_id": "start.skewed", "code": "incompatible_version"})
    );
    // A payload that is not JSON at all still gets a typed reply.
    let mut stream = UnixStream::connect(&served.socket).expect("connect");
    stream
        .write_all(
            encode_jsonl(&StdioFrame {
                channel: RUNTIME_CHANNEL.to_owned(),
                payload: "not json".to_owned(),
            })
            .as_bytes(),
        )
        .expect("write");
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).expect("read");
    let frame = decode_jsonl(&line).expect("frame");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&frame.payload).unwrap(),
        serde_json::json!({"kind": "failed", "request_id": "unknown", "code": "invalid_request"})
    );
}

/// A long-lived observation stream whose request is refused is told so
/// before the connection ends.
#[test]
fn a_refused_observation_stream_is_answered_before_it_closes() {
    let artifact = session_agent_artifact();
    let (served, instance, _owner_claim) = serve_bound_instance(&artifact);
    let reply = refused(
        &served.socket,
        serde_json::json!({
            "handshake": RuntimeHandshakeV2::server(),
            "request": {
                "method": "observation.subscribe",
                "stream": true,
                "context": {
                    "request_id": ":stream",
                    "scope_ref": READ_SCOPE,
                    "principal_ref": "principal.host-capability",
                    "grant_ref": "grant.host-capability",
                    "purpose": "observation",
                },
                "program_invocation_id": format!("{instance}:inv-1"),
                "limit": 100,
            },
        }),
    );
    assert_eq!(
        reply,
        serde_json::json!({"kind": "failed", "request_id": ":stream", "code": "invalid_request"})
    );
}
