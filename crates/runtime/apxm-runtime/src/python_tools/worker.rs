//! Python tool worker — subprocess bridge over NDJSON stdin/stdout.
//!
//! Spawns `python -m apxm.tool_worker` as a long-lived child process.
//! Requests are multiplexed by `req_id`; a background demuxer task routes
//! responses to the correct `oneshot::Sender`.

use super::constants::{
    CAPABILITY_NAME, MANIFEST_TEMPFILE_PREFIX, MANIFEST_TEMPFILE_SUFFIX, PYTHON_BIN,
    PYTHON_FRONTEND_PATH, PYTHON_MODULE_FLAG, PYTHONUNBUFFERED, REPO_MARKER, TRACE_TARGET,
    WORKER_MODULE,
};
use super::protocol::{
    CallRequest, CallResponse, CancelRequest, ErrorEnvelope, PROTOCOL_VERSION, WorkerRequest,
    WorkerResponse,
};
use apxm_core::error::RuntimeError;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;

/// Build a `RuntimeError::Capability` tagged with this module's capability
/// name. Centralized so the capability label cannot drift between sites.
fn cap_err(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Capability {
        capability: CAPABILITY_NAME.into(),
        message: message.into(),
    }
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    for candidate in start.ancestors() {
        if candidate.join(REPO_MARKER).is_file() {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

fn source_python_frontend_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    let repo_root = find_repo_root(&cwd)?;
    let mut path = repo_root;
    for segment in PYTHON_FRONTEND_PATH {
        path.push(segment);
    }
    path.is_dir().then_some(path)
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
/// Thread-safe: can be shared via `Arc<PythonToolWorker>` across executor tasks.
pub struct PythonToolWorker {
    /// Sender for writing NDJSON lines to worker stdin.
    stdin_tx: tokio::sync::Mutex<tokio::process::ChildStdin>,
    /// Pending response channels, keyed by req_id.
    /// Uses `parking_lot::RwLock` (never held across `.await`).
    pending: Arc<RwLock<HashMap<String, oneshot::Sender<CallResponse>>>>,
    /// Handle to the background demuxer task.
    _demuxer: tokio::task::JoinHandle<()>,
    /// Child process handle (held for Drop cleanup).
    _child: Arc<tokio::sync::Mutex<Child>>,
    /// Manifest tempfile — kept alive for the worker's lifetime so the
    /// path passed in argv stays valid even if the worker rereads it.
    _manifest: tempfile::NamedTempFile,
    /// Monotonic request counter for generating unique req_ids.
    next_id: std::sync::atomic::AtomicU64,
}

impl PythonToolWorker {
    /// Spawn the Python tool worker subprocess.
    ///
    /// The `manifest_json` is written to a temp file whose path is passed as
    /// argv[1] (the worker reads it via `_load_manifest`). A temp file avoids
    /// argv length limits and quoting hazards for non-trivial manifests.
    pub async fn spawn(manifest_json: &str) -> Result<Self, RuntimeError> {
        Self::spawn_with_env(manifest_json, &[]).await
    }

    /// Spawn with additional environment variables overlaid on the inherited env.
    ///
    /// Used to inject things like `PYTHONPATH` (for non-installed user modules),
    /// `PYTHONUNBUFFERED=1`, or thread-pool caps (`OPENBLAS_NUM_THREADS=1`).
    pub async fn spawn_with_env(
        manifest_json: &str,
        extra_env: &[(&str, &str)],
    ) -> Result<Self, RuntimeError> {
        // Write manifest to a temp file. Held across spawn so the file exists
        // when the worker imports its modules, then unlinked on Drop.
        let manifest_file = {
            let mut f = tempfile::Builder::new()
                .prefix(MANIFEST_TEMPFILE_PREFIX)
                .suffix(MANIFEST_TEMPFILE_SUFFIX)
                .tempfile()
                .map_err(|e| cap_err(format!("Failed to create manifest tempfile: {}", e)))?;
            use std::io::Write;
            f.write_all(manifest_json.as_bytes())
                .map_err(|e| cap_err(format!("Failed to write manifest tempfile: {}", e)))?;
            f.flush()
                .map_err(|e| cap_err(format!("Failed to flush manifest tempfile: {}", e)))?;
            f
        };
        let manifest_path = manifest_file.path().to_owned();
        let manifest_path_str = manifest_path
            .to_str()
            .ok_or_else(|| cap_err("manifest tempfile path is not valid UTF-8"))?;

        let mut cmd = Command::new(PYTHON_BIN);
        cmd.args([PYTHON_MODULE_FLAG, WORKER_MODULE, manifest_path_str])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .env(PYTHONUNBUFFERED, apxm_core::constants::env::flag_values::ENABLED);
        if let Some(pythonpath) = pythonpath_with_source_frontend() {
            cmd.env(apxm_core::constants::env::PYTHONPATH, pythonpath);
        }
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

        let pending: Arc<RwLock<HashMap<String, oneshot::Sender<CallResponse>>>> =
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

                match resp {
                    WorkerResponse::Result(call_resp) => {
                        // Clone req_id and remove sender under lock — never held across await.
                        let sender = {
                            let mut map = demux_pending.write();
                            map.remove(&call_resp.req_id)
                        };
                        if let Some(tx) = sender {
                            let _ = tx.send(call_resp);
                        } else {
                            tracing::warn!(
                                target: TRACE_TARGET,
                                "Received response for unknown req_id: {}",
                                call_resp.req_id
                            );
                        }
                    }
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
            _manifest: manifest_file,
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Invoke a tool handler on the worker.
    ///
    /// Sends a `call` request and waits for the matching response, subject
    /// to `deadline`. Returns the tool's return value on success.
    pub async fn call(
        &self,
        handler_id: &str,
        args: serde_json::Value,
        deadline: Duration,
    ) -> Result<serde_json::Value, RuntimeError> {
        let req_id = format!(
            "u-{}",
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );

        let (tx, rx) = oneshot::channel();

        // Register pending sender (lock is NOT held across await).
        {
            let mut map = self.pending.write();
            map.insert(req_id.clone(), tx);
        }

        let request = WorkerRequest::Call(CallRequest {
            v: PROTOCOL_VERSION,
            req_id: req_id.clone(),
            tool_id: handler_id.to_string(),
            args,
            deadline_ms: deadline.as_millis() as u64,
        });

        // Serialize and send.
        let mut line = serde_json::to_string(&request).map_err(|e| {
            // Clean up pending entry on serialization failure.
            self.pending.write().remove(&req_id);
            RuntimeError::Serialization(e.to_string())
        })?;
        line.push('\n');

        {
            let mut stdin = self.stdin_tx.lock().await;
            stdin.write_all(line.as_bytes()).await.map_err(|e| {
                self.pending.write().remove(&req_id);
                cap_err(format!("Failed to write to worker stdin: {}", e))
            })?;
            stdin.flush().await.map_err(|e| {
                self.pending.write().remove(&req_id);
                cap_err(format!("Failed to flush worker stdin: {}", e))
            })?;
        }

        // Wait for response with timeout.
        let resp = tokio::time::timeout(deadline, rx).await.map_err(|_| {
            // Timeout — remove pending entry and send cancel.
            self.pending.write().remove(&req_id);
            cap_err(format!(
                "Tool call {} timed out after {}ms",
                req_id,
                deadline.as_millis()
            ))
        })?;

        let call_resp = resp.map_err(|_| {
            cap_err(format!(
                "Worker dropped response channel for req_id {} (worker may have crashed)",
                req_id
            ))
        })?;

        if call_resp.ok {
            Ok(call_resp.value.unwrap_or(serde_json::Value::Null))
        } else {
            let err = call_resp.error.unwrap_or_else(|| ErrorEnvelope {
                kind: "unknown".into(),
                message: "Tool returned ok=false with no error envelope".into(),
                traceback: None,
            });
            Err(cap_err(format!("[{}] {}", err.kind, err.message)))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_missing_python_module() {
        // Spawning with an invalid manifest should succeed (process starts)
        // but actual calls would fail. If python is not available at all,
        // spawn itself will fail.
        let result = PythonToolWorker::spawn("[]").await;
        // This test is lenient: if Python is available, spawn succeeds;
        // if not, it's an expected error.
        match result {
            Ok(_worker) => {
                // Worker spawned — the module may not exist but the process started.
                // It will exit shortly, which is fine.
            }
            Err(e) => {
                let msg = format!("{}", e);
                assert!(msg.contains("Failed to spawn"), "Unexpected error: {}", msg);
            }
        }
    }
}
