//! The frontend selector and the interpreter boundary each selector owns.
//!
//! Capturing typed intent from Python source requires the Python authoring
//! frontend to run, and the same holds for TypeScript. That is the mechanism the
//! frontends require; it is not something a caller of this port configures,
//! observes, or works around. This module holds the whole of it: which
//! interpreter each selector runs, how the interpreter is confined, and how its
//! exit is translated into one closed diagnostic.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::diagnostic::{SourceDiagnostic, SourceDiagnosticCode};

/// The maximum time an authoring frontend may hold the source-port boundary.
/// The Python harness has interpreter resource limits, but the TypeScript
/// harness runs under Node's permission model, which does not bound CPU time.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
/// A frontend must not be able to make capture retain unbounded output. The
/// readers continue draining after this budget is full so noisy output cannot
/// deadlock the child on a full pipe.
const CAPTURE_MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const CAPTURE_READ_BUFFER_BYTES: usize = 16 * 1024;
const CAPTURE_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// The closed authoring-frontend selector. It matches the source-language
/// closure of the semantic surface exactly: there is no third frontend and no
/// alternate path within a selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Frontend {
    Python,
    Typescript,
}

impl Frontend {
    /// The canonical wire string for this selector.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Typescript => "typescript",
        }
    }

    /// The source language the captured graph declares for this selector.
    #[must_use]
    pub const fn source_language(self) -> apxm_program::SourceLanguage {
        match self {
            Self::Python => apxm_program::SourceLanguage::Python,
            Self::Typescript => apxm_program::SourceLanguage::Typescript,
        }
    }

    /// The capture harness this selector runs. The harness is embedded in the
    /// binary, so the port has no installed script to locate and no path a
    /// caller can redirect.
    const fn harness(self) -> &'static str {
        match self {
            Self::Python => include_str!("../harness/capture_python.py"),
            Self::Typescript => include_str!("../harness/capture_typescript.mjs"),
        }
    }

    /// The subdirectory of the frontend root the interpreter is allowed to read,
    /// where the interpreter enforces a read wall of its own.
    fn confinement_arguments(self, frontend_root: &Path) -> Vec<String> {
        match self {
            // The isolated interpreter ignores every `PYTHON*` variable and the
            // user site directory, and reads its program text from the argument
            // rather than from a file. Confinement inside the interpreter is the
            // harness's audit hook and resource limits.
            Self::Python => vec!["-I".to_string(), "-B".to_string(), "-c".to_string()],
            // Node's permission model is an OS-level read wall: the process can
            // read the declared frontend package and nothing else on the
            // filesystem, and can write nowhere at all. Synchronous loader
            // hooks keep submitted code in this same confined process; worker
            // threads are deliberately not granted.
            Self::Typescript => vec![
                // `--experimental-permission` is supported across the Node
                // versions used by the repository toolchain.
                "--no-warnings".to_string(),
                "--experimental-permission".to_string(),
                format!("--allow-fs-read={}", frontend_root.display()),
                "--input-type=module".to_string(),
                "--eval".to_string(),
            ],
        }
    }
}

/// One capture request handed to an interpreter on its standard input.
#[derive(Debug, Serialize)]
struct HarnessRequest<'a> {
    frontend_root: &'a Path,
    entrypoint: &'a str,
    source: &'a str,
    /// The host capability ids the package declares. The harness hands them to
    /// the frontend before evaluating the source, so the minted Capability set
    /// inside the interpreter is the builtin catalogue united with these.
    host_capabilities: &'a [String],
}

/// The single-field document a capture harness writes on success.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HarnessResponse {
    frontend_graph: serde_json::Value,
}

