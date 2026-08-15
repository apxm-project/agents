//! Compilation Service stdio JSONL framing.
//!
//! Protocol bytes never share a stream with service logs.

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};

use crate::CompilationService;
use apxm_compilation_protocol::{CompilationHandshake, CompilationRequest};

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
    serde_json::from_str(line.trim()).map_err(|error| error.to_string())
}

/// Serve Compilation protocol frames until stdin EOF.
pub fn serve_stdio<R: BufRead, W: Write>(
    reader: R,
    mut writer: W,
    mut service: CompilationService,
) -> Result<(), String> {
    for line in reader.lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let frame = decode_jsonl(&line)?;
        if handshake_cross_wired(&frame.channel) {
            return Err("cross-wired runtime handshake on compilation stdio".to_owned());
        }
        let envelope: Envelope =
            serde_json::from_str(&frame.payload).map_err(|error| error.to_string())?;
        let result = service
            .handle(&envelope.handshake, envelope.request)
            .map_err(|error| format!("{error:?}"))?;
        let reply = StdioFrame {
            channel: "compilation".to_owned(),
            payload: serde_json::to_string(&result).map_err(|error| error.to_string())?,
        };
        writer
            .write_all(encode_jsonl(&reply).as_bytes())
            .map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
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
    fn runtime_handshake_is_cross_wired() {
        assert!(handshake_cross_wired("apxm.runtime.protocol/1"));
        assert!(!handshake_cross_wired("apxm.compilation.protocol/1"));
    }
}
