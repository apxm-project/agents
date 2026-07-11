//! TypeScript tool worker — Node subprocess bridge over NDJSON stdin/stdout.

use super::constants::{
    CAPABILITY_NAME, MANIFEST_TEMPFILE_PREFIX, NODE_BIN, REPO_MARKER, TRACE_TARGET,
    TYPESCRIPT_FRONTEND_PATH, WORKER_SCRIPT,
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
use tokio::sync::mpsc;

fn cap_err(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Capability {
        capability: CAPABILITY_NAME.into(),
        message: message.into(),
    }
}

fn worker_script_from_repo_root(repo_root: PathBuf) -> Option<PathBuf> {
    let mut path = repo_root;
    for segment in TYPESCRIPT_FRONTEND_PATH {
        path.push(segment);
    }
    path.push("scripts");
    path.push(WORKER_SCRIPT);
    path.is_file().then_some(path)
}

fn find_worker_script_from_ancestors(start: &Path) -> Option<PathBuf> {
    for candidate in start.ancestors() {
        if candidate.join(REPO_MARKER).is_file()
            && let Some(path) = worker_script_from_repo_root(candidate.to_path_buf())
        {
            return Some(path);
        }
    }
    None
}

fn worker_script_path() -> Option<PathBuf> {
    if let Ok(cwd) = std::env::current_dir()
        && let Some(path) = find_worker_script_from_ancestors(&cwd)
    {
        return Some(path);
    }

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    find_worker_script_from_ancestors(manifest_dir)
}

fn resolve_node_bin() -> &'static str {
    for bin in ["node", NODE_BIN] {
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
    NODE_BIN
}

fn worker_environment(extra_env: &[(&str, &str)]) -> Vec<(String, String)> {
    crate::sandbox::constants::env::child_environment(extra_env.iter().copied())
}

pub struct TypeScriptHandlerWorker {
    stdin_tx: tokio::sync::Mutex<tokio::process::ChildStdin>,
    pending: Arc<RwLock<HashMap<String, mpsc::UnboundedSender<WorkerResponse>>>>,
    _demuxer: tokio::task::JoinHandle<()>,
    _child: Arc<tokio::sync::Mutex<crate::sandbox::WrappedChild>>,
    _workdir: tempfile::TempDir,
    next_id: std::sync::atomic::AtomicU64,
}

impl TypeScriptHandlerWorker {
    pub async fn spawn(manifest_json: &str) -> Result<Self, RuntimeError> {
        Self::spawn_with_env(manifest_json, &[], None, false).await
    }

    pub async fn spawn_with_env(
        manifest_json: &str,
        extra_env: &[(&str, &str)],
        sandbox: Option<&Arc<dyn crate::sandbox::SandboxBackend>>,
        sandbox_required: bool,
    ) -> Result<Self, RuntimeError> {
        let worker_script = worker_script_path().ok_or_else(|| {
            cap_err(format!(
                "TypeScript tool worker script not found (expected frontend path ending in scripts/{WORKER_SCRIPT})"
            ))
        })?;

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

        let node = resolve_node_bin().to_string();
        let base_args: Vec<String> = vec![
            worker_script.to_string_lossy().into_owned(),
            manifest_path_str.to_string(),
        ];
        let worker_env = worker_environment(extra_env);
        let sandbox_command = match sandbox.filter(|b| b.is_available()) {
            Some(backend) => {
                tracing::info!(
                    target: TRACE_TARGET,
                    backend = %backend.capabilities().name,
                    "Sandboxing typescript tool worker"
                );
                backend
                    .wrap_command(&node, &base_args, workdir.path(), false, &worker_env)
                    .map_err(|error| {
                        cap_err(format!("Failed to wrap TypeScript worker: {error}"))
                    })?
            }
            None if sandbox_required => {
                return Err(cap_err(
                    "typescript worker sandbox required but no OS-isolating backend is available",
                ));
            }
            None => crate::sandbox::WrappedCommand::direct(node, base_args, worker_env.clone())
                .map_err(|error| {
                    cap_err(format!("Invalid TypeScript worker environment: {error}"))
                })?,
        };

        let mut child = sandbox_command
            .spawn(|command| {
                command
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .kill_on_drop(true);
            })
            .map_err(|e| cap_err(format!("Failed to spawn TypeScript tool worker: {}", e)))?;

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
                            e,
                            line
                        );
                        continue;
                    }
                };
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

    pub async fn cancel(&self, req_id: &str) -> Result<(), RuntimeError> {
        let request = WorkerRequest::Cancel(CancelRequest {
            v: PROTOCOL_VERSION,
            req_id: req_id.to_string(),
        });
        self.write_request(&request).await
    }
}