/// Run one capture and return the FrontendGraph JSON value the frontend
/// recorded, or the one closed diagnostic that rejects it.
///
/// Whether a frontend package is usable is decided in exactly one place: the
/// harness, which resolves the package the way the language actually resolves
/// it. Re-deciding it here from the shape of the path would be a second answer
/// to the same question, and the two would drift.
pub(crate) fn capture(
    frontend: Frontend,
    frontend_root: &Path,
    driver: &Path,
    entrypoint: &str,
    source: &str,
    host_capabilities: &[String],
) -> Result<serde_json::Value, SourceDiagnostic> {
    let request = serde_json::to_vec(&HarnessRequest {
        frontend_root,
        entrypoint,
        source,
        host_capabilities,
    })
    .map_err(|error| {
        SourceDiagnostic::new(
            SourceDiagnosticCode::RequestInvalid,
            format!("the capture request does not serialize: {error}"),
        )
    })?;

    let output = spawn(frontend, frontend_root, driver, &request)?;

    if !output.status.success() {
        return Err(harness_rejection(frontend, &output.stderr));
    }

    decode_response(frontend, &output.stdout)
}

/// Decode the whole capture output as exactly one `HarnessResponse`.
///
/// The entire byte stream is decoded as one document carrying exactly one
/// field. Trailing bytes, leading bytes, and any field beyond `frontend_graph`
/// each reject: a capture that emitted anything besides the typed graph — AIR
/// among it — is not a capture this port accepts a graph from.
fn decode_response(
    frontend: Frontend,
    stdout: &[u8],
) -> Result<serde_json::Value, SourceDiagnostic> {
    let response: HarnessResponse = serde_json::from_slice(stdout).map_err(|error| {
        SourceDiagnostic::new(
            SourceDiagnosticCode::FrontendOutputInvalid,
            format!(
                "the {} authoring frontend did not emit exactly one FrontendGraph document: {error}",
                frontend.wire()
            ),
        )
    })?;
    Ok(response.frontend_graph)
}

/// Run the exact declared interpreter driver. A missing or unstartable driver
/// is unavailability, never a fallback search, panic, or silent success.
fn spawn(
    frontend: Frontend,
    frontend_root: &Path,
    driver: &Path,
    request: &[u8],
) -> Result<std::process::Output, SourceDiagnostic> {
    spawn_with_timeout(frontend, frontend_root, driver, request, CAPTURE_TIMEOUT)
}

/// Run one declared interpreter with a bounded lifetime.
fn spawn_with_timeout(
    frontend: Frontend,
    frontend_root: &Path,
    driver: &Path,
    request: &[u8],
    timeout: Duration,
) -> Result<std::process::Output, SourceDiagnostic> {
    let mut command = Command::new(driver);
    command
        .args(frontend.confinement_arguments(frontend_root))
        .arg(frontend.harness())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|error| {
        SourceDiagnostic::new(
            SourceDiagnosticCode::FrontendUnavailable,
            format!(
                "the declared {} authoring frontend driver '{}' could not start: {error}",
                frontend.wire(),
                driver.display()
            ),
        )
    })?;

    let started = Instant::now();
    let deadline = started + timeout;
    let output_budget = Arc::new(AtomicUsize::new(0));
    let writer = spawn_writer(child.stdin.take(), request.to_vec());
    let stdout = spawn_reader(child.stdout.take(), Arc::clone(&output_budget));
    let stderr = spawn_reader(child.stderr.take(), Arc::clone(&output_budget));

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = collect_output(status, stdout, stderr, writer, deadline).map_err(
                    |error| {
                        SourceDiagnostic::new(
                            SourceDiagnosticCode::FrontendUnavailable,
                            format!(
                                "the declared {} authoring frontend driver '{}' did not complete capture: {error}",
                                frontend.wire(),
                                driver.display()
                            ),
                        )
                    },
                )?;
                return Ok(output);
            }
            Ok(None) if Instant::now() >= deadline => {
                terminate(&mut child);
                return Err(SourceDiagnostic::new(
                    SourceDiagnosticCode::FrontendUnavailable,
                    format!(
                        "the declared {} authoring frontend driver '{}' exceeded the capture timeout of {} ms",
                        frontend.wire(),
                        driver.display(),
                        timeout.as_millis()
                    ),
                ));
            }
            Ok(None) => {
                thread::sleep(
                    CAPTURE_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(error) => {
                terminate(&mut child);
                return Err(SourceDiagnostic::new(
                    SourceDiagnosticCode::FrontendUnavailable,
                    format!(
                        "the declared {} authoring frontend driver '{}' could not be observed: {error}",
                        frontend.wire(),
                        driver.display()
                    ),
                ));
            }
        }
    }
}

