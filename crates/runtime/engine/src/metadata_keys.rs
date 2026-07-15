//! Execution metadata keys for runtime producers and readers.

pub const PARENT_EXECUTION_ID: &str = "parent_execution_id";
/// Server-owned execution id for the top-level run record. Distinct from the
/// runtime context id for hosts that wrap runtime execution in an HTTP run.
pub const EXECUTION_ID: &str = "execution_id";
/// Workflow id used to resolve the durable run artifact root.
pub const WORKFLOW_ID: &str = "workflow_id";
/// Durable run artifact root, usually `$APXM_RUNS_ROOT/{workflow_id}/{execution_id}`.
pub const RUN_ROOT: &str = "run_root";
/// End-to-end trace id stamped by the HTTP/server layer.
pub const TRACE_ID: &str = "trace_id";
/// Execution-scoped key the host (apxm-server) stamps so a parked execution can
/// release/reacquire its cross-execution admission slot via the admission registry.
pub const ADMISSION_ID: &str = "admission_id";
pub const SCOPE_ID: &str = "scope_id";
pub const PARENT_SCOPE_ID: &str = "parent_scope_id";
pub const SESSION_DIR: &str = "session_dir";
pub const SESSION_ROOT: &str = "session_root";
pub const DELEGATE_TASK_SPEC: &str = "delegate_task_spec";
pub const DELEGATE_TARGET: &str = "delegate_target";
pub const COMMUNICATE_SENDER: &str = "communicate_sender";
pub const COMMUNICATE_RECIPIENT: &str = "communicate_recipient";
pub const COMMUNICATE_MODE: &str = "communicate_mode";
pub const FLOW_CALL_DEPTH: &str = "flow_call_depth";
/// The effective side-effect policy granted to the current execution. Seeded
/// at the top-level execution by the trusted host.
pub const SIDE_EFFECT_POLICY: &str = "side_effect_policy";
/// JSON array of runtime-minted capability grants presented by the host.
/// INV_CAP write admission checks these opaque grants by `capability_binding`; raw
/// callable names are never authority.
pub const CAPABILITY_GRANTS: &str = "capability_grants";
/// JSON-serialized [`crate::context_envelope::SealedContextRuntimeTransport`]
/// supplied by the trusted host after durable pin verification. Child
/// workflows do not inherit this metadata without a typed child envelope.
pub const SEALED_CONTEXT_TRANSPORT_V1: &str = "sealed_context_transport_v1";
/// JSON-serialized canonical `ChildExecutionEnvelope` admitted by the trusted
/// host for one `WORKFLOW_SPAWN`. Its absence means the child is isolated; a
/// malformed or incomplete envelope is rejected before the spawner runs.
pub const CHILD_EXECUTION_ENVELOPE_V1: &str = "child_execution_envelope_v1";
/// Serialized sealed context transport paired with
/// [`CHILD_EXECUTION_ENVELOPE_V1`]. It is copied only after the envelope's
/// context identity and delegated selected-skill pins have been verified.
pub const CHILD_SEALED_CONTEXT_TRANSPORT_V1: &str = "child_sealed_context_transport_v1";
/// JSON array of opaque credential references admitted for the child. It never
/// contains credential material and is not a substitute for credential
/// resolution at the trusted host boundary.
pub const CREDENTIAL_REFS: &str = "credential_refs";
/// The opaque Server-owned memory policy selected for this child execution.
/// A child receives no parent memory state merely by carrying this reference.
pub const MEMORY_POLICY_REF: &str = "memory_policy_ref";
/// JSON-serialized generated [`apxm_core::types::context_contracts::BudgetSet`]
/// copied exactly from the host-persisted `AgentInvocationEnvelope`. The
/// runtime parses it once at root execution and shares its accounting ledger
/// with ordinary child contexts; malformed values fail closed.
pub const ADMITTED_INVOCATION_BUDGET_SET_V1: &str = "admitted_invocation_budget_set_v1";
/// JSON object `{capability_name: max_calls}` declaring the per-tool call-count
/// budget. Seeded by the program/request at the top-level execution
/// and parsed into `ExecutionContext::tool_call_budgets`; the shared per-tool
/// counter then propagates to children so a fan-out cannot multiply the budget.
pub const TOOL_CALL_BUDGETS: &str = "tool_call_budgets";
pub const TARGET_AGENT: &str = "target_agent";
pub const TARGET_FLOW: &str = "target_flow";

/// Partial replay (`rerun-from-node`): the node id in the recompiled graph to
/// restart execution from. When present (with [`REPLAY_TOKEN_VALUES`]), the
/// engine pre-completes the upstream nodes — reusing the prior run's boundary
/// token values — and re-executes only this node and its descendants.
pub const REPLAY_FROM_NODE: &str = "replay_from_node";
/// Partial replay (`rerun-from-node`): JSON object `{token_id: Value}` of the
/// prior run's captured token values, used to seed the replay boundary. Paired
/// with [`REPLAY_FROM_NODE`].
pub const REPLAY_TOKEN_VALUES: &str = "replay_token_values";
/// Source execution whose durable authority is reused by a partial replay.
pub const REPLAY_SOURCE_EXECUTION_ID: &str = "replay_source_execution_id";
/// JSON-serialized `CapabilityEffectReplayEvidenceEnvelope` supplied by the
/// host authority for replay-verifying skipped durable host effects.
pub const CAPABILITY_EFFECT_REPLAY_EVIDENCE: &str = "capability_effect_replay_evidence";
/// End-to-end delivery trace id minted at webhook ingress (apxm-os → apxm-server).
pub const CORRELATION_ID: &str = "correlation_id";
