//! Product-neutral APXM reference host over JSONL stdin/stdout.
//!
//! The host owns lifecycle admission and invokes the existing canonical driver
//! only after exact release, port, resource-ceiling, and provenance checks.
//! There is no HTTP compatibility route, runtime discovery, or fallback.

use std::io::BufRead;

use anyhow::Result;
use apxm_program::air::AirModule;
use serde::Deserialize;
use serde_json::{Value, json};

use apxm_cli::canonical_execute::execute_canonical_air;

const HOST_SCHEMA: &str = "apxm.runtime.host.v1";
const REQUEST_SCHEMA: &str = "apxm.runtime.host-request.v1";
const ADMISSION_SCHEMA: &str = "apxm.invocation-admission.v1";
const RELEASE_DIGEST: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PORT_BINDINGS_DIGEST: &str =
    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const RESOURCE_CEILING_DIGEST: &str =
    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Starting,
    Ready,
    Draining,
    Stopped,
}

impl State {
    fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Draining => "draining",
            Self::Stopped => "stopped",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema_version: String,
    operation: String,
    #[serde(default)]
    admission: Option<Admission>,
    #[serde(default)]
    air: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    schema_version: String,
    invocation_id: String,
    artifact_digest: String,
    release_digest: String,
    port_bindings_digest: String,
    resource_ceiling_digest: String,
    provenance_digest: String,
}

struct Host {
    state: State,
    in_flight: u64,
    admission_revoked: bool,
    last_runtime_evidence: Option<Value>,
}

impl Host {
    fn new() -> Self {
        Self {
            state: State::Starting,
            in_flight: 0,
            admission_revoked: false,
            last_runtime_evidence: None,
        }
    }

    fn mark_ready(&mut self) {
        if self.state == State::Starting {
            self.state = State::Ready;
        }
    }

    fn readiness(&self) -> Value {
        json!({
            "schema_version": HOST_SCHEMA,
            "contract_id": HOST_SCHEMA,
            "state": self.state.as_str(),
            "release_digest": RELEASE_DIGEST,
            "port_bindings_digest": PORT_BINDINGS_DIGEST,
            "resource_ceiling_digest": RESOURCE_CEILING_DIGEST,
            "in_flight": self.in_flight,
        })
    }

    fn reject(&self, code: &'static str, message: impl Into<String>) -> Value {
        json!({
            "schema_version": HOST_SCHEMA,
            "status": "rejected",
            "error": {"code": code, "message": message.into()},
            "readiness": self.readiness(),
        })
    }

    fn terminal(&mut self, status: &'static str, reason: &'static str) -> Value {
        self.state = State::Stopped;
        self.in_flight = 0;
        json!({
            "schema_version": HOST_SCHEMA,
            "status": status,
            "reason": reason,
            "readiness": self.readiness(),
        })
    }

    fn restart(&mut self) -> Value {
        self.state = State::Ready;
        self.in_flight = 0;
        self.admission_revoked = false;
        json!({
            "schema_version": HOST_SCHEMA,
            "status": "restarted",
            "recovery": {
                "contract": "reconcile_from_runtime_evidence",
                "status": if self.last_runtime_evidence.is_some() {
                    "reconciled"
                } else {
                    "no_runtime_evidence"
                },
                "runtime_evidence": self.last_runtime_evidence.clone(),
            },
            "readiness": self.readiness(),
        })
    }

    fn runtime_evidence(&self, invocation_id: &str, terminal_kind: &str) -> Value {
        json!({
            "schema_version": "apxm.runtime-evidence.v1",
            "program_identity": {
                "artifact_digest": RELEASE_DIGEST,
                "entrypoint": "reference-host",
                "agent_identity_binding": "apxm-reference-host",
                "program_instance_id": "reference-host.instance"
            },
            "facts": [
                {
                    "fact_id": format!("{invocation_id}.created"),
                    "event_sequence": 0,
                    "fact_kind": "instance.created",
                    "instance_state": "ready"
                },
                {
                    "fact_id": format!("{invocation_id}.admitted"),
                    "event_sequence": 1,
                    "fact_kind": "invocation.admitted",
                    "invocation_state": "running"
                },
                {
                    "fact_id": format!("{invocation_id}.terminal"),
                    "event_sequence": 2,
                    "fact_kind": terminal_kind,
                    "invocation_state": "committed_return",
                    "commit_sequence": 1
                }
            ]
        })
    }

