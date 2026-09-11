//! Runtime Service stdio and Unix-socket framing.

use std::io::{BufRead, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{MAX_ACTIVE_INVOCATIONS, PreparedInvocation, PreparedResume, RuntimeService};
use apxm_runtime_protocol::{
    RUNTIME_EXECUTION_ADMISSION_VERSION, RUNTIME_PROTOCOL_V2_VERSION,
    RuntimeExecutionAdmissionHandshake, RuntimeExecutionAdmissionRequest, RuntimeHandshake,
    RuntimeHandshakeV2, RuntimeRequest, RuntimeRequestV2, RuntimeResult,
    capability_fulfillment_is_well_formed,
};

/// Maximum encoded JSONL frame, including its line terminator.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
/// The only channel accepted by the Runtime Service framing boundary.
pub const RUNTIME_CHANNEL: &str = "runtime";
/// Per-connection I/O deadline for local Unix clients.
pub const UNIX_IO_TIMEOUT_MS: u64 = 5_000;
/// Maximum requests served on one local connection before it is drained.
pub const MAX_FRAMES_PER_CONNECTION: usize = 256;
/// Maximum concurrently active local Unix connections, including streams.
pub const MAX_ACTIVE_UNIX_CONNECTIONS: usize = 64;
const INVOCATION_WORKERS: usize = 4;
const RECOVERY_POLL_INTERVAL: Duration = Duration::from_millis(10);
const SOCKET_MODE: u32 = 0o600;

struct UnixConnectionPermit(Arc<AtomicUsize>);

impl UnixConnectionPermit {
    fn try_acquire(active: Arc<AtomicUsize>) -> Option<Self> {
        let mut current = active.load(Ordering::Acquire);
        loop {
            if current >= MAX_ACTIVE_UNIX_CONNECTIONS {
                return None;
            }
            match active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(Self(active)),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for UnixConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One stdio JSONL frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StdioFrame {
    /// Channel this frame belongs to.
    pub channel: String,
    /// UTF-8 JSON payload.
    pub payload: String,
}

/// Handshake plus request carried in one JSONL payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    handshake: RuntimeHandshake,
    request: RuntimeRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvelopeV2 {
    handshake: RuntimeHandshakeV2,
    request: RuntimeRequestV2,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionAdmissionEnvelope {
    handshake: RuntimeExecutionAdmissionHandshake,
    request: RuntimeExecutionAdmissionRequest,
}

/// Local Unix socket endpoint identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnixEndpoint {
    /// Absolute socket path owned by this service.
    pub path: String,
}

impl UnixEndpoint {
    /// Reject empty or relative paths.
    pub fn new(path: impl Into<String>) -> Result<Self, String> {
        let path = path.into();
        if !path.starts_with('/') {
            return Err("unix socket path must be absolute".to_owned());
        }
        Ok(Self { path })
    }
}

/// Encode one frame as JSONL.
#[must_use]
pub fn encode_jsonl(frame: &StdioFrame) -> String {
    format!(
        "{}\n",
        serde_json::to_string(frame).expect("frame is closed")
    )
}

/// Decode one JSONL line.
pub fn decode_jsonl(line: &str) -> Result<StdioFrame, String> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(format!("JSONL frame exceeds {MAX_FRAME_BYTES} bytes"));
    }
    serde_json::from_str(line.trim()).map_err(|error| error.to_string())
}

/// Serve Runtime protocol frames until stdin EOF. Logs never share this stream.
pub fn serve_stdio<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    mut service: RuntimeService,
) -> Result<(), String> {
    serve_frames(reader, writer, &mut service)
}

/// Bind an absolute Unix socket and serve concurrent local connections.
pub fn serve_unix(path: &str, service: RuntimeService) -> Result<(), String> {
    let endpoint = UnixEndpoint::new(path)?;
    let listener = bind_secure_unix(&endpoint.path)?;
    let shared = Arc::new(Mutex::new(service));
    shared
        .lock()
        .map_err(|_| "runtime service lock poisoned")?
        .reconcile_recovery_state()?;
    let dispatcher = InvocationDispatcher::new(shared.clone());
    dispatcher.recover_available()?;
    let active_connections = Arc::new(AtomicUsize::new(0));
    for incoming in listener.incoming() {
        let stream = incoming.map_err(|error| error.to_string())?;
        let Some(permit) = UnixConnectionPermit::try_acquire(active_connections.clone()) else {
            continue;
        };
        stream
            .set_read_timeout(Some(Duration::from_millis(UNIX_IO_TIMEOUT_MS)))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_millis(UNIX_IO_TIMEOUT_MS)))
            .map_err(|error| error.to_string())?;
        let reader =
            std::io::BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
        let shared = shared.clone();
        let dispatcher = dispatcher.clone();
        std::thread::spawn(move || {
            let _permit = permit;
            let _ = serve_shared_frames(reader, stream, shared, dispatcher);
        });
    }
    Ok(())
}

#[derive(Clone)]
pub struct InvocationDispatcher {
    service: Arc<Mutex<RuntimeService>>,
    sender: SyncSender<QueuedInvocation>,
    recovery_preference: Arc<Mutex<bool>>,
}

struct QueuedInvocation {
    work: InvocationWork,
    gate: StartGate,
}

impl QueuedInvocation {
    fn invocation_id(&self) -> String {
        match &self.work {
            InvocationWork::Start(prepared) => prepared.invocation_id.clone(),
            InvocationWork::Resume(prepared) => prepared.invocation_id.clone(),
        }
    }
}

enum InvocationWork {
    Start(Box<PreparedInvocation>),
    Resume(Box<PreparedResume>),
}

/// Prevent a queued worker from crossing the durable running marker until the
/// service has attempted to flush the admission acknowledgement. A failed
/// flush still releases accepted work so a disconnected client cannot strand
/// a durable pending invocation.
#[derive(Clone)]
struct StartGate(Arc<(Mutex<bool>, Condvar)>);

impl StartGate {
    fn new() -> Self {
        Self(Arc::new((Mutex::new(false), Condvar::new())))
    }

    fn release(&self) {
        let (released, signal) = self.0.as_ref();
        if let Ok(mut released) = released.lock() {
            *released = true;
            signal.notify_one();
        }
    }

    fn wait(&self) {
        let (released, signal) = self.0.as_ref();
        let mut released = released.lock().expect("invocation start gate lock");
        while !*released {
            released = signal.wait(released).expect("invocation start gate lock");
        }
    }
}

fn claim_recovery_work(
    service: &Arc<Mutex<RuntimeService>>,
    recovery_preference: &Arc<Mutex<bool>>,
) -> Result<Option<QueuedInvocation>, String> {
    let mut prefer_resume = recovery_preference
        .lock()
        .map_err(|_| "runtime recovery preference lock poisoned")?;
    let mut guard = service
        .lock()
        .map_err(|_| "runtime service lock poisoned")?;
    let work = if *prefer_resume {
        match guard.claim_next_continuation_resume()? {
            Some(resume) => Some(InvocationWork::Resume(Box::new(resume))),
            None => guard
                .claim_next_pending_invocation()?
                .map(|start| InvocationWork::Start(Box::new(start))),
        }
    } else {
        match guard.claim_next_pending_invocation()? {
            Some(start) => Some(InvocationWork::Start(Box::new(start))),
            None => guard
                .claim_next_continuation_resume()?
                .map(|resume| InvocationWork::Resume(Box::new(resume))),
        }
    };
    let Some(work) = work else {
        return Ok(None);
    };
    *prefer_resume = matches!(work, InvocationWork::Start(_));
    let gate = StartGate::new();
    gate.release();
    Ok(Some(QueuedInvocation { work, gate }))
}

