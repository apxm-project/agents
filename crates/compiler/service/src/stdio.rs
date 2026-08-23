//! Compilation Service stdio JSONL framing.
//!
//! Protocol bytes never share a stream with service logs.

use std::io::{BufRead, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::CompilationService;
use apxm_compilation_protocol::{CompilationHandshake, CompilationRequest};

/// Maximum encoded JSONL frame, including its line terminator.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
/// The only channel accepted by the Compilation Service framing boundary.
pub const COMPILATION_CHANNEL: &str = "compilation";
const SOCKET_MODE: u32 = 0o600;

/// One stdio JSONL frame. Unknown methods fail at handshake, not by coercion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StdioFrame {
    /// Channel this frame belongs to.
    pub channel: String,
    /// UTF-8 JSON payload.
    pub payload: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    handshake: CompilationHandshake,
    request: CompilationRequest,
}

/// Encode one frame as a single JSONL line including the trailing newline.
#[must_use]
pub fn encode_jsonl(frame: &StdioFrame) -> String {
    format!(
        "{}\n",
        serde_json::to_string(frame).expect("frame is closed")
    )
}

/// Decode one JSONL line. Extra data after one value is rejected.
pub fn decode_jsonl(line: &str) -> Result<StdioFrame, String> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(format!("JSONL frame exceeds {MAX_FRAME_BYTES} bytes"));
    }
    serde_json::from_str(line.trim()).map_err(|error| error.to_string())
}

/// Serve Compilation protocol frames until stdin EOF.
pub fn serve_stdio<R: BufRead, W: Write>(
    reader: R,
    writer: W,
    mut service: CompilationService,
) -> Result<(), String> {
    serve_frames(reader, writer, &mut service)
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

/// Bind an absolute Unix socket and serve one connection at a time.
pub fn serve_unix(path: &str, mut service: CompilationService) -> Result<(), String> {
    let endpoint = UnixEndpoint::new(path)?;
    let listener = bind_secure_unix(&endpoint.path)?;
    for incoming in listener.incoming() {
        let stream = incoming.map_err(|error| error.to_string())?;
        let reader =
            std::io::BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
        serve_frames(reader, stream, &mut service)?;
    }
    Ok(())
}

fn serve_frames<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    service: &mut CompilationService,
) -> Result<(), String> {
    while let Some(line) = read_limited_line(&mut reader)? {
        if line.trim().is_empty() {
            continue;
        }
        let frame = decode_jsonl(&line)?;
        if handshake_cross_wired(&frame.channel) {
            return Err("cross-wired runtime handshake on compilation stdio".to_owned());
        }
        if frame.channel != COMPILATION_CHANNEL {
            return Err(format!(
                "unexpected compilation channel '{}'; expected '{}'",
                frame.channel, COMPILATION_CHANNEL
            ));
        }
        let envelope: Envelope =
            serde_json::from_str(&frame.payload).map_err(|error| error.to_string())?;
        let result = service
            .handle(&envelope.handshake, envelope.request)
            .map_err(|error| format!("{error:?}"))?;
        let reply = StdioFrame {
            channel: COMPILATION_CHANNEL.to_owned(),
            payload: serde_json::to_string(&result).map_err(|error| error.to_string())?,
        };
        writer
            .write_all(encode_jsonl(&reply).as_bytes())
            .map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
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

/// Compilation and Runtime handshakes must not be interchangeable.
#[must_use]
pub fn handshake_cross_wired(other_protocol: &str) -> bool {
    other_protocol.starts_with("apxm.runtime.protocol/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_round_trips() {
        let frame = StdioFrame {
            channel: "compilation".to_owned(),
            payload: "{}".to_owned(),
        };
        let encoded = encode_jsonl(&frame);
        assert!(encoded.ends_with('\n'));
        assert_eq!(decode_jsonl(&encoded).unwrap(), frame);
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
        let error = serve_stdio(
            input.as_slice(),
            &mut Vec::new(),
            CompilationService::default(),
        )
        .expect_err("delimiterless oversized frame must fail closed");
        assert!(error.contains("exceeds"));
    }

    #[test]
    fn runtime_handshake_is_cross_wired() {
        assert!(handshake_cross_wired("apxm.runtime.protocol/1"));
        assert!(!handshake_cross_wired("apxm.compilation.protocol/1"));
    }

    #[test]
    fn relative_unix_path_is_rejected() {
        assert!(UnixEndpoint::new("compilation.sock").is_err());
        UnixEndpoint::new("/tmp/apxm-compilation.sock").unwrap();
    }

    #[test]
    fn serve_stdio_rejects_cross_wired_runtime_handshake() {
        let frame = StdioFrame {
            channel: "apxm.runtime.protocol/1".to_owned(),
            payload: "{}".to_owned(),
        };
        let err = serve_stdio(
            encode_jsonl(&frame).as_bytes(),
            &mut Vec::new(),
            CompilationService::default(),
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
            CompilationService::default(),
        )
        .unwrap_err();
        assert!(err.contains("unexpected compilation channel"));
    }

    #[test]
    fn serve_unix_rejects_relative_path_before_bind() {
        let err = serve_unix("compilation.sock", CompilationService::default()).unwrap_err();
        assert!(err.contains("absolute"));
    }

    #[cfg(unix)]
    #[test]
    fn secure_unix_bind_sets_private_mode_and_reclaims_its_stale_socket() {
        // Unix-domain socket paths are capped (typically at 104 bytes on
        // macOS).  Keep the test's path short instead of relying on the
        // platform-specific, often long, temporary-directory expansion.
        let directory = Path::new("/tmp").join(format!(
            "apxm-c-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
                % 1_000_000
        ));
        std::fs::create_dir(&directory).expect("test directory");
        let endpoint = directory.join("service.sock");
        let listener = bind_secure_unix(endpoint.to_str().expect("socket path"))
            .expect("trusted socket parent binds");
        let metadata = std::fs::symlink_metadata(&endpoint).expect("bound socket metadata");
        assert!(metadata.file_type().is_socket());
        assert_eq!(metadata.mode() & 0o777, SOCKET_MODE);
        drop(listener);

        let rebound = bind_secure_unix(endpoint.to_str().expect("socket path"))
            .expect("our private stale socket can be reclaimed");
        drop(rebound);
        std::fs::remove_file(&endpoint).expect("socket cleanup");
        std::fs::remove_dir(&directory).expect("directory cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn secure_unix_bind_refuses_a_symlink_endpoint() {
        use std::os::unix::fs::symlink;

        let directory = Path::new("/tmp").join(format!(
            "apxm-cs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
                % 1_000_000
        ));
        std::fs::create_dir(&directory).expect("test directory");
        let target = directory.join("target");
        std::fs::write(&target, "must survive").expect("target");
        let endpoint = directory.join("service.sock");
        symlink(&target, &endpoint).expect("endpoint symlink");
        let error = bind_secure_unix(endpoint.to_str().expect("socket path"))
            .expect_err("symlink endpoint must not be unlinked");
        assert!(error.contains("non-socket"));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "must survive");
        std::fs::remove_file(&endpoint).expect("symlink cleanup");
        std::fs::remove_file(&target).expect("target cleanup");
        std::fs::remove_dir(&directory).expect("directory cleanup");
    }
}
