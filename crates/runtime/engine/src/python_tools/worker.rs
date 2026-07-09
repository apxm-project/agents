//! Python tool worker — subprocess bridge over NDJSON stdin/stdout.
//!
//! Spawns `python -m apxm.tool_worker` as a long-lived child process.
//! Requests are multiplexed by `req_id`; a background demuxer task routes
//! responses to the correct `oneshot::Sender`.

use super::constants::{
    CAPABILITY_NAME, MANIFEST_TEMPFILE_PREFIX, PYTHON_BIN, PYTHON_FRONTEND_PATH,
    PYTHON_MODULE_FLAG, PYTHONUNBUFFERED, REPO_MARKER, TRACE_TARGET, WORKER_MODULE,
};
use super::protocol::{
    CallRequest, CancelRequest, ErrorEnvelope, HostResultResponse, PROTOCOL_VERSION, WorkerRequest,
    WorkerResponse,
};
use apxm_core::error::RuntimeError;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

/// Build a `RuntimeError::Capability` tagged with this module's capability
/// name. Centralized so the capability label cannot drift between sites.
fn cap_err(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Capability {
        capability: CAPABILITY_NAME.into(),
        message: message.into(),
    }
}

fn python_frontend_from_repo_root(repo_root: PathBuf) -> Option<PathBuf> {
    let mut path = repo_root;
    for segment in PYTHON_FRONTEND_PATH {
        path.push(segment);
    }
    path.is_dir().then_some(path)
}

fn find_python_frontend_from_ancestors(start: &Path) -> Option<PathBuf> {
    for candidate in start.ancestors() {
        if candidate.join(REPO_MARKER).is_file()
            && let Some(path) = python_frontend_from_repo_root(candidate.to_path_buf())
        {
            return Some(path);
        }
    }
    None
}

fn source_python_frontend_path() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir()
        && let Some(path) = find_python_frontend_from_ancestors(&cwd)
    {
        return Some(path);
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    find_python_frontend_from_ancestors(manifest_dir)
}

/// Resolve the python interpreter to spawn. Modern distros ship only `python3`
/// (no bare `python`), so prefer it and fall back to the `PYTHON_BIN` default.
/// Without this the worker spawn fails with ENOENT on python3-only hosts and no
/// python @tool/@hook handler can ever run.
fn resolve_python_bin() -> &'static str {
    for bin in ["python3", PYTHON_BIN] {
        let found = std::process::Command::new(bin)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok();
        if found {
            return bin;
        }
    }
    PYTHON_BIN
}

