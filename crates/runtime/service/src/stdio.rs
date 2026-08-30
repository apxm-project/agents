//! Runtime Service stdio and Unix-socket framing.

use std::io::{BufRead, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::RuntimeService;
use apxm_runtime_protocol::{
    RUNTIME_PROTOCOL_V2_VERSION, RuntimeHandshake, RuntimeHandshakeV2, RuntimeRequest,
    RuntimeRequestV2,
};

/// Maximum encoded JSONL frame, including its line terminator.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
/// The only channel accepted by the Runtime Service framing boundary.
pub const RUNTIME_CHANNEL: &str = "runtime";
/// Per-connection I/O deadline for local Unix clients.
pub const UNIX_IO_TIMEOUT_MS: u64 = 5_000;
/// Maximum requests served on one local connection before it is drained.
pub const MAX_FRAMES_PER_CONNECTION: usize = 256;
const SOCKET_MODE: u32 = 0o600;

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
    for incoming in listener.incoming() {
        let stream = incoming.map_err(|error| error.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_millis(UNIX_IO_TIMEOUT_MS)))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_millis(UNIX_IO_TIMEOUT_MS)))
            .map_err(|error| error.to_string())?;
        let reader =
            std::io::BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
        let shared = shared.clone();
        std::thread::spawn(move || {
            let _ = serve_shared_frames(reader, stream, shared);
        });
    }
    Ok(())
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
        let result = process_shared_payload(payload, service.clone())?;
        write_result(&mut writer, result)?;
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

    let envelope: Envelope = serde_json::from_value(payload).map_err(|error| error.to_string())?;
    if let RuntimeRequest::ProgramInvocationStart {
        request_id,
        program_instance_id,
        owner_claim,
        input,
    } = envelope.request
    {
        let prepared = {
            let mut guard = service
                .lock()
                .map_err(|_| "runtime service lock poisoned")?;
            guard.cleanup_expired();
            envelope
                .handshake
                .admit()
                .map_err(|error| format!("{error:?}"))?;
            guard.prepare_invocation(request_id, program_instance_id, owner_claim, input)
        };
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(result) => return serde_json::to_value(result).map_err(|error| error.to_string()),
        };
        let execution = prepared.execute();
        let result = {
            let mut guard = service
                .lock()
                .map_err(|_| "runtime service lock poisoned")?;
            guard.finish_invocation(&prepared, execution)
        };
        return serde_json::to_value(result).map_err(|error| error.to_string());
    }

    let mut guard = service
        .lock()
        .map_err(|_| "runtime service lock poisoned")?;
    serde_json::to_value(
        guard
            .handle(&envelope.handshake, envelope.request)
            .map_err(|error| format!("{error:?}"))?,
    )
    .map_err(|error| error.to_string())
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
    use apxm_program::air::AirModule;
    use apxm_program::artifact::ExecutableArtifact;
    use apxm_runtime_protocol::{RUNTIME_PROTOCOL_VERSION, RuntimeHandshake, RuntimeRequest};

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
