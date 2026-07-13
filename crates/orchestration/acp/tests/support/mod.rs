//! Shared test fixtures for `apxm-acp` integration tests.
//!
//! Tests spawn small Python scripts as stand-ins for real ACP agent CLIs.
//! Python gives us reliable JSON parsing without pulling test-only crates
//! into the fixtures, and `python3` is already a build-time dependency of
//! this workspace (see `crates/runtime/engine/src/python_tools`).

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use apxm_acp::{AcpAgentProfile, PermissionMode};

/// A minimal ACP agent fixture: handshakes (`initialize`, `session/new`),
/// answers `session/prompt` with a streamed chunk, usage update, and final
/// response, then exits cleanly on EOF (stdin closed).
pub const COOPERATIVE_AGENT: &str = r#"
import sys, json

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

SESSION_ID = "agent-session-fixture"

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method = msg.get("method")
    mid = msg.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": 1}})
    elif method == "session/new":
        send({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": SESSION_ID}})
    elif method == "session/prompt":
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": SESSION_ID,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"text": "hello from fixture"},
                },
            },
        })
        send({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": SESSION_ID,
                "update": {
                    "sessionUpdate": "usage_update",
                    "inputTokens": 7,
                    "outputTokens": 4,
                },
            },
        })
        send({
            "jsonrpc": "2.0",
            "id": mid,
            "result": {
                "stopReason": "end_turn",
                "model": "fixture-model",
            },
        })
    else:
        send({"jsonrpc": "2.0", "id": mid, "result": {}})
# EOF on stdin (session close) falls out of the loop -> clean process exit.
"#;

/// A fixture that completes the handshake, then ignores stdin entirely
/// (does not exit on EOF) so tests can exercise the SIGTERM/SIGKILL
/// termination path in `AcpSession::close` / `Drop`.
pub const STUBBORN_AGENT: &str = r#"
import sys, json, time

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

def read_one():
    line = sys.stdin.readline()
    return json.loads(line) if line.strip() else None

msg = read_one()
send({"jsonrpc": "2.0", "id": msg.get("id"), "result": {"protocolVersion": 1}})
msg = read_one()
send({"jsonrpc": "2.0", "id": msg.get("id"), "result": {"sessionId": "agent-session-stubborn"}})

# Never read stdin again, never exit on EOF - only a signal will stop this.
while True:
    time.sleep(3600)
"#;

/// A fixture used for JSON-RPC framing tests: echoes recognized methods
/// back verbatim so the test can assert on exact wire structure, and can
/// issue reverse requests / notifications on demand.
pub const FRAMING_AGENT: &str = r#"
import sys, json

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

def read_one():
    line = sys.stdin.readline()
    return json.loads(line) if line.strip() else None

while True:
    msg = read_one()
    if msg is None:
        break
    method = msg.get("method")
    mid = msg.get("id")

    if method == "ping":
        send({"jsonrpc": "2.0", "id": mid, "result": {"echo": msg.get("params")}})
    elif method == "boom":
        send({"jsonrpc": "2.0", "id": mid, "error": {"code": 123, "message": "nope"}})
    elif method == "fire_notification":
        send({"jsonrpc": "2.0", "method": "note", "params": {"x": 1}})
        send({"jsonrpc": "2.0", "id": mid, "result": {"ok": True}})
    elif method == "ask_reverse":
        send({"jsonrpc": "2.0", "id": 7, "method": "myreverse", "params": {"foo": "bar"}})
        resp = read_one()
        send({
            "jsonrpc": "2.0",
            "id": mid,
            "result": {"reverse_id": resp.get("id"), "reverse_result": resp.get("result")},
        })
    else:
        send({"jsonrpc": "2.0", "id": mid, "result": {}})
"#;

/// Write `script` to a fresh temp file and return its path.
///
/// `python3 <path>` is used as the profile command rather than a shebang +
/// executable bit, so this works regardless of the temp filesystem's mount
/// options (e.g. `noexec`).
pub fn write_fixture(dir: &Path, name: &str, script: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, script).expect("write fixture script");
    path
}

/// Build a profile that spawns `python3 <script>` with fast timeouts
/// suitable for tests.
pub fn fixture_profile(script: &Path) -> AcpAgentProfile {
    AcpAgentProfile {
        command: format!("python3 {}", shell_words::quote(&script.to_string_lossy())),
        description: None,
        close_grace_ms: 20,
        session_create_timeout_ms: 5_000,
        permission_mode: PermissionMode::DenyAll,
        env: Default::default(),
        default_mode: None,
        default_model: None,
        route_capabilities: apxm_acp::default_route_capabilities(),
        system_prompt: None,
        skip_preamble: true,
        capabilities: Vec::new(),
        sandbox: false,
    }
}

/// Whether a process with this pid is (still) alive, per `/proc` on Linux.
pub fn process_alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Poll `process_alive` until it returns `false` or `timeout` elapses.
/// Returns `true` if the process died within the timeout.
pub async fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !process_alive(pid) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub fn default_aam_context() -> apxm_core::types::aam::AamContext {
    apxm_core::types::aam::AamContext::default()
}
