//! Runtime Service stdio and Unix-socket framing.

use serde::{Deserialize, Serialize};

/// One stdio JSONL frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StdioFrame {
    /// Channel this frame belongs to.
    pub channel: String,
    /// UTF-8 JSON payload.
    pub payload: String,
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

/// Compilation handshake on a Runtime endpoint is cross-wiring.
#[must_use]
pub fn handshake_cross_wired(other_protocol: &str) -> bool {
    other_protocol.starts_with("apxm.compilation.protocol/")
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