fn execute_invocation_work(service: &Arc<Mutex<RuntimeService>>, queued: QueuedInvocation) {
    queued.gate.wait();
    match queued.work {
        InvocationWork::Start(prepared) => {
            let begin = service
                .lock()
                .map_err(|_| "runtime service lock poisoned".to_owned())
                .and_then(|mut guard| guard.begin_invocation(&prepared.invocation_id));
            if !matches!(begin, Ok(true)) {
                if let Ok(mut guard) = service.lock() {
                    guard.release_invocation_claim(&prepared.invocation_id);
                }
                return;
            }
            let execution =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepared.execute()))
                    .unwrap_or_else(|_| Err("outcome_unknown".to_owned()));
            if let Ok(mut guard) = service.lock() {
                guard.finish_invocation(&prepared, execution);
            }
        }
        InvocationWork::Resume(prepared) => {
            let begin = service
                .lock()
                .map_err(|_| "runtime service lock poisoned".to_owned())
                .and_then(|mut guard| guard.begin_continuation_resume(&prepared));
            if !matches!(begin, Ok(true)) {
                if let Ok(mut guard) = service.lock() {
                    guard.release_invocation_claim(&prepared.invocation_id);
                }
                return;
            }
            let execution =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepared.execute()))
                    .unwrap_or_else(|_| Err("outcome_unknown".to_owned()));
            if let Ok(mut guard) = service.lock() {
                guard.finish_continuation_resume(&prepared, execution);
            }
        }
    }
}

impl InvocationDispatcher {
    /// Start the same bounded workers used by native transports for an
    /// embedding transport. Dropping the last dispatcher closes its queue;
    /// accepted durable work is drained before workers exit.
    pub fn start(service: Arc<Mutex<RuntimeService>>) -> Result<Self, String> {
        service
            .lock()
            .map_err(|_| "runtime service lock poisoned")?
            .reconcile_recovery_state()?;
        let dispatcher = Self::new(service);
        dispatcher.recover_available()?;
        Ok(dispatcher)
    }

    fn new(service: Arc<Mutex<RuntimeService>>) -> Self {
        let (sender, receiver) = sync_channel::<QueuedInvocation>(MAX_ACTIVE_INVOCATIONS);
        let receiver = Arc::new(Mutex::new(receiver));
        let recovery_preference = Arc::new(Mutex::new(false));
        for worker in 0..INVOCATION_WORKERS {
            let receiver = receiver.clone();
            let service = service.clone();
            let recovery_preference = recovery_preference.clone();
            std::thread::Builder::new()
                .name(format!("apxm-invocation-{worker}"))
                .spawn(move || {
                    loop {
                        let queued = loop {
                            let received = receiver
                                .lock()
                                .expect("invocation queue lock")
                                .recv_timeout(RECOVERY_POLL_INTERVAL);
                            match received {
                                Ok(queued) => break queued,
                                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                    if let Ok(Some(recovered)) =
                                        claim_recovery_work(&service, &recovery_preference)
                                    {
                                        break recovered;
                                    }
                                }
                                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                    match claim_recovery_work(&service, &recovery_preference) {
                                        Ok(Some(recovered)) => break recovered,
                                        Ok(None) | Err(_) => return,
                                    }
                                }
                            }
                        };
                        execute_invocation_work(&service, queued);
                    }
                })
                .expect("bounded invocation worker starts");
        }
        Self {
            service,
            sender,
            recovery_preference,
        }
    }

    fn recover_available(&self) -> Result<(), String> {
        for _ in 0..MAX_ACTIVE_INVOCATIONS {
            let Some(queued) = claim_recovery_work(&self.service, &self.recovery_preference)?
            else {
                break;
            };
            let invocation_id = queued.invocation_id();
            match self.sender.try_send(queued) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    self.service
                        .lock()
                        .map_err(|_| "runtime service lock poisoned")?
                        .release_invocation_claim(&invocation_id);
                    break;
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.service
                        .lock()
                        .map_err(|_| "runtime service lock poisoned")?
                        .release_invocation_claim(&invocation_id);
                    return Err("invocation_workers_unavailable".to_owned());
                }
            }
        }
        Ok(())
    }

    fn schedule(&self, invocation_id: &str) -> Result<Option<StartGate>, String> {
        let prepared = self
            .service
            .lock()
            .map_err(|_| "runtime service lock poisoned")?
            .claim_pending_invocation(invocation_id)?;
        let Some(prepared) = prepared else {
            return Ok(None);
        };
        let gate = StartGate::new();
        self.enqueue(InvocationWork::Start(Box::new(prepared)), gate.clone())?;
        Ok(Some(gate))
    }

    fn schedule_resume(&self, capability_request_id: &str) -> Result<Option<StartGate>, String> {
        let prepared = match self
            .service
            .lock()
            .map_err(|_| "runtime service lock poisoned")?
            .claim_host_capability_resume(capability_request_id)
        {
            Err(error) if error == "invocation_capacity_exhausted" => {
                return Ok(None);
            }
            other => other?,
        };
        let Some(prepared) = prepared else {
            return Ok(None);
        };
        let gate = StartGate::new();
        self.enqueue(InvocationWork::Resume(Box::new(prepared)), gate.clone())?;
        Ok(Some(gate))
    }

    fn enqueue(&self, work: InvocationWork, gate: StartGate) -> Result<(), String> {
        let invocation_id = match &work {
            InvocationWork::Start(prepared) => prepared.invocation_id.clone(),
            InvocationWork::Resume(prepared) => prepared.invocation_id.clone(),
        };
        match self.sender.try_send(QueuedInvocation { work, gate }) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.service
                    .lock()
                    .map_err(|_| "runtime service lock poisoned")?
                    .release_invocation_claim(&invocation_id);
                Err("invocation_capacity_exhausted".to_owned())
            }
            Err(TrySendError::Disconnected(_)) => {
                self.service
                    .lock()
                    .map_err(|_| "runtime service lock poisoned")?
                    .release_invocation_claim(&invocation_id);
                Err("invocation_workers_unavailable".to_owned())
            }
        }
    }
}