fn pythonpath_with_source_frontend() -> Option<std::ffi::OsString> {
    let frontend = source_python_frontend_path()?;
    let mut entries = vec![frontend];
    if let Some(existing) = std::env::var_os(apxm_core::constants::env::PYTHONPATH) {
        entries.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(entries).ok()
}

/// Handle to the Python tool worker subprocess.
///
/// Manages a single child process that serves tool invocations over NDJSON.
/// Thread-safe: can be shared via `Arc<PythonHandlerWorker>` across executor tasks.
pub struct PythonHandlerWorker {
    /// Sender for writing NDJSON lines to worker stdin.
    stdin_tx: tokio::sync::Mutex<tokio::process::ChildStdin>,
    /// Pending response channels, keyed by req_id. An awaiting call receives a
    /// STREAM of frames (interleaved `host_call`s then a final `result`), so
    /// this is an mpsc sender rather than a one-shot.
    /// Uses `parking_lot::RwLock` (never held across `.await`).
    pending: Arc<RwLock<HashMap<String, mpsc::UnboundedSender<WorkerResponse>>>>,
    /// Handle to the background demuxer task.
    _demuxer: tokio::task::JoinHandle<()>,
    /// Child process handle (held for Drop cleanup).
    _child: Arc<tokio::sync::Mutex<Child>>,
    /// Manifest working dir — kept alive for the worker's lifetime so the
    /// manifest path stays valid (and, under sandbox, stays bound as the
    /// worker's writable cwd) even if the worker rereads it.
    _workdir: tempfile::TempDir,
    /// Monotonic request counter for generating unique req_ids.
    next_id: std::sync::atomic::AtomicU64,
}

impl PythonHandlerWorker {
    /// Spawn the Python tool worker subprocess.
    ///
    /// The `manifest_json` is written to a temp file whose path is passed as
    /// argv[1] (the worker reads it via `_load_manifest`). A temp file avoids
    /// argv length limits and quoting hazards for non-trivial manifests.
    pub async fn spawn(manifest_json: &str) -> Result<Self, RuntimeError> {
        Self::spawn_with_env(manifest_json, &[], None, false).await
    }

    /// Spawn with additional environment variables overlaid on the inherited env.
    ///
    /// Used to inject things like `PYTHONPATH` (for non-installed user modules),
    /// `PYTHONUNBUFFERED=1`, or thread-pool caps (`OPENBLAS_NUM_THREADS=1`).
    ///
    /// When `sandbox` is `Some` and the backend is available + isolates, the
    /// worker is launched inside that OS sandbox (e.g. bubblewrap): read-only
    /// root, ephemeral `/tmp`, network unshared, with the manifest working dir
    /// bound writable as the worker's cwd. Author @tool/@hook handlers then run
    /// confined. `None` (or an unavailable backend) runs the worker directly.
    pub async fn spawn_with_env(
        manifest_json: &str,
        extra_env: &[(&str, &str)],
        sandbox: Option<&Arc<dyn crate::sandbox::SandboxBackend>>,
        sandbox_required: bool,
    ) -> Result<Self, RuntimeError> {
        // Write the manifest into a temp DIRECTORY (not a bare file): under a
        // sandbox the dir is bound writable as the worker's cwd so the manifest
        // is reachable through the otherwise read-only/ tmpfs filesystem view.
        let workdir = tempfile::Builder::new()
            .prefix(MANIFEST_TEMPFILE_PREFIX)
            .tempdir()
            .map_err(|e| cap_err(format!("Failed to create manifest workdir: {}", e)))?;
        let manifest_path = workdir.path().join("manifest.json");
        std::fs::write(&manifest_path, manifest_json.as_bytes())
            .map_err(|e| cap_err(format!("Failed to write manifest: {}", e)))?;
        let manifest_path_str = manifest_path
            .to_str()
            .ok_or_else(|| cap_err("manifest path is not valid UTF-8"))?;

        // Base argv: `python -m apxm.tool_worker <manifest>`. Optionally wrapped
        // by the sandbox backend (bwrap) which forwards stdio transparently.
        let py = resolve_python_bin().to_string();
        let base_args: Vec<String> = vec![
            PYTHON_MODULE_FLAG.to_string(),
            WORKER_MODULE.to_string(),
            manifest_path_str.to_string(),
        ];
        let (program, args) = match sandbox.filter(|b| b.is_available()) {
            Some(backend) => {
                tracing::info!(
                    target: TRACE_TARGET,
                    backend = %backend.capabilities().name,
                    "Sandboxing python tool worker"
                );
                // No network for handlers (they must use capabilities for I/O);
                // workdir bound writable as cwd.
                backend.wrap_command(&py, &base_args, workdir.path(), false)
            }
            // Fail CLOSED: when sandboxing was required (the trusted server-python
            // path sets it) but no OS-isolating backend is available, refuse to
            // run author code unsandboxed rather than silently degrading — the
            // trust gate's guarantee must hold (security: no fail-open).
            None if sandbox_required => {
                return Err(cap_err(
                    "python worker sandbox required (APXM_SANDBOX_PYTHON) but no \
                     OS-isolating backend (e.g. bubblewrap) is available; refusing \
                     to run author python unsandboxed",
                ));
            }
            None => (py, base_args),
        };

        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        // Security (constitution-aligned hardening): scrub secret-looking env
        // vars so an author @tool/@hook handler cannot read the host's
        // credentials (KEK, LLM gateway keys, auth bearer, AWS/DB creds). We use
        // a denylist rather than env_clear so the worker keeps the vars python
        // genuinely needs to run (PATH, venv/loader vars) across environments;
        // handlers that need credentials must go through the capability/
        // credential system, not raw env inheritance.
        const SECRET_MARKERS: &[&str] = &[
            "KEY",
            "TOKEN",
            "SECRET",
            "PASSWORD",
            "PASSWD",
            "CREDENTIAL",
            "BEARER",
            "KEK",
            "PRIVATE",
        ];
        for (key, _) in std::env::vars() {
            let upper = key.to_ascii_uppercase();
            if SECRET_MARKERS.iter().any(|m| upper.contains(m)) {
                cmd.env_remove(&key);
            }
        }
        cmd.env(
            PYTHONUNBUFFERED,
            apxm_core::constants::env::flag_values::ENABLED,
        );
        if let Some(pythonpath) = pythonpath_with_source_frontend() {
            cmd.env(apxm_core::constants::env::PYTHONPATH, pythonpath);
        }
        // Explicit caller-supplied env is trusted (set by the runtime, not the
        // handler) and is applied after the allowlist.
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| cap_err(format!("Failed to spawn Python tool worker: {}", e)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| cap_err("Failed to capture worker stdin"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| cap_err("Failed to capture worker stdout"))?;

        let pending: Arc<RwLock<HashMap<String, mpsc::UnboundedSender<WorkerResponse>>>> =
            Arc::new(RwLock::new(HashMap::new()));

        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!(target: TRACE_TARGET, "stderr: {}", line);
                }
            });
        }

        // Spawn stdout demuxer task.
        let demux_pending = Arc::clone(&pending);
        let demuxer = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let resp: WorkerResponse = match serde_json::from_str(&line) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!(
                            target: TRACE_TARGET,
                            "Failed to parse worker response: {} — line: {}",
                            e, line
                        );
                        continue;
                    }
                };

                // Route to the awaiting call's channel. A `result` is final and
                // is keyed by its own req_id; a `host_call` is an intermediate
                // frame keyed by the PARENT call it is raised within. Cleanup of
                // the pending entry is the awaiting call's responsibility.
                let (key, frame) = match resp {
                    WorkerResponse::Result(r) => (r.req_id.clone(), WorkerResponse::Result(r)),
                    WorkerResponse::HostCall(h) => {
                        (h.parent_req_id.clone(), WorkerResponse::HostCall(h))
                    }
                };
                let sender = {
                    let map = demux_pending.read();
                    map.get(&key).cloned()
                };
                if let Some(tx) = sender {
                    let _ = tx.send(frame);
                } else {
                    tracing::warn!(
                        target: TRACE_TARGET,
                        "Received worker frame for unknown req_id: {}",
                        key
                    );
                }
            }
            tracing::info!(target: TRACE_TARGET, "Demuxer task exited (worker stdout closed)");
        });

        let child = Arc::new(tokio::sync::Mutex::new(child));

        Ok(Self {
            stdin_tx: tokio::sync::Mutex::new(stdin),
            pending,
            _demuxer: demuxer,
            _child: child,
            _workdir: workdir,
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Invoke a tool handler on the worker.
    ///
    /// Sends a `call` request and waits for the matching response, subject to
    /// `deadline`. Tool handlers do not raise host calls; any attempt is
    /// rejected so the worker cannot stall.
    pub async fn call(
        &self,
        handler_id: &str,
        args: serde_json::Value,
        deadline: Duration,
    ) -> Result<serde_json::Value, RuntimeError> {
        self.dispatch(handler_id, args, deadline, |method, _params| async move {
            Err(format!(
                "host call '{}' is not available on the tool path",
                method
            ))
        })
        .await
    }

    /// Invoke a handler that MAY raise host calls (e.g. a lifecycle hook calling
    /// `ctx.summarize` → `llm.ask`). `host` services each host call with the
    /// caller's `ExecutionContext` and is awaited inline, so no `'static` ctx is
    /// required — the same context that owns the hook services its LLM calls.
    pub async fn call_with_host<F, Fut>(
        &self,
        handler_id: &str,
        args: serde_json::Value,
        deadline: Duration,
        host: F,
    ) -> Result<serde_json::Value, RuntimeError>
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: Future<Output = Result<serde_json::Value, String>>,
    {
        self.dispatch(handler_id, args, deadline, host).await
    }

    /// Core request loop: send a `call`, then consume frames until the final
    /// `result`. Intermediate `host_call` frames are serviced by `host` and
    /// answered with a `host_result`. `deadline` bounds the WHOLE exchange.
    async fn dispatch<F, Fut>(
        &self,
        handler_id: &str,
        args: serde_json::Value,
        deadline: Duration,
        host: F,
    ) -> Result<serde_json::Value, RuntimeError>
    where
        F: Fn(String, serde_json::Value) -> Fut,
        Fut: Future<Output = Result<serde_json::Value, String>>,
    {
        let req_id = format!(
            "u-{}",
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );

        let (tx, mut rx) = mpsc::unbounded_channel();
        self.pending.write().insert(req_id.clone(), tx);

        let request = WorkerRequest::Call(CallRequest {
            v: PROTOCOL_VERSION,
            req_id: req_id.clone(),
            tool_id: handler_id.to_string(),
            args,
            deadline_ms: deadline.as_millis() as u64,
        });
        if let Err(e) = self.write_request(&request).await {
            self.pending.write().remove(&req_id);
            return Err(e);
        }

        let deadline_at = tokio::time::Instant::now() + deadline;
        loop {
            match tokio::time::timeout_at(deadline_at, rx.recv()).await {
                Err(_) => {
                    // Overall deadline elapsed — drop the pending entry and
                    // proactively cancel the in-flight worker task so its
                    // compute is reclaimed (best-effort; the worker also
                    // enforces `deadline_ms`).
                    self.pending.write().remove(&req_id);
                    let _ = self.cancel(&req_id).await;
                    return Err(cap_err(format!(
                        "Tool call {} timed out after {}ms",
                        req_id,
                        deadline.as_millis()
                    )));
                }
                Ok(None) => {
                    self.pending.write().remove(&req_id);
                    return Err(cap_err(format!(
                        "Worker dropped response channel for req_id {} (worker may have crashed)",
                        req_id
                    )));
                }
                Ok(Some(WorkerResponse::HostCall(h))) => {
                    let outcome = host(h.method.clone(), h.params.clone()).await;
                    let host_result = match outcome {
                        Ok(value) => HostResultResponse {
                            v: PROTOCOL_VERSION,
                            req_id: h.req_id,
                            ok: true,
                            value: Some(value),
                            error: None,
                        },
                        Err(message) => HostResultResponse {
                            v: PROTOCOL_VERSION,
                            req_id: h.req_id,
                            ok: false,
                            value: None,
                            error: Some(ErrorEnvelope {
                                kind: "host_error".into(),
                                message,
                                traceback: None,
                            }),
                        },
                    };
                    if let Err(e) = self
                        .write_request(&WorkerRequest::HostResult(host_result))
                        .await
                    {
                        self.pending.write().remove(&req_id);
                        return Err(e);
                    }
                }
                Ok(Some(WorkerResponse::Result(call_resp))) => {
                    self.pending.write().remove(&req_id);
                    return if call_resp.ok {
                        Ok(call_resp.value.unwrap_or(serde_json::Value::Null))
                    } else {
                        let err = call_resp.error.unwrap_or_else(|| ErrorEnvelope {
                            kind: "unknown".into(),
                            message: "Tool returned ok=false with no error envelope".into(),
                            traceback: None,
                        });
                        Err(cap_err(format!("[{}] {}", err.kind, err.message)))
                    };
                }
            }
        }
    }

    /// Serialize a request and write it to the worker's stdin under the lock.
    async fn write_request(&self, request: &WorkerRequest) -> Result<(), RuntimeError> {
        let mut line = serde_json::to_string(request)
            .map_err(|e| RuntimeError::Serialization(e.to_string()))?;
        line.push('\n');
        let mut stdin = self.stdin_tx.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| cap_err(format!("Failed to write to worker stdin: {}", e)))?;
        stdin
            .flush()
            .await
            .map_err(|e| cap_err(format!("Failed to flush worker stdin: {}", e)))?;
        Ok(())
    }

    /// Send a cancel request for an in-flight call.
    pub async fn cancel(&self, req_id: &str) -> Result<(), RuntimeError> {
        let request = WorkerRequest::Cancel(CancelRequest {
            v: PROTOCOL_VERSION,
            req_id: req_id.to_string(),
        });

        let mut line = serde_json::to_string(&request)
            .map_err(|e| RuntimeError::Serialization(e.to_string()))?;
        line.push('\n');

        let mut stdin = self.stdin_tx.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| cap_err(format!("Failed to send cancel request: {}", e)))?;
        stdin
            .flush()
            .await
            .map_err(|e| cap_err(format!("Failed to flush cancel request: {}", e)))?;

        Ok(())
    }
}