type CaptureReceiver<T> = mpsc::Receiver<std::io::Result<T>>;

fn spawn_writer(stdin: Option<ChildStdin>, request: Vec<u8>) -> CaptureReceiver<()> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = stdin.map_or(Ok(()), |mut stdin| stdin.write_all(&request));
        let _ = sender.send(result);
    });
    receiver
}

fn spawn_reader<R>(reader: Option<R>, output_budget: Arc<AtomicUsize>) -> CaptureReceiver<Vec<u8>>
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = reader.map_or_else(
            || Ok(Vec::new()),
            |reader| read_stream(reader, output_budget),
        );
        let _ = sender.send(result);
    });
    receiver
}

fn read_stream<R: Read>(
    mut reader: R,
    output_budget: Arc<AtomicUsize>,
) -> std::io::Result<Vec<u8>> {
    let mut captured = Vec::new();
    let mut buffer = [0_u8; CAPTURE_READ_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(captured);
        }

        let retained = reserve_output(&output_budget, CAPTURE_MAX_OUTPUT_BYTES, read);
        captured.extend_from_slice(&buffer[..retained]);
    }
}

fn reserve_output(budget: &AtomicUsize, limit: usize, requested: usize) -> usize {
    loop {
        let used = budget.load(Ordering::Acquire);
        let available = limit.saturating_sub(used);
        let retained = available.min(requested);
        if retained == 0 {
            return 0;
        }
        if budget
            .compare_exchange_weak(used, used + retained, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return retained;
        }
    }
}