    fn validate_admission(&self, admission: &Admission) -> std::result::Result<(), Value> {
        if self.admission_revoked {
            return Err(self.reject(
                "admission_revoked",
                "invocation admission is closed until an explicit restart",
            ));
        }
        if self.state != State::Ready {
            return Err(self.reject(
                "host_not_accepting",
                format!("host state is {}", self.state.as_str()),
            ));
        }
        if admission.schema_version != ADMISSION_SCHEMA {
            return Err(self.reject(
                "invalid_admission_schema",
                "exact apxm.invocation-admission.v1 is required",
            ));
        }
        if admission.invocation_id.is_empty() {
            return Err(self.reject("missing_invocation_id", "invocation_id is required"));
        }
        if admission.artifact_digest != RELEASE_DIGEST
            || admission.provenance_digest != admission.artifact_digest
        {
            return Err(self.reject(
                "provenance_mismatch",
                "artifact and provenance digests must match the admitted release",
            ));
        }
        if admission.release_digest != RELEASE_DIGEST {
            return Err(self.reject("release_mismatch", "release digest is not admitted"));
        }
        if admission.port_bindings_digest != PORT_BINDINGS_DIGEST {
            return Err(self.reject(
                "port_bindings_mismatch",
                "port binding digest is not admitted",
            ));
        }
        if admission.resource_ceiling_digest != RESOURCE_CEILING_DIGEST {
            return Err(self.reject(
                "resource_ceiling_mismatch",
                "resource ceiling digest is not admitted",
            ));
        }
        Ok(())
    }

    async fn invoke(&mut self, request: Request) -> Value {
        let Request { admission, air, .. } = request;
        let Some(admission) = admission else {
            return self.reject("missing_admission", "invocation admission is required");
        };
        if let Err(rejection) = self.validate_admission(&admission) {
            return rejection;
        }
        let Some(air_value) = air else {
            return self.reject("missing_air", "canonical apxm.air.v1 is required");
        };
        let air: AirModule = match serde_json::from_value(air_value) {
            Ok(air) => air,
            Err(error) => return self.reject("invalid_air", error.to_string()),
        };
        if !air.verify().is_accepted() {
            return self.reject("invalid_air", "canonical AIR verification failed");
        }
        self.in_flight = 1;
        let result = match execute_canonical_air(air).await {
            Ok(output) => {
                let runtime_evidence =
                    self.runtime_evidence(&admission.invocation_id, "invocation.committed");
                self.last_runtime_evidence = Some(runtime_evidence.clone());
                json!({
                    "schema_version": HOST_SCHEMA,
                    "status": "committed",
                    "invocation_id": admission.invocation_id,
                    "result": output,
                    "runtime_evidence": runtime_evidence,
                })
            }
            Err(error) => json!({
                "schema_version": HOST_SCHEMA,
                "status": "failed",
                "invocation_id": admission.invocation_id,
                "error": {"code": "canonical_execution_failed", "message": error.to_string()},
            }),
        };
        self.in_flight = 0;
        if self.state == State::Draining {
            self.state = State::Stopped;
        }
        result
    }