fn serve_frames<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    service: &mut RuntimeService,
) -> Result<(), String> {
    let mut frame_count = 0;
    while let Some(line) = read_limited_line(&mut reader)? {
        if line.trim().is_empty() {
            continue;
        }
        let frame = decode_jsonl(&line)?;
        frame_count += 1;
        if frame_count > MAX_FRAMES_PER_CONNECTION {
            return Err(format!(
                "Runtime connection exceeds {MAX_FRAMES_PER_CONNECTION} frames"
            ));
        }
        if handshake_cross_wired(&frame.channel) {
            return Err("cross-wired compilation handshake on runtime stdio".to_owned());
        }
        if frame.channel != RUNTIME_CHANNEL {
            return Err(format!(
                "unexpected runtime channel '{}'; expected '{}'",
                frame.channel, RUNTIME_CHANNEL
            ));
        }
        let payload: serde_json::Value =
            serde_json::from_str(&frame.payload).map_err(|error| error.to_string())?;
        if is_observation_stream(&payload) {
            return Err("long-lived observation streams require a Unix endpoint".to_owned());
        }
        let result = process_payload(payload, service)?;
        write_result(&mut writer, result)?;
    }
    Ok(())
}

fn process_payload(
    payload: serde_json::Value,
    service: &mut RuntimeService,
) -> Result<serde_json::Value, String> {
    let protocol_version = payload
        .get("handshake")
        .and_then(|handshake| handshake.get("protocol_version"))
        .and_then(serde_json::Value::as_str);
    if protocol_version == Some(RUNTIME_PROTOCOL_V2_VERSION) {
        let envelope: EnvelopeV2 =
            serde_json::from_value(payload).map_err(|error| error.to_string())?;
        serde_json::to_value(
            service
                .handle_v2(&envelope.handshake, envelope.request)
                .map_err(|error| format!("{error:?}"))?,
        )
        .map_err(|error| error.to_string())
    } else if protocol_version == Some(RUNTIME_EXECUTION_ADMISSION_VERSION) {
        let envelope: ExecutionAdmissionEnvelope =
            serde_json::from_value(payload).map_err(|error| error.to_string())?;
        serde_json::to_value(
            service.handle_execution_admission(&envelope.handshake, envelope.request),
        )
        .map_err(|error| error.to_string())
    } else {
        let envelope: Envelope =
            serde_json::from_value(payload).map_err(|error| error.to_string())?;
        serde_json::to_value(
            service
                .handle(&envelope.handshake, envelope.request)
                .map_err(|error| format!("{error:?}"))?,
        )
        .map_err(|error| error.to_string())
    }
}