fn collect_output(
    status: std::process::ExitStatus,
    stdout: CaptureReceiver<Vec<u8>>,
    stderr: CaptureReceiver<Vec<u8>>,
    writer: CaptureReceiver<()>,
    deadline: Instant,
) -> std::io::Result<std::process::Output> {
    // Use one absolute deadline for all pipe workers. A successful child must
    // not extend the lifecycle merely because a worker failed to close.
    let stdout = receive_until(stdout, deadline)?;
    let stderr = receive_until(stderr, deadline)?;
    receive_until(writer, deadline)?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn receive_until<T>(receiver: CaptureReceiver<T>, deadline: Instant) -> std::io::Result<T> {
    receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "capture pipe worker timed out",
            ),
            mpsc::RecvTimeoutError::Disconnected => std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "capture pipe worker disconnected",
            ),
        })?
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Translate a harness exit into one closed diagnostic. The first stderr line is
/// the closed reason token; the rest is its detail.
fn harness_rejection(frontend: Frontend, stderr: &[u8]) -> SourceDiagnostic {
    let rendered = String::from_utf8_lossy(stderr);
    let mut lines = rendered.splitn(2, '\n');
    let token = lines.next().unwrap_or("").trim();
    let detail = lines.next().unwrap_or("").trim();

    match SourceDiagnosticCode::from_harness_token(token) {
        Some(code) => SourceDiagnostic::new(code, detail.to_string()),
        // A harness that died without reporting a closed reason — killed by a
        // resource limit, or crashed — is unavailability of the capture boundary
        // itself. It is never reported as an accepted or partially captured
        // program.
        None => SourceDiagnostic::new(
            SourceDiagnosticCode::FrontendUnavailable,
            format!(
                "the {} authoring frontend capture ended without a reported reason: {}",
                frontend.wire(),
                rendered.trim()
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{Frontend, decode_response, harness_rejection, spawn_with_timeout};
    use crate::diagnostic::SourceDiagnosticCode;

    /// The one accepted shape: exactly one document carrying exactly the graph.
    #[test]
    fn exactly_one_graph_field_decodes() {
        let graph = decode_response(Frontend::Python, br#"{"frontend_graph": {"k": 1}}"#)
            .expect("a capture output holding exactly the graph decodes");
        assert_eq!(graph, serde_json::json!({"k": 1}));
    }

    /// A capture that emitted AIR alongside the graph is rejected rather than
    /// having the AIR ignored. Only Rust lowering produces AIR, so a frontend
    /// that produced any is a frontend this port takes no graph from.
    #[test]
    fn a_capture_that_also_emitted_air_is_rejected() {
        let diagnostic = decode_response(
            Frontend::Python,
            br#"{"frontend_graph": {"k": 1}, "air": "module { }"}"#,
        )
        .expect_err("a capture output carrying AIR beside the graph is rejected");
        assert_eq!(diagnostic.code, SourceDiagnosticCode::FrontendOutputInvalid);
    }

    /// Bytes after the document reject: the whole output is the document.
    #[test]
    fn output_with_trailing_bytes_is_rejected() {
        let diagnostic = decode_response(
            Frontend::Typescript,
            br#"{"frontend_graph": {"k": 1}} trailing"#,
        )
        .expect_err("a capture output with trailing bytes is rejected");
        assert_eq!(diagnostic.code, SourceDiagnosticCode::FrontendOutputInvalid);
    }

    #[test]
    fn a_capture_without_a_closed_reason_is_unavailable() {
        let diagnostic = harness_rejection(Frontend::Python, b"");

        assert_eq!(diagnostic.code, SourceDiagnosticCode::FrontendUnavailable);
        assert!(diagnostic.message.contains("without a reported reason"));
    }

    #[cfg(unix)]
    #[test]
    fn a_capture_that_does_not_exit_is_killed_at_the_boundary() {
        use std::os::unix::fs::PermissionsExt;

        let path = unique_test_path("sleeping-driver");
        fs::write(&path, "#!/bin/sh\nwhile :; do :; done\n").expect("write sleeping driver");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("make sleeping driver executable");

        let input = vec![b'x'; 128 * 1024];
        let diagnostic = spawn_with_timeout(
            Frontend::Python,
            std::path::Path::new("/tmp"),
            &path,
            &input,
            Duration::from_millis(25),
        )
        .expect_err("a capture that exceeds its deadline is rejected");

        let _ = fs::remove_file(&path);
        assert_eq!(diagnostic.code, SourceDiagnosticCode::FrontendUnavailable);
        assert!(diagnostic.message.contains("capture timeout"));
    }

    #[cfg(unix)]
    #[test]
    fn a_capture_drains_large_stdout_and_stderr_while_writing_input() {
        use std::os::unix::fs::PermissionsExt;

        let path = unique_test_path("large-output-driver");
        fs::write(
            &path,
            "#!/bin/sh\ncat >/dev/null\ndd if=/dev/zero bs=131072 count=1 2>/dev/null\ndd if=/dev/zero bs=131072 count=1 1>&2 2>/dev/null\n",
        )
        .expect("write large-output driver");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("make large-output driver executable");

        let input = vec![b'x'; 512 * 1024];
        let output = spawn_with_timeout(
            Frontend::Python,
            std::path::Path::new("/tmp"),
            &path,
            &input,
            Duration::from_secs(2),
        )
        .expect("large bidirectional output must not deadlock capture");

        let _ = fs::remove_file(&path);
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), 131_072);
        assert_eq!(output.stderr.len(), 131_072);
    }

    #[test]
    fn a_decode_failure_does_not_poison_the_next_capture() {
        let failed = decode_response(Frontend::Typescript, b"not-json")
            .expect_err("invalid capture output is rejected");
        assert_eq!(failed.code, SourceDiagnosticCode::FrontendOutputInvalid);

        let recovered = decode_response(
            Frontend::Typescript,
            br#"{"frontend_graph":{"schema_version":"apxm.frontend-graph"}}"#,
        )
        .expect("the next independent capture still decodes");
        assert_eq!(
            recovered,
            serde_json::json!({"schema_version": "apxm.frontend-graph"})
        );
    }

    #[cfg(unix)]
    fn unique_test_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "apxm-source-port-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is after the Unix epoch")
                .as_nanos()
        ))
    }
}
