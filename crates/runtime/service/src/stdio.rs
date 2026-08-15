//! Runtime Service stdio and Unix-socket framing.

use std::io::{BufRead, Write};
use std::os::unix::net::UnixListener;

use serde::{Deserialize, Serialize};

use crate::RuntimeService;
use apxm_runtime_protocol::{RuntimeHandshake, RuntimeRequest};

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

/// Bind an absolute Unix socket and serve one connection at a time.
pub fn serve_unix(path: &str, mut service: RuntimeService) -> Result<(), String> {
    let endpoint = UnixEndpoint::new(path)?;
    let _ = std::fs::remove_file(&endpoint.path);
    let listener = UnixListener::bind(&endpoint.path).map_err(|error| error.to_string())?;
    for incoming in listener.incoming() {
        let stream = incoming.map_err(|error| error.to_string())?;
        let reader =
            std::io::BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
        serve_frames(reader, stream, &mut service)?;
    }
    Ok(())
}

fn serve_frames<R: BufRead, W: Write>(
    reader: R,
    mut writer: W,
    service: &mut RuntimeService,
) -> Result<(), String> {
    for line in reader.lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let frame = decode_jsonl(&line)?;
        if handshake_cross_wired(&frame.channel) {
            return Err("cross-wired compilation handshake on runtime stdio".to_owned());
        }
        let envelope: Envelope =
            serde_json::from_str(&frame.payload).map_err(|error| error.to_string())?;
        let result = service
            .handle(&envelope.handshake, envelope.request)
            .map_err(|error| format!("{error:?}"))?;
        let reply = StdioFrame {
            channel: "runtime".to_owned(),
            payload: serde_json::to_string(&result).map_err(|error| error.to_string())?,
        };
        writer
            .write_all(encode_jsonl(&reply).as_bytes())
            .map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
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
    use apxm_runtime_protocol::{RUNTIME_PROTOCOL_VERSION, RuntimeHandshake, RuntimeRequest};

    #[test]
    fn jsonl_round_trips() {
        let frame = StdioFrame {
            channel: "runtime".to_owned(),
            payload: "{}".to_owned(),
        };
        assert_eq!(decode_jsonl(&encode_jsonl(&frame)).unwrap(), frame);
    }

    #[test]
    fn compilation_handshake_is_cross_wired() {
        assert!(handshake_cross_wired("apxm.compilation.protocol/1"));
        assert!(!handshake_cross_wired("apxm.runtime.protocol/1"));
    }

    #[test]
    fn relative_unix_path_is_rejected() {
        assert!(UnixEndpoint::new("runtime.sock").is_err());
        UnixEndpoint::new("/tmp/apxm-runtime.sock").unwrap();
    }

    #[test]
    fn serve_stdio_handles_one_create() {
        let mut service = RuntimeService::default();
        let digest = service.admit_artifact(br#"{"schema_version":"apxm.air","semantic_operations":[],"structural_ir":[],"context_flow":[],"source_map":{"schema_version":"apxm.source-map","source_language":"python","node_spans":[],"region_annotations":[]}}"#.to_vec());
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
        let bytes = br#"{"schema_version":"apxm.air","semantic_operations":[],"structural_ir":[],"context_flow":[],"source_map":{"schema_version":"apxm.source-map","source_language":"python","node_spans":[],"region_annotations":[]}}"#;
        let digest = crate::artifact_digest(bytes);
        std::fs::write(dir.join(digest.replace(':', "-")), bytes).unwrap();
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
}
