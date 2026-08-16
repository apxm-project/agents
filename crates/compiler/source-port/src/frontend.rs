//! The frontend selector and the interpreter boundary each selector owns.
//!
//! Capturing typed intent from Python source requires the Python authoring
//! frontend to run, and the same holds for TypeScript. That is the mechanism the
//! frontends require; it is not something a caller of this port configures,
//! observes, or works around. This module holds the whole of it: which
//! interpreter each selector runs, how the interpreter is confined, and how its
//! exit is translated into one closed diagnostic.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::diagnostic::{SourceDiagnostic, SourceDiagnosticCode};

/// The maximum time an authoring frontend may hold the source-port boundary.
/// The Python harness has interpreter resource limits, but the TypeScript
/// harness runs under Node's permission model, which does not bound CPU time.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);

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
            // filesystem, and can write nowhere at all.
            Self::Typescript => vec![
                // `--experimental-permission` is supported across the Node
                // versions used by the repository toolchain.
                "--no-warnings".to_string(),
                "--experimental-permission".to_string(),
                // Node's ESM loader hooks run in a worker. Granting the
                // loader worker explicitly keeps the permission wall active
                // while allowing the closed module table in the harness to
                // install its hooks.
                "--allow-worker".to_string(),
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
) -> Result<serde_json::Value, SourceDiagnostic> {
    let request = serde_json::to_vec(&HarnessRequest {
        frontend_root,
        entrypoint,
        source,
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
    let stdin = child.stdin.take();
    let request = request.to_vec();
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            // A harness that exits before reading closes the pipe; that is a
            // rejection to read from its status, not a failure to report here.
            let _ = stdin.write_all(&request);
        }
    });

    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output().map_err(|error| {
                    SourceDiagnostic::new(
                        SourceDiagnosticCode::FrontendUnavailable,
                        format!(
                            "the declared {} authoring frontend driver '{}' did not complete: {error}",
                            frontend.wire(),
                            driver.display()
                        ),
                    )
                });
                let _ = writer.join();
                return output;
            }
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
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
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
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

        let diagnostic = spawn_with_timeout(
            Frontend::Python,
            std::path::Path::new("/tmp"),
            &path,
            &[b'x'; 128 * 1024],
            Duration::from_millis(25),
        )
        .expect_err("a capture that exceeds its deadline is rejected");

        let _ = fs::remove_file(&path);
        assert_eq!(diagnostic.code, SourceDiagnosticCode::FrontendUnavailable);
        assert!(diagnostic.message.contains("capture timeout"));
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