    async fn handle(&mut self, request: Request) -> Value {
        if request.schema_version != REQUEST_SCHEMA {
            return self.reject(
                "invalid_request_schema",
                "exact host request schema is required",
            );
        }
        match request.operation.as_str() {
            "readiness" => self.readiness(),
            "drain" if self.state == State::Ready => {
                self.state = State::Draining;
                self.readiness()
            }
            "drain" => self.reject("invalid_transition", "host is not ready to drain"),
            "cancel" if self.state == State::Ready && self.in_flight == 0 => {
                self.terminal("cancelled", "cancelled_before_admission")
            }
            "cancel" => self.reject(
                "cancellation_unconfirmed",
                "in-flight cancellation requires durable host evidence",
            ),
            "shutdown" if self.in_flight == 0 && self.state != State::Stopped => {
                self.terminal("shutdown", "shutdown_terminal")
            }
            "shutdown" if self.state == State::Stopped => {
                self.reject("invalid_transition", "host is already stopped")
            }
            "shutdown" => self.reject(
                "shutdown_unconfirmed",
                "in-flight shutdown requires durable host evidence",
            ),
            "revoke" if self.in_flight == 0 && self.state != State::Stopped => {
                self.admission_revoked = true;
                self.terminal("revoked", "admission_revoked")
            }
            "revoke" if self.state == State::Stopped && self.admission_revoked => {
                self.reject("invalid_transition", "host admission is already revoked")
            }
            "revoke" if self.state == State::Stopped => {
                self.reject("invalid_transition", "host is already stopped")
            }
            "revoke" => self.reject(
                "revocation_unconfirmed",
                "in-flight revocation requires durable host evidence",
            ),
            "restart" if self.state == State::Stopped && self.in_flight == 0 => self.restart(),
            "restart" => self.reject(
                "invalid_transition",
                "host restart requires a stopped terminal state",
            ),
            "invoke" => self.invoke(request).await,
            _ => self.reject(
                "unknown_operation",
                "operation must be readiness, invoke, drain, cancel, shutdown, revoke, or restart",
            ),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let stdin = std::io::stdin();
    let mut host = Host::new();
    host.mark_ready();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => host.handle(request).await,
            Err(error) => host.reject("invalid_request", error.to_string()),
        };
        println!("{}", serde_json::to_string(&response)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admission() -> Admission {
        Admission {
            schema_version: ADMISSION_SCHEMA.into(),
            invocation_id: "invocation.1".into(),
            artifact_digest: RELEASE_DIGEST.into(),
            release_digest: RELEASE_DIGEST.into(),
            port_bindings_digest: PORT_BINDINGS_DIGEST.into(),
            resource_ceiling_digest: RESOURCE_CEILING_DIGEST.into(),
            provenance_digest: RELEASE_DIGEST.into(),
        }
    }

    fn request(operation: &str) -> Request {
        Request {
            schema_version: REQUEST_SCHEMA.into(),
            operation: operation.into(),
            admission: None,
            air: None,
        }
    }

    fn minimal_air() -> Value {
        json!({
            "schema_version": "apxm.air.v1",
            "semantic_operations": [],
            "structural_ir": [],
            "context_flow": [],
            "source_map": {
                "schema_version": "apxm.source-map.v1",
                "source_language": "python",
                "node_spans": [],
                "region_annotations": []
            }
        })
    }

    #[tokio::test]
    async fn readiness_and_drain_are_explicit() {
        let mut host = Host::new();
        assert_eq!(host.handle(request("readiness")).await["state"], "starting");
        host.mark_ready();
        assert_eq!(host.handle(request("readiness")).await["state"], "ready");
        assert_eq!(host.handle(request("drain")).await["state"], "draining");
        assert_eq!(host.handle(request("readiness")).await["state"], "draining");
    }

    #[tokio::test]
    async fn cancellation_before_admission_is_terminal_and_recoverable_by_restart() {
        let mut host = Host::new();
        host.mark_ready();
        let response = host.handle(request("cancel")).await;
        assert_eq!(response["status"], "cancelled");
        assert_eq!(response["readiness"]["state"], "stopped");
        assert_eq!(host.handle(request("readiness")).await["state"], "stopped");
        let restart = host.handle(request("restart")).await;
        assert_eq!(restart["status"], "restarted");
        assert_eq!(restart["recovery"]["status"], "no_runtime_evidence");
        assert_eq!(restart["readiness"]["state"], "ready");
    }

    #[tokio::test]
    async fn in_flight_lifecycle_operations_fail_closed_until_runtime_evidence_exists() {
        let mut host = Host::new();
        host.mark_ready();
        host.in_flight = 1;

        let cancel = host.handle(request("cancel")).await;
        assert_eq!(
            cancel,
            json!({
                "schema_version": HOST_SCHEMA,
                "status": "rejected",
                "error": {
                    "code": "cancellation_unconfirmed",
                    "message": "in-flight cancellation requires durable host evidence",
                },
                "readiness": {
                    "schema_version": HOST_SCHEMA,
                    "contract_id": HOST_SCHEMA,
                    "state": "ready",
                    "release_digest": RELEASE_DIGEST,
                    "port_bindings_digest": PORT_BINDINGS_DIGEST,
                    "resource_ceiling_digest": RESOURCE_CEILING_DIGEST,
                    "in_flight": 1,
                },
            })
        );

        let shutdown = host.handle(request("shutdown")).await;
        assert_eq!(
            shutdown,
            json!({
                "schema_version": HOST_SCHEMA,
                "status": "rejected",
                "error": {
                    "code": "shutdown_unconfirmed",
                    "message": "in-flight shutdown requires durable host evidence",
                },
                "readiness": {
                    "schema_version": HOST_SCHEMA,
                    "contract_id": HOST_SCHEMA,
                    "state": "ready",
                    "release_digest": RELEASE_DIGEST,
                    "port_bindings_digest": PORT_BINDINGS_DIGEST,
                    "resource_ceiling_digest": RESOURCE_CEILING_DIGEST,
                    "in_flight": 1,
                },
            })
        );

        let revoke = host.handle(request("revoke")).await;
        assert_eq!(
            revoke,
            json!({
                "schema_version": HOST_SCHEMA,
                "status": "rejected",
                "error": {
                    "code": "revocation_unconfirmed",
                    "message": "in-flight revocation requires durable host evidence",
                },
                "readiness": {
                    "schema_version": HOST_SCHEMA,
                    "contract_id": HOST_SCHEMA,
                    "state": "ready",
                    "release_digest": RELEASE_DIGEST,
                    "port_bindings_digest": PORT_BINDINGS_DIGEST,
                    "resource_ceiling_digest": RESOURCE_CEILING_DIGEST,
                    "in_flight": 1,
                },
            })
        );
    }

    #[tokio::test]
    async fn missing_or_mismatched_admission_is_rejected() {
        let mut host = Host::new();
        host.mark_ready();
        let response = host.handle(request("invoke")).await;
        assert_eq!(response["status"], "rejected");
        assert_eq!(response["error"]["code"], "missing_admission");
    }

    #[tokio::test]
    async fn draining_host_rejects_new_invocations_before_dispatch() {
        let mut host = Host::new();
        host.mark_ready();
        assert_eq!(host.handle(request("drain")).await["state"], "draining");

        let response = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission()),
                air: Some(minimal_air()),
            })
            .await;
        assert_eq!(
            response,
            json!({
                "schema_version": HOST_SCHEMA,
                "status": "rejected",
                "error": {
                    "code": "host_not_accepting",
                    "message": "host state is draining",
                },
                "readiness": {
                    "schema_version": HOST_SCHEMA,
                    "contract_id": HOST_SCHEMA,
                    "state": "draining",
                    "release_digest": RELEASE_DIGEST,
                    "port_bindings_digest": PORT_BINDINGS_DIGEST,
                    "resource_ceiling_digest": RESOURCE_CEILING_DIGEST,
                    "in_flight": 0,
                },
            })
        );
    }

    #[tokio::test]
    async fn admitted_empty_air_reaches_the_canonical_commit_boundary() {
        let mut host = Host::new();
        host.mark_ready();
        let response = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission()),
                air: Some(minimal_air()),
            })
            .await;
        assert_eq!(response["status"], "committed");
        assert_eq!(response["invocation_id"], "invocation.1");
    }

    #[tokio::test]
    async fn shutdown_is_terminal_until_explicit_restart() {
        let mut host = Host::new();
        host.mark_ready();
        let shutdown = host.handle(request("shutdown")).await;
        assert_eq!(shutdown["status"], "shutdown");
        assert_eq!(shutdown["readiness"]["state"], "stopped");

        let restarted = host.handle(request("restart")).await;
        assert_eq!(restarted["status"], "restarted");
        assert_eq!(
            restarted["recovery"]["contract"],
            "reconcile_from_runtime_evidence"
        );
        assert_eq!(restarted["readiness"]["state"], "ready");
    }

    #[tokio::test]
    async fn revocation_closes_admission_until_restart() {
        let mut host = Host::new();
        host.mark_ready();

        let revoke = host.handle(request("revoke")).await;
        assert_eq!(revoke["status"], "revoked");
        assert_eq!(revoke["readiness"]["state"], "stopped");

        let rejected = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission()),
                air: Some(minimal_air()),
            })
            .await;
        assert_eq!(rejected["status"], "rejected");
        assert_eq!(rejected["error"]["code"], "admission_revoked");

        let restarted = host.handle(request("restart")).await;
        assert_eq!(restarted["status"], "restarted");
        assert_eq!(restarted["readiness"]["state"], "ready");

        let committed = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission()),
                air: Some(minimal_air()),
            })
            .await;
        assert_eq!(committed["status"], "committed");
    }

    #[tokio::test]
    async fn restart_reports_recovery_from_last_runtime_evidence() {
        let mut host = Host::new();
        host.mark_ready();

        let committed = host
            .handle(Request {
                schema_version: REQUEST_SCHEMA.into(),
                operation: "invoke".into(),
                admission: Some(admission()),
                air: Some(minimal_air()),
            })
            .await;
        let runtime_evidence = committed["runtime_evidence"].clone();
        assert_eq!(committed["status"], "committed");

        let shutdown = host.handle(request("shutdown")).await;
        assert_eq!(shutdown["status"], "shutdown");

        let restarted = host.handle(request("restart")).await;
        assert_eq!(restarted["status"], "restarted");
        assert_eq!(restarted["recovery"]["status"], "reconciled");
        assert_eq!(restarted["recovery"]["runtime_evidence"], runtime_evidence);
    }
}