fn write_result<W: Write>(writer: &mut W, result: serde_json::Value) -> Result<(), String> {
    let reply = StdioFrame {
        channel: RUNTIME_CHANNEL.to_owned(),
        payload: result.to_string(),
    };
    writer
        .write_all(encode_jsonl(&reply).as_bytes())
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn write_result_and_release<W: Write>(
    writer: &mut W,
    result: serde_json::Value,
    gate: Option<StartGate>,
) -> Result<(), String> {
    let written = write_result(writer, result);
    if let Some(gate) = gate {
        gate.release();
    }
    written
}

fn is_observation_stream(payload: &serde_json::Value) -> bool {
    payload
        .get("handshake")
        .and_then(|value| value.get("protocol_version"))
        .and_then(serde_json::Value::as_str)
        == Some(RUNTIME_PROTOCOL_V2_VERSION)
        && payload
            .get("request")
            .and_then(|value| value.get("method"))
            .and_then(serde_json::Value::as_str)
            == Some("observation.subscribe")
        && payload
            .get("request")
            .and_then(|value| value.get("stream"))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
}

/// Serve independent Unix clients against one shared service.  A stream
/// releases the service mutex while waiting for the driver's commit signal,
/// allowing another client to start/finish execution and wake the subscriber.
fn serve_shared_frames(
    mut reader: std::io::BufReader<UnixStream>,
    mut writer: UnixStream,
    service: Arc<Mutex<RuntimeService>>,
    dispatcher: InvocationDispatcher,
) -> Result<(), String> {
    let mut frame_count = 0;
    while let Some(line) = read_limited_line(&mut reader)? {
        if line.trim().is_empty() {
            continue;
        }
        let frame = decode_jsonl(&line)?;
        frame_count += 1;
        if frame_count > MAX_FRAMES_PER_CONNECTION {
            return Err(format!(
                "Runtime connection exceeds {MAX_FRAMES_PER_CONNECTION} frames"
            ));
        }
        validate_frame(&frame)?;
        let payload: serde_json::Value =
            serde_json::from_str(&frame.payload).map_err(|error| error.to_string())?;
        if is_observation_stream(&payload) {
            return serve_observation_stream(payload, &mut writer, service);
        }
        let mut result = process_shared_payload(payload, service.clone())?;
        let invocation_id = result
            .get("program_invocation_id")
            .and_then(serde_json::Value::as_str)
            .filter(|_| {
                result.get("kind").and_then(serde_json::Value::as_str)
                    == Some("program_invocation_started")
            })
            .map(str::to_owned);
        let capability_request_id = result
            .get("capability_request_id")
            .and_then(serde_json::Value::as_str)
            .filter(|_| {
                result.get("kind").and_then(serde_json::Value::as_str) == Some("capability_settled")
            })
            .map(str::to_owned);
        let gate = if let Some(invocation_id) = invocation_id {
            match dispatcher.schedule(&invocation_id) {
                Ok(gate) => gate,
                Err(error) => {
                    let code = match error.as_str() {
                        "invocation_capacity_exhausted" | "invocation_workers_unavailable" => error,
                        _ => "runtime_state_unavailable".to_owned(),
                    };
                    let failure = service
                        .lock()
                        .map_err(|_| "runtime service lock poisoned")?
                        .fail_pending_invocation(&invocation_id, &code);
                    result = serde_json::to_value(failure).map_err(|error| error.to_string())?;
                    None
                }
            }
        } else if let Some(capability_request_id) = capability_request_id {
            match dispatcher.schedule_resume(&capability_request_id) {
                Ok(gate) => gate,
                Err(error) => {
                    let code = match error.as_str() {
                        "invocation_capacity_exhausted" | "invocation_workers_unavailable" => error,
                        _ => "runtime_state_unavailable".to_owned(),
                    };
                    let request_id = result
                        .get("request_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown")
                        .to_owned();
                    result = serde_json::to_value(RuntimeResult::Failed { request_id, code })
                        .map_err(|error| error.to_string())?;
                    None
                }
            }
        } else {
            None
        };
        write_result_and_release(&mut writer, result, gate)?;
    }
    Ok(())
}

/// Dispatch one Unix request while allowing an invocation driver to run
/// without the shared service mutex. Admission/claim and finalization remain
/// serialized by that mutex; only the immutable execution lease crosses the
/// unlocked interval.
fn process_shared_payload(
    payload: serde_json::Value,
    service: Arc<Mutex<RuntimeService>>,
) -> Result<serde_json::Value, String> {
    let protocol_version = payload
        .get("handshake")
        .and_then(|handshake| handshake.get("protocol_version"))
        .and_then(serde_json::Value::as_str);
    if protocol_version == Some(RUNTIME_PROTOCOL_V2_VERSION) {
        let mut guard = service
            .lock()
            .map_err(|_| "runtime service lock poisoned")?;
        return process_payload(payload, &mut guard);
    }

    if protocol_version == Some(RUNTIME_EXECUTION_ADMISSION_VERSION) {
        let envelope: ExecutionAdmissionEnvelope =
            serde_json::from_value(payload).map_err(|error| error.to_string())?;
        let mut guard = service
            .lock()
            .map_err(|_| "runtime service lock poisoned")?;
        return serde_json::to_value(
            guard.handle_execution_admission(&envelope.handshake, envelope.request),
        )
        .map_err(|error| error.to_string());
    }

    let envelope: Envelope = serde_json::from_value(payload).map_err(|error| error.to_string())?;
    match envelope.request {
        RuntimeRequest::ProgramInvocationStart {
            request_id,
            program_instance_id,
            owner_claim,
            input,
        } => {
            let prepared = {
                let mut guard = service
                    .lock()
                    .map_err(|_| "runtime service lock poisoned")?;
                guard.cleanup_expired()?;
                envelope
                    .handshake
                    .admit()
                    .map_err(|error| format!("{error:?}"))?;
                guard.prepare_invocation(request_id, program_instance_id, owner_claim, input)
            };
            let prepared = match prepared {
                Ok(prepared) => prepared,
                Err(result) => {
                    return serde_json::to_value(result).map_err(|error| error.to_string());
                }
            };
            let mut guard = service
                .lock()
                .map_err(|_| "runtime service lock poisoned")?;
            guard.release_invocation_claim(&prepared.invocation_id);
            serde_json::to_value(RuntimeResult::ProgramInvocationStarted {
                request_id: prepared.request_id.clone(),
                program_invocation_id: prepared.invocation_id.clone(),
            })
            .map_err(|error| error.to_string())
        }
        RuntimeRequest::CapabilityFulfill {
            request_id,
            owner_claim,
            capability_request_id,
            outcome,
            output,
            receipt_ref,
            message,
        } => {
            let mut guard = service
                .lock()
                .map_err(|_| "runtime service lock poisoned")?;
            guard.cleanup_expired()?;
            envelope
                .handshake
                .admit()
                .map_err(|error| format!("{error:?}"))?;
            let result = if request_id.trim().is_empty()
                || !capability_fulfillment_is_well_formed(
                    &capability_request_id,
                    outcome,
                    output.as_deref(),
                ) {
                RuntimeResult::Failed {
                    request_id,
                    code: "invalid_request".to_owned(),
                }
            } else {
                guard.settle_host_capability(
                    request_id,
                    owner_claim,
                    capability_request_id,
                    outcome,
                    output,
                    receipt_ref,
                    message,
                    false,
                )
            };
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        RuntimeRequest::CapabilityCancel {
            request_id,
            owner_claim,
            capability_request_id,
            message,
        } => {
            let mut guard = service
                .lock()
                .map_err(|_| "runtime service lock poisoned")?;
            guard.cleanup_expired()?;
            envelope
                .handshake
                .admit()
                .map_err(|error| format!("{error:?}"))?;
            let result = if request_id.trim().is_empty()
                || !apxm_core::types::host_capability::is_host_capability_request_id(
                    &capability_request_id,
                ) {
                RuntimeResult::Failed {
                    request_id,
                    code: "invalid_request".to_owned(),
                }
            } else {
                guard.settle_host_capability(
                    request_id,
                    owner_claim,
                    capability_request_id,
                    apxm_core::types::host_capability::HostCapabilityOutcomeKind::Cancelled,
                    None,
                    None,
                    message,
                    false,
                )
            };
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        request => {
            let mut guard = service
                .lock()
                .map_err(|_| "runtime service lock poisoned")?;
            serde_json::to_value(
                guard
                    .handle(&envelope.handshake, request)
                    .map_err(|error| format!("{error:?}"))?,
            )
            .map_err(|error| error.to_string())
        }
    }
}

fn validate_frame(frame: &StdioFrame) -> Result<(), String> {
    if handshake_cross_wired(&frame.channel) {
        return Err("cross-wired compilation handshake on runtime stdio".to_owned());
    }
    if frame.channel != RUNTIME_CHANNEL {
        return Err(format!(
            "unexpected runtime channel '{}'; expected '{}'",
            frame.channel, RUNTIME_CHANNEL
        ));
    }
    Ok(())
}

fn serve_observation_stream(
    mut payload: serde_json::Value,
    writer: &mut UnixStream,
    service: Arc<Mutex<RuntimeService>>,
) -> Result<(), String> {
    let signal = service
        .lock()
        .map_err(|_| "runtime service lock poisoned")?
        .observation_signal();
    let request_value = payload
        .get_mut("request")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "observation stream request is not an object".to_owned())?;
    request_value.remove("stream");
    let envelope: EnvelopeV2 =
        serde_json::from_value(payload).map_err(|error| error.to_string())?;
    let handshake = envelope.handshake;
    let mut request = envelope.request;
    if !matches!(request, RuntimeRequestV2::ObservationSubscribe { .. }) {
        return Err("stream is only supported for observation.subscribe".to_owned());
    }
    loop {
        let generation = signal.generation();
        let result = {
            let guard = service
                .lock()
                .map_err(|_| "runtime service lock poisoned")?;
            guard
                .handle_v2(&handshake, request.clone())
                .map_err(|error| format!("{error:?}"))?
        };
        let is_page = matches!(
            &result,
            apxm_runtime_protocol::RuntimeResultV2::ObservationPage { .. }
        );
        let page = match &result {
            apxm_runtime_protocol::RuntimeResultV2::ObservationPage { page, .. } => {
                Some(page.clone())
            }
            apxm_runtime_protocol::RuntimeResultV2::Failed { .. } => None,
            _ => return Err("observation stream returned a non-page result".to_owned()),
        };
        write_result(
            writer,
            serde_json::to_value(&result).map_err(|error| error.to_string())?,
        )?;
        if !is_page {
            return Ok(());
        }
        let page = page.expect("observation page result");
        if let RuntimeRequestV2::ObservationSubscribe { after_cursor, .. } = &mut request {
            *after_cursor = page
                .items
                .last()
                .map(|item| item.cursor.clone())
                .or_else(|| Some(page.high_watermark.clone()));
        }
        if page.has_more {
            continue;
        }
        signal.wait_for_change(generation);
    }
}

fn read_limited_line<R: BufRead>(reader: &mut R) -> Result<Option<String>, String> {
    let mut bytes = Vec::new();
    loop {
        let (take, newline) = {
            let available = reader.fill_buf().map_err(|error| error.to_string())?;
            if available.is_empty() {
                if bytes.is_empty() {
                    return Ok(None);
                }
                return String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|error| format!("JSONL frame is not UTF-8: {error}"));
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
                .map(Some)
                .map_err(|error| format!("JSONL frame is not UTF-8: {error}"));
        }
    }
}

fn bind_secure_unix(path: &str) -> Result<UnixListener, String> {
    let endpoint = Path::new(path);
    let parent = endpoint
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "unix socket has no parent directory".to_owned())?;
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| format!("stat unix socket parent '{}': {error}", parent.display()))?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(format!(
            "unix socket parent '{}' must be a real directory",
            parent.display()
        ));
    }
    if parent_metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "unix socket parent '{}' is group/world writable",
            parent.display()
        ));
    }

    match std::fs::symlink_metadata(endpoint) {
        Ok(existing) => {
            if existing.file_type().is_symlink() || !existing.file_type().is_socket() {
                return Err(format!(
                    "refusing to replace non-socket unix endpoint '{}'",
                    endpoint.display()
                ));
            }
            if existing.uid() != parent_metadata.uid() || existing.mode() & 0o077 != 0 {
                return Err(format!(
                    "existing unix endpoint '{}' has unsafe owner or mode",
                    endpoint.display()
                ));
            }
            match UnixStream::connect(endpoint) {
                Ok(_) => {
                    return Err(format!(
                        "unix endpoint '{}' is already in use",
                        endpoint.display()
                    ));
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                    ) => {}
                Err(error) => {
                    return Err(format!(
                        "cannot probe existing unix endpoint '{}': {error}",
                        endpoint.display()
                    ));
                }
            }
            let rechecked = std::fs::symlink_metadata(endpoint).map_err(|error| {
                format!("recheck unix endpoint '{}': {error}", endpoint.display())
            })?;
            if rechecked.file_type().is_symlink()
                || !rechecked.file_type().is_socket()
                || rechecked.uid() != parent_metadata.uid()
                || rechecked.mode() & 0o077 != 0
            {
                return Err(format!(
                    "unix endpoint '{}' changed while checking it",
                    endpoint.display()
                ));
            }
            std::fs::remove_file(endpoint).map_err(|error| {
                format!(
                    "remove stale unix endpoint '{}': {error}",
                    endpoint.display()
                )
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "stat unix endpoint '{}': {error}",
                endpoint.display()
            ));
        }
    }

    let listener = UnixListener::bind(endpoint)
        .map_err(|error| format!("bind unix endpoint '{}': {error}", endpoint.display()))?;
    std::fs::set_permissions(endpoint, std::fs::Permissions::from_mode(SOCKET_MODE))
        .map_err(|error| format!("set unix endpoint mode '{}': {error}", endpoint.display()))?;
    let metadata = std::fs::symlink_metadata(endpoint)
        .map_err(|error| format!("stat bound unix endpoint '{}': {error}", endpoint.display()))?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != parent_metadata.uid()
        || metadata.mode() & 0o077 != 0
    {
        return Err(format!(
            "bound unix endpoint '{}' failed ownership/mode verification",
            endpoint.display()
        ));
    }
    Ok(listener)
}

