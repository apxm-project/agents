//! Python tool worker — subprocess bridge over NDJSON stdin/stdout.
//!
//! Spawns `python -m apxm.tool_worker` as a long-lived child process.
//! Requests are multiplexed by `req_id`; a background demuxer task routes
//! responses to the correct `oneshot::Sender`.

use super::protocol::{
    CallRequest, CallResponse, CancelRequest, WorkerRequest, WorkerResponse, PROTOCOL_VERSION,
};
use apxm_core::error::RuntimeError;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;

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
    /// Monotonic request counter for generating unique req_ids.
    next_id: std::sync::atomic::AtomicU64,
}

impl PythonToolWorker {
    /// Spawn the Python tool worker subprocess.
    ///
    /// The `manifest_json` is the serialized `tools.json` content passed as
    /// a command-line argument so the worker knows which tools to load.
    pub async fn spawn(manifest_json: &str) -> Result<Self, RuntimeError> {
        let mut child = Command::new("python")
            .args(["-m", "apxm.tool_worker", manifest_json])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| RuntimeError::Capability {
                capability: "python_tools".into(),
                message: format!("Failed to spawn Python tool worker: {}", e),
            })?;

        let stdin = child.stdin.take().ok_or_else(|| RuntimeError::Capability {
            capability: "python_tools".into(),
            message: "Failed to capture worker stdin".into(),
        })?;

        let stdout = child.stdout.take().ok_or_else(|| RuntimeError::Capability {
            capability: "python_tools".into(),
            message: "Failed to capture worker stdout".into(),
        })?;

        let pending: Arc<RwLock<HashMap<String, oneshot::Sender<CallResponse>>>> =
            Arc::new(RwLock::new(HashMap::new()));

        // Spawn stderr reader to log worker errors.
        let stderr = child.stderr.take();
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let reader = BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::warn!(target: "python_tool_worker", "stderr: {}", line);
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
                            target: "python_tool_worker",
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
                                target: "python_tool_worker",
                                "Received response for unknown req_id: {}",
                                call_resp.req_id
                            );
                        }
                    }
                }
            }
            tracing::info!(target: "python_tool_worker", "Demuxer task exited (worker stdout closed)");
        });

        let child = Arc::new(tokio::sync::Mutex::new(child));

        Ok(Self {
            stdin_tx: tokio::sync::Mutex::new(stdin),
            pending,
            _demuxer: demuxer,
            _child: child,
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
                RuntimeError::Capability {
                    capability: "python_tools".into(),
                    message: format!("Failed to write to worker stdin: {}", e),
                }
            })?;
            stdin.flush().await.map_err(|e| {
                self.pending.write().remove(&req_id);
                RuntimeError::Capability {
                    capability: "python_tools".into(),
                    message: format!("Failed to flush worker stdin: {}", e),
                }
            })?;
        }

        // Wait for response with timeout.
        let resp = tokio::time::timeout(deadline, rx).await.map_err(|_| {
            // Timeout — remove pending entry and send cancel.
            self.pending.write().remove(&req_id);
            RuntimeError::Capability {
                capability: "python_tools".into(),
                message: format!(
                    "Tool call {} timed out after {}ms",
                    req_id,
                    deadline.as_millis()
                ),
            }
        })?;

        let call_resp = resp.map_err(|_| RuntimeError::Capability {
            capability: "python_tools".into(),
            message: format!(
                "Worker dropped response channel for req_id {} (worker may have crashed)",
                req_id
            ),
        })?;

        if call_resp.ok {
            Ok(call_resp.value.unwrap_or(serde_json::Value::Null))
        } else {
            let err = call_resp.error.unwrap_or_else(|| super::protocol::ErrorEnvelope {
                kind: "unknown".into(),
                message: "Tool returned ok=false with no error envelope".into(),
                traceback: None,
            });
            Err(RuntimeError::Capability {
                capability: "python_tools".into(),
                message: format!("[{}] {}", err.kind, err.message),
            })
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
            .map_err(|e| RuntimeError::Capability {
                capability: "python_tools".into(),
                message: format!("Failed to send cancel request: {}", e),
            })?;
        stdin
            .flush()
            .await
            .map_err(|e| RuntimeError::Capability {
                capability: "python_tools".into(),
                message: format!("Failed to flush cancel request: {}", e),
            })?;

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
                assert!(
                    msg.contains("Failed to spawn"),
                    "Unexpected error: {}",
                    msg
                );
            }
        }
    }
}