/// Compilation handshake on a Runtime endpoint is cross-wiring.
#[must_use]
pub fn handshake_cross_wired(other_protocol: &str) -> bool {
    other_protocol.starts_with("apxm.compilation.protocol/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuntimeService;
    use apxm_core::types::host_capability::HostCapabilityOutcomeKind;
    use apxm_execution::Continuation;
    use apxm_kernel::ProgramInstanceRef;
    use apxm_program::air::AirModule;
    use apxm_program::artifact::ExecutableArtifact;
    use apxm_runtime_protocol::{
        RUNTIME_PROTOCOL_VERSION, RuntimeHandshake, RuntimeRequest, RuntimeResult,
    };
    use std::sync::mpsc;

    struct BrokenWriter;

    impl Write for BrokenWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "client disconnected",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn fixture_artifact_bytes() -> Vec<u8> {
        let air: AirModule = serde_json::from_value(serde_json::json!({
            "schema_version": "apxm.air",
            "semantic_operations": [],
            "structural_ir": [],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        }))
        .expect("fixture AIR");
        ExecutableArtifact::from_air(&air)
            .expect("fixture artifact")
            .encode()
            .expect("fixture artifact JSON")
    }

    fn host_capability_artifact_bytes() -> Vec<u8> {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("runtime-service repository root")
            .join("tools/tests/fixtures/canonical-host-capability-execute.air.json");
        let air: AirModule = serde_json::from_slice(&std::fs::read(fixture).expect("fixture AIR"))
            .expect("fixture AIR JSON");
        ExecutableArtifact::from_air(&air)
            .expect("fixture artifact")
            .encode()
            .expect("fixture artifact JSON")
    }

    fn create_bound_instance(
        service: &mut RuntimeService,
        artifact: &[u8],
        label: &str,
    ) -> (String, apxm_runtime_protocol::RuntimeOwnerClaim) {
        let digest = service
            .try_admit_artifact(artifact.to_vec())
            .expect("artifact admission");
        let created = service
            .handle(
                &RuntimeHandshake {
                    protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
                },
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: format!("{label}.create"),
                    artifact_digest: digest,
                },
            )
            .expect("instance creation");
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            owner_claim,
            ..
        } = created
        else {
            panic!("instance creation failed: {created:?}");
        };
        service
            .bind_admission(
                &program_instance_id,
                crate::materials_for_artifact(
                    artifact,
                    format!("{program_instance_id}:admission"),
                    b"{}".to_vec(),
                    b"{}".to_vec(),
                ),
            )
            .expect("invocation admission");
        (program_instance_id, owner_claim)
    }

    fn park_and_settle_host_capability(
        service: &mut RuntimeService,
        artifact: &[u8],
        label: &str,
    ) -> String {
        let (program_instance_id, owner_claim) = create_bound_instance(service, artifact, label);
        let started = service
            .handle(
                &RuntimeHandshake {
                    protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
                },
                RuntimeRequest::ProgramInvocationStart {
                    request_id: format!("{label}.start"),
                    program_instance_id: program_instance_id.clone(),
                    owner_claim: owner_claim.clone(),
                    input: serde_json::Value::Null,
                },
            )
            .expect("host invocation start");
        assert!(matches!(
            started,
            RuntimeResult::ProgramInvocationStarted { .. }
        ));
        let continuation = service
            .execution_backend
            .load_continuation(&ProgramInstanceRef::new(program_instance_id))
            .expect("host continuation");
        let continuation: Continuation =
            serde_json::from_value(continuation.payload).expect("host continuation JSON");
        let capability_request_id = continuation
            .event_ref
            .expect("host request")
            .as_str()
            .to_owned();
        let settled = service.settle_host_capability(
            format!("{label}.fulfill"),
            owner_claim,
            capability_request_id.clone(),
            HostCapabilityOutcomeKind::Ok,
            Some("{\"matches\":1}".to_owned()),
            Some(format!("receipt.{label}")),
            None,
            false,
        );
        assert!(matches!(settled, RuntimeResult::CapabilitySettled { .. }));
        capability_request_id
    }

    fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + timeout;
        while !predicate() {
            assert!(
                std::time::Instant::now() < deadline,
                "condition was not reached before the test deadline"
            );
            std::thread::yield_now();
        }
    }

    fn rehydrate_in_memory(service: RuntimeService) -> RuntimeService {
        let backend = match &service.execution_backend {
            crate::RuntimeExecutionBackend::Memory(commit) => {
                crate::RuntimeExecutionBackend::Memory(commit.clone())
            }
            _ => panic!("test service must use the durable in-memory commit adapter"),
        };
        let mut reopened = RuntimeService::in_memory();
        reopened.execution_backend = backend;
        reopened
            .rehydrate_runtime_state()
            .expect("rehydrate durable runtime state");
        reopened
    }

    #[test]
    fn jsonl_round_trips() {
        let frame = StdioFrame {
            channel: "runtime".to_owned(),
            payload: "{}".to_owned(),
        };
        assert_eq!(decode_jsonl(&encode_jsonl(&frame)).unwrap(), frame);
    }

    #[test]
    fn oversized_jsonl_frame_is_rejected_before_decode() {
        let oversized = "x".repeat(MAX_FRAME_BYTES + 1);
        let error = decode_jsonl(&oversized).expect_err("oversized frame must fail closed");
        assert!(error.contains("exceeds"));
    }

    #[test]
    fn unix_connection_permits_are_bounded_and_released() {
        let active = Arc::new(AtomicUsize::new(0));
        let mut permits = (0..MAX_ACTIVE_UNIX_CONNECTIONS)
            .map(|_| {
                UnixConnectionPermit::try_acquire(active.clone())
                    .expect("permit should be available below the limit")
            })
            .collect::<Vec<_>>();
        assert!(UnixConnectionPermit::try_acquire(active.clone()).is_none());
        drop(permits.pop());
        assert!(UnixConnectionPermit::try_acquire(active.clone()).is_some());
    }

    #[test]
    fn failed_acknowledgement_releases_reserved_execution() {
        let gate = StartGate::new();
        let worker_gate = gate.clone();
        let (sent, received) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            worker_gate.wait();
            sent.send(()).expect("worker completion");
        });

        assert!(
            write_result_and_release(
                &mut BrokenWriter,
                serde_json::json!({"kind": "program_invocation_started"}),
                Some(gate),
            )
            .is_err()
        );
        received
            .recv_timeout(Duration::from_secs(1))
            .expect("disconnect must not strand accepted work");
        worker.join().expect("worker joins");
    }

    #[test]
    fn recovery_drains_more_host_resumes_than_process_capacity() {
        let artifact = host_capability_artifact_bytes();
        let execution_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let mut service = RuntimeService::in_memory();
        for index in 0..=MAX_ACTIVE_INVOCATIONS {
            park_and_settle_host_capability(
                &mut service,
                &artifact,
                &format!("capacity.resume.{index}"),
            );
        }

        let mut reopened = rehydrate_in_memory(service);
        reopened.resume_test_gate = Some(execution_gate.clone());
        reopened
            .reconcile_recovery_state()
            .expect("reconcile durable recovery markers");
        let shared = Arc::new(Mutex::new(reopened));
        let dispatcher = InvocationDispatcher::new(shared.clone());
        dispatcher
            .recover_available()
            .expect("bounded startup recovery remains available");

        wait_until(Duration::from_secs(2), || {
            shared
                .lock()
                .expect("runtime service")
                .active_cancellations
                .len()
                == MAX_ACTIVE_INVOCATIONS
        });
        {
            let mut service = shared.lock().expect("runtime service");
            let unclaimed = service
                .host_capability_settlements
                .values()
                .filter(|state| !state.resume_started)
                .filter(|state| {
                    service
                        .instances
                        .get(&state.program_instance_id)
                        .and_then(|instance| instance.invocation.as_ref())
                        .is_some_and(|invocation| {
                            !service
                                .active_cancellations
                                .contains_key(&invocation.program_invocation_id)
                        })
                })
                .count();
            assert_eq!(unclaimed, 1, "overflow remains durably pending");
            let claimed = service
                .host_capability_settlements
                .iter()
                .filter_map(|(capability_request_id, state)| {
                    service
                        .instances
                        .get(&state.program_instance_id)
                        .and_then(|instance| instance.invocation.as_ref())
                        .filter(|invocation| {
                            service
                                .active_cancellations
                                .contains_key(&invocation.program_invocation_id)
                        })
                        .map(|_| capability_request_id.clone())
                })
                .collect::<Vec<_>>();
            for capability_request_id in claimed {
                service
                    .host_capability_settlements
                    .get_mut(&capability_request_id)
                    .expect("claimed settlement")
                    .resume_started = true;
            }
        }

        {
            let (released, signal) = execution_gate.as_ref();
            *released.lock().expect("execution gate") = true;
            signal.notify_all();
        }
        wait_until(Duration::from_secs(5), || {
            let service = shared.lock().expect("runtime service");
            service.active_cancellations.is_empty()
                && service
                    .host_capability_settlements
                    .values()
                    .all(|state| state.resume_started)
        });
    }

    #[test]
    fn recovery_fairly_claims_host_resume_among_pending_starts() {
        let artifact = host_capability_artifact_bytes();
        let start_artifact = fixture_artifact_bytes();
        let execution_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let mut service = RuntimeService::in_memory();
        let capability_request_id =
            park_and_settle_host_capability(&mut service, &artifact, "mixed.resume");
        for index in 0..MAX_ACTIVE_INVOCATIONS {
            let label = format!("mixed.start.{index}");
            let (program_instance_id, owner_claim) =
                create_bound_instance(&mut service, &start_artifact, &label);
            let prepared = service
                .prepare_invocation(
                    format!("{label}.invoke"),
                    program_instance_id,
                    owner_claim,
                    serde_json::Value::Null,
                )
                .expect("durable pending invocation");
            service.release_invocation_claim(&prepared.invocation_id);
        }
        let mut service = rehydrate_in_memory(service);
        service.resume_test_gate = Some(execution_gate.clone());
        service
            .reconcile_recovery_state()
            .expect("reconcile durable recovery markers");
        let shared = Arc::new(Mutex::new(service));
        let dispatcher = InvocationDispatcher::new(shared.clone());
        dispatcher
            .recover_available()
            .expect("mixed startup recovery remains available");

        wait_until(Duration::from_secs(2), || {
            shared
                .lock()
                .expect("runtime service")
                .host_capability_settlements[&capability_request_id]
                .resume_started
        });
        {
            let (released, signal) = execution_gate.as_ref();
            *released.lock().expect("execution gate") = true;
            signal.notify_all();
        }
        wait_until(Duration::from_secs(5), || {
            shared
                .lock()
                .expect("runtime service")
                .active_cancellations
                .is_empty()
        });
    }

    #[test]
    fn unix_capability_settlement_is_acknowledged_before_async_resume() {
        let artifact = host_capability_artifact_bytes();
        let mut service = RuntimeService::default().with_embedded_read_access();
        let digest = service
            .try_admit_artifact(artifact.clone())
            .expect("host artifact admission");
        let created = service
            .handle(
                &RuntimeHandshake {
                    protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
                },
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "unix.host.create".to_owned(),
                    artifact_digest: digest,
                },
            )
            .expect("host instance creation");
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            owner_claim,
            ..
        } = created
        else {
            panic!("host instance creation failed: {created:?}");
        };
        service
            .bind_admission(
                &program_instance_id,
                crate::materials_for_artifact(
                    &artifact,
                    format!("{program_instance_id}:admission"),
                    b"{}".to_vec(),
                    b"{}".to_vec(),
                ),
            )
            .expect("host invocation admission");
        let started = service
            .handle(
                &RuntimeHandshake {
                    protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
                },
                RuntimeRequest::ProgramInvocationStart {
                    request_id: "unix.host.start".to_owned(),
                    program_instance_id: program_instance_id.clone(),
                    owner_claim: owner_claim.clone(),
                    input: serde_json::Value::Null,
                },
            )
            .expect("host invocation start");
        let RuntimeResult::ProgramInvocationStarted {
            program_invocation_id,
            ..
        } = started
        else {
            panic!("host invocation did not start: {started:?}");
        };
        let continuation = service
            .execution_backend
            .load_continuation(&ProgramInstanceRef::new(program_instance_id))
            .expect("host continuation");
        let continuation: Continuation =
            serde_json::from_value(continuation.payload).expect("host continuation JSON");
        let capability_request_id = continuation
            .event_ref
            .expect("host request")
            .as_str()
            .to_owned();

        let resume_gate = Arc::new((Mutex::new(false), Condvar::new()));
        service.resume_test_gate = Some(resume_gate.clone());
        let shared = Arc::new(Mutex::new(service));
        let dispatcher = InvocationDispatcher::new(shared.clone());
        let cancellation_dispatcher = dispatcher.clone();
        let cancellation_service = shared.clone();
        let (mut client, server) = UnixStream::pair().expect("Unix stream pair");
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("client deadline");
        let reader = std::io::BufReader::new(server.try_clone().expect("server reader"));
        let server_task =
            std::thread::spawn(move || serve_shared_frames(reader, server, shared, dispatcher));
        let payload = serde_json::json!({
            "handshake": {"protocol_version": RUNTIME_PROTOCOL_VERSION},
            "request": {
                "method": "capability_fulfill",
                "request_id": "unix.host.fulfill",
                "owner_claim": owner_claim.clone(),
                "capability_request_id": capability_request_id,
                "outcome": HostCapabilityOutcomeKind::Ok,
                "output": "{\"matches\":1}",
                "receipt_ref": "receipt.unix.host"
            }
        });
        let frame = StdioFrame {
            channel: RUNTIME_CHANNEL.to_owned(),
            payload: payload.to_string(),
        };
        client
            .write_all(encode_jsonl(&frame).as_bytes())
            .expect("write fulfillment");
        client.flush().expect("flush fulfillment");
        let mut response = String::new();
        std::io::BufReader::new(client.try_clone().expect("client reader"))
            .read_line(&mut response)
            .expect("the durable settlement is acknowledged within the Unix deadline");
        let response = decode_jsonl(&response).expect("response frame");
        let result: RuntimeResult =
            serde_json::from_str(&response.payload).expect("runtime response");
        assert!(matches!(
            result,
            RuntimeResult::CapabilitySettled { request_id, .. }
                if request_id == "unix.host.fulfill"
        ));
        assert!(
            !*resume_gate.0.lock().expect("resume test gate"),
            "the fulfillment acknowledgement must precede resumed execution"
        );

        let (mut cancellation_client, cancellation_server) =
            UnixStream::pair().expect("cancellation Unix stream pair");
        cancellation_client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("cancellation deadline");
        let cancellation_reader = std::io::BufReader::new(
            cancellation_server
                .try_clone()
                .expect("cancellation server reader"),
        );
        let cancellation_task = std::thread::spawn(move || {
            serve_shared_frames(
                cancellation_reader,
                cancellation_server,
                cancellation_service,
                cancellation_dispatcher,
            )
        });
        let cancellation_payload = serde_json::json!({
            "handshake": {"protocol_version": RUNTIME_PROTOCOL_VERSION},
            "request": {
                "method": "program_invocation_cancel",
                "request_id": "unix.host.cancel",
                "owner_claim": owner_claim,
                "program_invocation_id": program_invocation_id
            }
        });
        let cancellation_frame = StdioFrame {
            channel: RUNTIME_CHANNEL.to_owned(),
            payload: cancellation_payload.to_string(),
        };
        cancellation_client
            .write_all(encode_jsonl(&cancellation_frame).as_bytes())
            .expect("write cancellation");
        cancellation_client.flush().expect("flush cancellation");
        let mut cancellation_response = String::new();
        std::io::BufReader::new(
            cancellation_client
                .try_clone()
                .expect("cancellation client reader"),
        )
        .read_line(&mut cancellation_response)
        .expect("cancellation remains responsive while resume is blocked");
        let cancellation_response =
            decode_jsonl(&cancellation_response).expect("cancellation response frame");
        let cancellation_result: RuntimeResult =
            serde_json::from_str(&cancellation_response.payload)
                .expect("cancellation runtime response");
        assert!(matches!(
            cancellation_result,
            RuntimeResult::Cancelled { request_id } if request_id == "unix.host.cancel"
        ));

        {
            let (released, signal) = resume_gate.as_ref();
            *released.lock().expect("resume test gate") = true;
            signal.notify_one();
        }
        client
            .shutdown(std::net::Shutdown::Both)
            .expect("close client");
        assert!(server_task.join().expect("server task").is_ok());
        cancellation_client
            .shutdown(std::net::Shutdown::Both)
            .expect("close cancellation client");
        assert!(
            cancellation_task
                .join()
                .expect("cancellation server task")
                .is_ok()
        );
    }

    #[test]
    fn streaming_reader_rejects_delimiterless_frame_without_unbounded_growth() {
        let input = vec![b'x'; MAX_FRAME_BYTES + 1];
        let error = serve_stdio(input.as_slice(), &mut Vec::new(), RuntimeService::default())
            .expect_err("delimiterless oversized frame must fail closed");
        assert!(error.contains("exceeds"));
    }

    #[test]
    fn compilation_handshake_is_cross_wired() {
        assert!(handshake_cross_wired("apxm.compilation.protocol/1"));
        assert!(!handshake_cross_wired("apxm.runtime.protocol/1"));
    }

    #[test]
    fn observation_stream_requires_unix_transport() {
        let payload = serde_json::json!({
            "handshake": {"protocol_version": RUNTIME_PROTOCOL_V2_VERSION},
            "request": {"method": "observation.subscribe", "stream": true}
        });
        assert!(is_observation_stream(&payload));
        let frame = StdioFrame {
            channel: RUNTIME_CHANNEL.to_owned(),
            payload: payload.to_string(),
        };
        let error = serve_stdio(
            encode_jsonl(&frame).as_bytes(),
            &mut Vec::new(),
            RuntimeService::default(),
        )
        .expect_err("stdio cannot carry a long-lived stream");
        assert!(error.contains("Unix"));
    }

    #[test]
    fn relative_unix_path_is_rejected() {
        assert!(UnixEndpoint::new("runtime.sock").is_err());
        UnixEndpoint::new("/tmp/apxm-runtime.sock").unwrap();
    }

    #[test]
    fn unix_shared_dispatch_rejects_blank_invocation_id_before_mutation() {
        let bytes = crate::tests::event_air_bytes();
        let mut service = RuntimeService::default();
        let digest = service
            .try_admit_artifact(bytes.clone())
            .expect("fixture artifact admission");
        let created = service
            .handle(
                &RuntimeHandshake {
                    protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
                },
                RuntimeRequest::ProgramInstanceCreate {
                    request_id: "create".to_owned(),
                    artifact_digest: digest,
                },
            )
            .expect("instance creation");
        let RuntimeResult::ProgramInstanceCreated {
            program_instance_id,
            owner_claim,
            ..
        } = created
        else {
            panic!("expected instance creation");
        };
        service
            .bind_admission(
                &program_instance_id,
                crate::materials_for_artifact(&bytes, "invocation", Vec::new(), Vec::new()),
            )
            .expect("invocation admission");
        let reserved = service
            .handle(
                &RuntimeHandshake {
                    protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
                },
                RuntimeRequest::EventReserve {
                    request_id: "reserve".to_owned(),
                    program_instance_id: program_instance_id.clone(),
                    owner_claim: owner_claim.clone(),
                    type_id: "UserInput".to_owned(),
                },
            )
            .expect("event reservation");
        assert!(matches!(reserved, RuntimeResult::EventReserved { .. }));

        let instance_before = service.instances.get(&program_instance_id).map(|instance| {
            (
                instance.invocation.as_ref().map(|invocation| {
                    (
                        invocation.request_id.clone(),
                        invocation.program_invocation_id.clone(),
                        invocation.result.clone(),
                    )
                }),
                instance.invocation_history.len(),
                instance.invocation_bytes,
                instance.state_entry.bytes,
            )
        });
        let reservation_count_before = service.reservations.len();
        let reservation_generation_before = service.next_generation;
        let invocation_index_before = service.invocation_index.clone();
        let active_cancellations_before = service.active_cancellations.len();
        let shared = Arc::new(Mutex::new(service));
        let payload = serde_json::json!({
            "handshake": {"protocol_version": RUNTIME_PROTOCOL_VERSION},
            "request": {
                "method": "program_invocation_start",
                "request_id": " \t\n ",
                "program_instance_id": program_instance_id,
                "owner_claim": owner_claim,
                "input": null
            }
        });

        let result = process_shared_payload(payload, Arc::clone(&shared)).expect("dispatch");
        let result: RuntimeResult = serde_json::from_value(result).expect("runtime result");
        assert!(matches!(
            result,
            RuntimeResult::Failed { request_id, code }
                if request_id == " \t\n " && code == "invalid_request"
        ));

        let service = shared.lock().expect("service lock");
        let instance_after = service.instances.get(&program_instance_id).map(|instance| {
            (
                instance.invocation.as_ref().map(|invocation| {
                    (
                        invocation.request_id.clone(),
                        invocation.program_invocation_id.clone(),
                        invocation.result.clone(),
                    )
                }),
                instance.invocation_history.len(),
                instance.invocation_bytes,
                instance.state_entry.bytes,
            )
        });
        assert_eq!(instance_after, instance_before);
        assert_eq!(service.reservations.len(), reservation_count_before);
        assert_eq!(service.next_generation, reservation_generation_before);
        assert_eq!(service.invocation_index, invocation_index_before);
        assert_eq!(
            service.active_cancellations.len(),
            active_cancellations_before
        );
    }

    #[test]
    fn serve_stdio_handles_one_create() {
        let mut service = RuntimeService::default();
        let digest = service.admit_artifact(fixture_artifact_bytes());
        let envelope = Envelope {
            handshake: RuntimeHandshake {
                protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
            },
            request: RuntimeRequest::ProgramInstanceCreate {
                request_id: "c".to_owned(),
                artifact_digest: digest,
            },
        };
        let frame = StdioFrame {
            channel: "runtime".to_owned(),
            payload: serde_json::to_string(&envelope).unwrap(),
        };
        let mut out = Vec::new();
        serve_stdio(encode_jsonl(&frame).as_bytes(), &mut out, service).unwrap();
        assert!(!out.is_empty());
    }

    #[test]
    fn serve_stdio_rejects_cross_wired_compilation_handshake() {
        let frame = StdioFrame {
            channel: "apxm.compilation.protocol/1".to_owned(),
            payload: "{}".to_owned(),
        };
        let err = serve_stdio(
            encode_jsonl(&frame).as_bytes(),
            &mut Vec::new(),
            RuntimeService::default(),
        )
        .unwrap_err();
        assert!(err.contains("cross-wired"));
    }

    #[test]
    fn serve_stdio_rejects_unknown_channel() {
        let frame = StdioFrame {
            channel: "other".to_owned(),
            payload: "{}".to_owned(),
        };
        let err = serve_stdio(
            encode_jsonl(&frame).as_bytes(),
            &mut Vec::new(),
            RuntimeService::default(),
        )
        .unwrap_err();
        assert!(err.contains("unexpected runtime channel"));
    }

    #[test]
    fn jsonl_invokes_a_persisted_artifact() {
        let dir = std::env::temp_dir().join(format!(
            "apxm-runtime-jsonl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let bytes = fixture_artifact_bytes();
        let digest = crate::canonical_artifact_digest(&bytes).expect("artifact digest");
        std::fs::write(dir.join(digest.replace(':', "-")), &bytes).unwrap();
        let service = RuntimeService::default().with_artifact_dir(dir);
        let envelope = Envelope {
            handshake: RuntimeHandshake {
                protocol_version: RUNTIME_PROTOCOL_VERSION.to_owned(),
            },
            request: RuntimeRequest::ProgramInstanceCreate {
                request_id: "c".to_owned(),
                artifact_digest: digest,
            },
        };
        let frame = StdioFrame {
            channel: "runtime".to_owned(),
            payload: serde_json::to_string(&envelope).unwrap(),
        };
        let mut out = Vec::new();
        serve_stdio(encode_jsonl(&frame).as_bytes(), &mut out, service).unwrap();
        let reply = decode_jsonl(std::str::from_utf8(&out).unwrap()).unwrap();
        let result: apxm_runtime_protocol::RuntimeResult =
            serde_json::from_str(&reply.payload).unwrap();
        assert!(matches!(
            result,
            apxm_runtime_protocol::RuntimeResult::ProgramInstanceCreated { .. }
        ));
    }

    #[test]
    fn serve_unix_rejects_relative_path_before_bind() {
        let err = serve_unix("runtime.sock", RuntimeService::default()).unwrap_err();
        assert!(err.contains("absolute"));
    }

    #[cfg(unix)]
    #[test]
    fn secure_unix_bind_sets_private_mode_and_reclaims_its_stale_socket() {
        let directory = tempfile::tempdir().expect("test directory");
        let endpoint = directory.path().join("service.sock");
        let listener = bind_secure_unix(endpoint.to_str().expect("socket path"))
            .expect("trusted socket parent binds");
        let metadata = std::fs::symlink_metadata(&endpoint).expect("bound socket metadata");
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.mode() & 0o777, SOCKET_MODE);
        drop(listener);

        let rebound = bind_secure_unix(endpoint.to_str().expect("socket path"))
            .expect("our private stale socket can be reclaimed");
        drop(rebound);
    }

    #[cfg(unix)]
    #[test]
    fn secure_unix_bind_refuses_a_symlink_endpoint() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("test directory");
        let target = directory.path().join("target");
        std::fs::write(&target, "must survive").expect("target");
        let endpoint = directory.path().join("service.sock");
        symlink(&target, &endpoint).expect("endpoint symlink");
        let error = bind_secure_unix(endpoint.to_str().expect("socket path"))
            .expect_err("symlink endpoint must not be unlinked");
        assert!(error.contains("non-socket"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "must survive");
    }
}
