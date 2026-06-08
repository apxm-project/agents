use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use apxm_artifact::Artifact;
use apxm_backends::{
    LLMRequest, Message as LLMMessage, Role as LLMRole, ToolChoice, ToolDefinition,
};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::events::payload::ErrorPayload;
use apxm_core::events::{ApxmEvent, EventCategory, EventKind, EventSource, SkillEventProvenance};
use apxm_core::impl_event_payload;
use apxm_core::paths::ApxmPaths;
use apxm_core::types::{AISOperationType, Value};
use apxm_runtime::capability::CapabilitySandboxPreflight;
use apxm_skill::{
    CapabilityPolicy, SkillExecutionProvenance, SkillManifest, SkillPackageHashes,
    SkillValidationReport, ValidationStatus,
};
use axum::Json;
use axum::extract::{Path as AxumPath, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::error::ApiError;
use crate::execute::{ExecuteResponse, to_execute_response};
use crate::executions::ExecutionRecordingEmitter;
use crate::rollout::{RolloutEmitter, session_meta_from_skill};
use crate::runs::RunBusFanOutEmitter;
use crate::skill_resources::{
    SkillResource, SkillResourceContent, SkillResourceError, list_skill_resources,
    parse_cli_skill_roots as parse_cli_skill_roots_impl,
    parse_skill_roots as parse_skill_roots_impl, resolve_skill_uri, resource_package,
};
use crate::state::{AppState, TokioChannelEmitter};
use crate::webhook::WebhookEmitter;

const MANIFEST_FILE: &str = apxm_skill::MANIFEST_FILE;
const PACK_FILE: &str = "pack.toml";
const SOURCE_FILE: &str = "SKILL.md";
const AIR_FILE: &str = "skill.air";
const ARTIFACT_FILE: &str = "skill.apxmobj";
const CONVERSION_REPORT_FILE: &str = "conversion-report.json";
const RESOURCES_DIR: &str = "resources";
const TESTS_DIR: &str = "tests";
const SKILL_SESSION_DIR: &str = "skills";
const SKILL_EXECUTE_STARTED: EventKind =
    EventKind::new("skill_execute_started", EventCategory::Lifecycle, false);
const SKILL_EXECUTE_COMPLETE: EventKind =
    EventKind::new("skill_execute_complete", EventCategory::Lifecycle, true);

#[derive(Debug, Clone)]
pub(crate) struct SkillLibrary {
    roots: Arc<Vec<PathBuf>>,
}

impl SkillLibrary {
    pub(crate) fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots: Arc::new(roots),
        }
    }

    pub(crate) fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    pub(crate) fn scan(&self) -> SkillScan {
        let mut warnings = Vec::new();
        let mut packages = Vec::new();

        for root in self.roots() {
            if !root.exists() {
                warnings.push("configured skill root does not exist".to_string());
                continue;
            }
            if !root.is_dir() {
                warnings.push("configured skill root is not a directory".to_string());
                continue;
            }
            find_manifest_dirs(root, &mut packages, &mut warnings);
        }

        packages.sort();
        let mut records: Vec<SkillRecord> = packages
            .iter()
            .map(|package_dir| load_record(package_dir))
            .collect();
        annotate_duplicates(&mut records);

        SkillScan {
            object: "list",
            warnings,
            data: records,
        }
    }

    pub(crate) fn find(&self, requested_id: &str) -> Result<SkillRecord, SkillLookupError> {
        find_record(self.scan().data, requested_id)
    }

    pub(crate) fn find_executable(
        &self,
        requested_id: &str,
    ) -> Result<ExecutableSkill, SkillLookupError> {
        let record = self.find(requested_id)?;
        let artifact_path = record.package_dir.join(ARTIFACT_FILE);
        Ok(ExecutableSkill {
            record,
            artifact_path,
        })
    }

    pub(crate) fn list_skill_resources(&self) -> Vec<SkillResource> {
        list_skill_resources(&self.resource_packages())
    }

    pub(crate) fn resolve_skill_uri(
        &self,
        uri: &str,
    ) -> Result<SkillResourceContent, SkillResourceError> {
        resolve_skill_uri(&self.resource_packages(), uri)
    }

    fn resource_packages(&self) -> Vec<crate::skill_resources::ResourcePackage> {
        self.scan()
            .data
            .into_iter()
            .filter_map(|record| {
                let manifest = record.manifest.as_ref()?;
                let manifest_value = serde_json::to_value(manifest).ok()?;
                Some(resource_package(
                    manifest.skill_id.clone(),
                    Some(manifest.version.clone()),
                    manifest.display_name.clone(),
                    manifest.description.clone(),
                    record.package_dir,
                    manifest_value,
                ))
            })
            .collect()
    }
}

impl Default for SkillLibrary {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SkillScan {
    pub(crate) object: &'static str,
    pub(crate) warnings: Vec<String>,
    pub(crate) data: Vec<SkillRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SkillRecord {
    pub(crate) skill_id: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) manifest: Option<SkillManifest>,
    pub(crate) package: SkillPackageSummary,
    pub(crate) files: SkillPackageFiles,
    pub(crate) hashes: SkillPackageHashes,
    pub(crate) compile_status: CompileStatus,
    pub(crate) validation: SkillValidationReport,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pack: Option<PackInfo>,
    #[serde(skip)]
    pub(crate) package_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PackInfo {
    pub(crate) pack_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pack_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_upstream: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ExecutableSkill {
    pub(crate) record: SkillRecord,
    pub(crate) artifact_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SkillPackageSummary {
    pub(crate) name: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SkillPackageFiles {
    pub(crate) has_skill_md: bool,
    pub(crate) has_air: bool,
    pub(crate) has_artifact: bool,
    pub(crate) has_conversion_report: bool,
    pub(crate) has_resources: bool,
    pub(crate) has_tests: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CompileStatus {
    Compiled,
    NotCompiled,
    Invalid,
}

#[derive(Debug)]
pub(crate) enum SkillLookupError {
    NotFound(String),
    Ambiguous(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkillExecuteRequest {
    #[serde(default)]
    pub(crate) args: Vec<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    /// Minimum sandbox isolation the caller requires for this skill's tool
    /// execution (e.g. an apxm-os agent's `sandbox = bubblewrap`). When set, the
    /// server confirms a backend that can satisfy it is available and **fails
    /// closed** otherwise, rather than running the skill less confined than the
    /// caller asked for. Accepted: `none`, `bubblewrap`, `docker`, `wasm`.
    #[serde(default)]
    pub(crate) sandbox_hint: Option<String>,
}

/// Map a caller's sandbox hint to the minimum [`IsolationLevel`] it implies, or
/// `None` for an absent/`"none"` hint (no requirement).
fn sandbox_hint_min_isolation(
    hint: Option<&str>,
) -> Result<Option<apxm_runtime::sandbox::IsolationLevel>, ApiError> {
    use apxm_runtime::sandbox::IsolationLevel;
    match hint.map(str::trim).filter(|h| !h.is_empty()) {
        None => Ok(None),
        Some(h) => match h.to_ascii_lowercase().as_str() {
            "none" => Ok(None),
            "bubblewrap" | "docker" => Ok(Some(IsolationLevel::Container)),
            "wasm" => Ok(Some(IsolationLevel::Wasm)),
            other => Err(ApiError::bad_request(format!(
                "unknown sandbox_hint '{other}' (expected none|bubblewrap|docker|wasm)"
            ))),
        },
    }
}

/// Fail closed when the caller required a sandbox the server cannot provide.
fn enforce_sandbox_hint(state: &AppState, req: &SkillExecuteRequest) -> Result<(), ApiError> {
    let Some(min) = sandbox_hint_min_isolation(req.sandbox_hint.as_deref())? else {
        return Ok(());
    };
    state
        .runtime
        .sandbox_registry()
        .select(min)
        .map(|_| ())
        .map_err(|e| {
            ApiError::bad_request(format!(
                "skill requires sandbox isolation '{}' but no capable backend is available: {e}",
                req.sandbox_hint.as_deref().unwrap_or_default()
            ))
        })
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SkillExecuteResponse {
    pub(crate) execution_id: String,
    #[serde(flatten)]
    pub(crate) response: ExecuteResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SkillExecuteStartedPayload {
    pub(crate) execution_id: String,
    pub(crate) skill_id: String,
    pub(crate) skill_version: String,
    pub(crate) session_id: String,
}
impl_event_payload!(SkillExecuteStartedPayload, SKILL_EXECUTE_STARTED);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SkillExecuteCompletePayload {
    pub(crate) execution_id: String,
    pub(crate) result: ExecuteResponse,
}
impl_event_payload!(SkillExecuteCompletePayload, SKILL_EXECUTE_COMPLETE);

pub(crate) fn register_skill_event_payloads() {
    register_event_payload_once::<SkillExecuteStartedPayload>(SKILL_EXECUTE_STARTED);
    register_event_payload_once::<SkillExecuteCompletePayload>(SKILL_EXECUTE_COMPLETE);
}

fn register_event_payload_once<T>(kind: EventKind)
where
    T: apxm_core::events::EventPayload + serde::de::DeserializeOwned,
{
    match apxm_core::events::register_event_payload::<T>(kind) {
        Ok(()) | Err(apxm_core::events::EventRegistryError::AlreadyRegistered { .. }) => {}
        Err(apxm_core::events::EventRegistryError::CoreKind { kind }) => {
            panic!("server skill event kind `{kind}` conflicts with core event kind");
        }
    }
}

struct PreparedCompiledExecution {
    artifact: Artifact,
    args: Vec<String>,
    session_id: String,
    session_dir: String,
    timeout_ms: Option<u64>,
    execution_id: String,
    skill_id: String,
    skill_version: String,
    entry_flow: String,
    /// The launching skill's declared `side_effect_policy` (wire form), seeded
    /// into the top-level execution metadata so CALL_SKILL admission compares a
    /// child against this real grant instead of the conservative default.
    side_effect_policy: Option<String>,
}

impl PreparedCompiledExecution {
    fn skill_provenance(&self) -> SkillEventProvenance {
        SkillEventProvenance {
            skill_id: self.skill_id.clone(),
            skill_version: self.skill_version.clone(),
            parent_skill_id: None,
            parent_execution_id: None,
            flow_name: Some(self.entry_flow.clone()),
        }
    }
}

/// Build the top-level execution metadata that seeds the launching skill's
/// effective `side_effect_policy`, so a CALL_SKILL from this execution is
/// admitted against the real grant rather than the conservative `read_only`
/// default. An absent policy yields an empty map (the default applies).
fn side_effect_policy_metadata(policy: Option<&str>) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Some(policy) = policy {
        map.insert(
            apxm_runtime::metadata_keys::SIDE_EFFECT_POLICY.to_string(),
            policy.to_string(),
        );
    }
    map
}

struct PreparedPromptOnlyExecution {
    skill_md_body: String,
    args: Vec<String>,
    session_id: String,
    session_dir: String,
    timeout_ms: Option<u64>,
    execution_id: String,
    skill_id: String,
    skill_version: String,
    required_capabilities: Vec<String>,
    allowed_tools: Vec<String>,
}

enum PreparedSkillExecution {
    Compiled(PreparedCompiledExecution),
    PromptOnly(PreparedPromptOnlyExecution),
}

pub(crate) async fn list_skills(State(state): State<AppState>) -> Json<SkillScan> {
    Json(state.skill_library.scan())
}

pub(crate) async fn get_skill(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<SkillRecord>, ApiError> {
    state
        .skill_library
        .find(&id)
        .map(Json)
        .map_err(skill_lookup_error)
}

pub(crate) async fn validate_skill(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<SkillRecord>, ApiError> {
    state
        .skill_library
        .find(&id)
        .map(Json)
        .map_err(skill_lookup_error)
}

pub(crate) async fn execute_skill(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<SkillExecuteRequest>,
) -> Result<Json<SkillExecuteResponse>, ApiError> {
    execute_skill_by_id(&state, &id, req).await.map(Json)
}

pub(crate) async fn execute_skill_by_id(
    state: &AppState,
    id: &str,
    req: SkillExecuteRequest,
) -> Result<SkillExecuteResponse, ApiError> {
    enforce_sandbox_hint(state, &req)?;
    match prepare_skill_execution(state, id, req)? {
        PreparedSkillExecution::Compiled(prepared) => execute_compiled_skill(state, prepared).await,
        PreparedSkillExecution::PromptOnly(prepared) => {
            execute_prompt_only_skill(state, prepared).await
        }
    }
}

async fn execute_compiled_skill(
    state: &AppState,
    prepared: PreparedCompiledExecution,
) -> Result<SkillExecuteResponse, ApiError> {
    let _permit = state.inference_limiter.acquire().await?;
    // Open the rollout recorder BEFORE any event lands on the in-memory
    // bus — the JSONL sink relies on the file being ready at first emit.
    ensure_rollout_open(
        state,
        &prepared.execution_id,
        &prepared.session_id,
        &prepared.skill_id,
        &prepared.skill_version,
        None,
        None,
        None,
        prepared.args.clone(),
    )
    .await;

    // Record skill_execute_started so observers see the lifecycle
    // event on the non-streaming compiled path (matches the SSE path).
    let started_event = ApxmEvent::root(
        SkillExecuteStartedPayload {
            execution_id: prepared.execution_id.clone(),
            skill_id: prepared.skill_id.clone(),
            skill_version: prepared.skill_version.clone(),
            session_id: prepared.session_id.clone(),
        },
        EventSource::Server,
        &prepared.execution_id,
    );
    emit_recorded_run_event(state, &prepared.execution_id, started_event);

    // Fan event sinks out to: execution-record persistence, the run
    // event bus (powering /v1/runs/... + SSE), and any
    // optionally-configured webhook/OTEL pipelines. Each sink is
    // opt-in — None instances are skipped.
    let event_sinks =
        build_skill_event_sinks(state, &prepared.execution_id, /*include_channel*/ None);
    let emitter = Arc::new(
        apxm_runtime::EmitterAdapter::new(
            Arc::new(apxm_core::events::FanOutEmitter::new(event_sinks)),
            EventSource::Runtime,
            &prepared.execution_id,
        )
        .with_skill_provenance(prepared.skill_provenance()),
    );
    let runtime_execution = state
        .runtime
        .execute_artifact_with_session_emitter_and_metadata(
            prepared.artifact,
            prepared.args,
            Some(prepared.session_id),
            Some(emitter),
            Some(prepared.session_dir.clone()),
            side_effect_policy_metadata(prepared.side_effect_policy.as_deref()),
        );
    let result = await_skill_execution(
        state,
        &prepared.execution_id,
        prepared.timeout_ms,
        runtime_execution,
    )
    .await?;

    let response = to_execute_response(result, Some(prepared.session_dir));
    state
        .execution_store
        .complete_success(&prepared.execution_id, response.clone());
    let complete_event = ApxmEvent::root(
        SkillExecuteCompletePayload {
            execution_id: prepared.execution_id.clone(),
            result: response.clone(),
        },
        EventSource::Server,
        &prepared.execution_id,
    );
    emit_recorded_run_event(state, &prepared.execution_id, complete_event);
    state.rollout_registry.close(&prepared.execution_id).await;
    Ok(SkillExecuteResponse {
        execution_id: prepared.execution_id,
        response,
    })
}

async fn execute_prompt_only_skill(
    state: &AppState,
    prepared: PreparedPromptOnlyExecution,
) -> Result<SkillExecuteResponse, ApiError> {
    let _permit = state.inference_limiter.acquire().await?;
    let request = build_prompt_only_request(state, &prepared);

    // Open rollout BEFORE the first event so the JSONL sink doesn't miss
    // the skill_execute_started line. The prompt-only path doesn't ship a
    // .apxmobj, so artifact/air hashes go in blank — source_hash still pins
    // SKILL.md identity.
    ensure_rollout_open(
        state,
        &prepared.execution_id,
        &prepared.session_id,
        &prepared.skill_id,
        &prepared.skill_version,
        None,
        None,
        None,
        prepared.args.clone(),
    )
    .await;

    // Record the skill_execute_started event into the run bus so
    // /v1/runs/.../events and the webhook see the lifecycle event
    // even on the fast prompt-only path.
    emit_skill_started(state, &prepared);

    let started_ms = now_ms_u128();
    let result = state.runtime.llm_registry().generate(request).await;
    let elapsed_ms = now_ms_u128().saturating_sub(started_ms);

    match result {
        Ok(llm_response) => {
            // Build a synthetic ExecuteResponse: the LLM message is the
            // entire user-visible content, results map is empty, and
            // llm_usage reflects the single round-trip.
            let response = ExecuteResponse {
                results: std::collections::HashMap::new(),
                content: Some(llm_response.content.clone()),
                session_dir: Some(prepared.session_dir.clone()),
                stats: crate::types::responses::ExecutionStats {
                    executed_nodes: 0,
                    failed_nodes: 0,
                    duration_ms: elapsed_ms,
                },
                llm_usage: crate::types::responses::LlmUsageSummary {
                    input_tokens: llm_response.usage.input_tokens,
                    output_tokens: llm_response.usage.output_tokens,
                    total_requests: 1,
                },
            };
            state
                .execution_store
                .complete_success(&prepared.execution_id, response.clone());
            emit_skill_completed(state, &prepared.execution_id, response.clone());
            state.rollout_registry.close(&prepared.execution_id).await;
            Ok(SkillExecuteResponse {
                execution_id: prepared.execution_id,
                response,
            })
        }
        Err(error) => {
            let message = format!("prompt-only skill LLM call failed: {error}");
            state
                .execution_store
                .complete_failure(&prepared.execution_id, message.clone());
            emit_skill_failed(state, &prepared.execution_id, &message);
            state.rollout_registry.close(&prepared.execution_id).await;
            Err(ApiError::internal_message(message))
        }
    }
}

fn emit_skill_started(state: &AppState, prepared: &PreparedPromptOnlyExecution) {
    let event = ApxmEvent::root(
        SkillExecuteStartedPayload {
            execution_id: prepared.execution_id.clone(),
            skill_id: prepared.skill_id.clone(),
            skill_version: prepared.skill_version.clone(),
            session_id: prepared.session_id.clone(),
        },
        EventSource::Server,
        &prepared.execution_id,
    );
    emit_recorded_run_event(state, &prepared.execution_id, event);
}

fn emit_skill_completed(state: &AppState, execution_id: &str, result: ExecuteResponse) {
    let event = ApxmEvent::root(
        SkillExecuteCompletePayload {
            execution_id: execution_id.to_string(),
            result,
        },
        EventSource::Server,
        execution_id,
    );
    emit_recorded_run_event(state, execution_id, event);
}

fn emit_skill_failed(state: &AppState, execution_id: &str, message: &str) {
    let event = ApxmEvent::root(
        ErrorPayload {
            message: message.to_string(),
            status: None,
            recoverable: false,
        },
        EventSource::Server,
        execution_id,
    );
    emit_recorded_run_event(state, execution_id, event);
}

fn build_prompt_only_request(
    state: &AppState,
    prepared: &PreparedPromptOnlyExecution,
) -> LLMRequest {
    let system = prepared.skill_md_body.clone();
    let user = prepared.args.first().cloned().unwrap_or_default();
    let messages = vec![
        LLMMessage::text(LLMRole::System, system),
        LLMMessage::text(LLMRole::User, user),
    ];
    let mut request = LLMRequest::from_messages(messages);
    request.trace_id = Some(prepared.execution_id.clone());

    // ACL: the tool surface exposed to the LLM must equal
    // manifest.allowed_tools (when set) ∪ manifest.required_capabilities.
    // When `allowed_tools` is empty we fall back to `required_capabilities`
    // because that's the legacy single-list shape some packs still ship.
    let allowed: HashSet<&str> = if !prepared.allowed_tools.is_empty() {
        prepared.allowed_tools.iter().map(String::as_str).collect()
    } else {
        prepared
            .required_capabilities
            .iter()
            .map(String::as_str)
            .collect()
    };

    // Empty surface ⇒ skill explicitly opts out of tool use: ToolChoice
    // stays None and `tools` stays unset (no surface advertised).
    if allowed.is_empty() {
        return request;
    }

    let capability_system = state.runtime.capability_system();
    let tools: Vec<ToolDefinition> = capability_system
        .list_capabilities()
        .into_iter()
        .filter(|cap| allowed.contains(cap.name.as_str()))
        .map(|cap| {
            ToolDefinition::new(
                cap.name.clone(),
                cap.description.clone(),
                cap.parameters_schema,
            )
        })
        .collect();

    // If none of the declared tools resolved to a registered capability we
    // emit no tools and no tool_choice — better than advertising a partial
    // surface that silently drops what the manifest promised.
    if tools.is_empty() {
        return request;
    }

    request.tools = Some(tools);
    request.tool_choice = Some(ToolChoice::Auto);
    request
}

fn now_ms_u128() -> u128 {
    crate::helpers::now_ms() as u128
}

fn record_run_event(state: &AppState, execution_id: &str, event: ApxmEvent) -> ApxmEvent {
    let event = state.run_event_bus.record(execution_id, event);
    state
        .rollout_registry
        .try_record(execution_id, event.clone());
    event
}

fn emit_recorded_run_event(state: &AppState, execution_id: &str, event: ApxmEvent) {
    let event = record_run_event(state, execution_id, event);
    if let Some(dispatcher) = &state.webhook_dispatcher {
        dispatcher.dispatch(event);
    }
}

async fn send_recorded_run_event(
    state: &AppState,
    tx: &mpsc::Sender<ApxmEvent>,
    execution_id: &str,
    event: ApxmEvent,
) {
    let event = record_run_event(state, execution_id, event);
    if let Some(dispatcher) = &state.webhook_dispatcher {
        dispatcher.dispatch(event.clone());
    }
    let _ = tx.send(event).await;
}

/// Build the EventEmitter fan-out used by skill execution: execution-record
/// sink → run event bus → rollout/webhook/channel with normalized run seq.
///
/// Adding a sink is additive — every consumer sees the same event so
/// the SSE stream, the persisted record, and the lifecycle webhook
/// never diverge. The channel emitter is opt-in because only the
/// streaming endpoint needs it.
pub(crate) fn build_skill_event_sinks(
    state: &AppState,
    execution_id: &str,
    channel: Option<Arc<dyn apxm_core::events::EventEmitter>>,
) -> Vec<Arc<dyn apxm_core::events::EventEmitter>> {
    let mut normalized_sinks: Vec<Arc<dyn apxm_core::events::EventEmitter>> = vec![
        // Phase 14.8.E — durable JSONL mirror. Failures are logged
        // inside the emitter, never propagated, so a write error
        // doesn't tear the run.
        Arc::new(RolloutEmitter::new(
            state.rollout_registry.clone(),
            execution_id.to_string(),
        )),
    ];
    if let Some(channel) = channel {
        normalized_sinks.push(channel);
    }
    if let Some(dispatcher) = &state.webhook_dispatcher {
        normalized_sinks.push(Arc::new(WebhookEmitter::new(dispatcher.clone())));
    }
    vec![
        Arc::new(ExecutionRecordingEmitter::new(
            state.execution_store.clone(),
            execution_id.to_string(),
        )),
        Arc::new(RunBusFanOutEmitter::new(
            state.run_event_bus.clone(),
            execution_id.to_string(),
            normalized_sinks,
        )),
    ]
}

/// Open a rollout recorder for an execution if not already open. Idempotent.
/// Pulled out so both compiled + prompt-only execution paths share the same
/// "open before first event" entry point.
pub(crate) async fn ensure_rollout_open(
    state: &AppState,
    execution_id: &str,
    session_id: &str,
    skill_id: &str,
    skill_version: &str,
    artifact_hash: Option<&str>,
    source_hash: Option<&str>,
    air_hash: Option<&str>,
    args: Vec<String>,
) {
    let session_meta = session_meta_from_skill(
        execution_id,
        session_id,
        skill_id,
        skill_version,
        artifact_hash,
        source_hash,
        air_hash,
        args,
    );
    state
        .rollout_registry
        .open_for_run(
            state.rollout_paths.clone(),
            Some(state.rollout_index.clone()),
            execution_id,
            session_id,
            session_meta,
        )
        .await;
}

pub(crate) async fn execute_skill_stream(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(req): Json<SkillExecuteRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>>, ApiError> {
    let stream_config = state.server_config.execution_stream;
    let (tx, mut rx) = mpsc::channel::<ApxmEvent>(stream_config.channel_capacity.max(1));
    let prepared = prepare_skill_execution(&state, &id, req)?;
    let permit = state.inference_limiter.acquire().await?;
    let compiled = match prepared {
        PreparedSkillExecution::Compiled(prep) => Some((prep, permit)),
        PreparedSkillExecution::PromptOnly(prep) => {
            spawn_prompt_only_stream_task(state.clone(), prep, tx.clone(), permit);
            None
        }
    };

    if let Some((prepared, permit)) = compiled {
        let runtime = Arc::clone(&state.runtime);
        let execution_store = state.execution_store.clone();
        let trace_id = prepared.execution_id.clone();
        let tx = tx.clone();

        // Open rollout up front on the streaming compiled path. Subagent
        // recorders are opened later by the executor's SPAWN_AGENT
        // handler — the parent recorder must exist first.
        ensure_rollout_open(
            &state,
            &prepared.execution_id,
            &prepared.session_id,
            &prepared.skill_id,
            &prepared.skill_version,
            None,
            None,
            None,
            prepared.args.clone(),
        )
        .await;

        tokio::spawn(async move {
            let _permit = permit;
            send_recorded_run_event(
                &state,
                &tx,
                &prepared.execution_id,
                ApxmEvent::root(
                    SkillExecuteStartedPayload {
                        execution_id: prepared.execution_id.clone(),
                        skill_id: prepared.skill_id.clone(),
                        skill_version: prepared.skill_version.clone(),
                        session_id: prepared.session_id.clone(),
                    },
                    EventSource::Server,
                    &trace_id,
                ),
            )
            .await;
            let event_sinks = build_skill_event_sinks(
                &state,
                &prepared.execution_id,
                Some(Arc::new(TokioChannelEmitter(tx.clone()))),
            );
            let emitter = Arc::new(
                apxm_runtime::EmitterAdapter::new(
                    Arc::new(apxm_core::events::FanOutEmitter::new(event_sinks)),
                    EventSource::Runtime,
                    &trace_id,
                )
                .with_skill_provenance(prepared.skill_provenance()),
            );
            let runtime_execution = runtime.execute_artifact_with_session_emitter_and_metadata(
                prepared.artifact,
                prepared.args,
                Some(prepared.session_id),
                Some(emitter),
                Some(prepared.session_dir.clone()),
                side_effect_policy_metadata(prepared.side_effect_policy.as_deref()),
            );
            let result = if let Some(timeout_ms) = prepared.timeout_ms {
                match tokio::time::timeout(Duration::from_millis(timeout_ms), runtime_execution)
                    .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        let message = "skill execution timed out".to_string();
                        execution_store.complete_failure(&prepared.execution_id, message.clone());
                        send_recorded_run_event(
                            &state,
                            &tx,
                            &prepared.execution_id,
                            ApxmEvent::root(
                                ErrorPayload {
                                    message,
                                    status: None,
                                    recoverable: false,
                                },
                                EventSource::Server,
                                &trace_id,
                            ),
                        )
                        .await;
                        return;
                    }
                }
            } else {
                runtime_execution.await
            };

            match result {
                Ok(result) => {
                    let response = to_execute_response(result, Some(prepared.session_dir));
                    execution_store.complete_success(&prepared.execution_id, response.clone());
                    send_recorded_run_event(
                        &state,
                        &tx,
                        &prepared.execution_id,
                        ApxmEvent::root(
                            SkillExecuteCompletePayload {
                                execution_id: prepared.execution_id.clone(),
                                result: response,
                            },
                            EventSource::Server,
                            &trace_id,
                        ),
                    )
                    .await;
                }
                Err(error) => {
                    let message = error.to_string();
                    execution_store.complete_failure(&prepared.execution_id, message.clone());
                    send_recorded_run_event(
                        &state,
                        &tx,
                        &prepared.execution_id,
                        ApxmEvent::root(
                            ErrorPayload {
                                message,
                                status: None,
                                recoverable: false,
                            },
                            EventSource::Server,
                            &trace_id,
                        ),
                    )
                    .await;
                }
            }
            state.rollout_registry.close(&prepared.execution_id).await;
        });
    }
    drop(tx);

    let stream = async_stream::stream! {
        while let Some(item) = rx.recv().await {
            let data = serde_json::to_string(&item).unwrap_or_else(|_| "{}".to_string());
            yield Ok(Event::default().data(data));
        }
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new().interval(Duration::from_secs(stream_config.keep_alive_secs.max(1))),
    ))
}

fn spawn_prompt_only_stream_task(
    state: AppState,
    prepared: PreparedPromptOnlyExecution,
    tx: mpsc::Sender<ApxmEvent>,
    permit: crate::state::InferencePermit,
) {
    let trace_id = prepared.execution_id.clone();

    tokio::spawn(async move {
        let _permit = permit;
        ensure_rollout_open(
            &state,
            &prepared.execution_id,
            &prepared.session_id,
            &prepared.skill_id,
            &prepared.skill_version,
            None,
            None,
            None,
            prepared.args.clone(),
        )
        .await;
        send_recorded_run_event(
            &state,
            &tx,
            &prepared.execution_id,
            ApxmEvent::root(
                SkillExecuteStartedPayload {
                    execution_id: prepared.execution_id.clone(),
                    skill_id: prepared.skill_id.clone(),
                    skill_version: prepared.skill_version.clone(),
                    session_id: prepared.session_id.clone(),
                },
                EventSource::Server,
                &trace_id,
            ),
        )
        .await;

        let request = build_prompt_only_request(&state, &prepared);
        let started_ms = now_ms_u128();
        let call = state.runtime.llm_registry().generate(request);
        let result = if let Some(timeout_ms) = prepared.timeout_ms {
            match tokio::time::timeout(Duration::from_millis(timeout_ms), call).await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(_) => Err("prompt-only skill LLM call timed out".to_string()),
            }
        } else {
            call.await.map_err(|error| error.to_string())
        };

        match result {
            Ok(llm_response) => {
                let elapsed_ms = now_ms_u128().saturating_sub(started_ms);
                let response = ExecuteResponse {
                    results: std::collections::HashMap::new(),
                    content: Some(llm_response.content.clone()),
                    session_dir: Some(prepared.session_dir.clone()),
                    stats: crate::types::responses::ExecutionStats {
                        executed_nodes: 0,
                        failed_nodes: 0,
                        duration_ms: elapsed_ms,
                    },
                    llm_usage: crate::types::responses::LlmUsageSummary {
                        input_tokens: llm_response.usage.input_tokens,
                        output_tokens: llm_response.usage.output_tokens,
                        total_requests: 1,
                    },
                };
                state
                    .execution_store
                    .complete_success(&prepared.execution_id, response.clone());
                send_recorded_run_event(
                    &state,
                    &tx,
                    &prepared.execution_id,
                    ApxmEvent::root(
                        SkillExecuteCompletePayload {
                            execution_id: prepared.execution_id.clone(),
                            result: response,
                        },
                        EventSource::Server,
                        &trace_id,
                    ),
                )
                .await;
            }
            Err(message) => {
                state
                    .execution_store
                    .complete_failure(&prepared.execution_id, message.clone());
                send_recorded_run_event(
                    &state,
                    &tx,
                    &prepared.execution_id,
                    ApxmEvent::root(
                        ErrorPayload {
                            message,
                            status: None,
                            recoverable: false,
                        },
                        EventSource::Server,
                        &trace_id,
                    ),
                )
                .await;
            }
        }
        state.rollout_registry.close(&prepared.execution_id).await;
    });
}

fn prepare_skill_execution(
    state: &AppState,
    id: &str,
    req: SkillExecuteRequest,
) -> Result<PreparedSkillExecution, ApiError> {
    let executable = state
        .skill_library
        .find_executable(id)
        .map_err(skill_lookup_error)?;
    let manifest = executable
        .record
        .manifest
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("skill manifest is invalid or missing"))?;
    // Prompt-only fallback: scaffolded packs ship manifest + SKILL.md but
    // no compiled .apxmobj. Run them through the LLM with SKILL.md as
    // the system prompt instead of rejecting the request.
    if matches!(executable.record.compile_status, CompileStatus::NotCompiled)
        && executable.record.files.has_skill_md
    {
        if executable.record.validation.status != ValidationStatus::Valid {
            let details = executable.record.validation.errors.join("; ");
            let message = if details.is_empty() {
                "skill validation failed".to_string()
            } else {
                format!("skill validation failed: {details}")
            };
            return Err(ApiError::bad_request(message));
        }
        return Ok(PreparedSkillExecution::PromptOnly(
            prepare_prompt_only_execution(state, &executable, manifest, req)?,
        ));
    }
    let artifact = load_static_skill_artifact(&executable, manifest, state)?;
    let session_id = req
        .session_id
        .map(validate_session_id)
        .transpose()?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let session_dir = skill_session_dir(&manifest.skill_id, &session_id)?;
    let execution = state.execution_store.start_skill_execution_with_provenance(
        SkillExecutionProvenance {
            skill_id: manifest.skill_id.clone(),
            skill_version: manifest.version.clone(),
            entry_flow: Some(manifest.entry_flow.clone()),
            source_hash: executable.record.hashes.source_hash.clone(),
            air_hash: executable.record.hashes.air_hash.clone(),
            artifact_hash: executable.record.hashes.artifact_hash.clone(),
            parent_execution_id: None,
            parent_skill_id: None,
            parent_skill_version: None,
            scope_id: None,
        },
        &session_id,
        &session_dir,
    );
    Ok(PreparedSkillExecution::Compiled(
        PreparedCompiledExecution {
            artifact,
            args: req.args,
            session_id,
            session_dir,
            timeout_ms: manifest.timeout_ms,
            execution_id: execution.execution_id,
            skill_id: manifest.skill_id.clone(),
            skill_version: manifest.version.clone(),
            entry_flow: manifest.entry_flow.clone(),
            side_effect_policy: manifest.side_effect_policy.clone(),
        },
    ))
}

fn prepare_prompt_only_execution(
    state: &AppState,
    executable: &ExecutableSkill,
    manifest: &SkillManifest,
    req: SkillExecuteRequest,
) -> Result<PreparedPromptOnlyExecution, ApiError> {
    let source_path = executable.record.package_dir.join(SOURCE_FILE);
    let raw = fs::read_to_string(&source_path).map_err(|error| {
        ApiError::bad_request(format!(
            "failed to read {SOURCE_FILE} at {}: {error}",
            source_path.display()
        ))
    })?;
    let skill_md_body = strip_yaml_frontmatter(&raw).trim().to_string();
    if skill_md_body.is_empty() {
        return Err(ApiError::bad_request(format!(
            "{SOURCE_FILE} body is empty after stripping frontmatter"
        )));
    }
    let session_id = req
        .session_id
        .map(validate_session_id)
        .transpose()?
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let session_dir = skill_session_dir(&manifest.skill_id, &session_id)?;
    let execution = state.execution_store.start_skill_execution_with_provenance(
        SkillExecutionProvenance {
            skill_id: manifest.skill_id.clone(),
            skill_version: manifest.version.clone(),
            entry_flow: Some(manifest.entry_flow.clone()),
            source_hash: executable.record.hashes.source_hash.clone(),
            air_hash: None,
            artifact_hash: None,
            parent_execution_id: None,
            parent_skill_id: None,
            parent_skill_version: None,
            scope_id: None,
        },
        &session_id,
        &session_dir,
    );
    Ok(PreparedPromptOnlyExecution {
        skill_md_body,
        args: req.args,
        session_id,
        session_dir,
        timeout_ms: manifest.timeout_ms,
        execution_id: execution.execution_id,
        skill_id: manifest.skill_id.clone(),
        skill_version: manifest.version.clone(),
        required_capabilities: manifest.required_capabilities.clone(),
        allowed_tools: manifest.allowed_tools.clone(),
    })
}

fn strip_yaml_frontmatter(source: &str) -> &str {
    // SKILL.md packs (Anthropic-style) start with a YAML frontmatter
    // block delimited by `---` lines. The body after the closing `---`
    // is the system prompt. Returns the original string when no
    // recognizable frontmatter is present.
    let trimmed = source.trim_start_matches('\u{feff}');
    let Some(rest) = trimmed.strip_prefix("---") else {
        return source;
    };
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let mut search = rest;
    while let Some(idx) = search.find("---") {
        let before_ok = idx == 0 || search[..idx].ends_with('\n');
        let after = &search[idx + 3..];
        let after_ok = after.is_empty() || after.starts_with('\n') || after.starts_with('\r');
        if before_ok && after_ok {
            return after.strip_prefix('\n').unwrap_or(after);
        }
        search = &search[idx + 3..];
    }
    source
}

async fn await_skill_execution(
    state: &AppState,
    execution_id: &str,
    timeout_ms: Option<u64>,
    runtime_execution: impl std::future::Future<
        Output = apxm_runtime::RuntimeResult<apxm_runtime::RuntimeExecutionResult>,
    >,
) -> Result<apxm_runtime::RuntimeExecutionResult, ApiError> {
    let result = if let Some(timeout_ms) = timeout_ms {
        match tokio::time::timeout(Duration::from_millis(timeout_ms), runtime_execution).await {
            Ok(result) => result,
            Err(_) => {
                let message = "skill execution timed out".to_string();
                state
                    .execution_store
                    .complete_failure(&execution_id, message.clone());
                return Err(ApiError::bad_request(message));
            }
        }
    } else {
        runtime_execution.await
    };
    result.map_err(|error| {
        let message = error.to_string();
        state
            .execution_store
            .complete_failure(execution_id, message.clone());
        ApiError::runtime(error)
    })
}

pub(crate) fn parse_skill_roots(args: &[String]) -> Vec<PathBuf> {
    parse_skill_roots_impl(args)
}

// Used by in-crate integration tests; the release binary uses `parse_skill_roots`.
#[allow(dead_code)]
pub(crate) fn parse_cli_skill_roots(args: &[String]) -> Vec<PathBuf> {
    parse_cli_skill_roots_impl(args)
}

fn load_record(package_dir: &Path) -> SkillRecord {
    let package = SkillPackageSummary {
        name: package_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_string(),
    };
    let manifest_path = package_dir.join(MANIFEST_FILE);
    let source_path = package_dir.join(SOURCE_FILE);
    let air_path = package_dir.join(AIR_FILE);
    let artifact_path = package_dir.join(ARTIFACT_FILE);

    let files = SkillPackageFiles {
        has_skill_md: source_path.is_file(),
        has_air: air_path.is_file(),
        has_artifact: artifact_path.is_file(),
        has_conversion_report: package_dir.join(CONVERSION_REPORT_FILE).is_file(),
        has_resources: package_dir.join(RESOURCES_DIR).is_dir(),
        has_tests: package_dir.join(TESTS_DIR).is_dir(),
    };

    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let manifest = match parse_manifest_file(&manifest_path) {
        Ok(manifest) => {
            errors.extend(apxm_skill::manifest_shape_errors(&manifest));
            Some(manifest)
        }
        Err(error) => {
            errors.push(error);
            None
        }
    };

    let hashes = SkillPackageHashes {
        source_hash: file_hash_if_present(&source_path, &mut errors),
        air_hash: file_hash_if_present(&air_path, &mut errors),
        artifact_hash: file_hash_if_present(&artifact_path, &mut errors),
    };

    if let Some(manifest) = &manifest {
        validate_declared_hash(
            "source_hash",
            manifest.source_hash.as_deref(),
            hashes.source_hash.as_deref(),
            files.has_skill_md,
            &mut errors,
            &mut warnings,
        );
        validate_declared_hash(
            "air_hash",
            manifest.air_hash.as_deref(),
            hashes.air_hash.as_deref(),
            files.has_air,
            &mut errors,
            &mut warnings,
        );
        validate_declared_hash(
            "artifact_hash",
            manifest.artifact_hash.as_deref(),
            hashes.artifact_hash.as_deref(),
            files.has_artifact,
            &mut errors,
            &mut warnings,
        );
    }

    if !files.has_artifact {
        warnings.push(format!("{ARTIFACT_FILE} missing; skill is not compiled"));
    }

    let compile_status = if !errors.is_empty() {
        CompileStatus::Invalid
    } else if files.has_artifact {
        CompileStatus::Compiled
    } else {
        CompileStatus::NotCompiled
    };
    let validation = SkillValidationReport {
        status: if errors.is_empty() {
            ValidationStatus::Valid
        } else {
            ValidationStatus::Invalid
        },
        errors,
        warnings,
    };

    let pack = load_pack_info(package_dir);

    SkillRecord {
        skill_id: manifest.as_ref().map(|manifest| manifest.skill_id.clone()),
        version: manifest.as_ref().map(|manifest| manifest.version.clone()),
        manifest,
        package,
        files,
        hashes,
        compile_status,
        validation,
        pack,
        package_dir: package_dir.to_path_buf(),
    }
}

/// Walk up from a skill package directory to find an enclosing `pack.toml`.
///
/// Pack layout: `<libs-root>/<pack-id>/skills/<skill-id>/skill.toml`. The
/// pack manifest sits two levels above the skill manifest dir.
fn load_pack_info(package_dir: &Path) -> Option<PackInfo> {
    let pack_dir = package_dir.parent()?.parent()?;
    let pack_path = pack_dir.join(PACK_FILE);
    if !pack_path.is_file() {
        return None;
    }
    let text = fs::read_to_string(&pack_path).ok()?;
    let value: toml::Value = toml::from_str(&text).ok()?;
    let pack_id = value.get("pack_id")?.as_str()?.to_string();
    let pack_version = value
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let source = value.get("source").and_then(|v| v.as_table());
    let source_kind = source
        .and_then(|s| s.get("kind"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let source_upstream = source
        .and_then(|s| s.get("upstream"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(PackInfo {
        pack_id,
        pack_version,
        source_kind,
        source_upstream,
    })
}

fn load_static_skill_artifact(
    executable: &ExecutableSkill,
    manifest: &SkillManifest,
    state: &AppState,
) -> Result<Artifact, ApiError> {
    if executable.record.validation.status != ValidationStatus::Valid {
        let details = executable.record.validation.errors.join("; ");
        let message = if details.is_empty() {
            "skill validation failed".to_string()
        } else {
            format!("skill validation failed: {details}")
        };
        return Err(ApiError::bad_request(message));
    }
    if !matches!(executable.record.compile_status, CompileStatus::Compiled) {
        return Err(ApiError::bad_request("skill is not compiled"));
    }
    let declared_hash = manifest
        .artifact_hash
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("manifest artifact_hash is required"))?;
    if is_symlink(&executable.artifact_path) {
        return Err(ApiError::bad_request(
            "skill artifact must not be a symlink",
        ));
    }

    let bytes = fs::read(&executable.artifact_path).map_err(|error| {
        ApiError::bad_request(format!("failed to read skill artifact: {error}"))
    })?;
    let actual_hash = tagged_blake3(&bytes);
    if !hashes_match(declared_hash, &actual_hash) {
        return Err(ApiError::bad_request("skill artifact hash mismatch"));
    }

    let artifact = Artifact::from_bytes(&bytes)
        .map_err(|error| ApiError::bad_request(format!("invalid skill artifact: {error}")))?;
    validate_embedded_skill_manifest(&artifact, manifest)?;
    validate_artifact_entry_flow(&artifact, &manifest.entry_flow)?;
    validate_static_skill_admission(manifest, &artifact, state)?;
    Ok(artifact)
}

fn validate_embedded_skill_manifest(
    artifact: &Artifact,
    manifest: &SkillManifest,
) -> Result<(), ApiError> {
    let Some(data) = artifact.section_data(apxm_artifact::section_kinds::SKILL_MANIFEST_V1) else {
        return Ok(());
    };
    let text = std::str::from_utf8(data).map_err(|error| {
        ApiError::bad_request(format!("embedded skill manifest is not UTF-8: {error}"))
    })?;
    let embedded = apxm_skill::parse_manifest(text).map_err(|error| {
        ApiError::bad_request(format!("failed to parse embedded skill manifest: {error}"))
    })?;

    validate_embedded_manifest_field("skill_id", &embedded.skill_id, &manifest.skill_id)?;
    validate_embedded_manifest_field("version", &embedded.version, &manifest.version)?;
    validate_embedded_manifest_field("entry_flow", &embedded.entry_flow, &manifest.entry_flow)?;
    validate_embedded_manifest_option(
        "source_hash",
        embedded.source_hash.as_deref(),
        manifest.source_hash.as_deref(),
    )?;
    validate_embedded_manifest_option(
        "air_hash",
        embedded.air_hash.as_deref(),
        manifest.air_hash.as_deref(),
    )?;
    // artifact_hash is the BLAKE3 of the artifact bytes themselves;
    // it cannot live *inside* the artifact it hashes. The compile step
    // strips it from the embedded copy by design. The only legal pair
    // here is embedded=None, manifest=Some(...); anything else means a
    // tampered/divergent embed.
    match (
        embedded.artifact_hash.as_deref(),
        manifest.artifact_hash.as_deref(),
    ) {
        (None, _) => {}
        (Some(e), Some(m)) if e == m => {}
        _ => {
            return Err(ApiError::bad_request(
                "embedded skill manifest artifact_hash does not match skill.toml",
            ));
        }
    }
    Ok(())
}

fn validate_embedded_manifest_field(
    field: &str,
    embedded: &str,
    manifest: &str,
) -> Result<(), ApiError> {
    if embedded == manifest {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "embedded skill manifest {field} '{embedded}' does not match skill.toml '{manifest}'"
        )))
    }
}

fn validate_embedded_manifest_option(
    field: &str,
    embedded: Option<&str>,
    manifest: Option<&str>,
) -> Result<(), ApiError> {
    if embedded == manifest {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "embedded skill manifest {field} does not match skill.toml"
        )))
    }
}

fn validate_static_skill_admission(
    manifest: &SkillManifest,
    artifact: &Artifact,
    state: &AppState,
) -> Result<(), ApiError> {
    let capability_system = state.runtime.capability_system();
    for name in manifest
        .required_capabilities
        .iter()
        .chain(manifest.allowed_tools.iter())
    {
        if !capability_system.has_capability(name) {
            return Err(ApiError::bad_request(format!(
                "skill capability '{name}' is not registered"
            )));
        }
    }

    let declared_policy_value = manifest.side_effect_policy.as_deref();
    let policy = CapabilityPolicy::from_manifest_value(declared_policy_value).ok_or_else(|| {
        let declared = declared_policy_value.unwrap_or("");
        ApiError::bad_request(format!(
            "static skill execution does not support side_effect_policy '{declared}' yet"
        ))
    })?;

    let allowed_tools: HashSet<&str> = if manifest.allowed_tools.is_empty() {
        manifest
            .required_capabilities
            .iter()
            .map(String::as_str)
            .collect()
    } else {
        manifest.allowed_tools.iter().map(String::as_str).collect()
    };

    match &policy {
        CapabilityPolicy::ReadOnly => {
            for name in &allowed_tools {
                if !capability_system.is_read_only(name) {
                    return Err(ApiError::bad_request(format!(
                        "skill capability '{name}' is not read-only"
                    )));
                }
            }
        }
        CapabilityPolicy::Sandboxed => {}
        CapabilityPolicy::Broader { admits } => {
            for name in &allowed_tools {
                if !admits.contains(*name) && !capability_system.is_read_only(name) {
                    return Err(ApiError::bad_request(format!(
                        "skill capability '{name}' is not admitted by broader policy {}",
                        policy.name()
                    )));
                }
            }
        }
    }

    for dag in artifact.dags() {
        for node in &dag.nodes {
            if node.op_type == AISOperationType::InvTool {
                validate_inv_tool_node(node, &allowed_tools)?;
                if matches!(policy, CapabilityPolicy::Sandboxed) {
                    validate_sandboxed_inv_tool_node(node, state)?;
                }
            } else if !is_allowed_static_skill_op(node.op_type) {
                return Err(ApiError::bad_request(format!(
                    "skill artifact uses disallowed operation {}",
                    node.op_type
                )));
            }
        }
    }
    Ok(())
}

fn validate_sandboxed_inv_tool_node(
    node: &apxm_core::types::execution::Node,
    state: &AppState,
) -> Result<(), ApiError> {
    let capability = node
        .attributes
        .get(graph_attrs::CAPABILITY)
        .and_then(|value| value.as_string())
        .ok_or_else(|| ApiError::bad_request("INV_TOOL missing capability attribute"))?;
    let capability_system = state.runtime.capability_system();
    if capability_system.is_read_only(&capability) {
        return Ok(());
    }

    let args = inv_tool_static_args(node)?;
    match capability_system.sandbox_preflight(&capability, &args) {
        Ok(CapabilitySandboxPreflight::Sandboxed { .. }) => Ok(()),
        Ok(CapabilitySandboxPreflight::Direct) => Err(ApiError::bad_request(format!(
            "skill capability '{capability}' is side-effectful but does not declare sandbox execution"
        ))),
        Err(error) => Err(ApiError::bad_request(format!(
            "skill capability '{capability}' failed sandbox preflight: {error}"
        ))),
    }
}

fn inv_tool_static_args(
    node: &apxm_core::types::execution::Node,
) -> Result<HashMap<String, Value>, ApiError> {
    let mut args = HashMap::new();
    let Some(params_json) = node
        .attributes
        .get(graph_attrs::PARAMS_JSON)
        .and_then(|value| value.as_string())
    else {
        return Ok(args);
    };

    let parsed: serde_json::Value = serde_json::from_str(&params_json)
        .map_err(|error| ApiError::bad_request(format!("invalid INV_TOOL params_json: {error}")))?;
    let Some(obj) = parsed.as_object() else {
        return Err(ApiError::bad_request(
            "INV_TOOL params_json must be a JSON object",
        ));
    };

    for (key, value) in obj {
        let value = match value {
            serde_json::Value::String(value) => Value::String(value.clone()),
            serde_json::Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    Value::Number(apxm_core::types::values::Number::Integer(value))
                } else if let Some(value) = value.as_f64() {
                    Value::Number(apxm_core::types::values::Number::Float(value))
                } else {
                    continue;
                }
            }
            serde_json::Value::Bool(value) => Value::Bool(*value),
            _ => continue,
        };
        args.insert(key.clone(), value);
    }
    Ok(args)
}

fn validate_inv_tool_node(
    node: &apxm_core::types::execution::Node,
    allowed_tools: &HashSet<&str>,
) -> Result<(), ApiError> {
    if node.attributes.contains_key(graph_attrs::PYTHON_HANDLER_ID) {
        return Err(ApiError::bad_request(
            "static skill execution does not support python-backed INV_TOOL",
        ));
    }
    let capability = node
        .attributes
        .get(graph_attrs::CAPABILITY)
        .and_then(|value| value.as_string())
        .ok_or_else(|| ApiError::bad_request("INV_TOOL missing capability attribute"))?;
    if !allowed_tools.contains(capability.as_str()) {
        return Err(ApiError::bad_request(format!(
            "skill artifact invokes undeclared capability '{capability}'"
        )));
    }
    Ok(())
}

fn validate_artifact_entry_flow(artifact: &Artifact, entry_flow: &str) -> Result<(), ApiError> {
    let entry = artifact
        .entry_dag()
        .ok_or_else(|| ApiError::bad_request("skill artifact has no @entry DAG"))?;
    let Some(name) = entry.metadata.name.as_deref() else {
        return Err(ApiError::bad_request("skill artifact entry DAG is unnamed"));
    };
    if name != entry_flow {
        return Err(ApiError::bad_request(format!(
            "skill artifact entry flow '{name}' does not match manifest entry_flow '{entry_flow}'"
        )));
    }
    Ok(())
}

fn is_allowed_static_skill_op(op: AISOperationType) -> bool {
    matches!(
        op,
        AISOperationType::Agent
            | AISOperationType::ConstStr
            | AISOperationType::Nop
            | AISOperationType::Identity
            | AISOperationType::Merge
            | AISOperationType::Fence
            | AISOperationType::WaitAll
            | AISOperationType::Yield
            // Agent-graph orchestration: SPAWN_AGENT spawns a child agent
            // under the active scope policy; COMMUNICATE delivers a message
            // to the spawned target.
            | AISOperationType::SpawnAgent
            | AISOperationType::Communicate
            | AISOperationType::Return
    )
}

fn parse_manifest_file(path: &Path) -> Result<SkillManifest, String> {
    apxm_skill::parse_manifest_file(path)
}

// String-form manifest parser used by the in-crate unit tests
// (`parses_top_level_manifest`, `parses_nested_skill_manifest`). The release
// binary parses manifests via `parse_manifest_file`.
#[allow(dead_code)]
fn parse_manifest(contents: &str) -> Result<SkillManifest, toml::de::Error> {
    apxm_skill::parse_manifest(contents)
}

fn validate_declared_hash(
    field: &str,
    declared: Option<&str>,
    actual: Option<&str>,
    file_exists: bool,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    apxm_skill::validate_declared_hash(field, declared, actual, file_exists, errors, warnings);
}

fn hashes_match(declared: &str, actual: &str) -> bool {
    apxm_skill::hashes_match(declared, actual)
}

fn file_hash_if_present(path: &Path, errors: &mut Vec<String>) -> Option<String> {
    apxm_skill::file_hash_if_present(path, errors)
}

fn tagged_blake3(bytes: &[u8]) -> String {
    apxm_skill::tagged_blake3(bytes)
}

fn skill_session_dir(skill_id: &str, session_id: &str) -> Result<String, ApiError> {
    let base = ApxmPaths::discover()
        .map_err(|error| {
            ApiError::internal_message(format!("failed to discover APXM paths: {error}"))
        })?
        .sessions_dir()
        .map_err(|error| {
            ApiError::internal_message(format!("failed to resolve sessions dir: {error}"))
        })?;
    let session_dir = base
        .join(SKILL_SESSION_DIR)
        .join(safe_path_component(skill_id))
        .join(session_id);
    fs::create_dir_all(&session_dir).map_err(|error| {
        ApiError::internal_message(format!("failed to create skill session dir: {error}"))
    })?;
    Ok(session_dir.to_string_lossy().to_string())
}

fn validate_session_id(session_id: String) -> Result<String, ApiError> {
    if session_id.is_empty()
        || session_id == "."
        || session_id == ".."
        || !session_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(ApiError::bad_request(
            "session_id must contain only ASCII letters, digits, '-', '_', or '.', and must not be '.' or '..'",
        ));
    }
    Ok(session_id)
}

fn safe_path_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '@') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn find_manifest_dirs(root: &Path, packages: &mut Vec<PathBuf>, warnings: &mut Vec<String>) {
    if root.join(MANIFEST_FILE).is_file() {
        packages.push(root.to_path_buf());
        return;
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) => {
            warnings.push(format!(
                "failed to read skill root {}: {error}",
                root.display()
            ));
            return;
        }
    };

    let mut dirs = Vec::new();
    for entry in entries {
        match entry {
            Ok(entry) => {
                let path = entry.path();
                if path.is_dir() && !is_symlink(&path) {
                    dirs.push(path);
                }
            }
            Err(error) => warnings.push(format!("failed to read skill entry: {error}")),
        }
    }
    dirs.sort();

    for dir in dirs {
        find_manifest_dirs(&dir, packages, warnings);
    }
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
}

fn annotate_duplicates(records: &mut [SkillRecord]) {
    let mut counts: HashMap<(String, String), usize> = HashMap::new();
    for record in records.iter() {
        if let (Some(skill_id), Some(version)) = (&record.skill_id, &record.version) {
            *counts
                .entry((skill_id.clone(), version.clone()))
                .or_default() += 1;
        }
    }

    for record in records {
        if let (Some(skill_id), Some(version)) = (&record.skill_id, &record.version) {
            if counts
                .get(&(skill_id.clone(), version.clone()))
                .copied()
                .unwrap_or_default()
                > 1
            {
                record.validation.status = ValidationStatus::Invalid;
                record.compile_status = CompileStatus::Invalid;
                record
                    .validation
                    .errors
                    .push(format!("duplicate skill id/version: {skill_id}@{version}"));
            }
        }
    }
}

fn find_record(
    records: Vec<SkillRecord>,
    requested_id: &str,
) -> Result<SkillRecord, SkillLookupError> {
    let (full_id, requested_version) = split_requested_id(requested_id);
    // Namespaced ids (`lib::skill`) filter by pack, disambiguating collisions;
    // bare ids match by skill_id across all packs (unchanged behaviour).
    let (pack_filter, skill_id) = split_namespaced_id(full_id);
    let mut matches: Vec<SkillRecord> = records
        .into_iter()
        .filter(|record| record.skill_id.as_deref() == Some(skill_id))
        .filter(|record| match pack_filter {
            Some(pack) => record
                .pack
                .as_ref()
                .map(|p| p.pack_id == pack)
                .unwrap_or(false),
            None => true,
        })
        .filter(|record| {
            requested_version.is_none() || record.version.as_deref() == requested_version
        })
        .collect();

    match matches.len() {
        0 => Err(SkillLookupError::NotFound(requested_id.to_string())),
        1 => Ok(matches.remove(0)),
        _ => Err(SkillLookupError::Ambiguous(requested_id.to_string())),
    }
}

/// Split a `lib::skill` namespaced id into `(Some(lib), skill)`; a bare id
/// returns `(None, id)`. Only the first `::` separates the library.
fn split_namespaced_id(id: &str) -> (Option<&str>, &str) {
    match id.split_once("::") {
        Some((pack, name)) if !pack.is_empty() && !name.is_empty() => (Some(pack), name),
        _ => (None, id),
    }
}

fn split_requested_id(requested_id: &str) -> (&str, Option<&str>) {
    requested_id
        .rsplit_once('@')
        .map_or((requested_id, None), |(skill_id, version)| {
            (skill_id, Some(version))
        })
}

fn skill_lookup_error(error: SkillLookupError) -> ApiError {
    match error {
        SkillLookupError::NotFound(id) => ApiError::not_found(format!("skill not found: {id}")),
        SkillLookupError::Ambiguous(id) => ApiError::bad_request(format!(
            "skill id has multiple versions; request {id}@<version>"
        )),
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    const TEST_SKILL_ID: &str = "checkout-context-triage";
    const TEST_SKILL_VERSION: &str = "0.1.0";
    const TEST_ENTRY_FLOW: &str = "main";

    #[test]
    fn splits_namespaced_skill_id() {
        assert_eq!(split_namespaced_id("ops::deploy"), (Some("ops"), "deploy"));
        assert_eq!(split_namespaced_id("deploy"), (None, "deploy"));
        assert_eq!(split_namespaced_id("::deploy"), (None, "::deploy"));
        assert_eq!(split_namespaced_id("ops::"), (None, "ops::"));
    }

    #[test]
    fn parses_top_level_manifest() {
        let manifest = parse_manifest(&format!(
            r#"
skill_id = "{TEST_SKILL_ID}"
version = "{TEST_SKILL_VERSION}"
entry_flow = "{TEST_ENTRY_FLOW}"
"#
        ))
        .expect("manifest");

        assert_eq!(manifest.skill_id, TEST_SKILL_ID);
        assert_eq!(manifest.version, TEST_SKILL_VERSION);
        assert_eq!(manifest.entry_flow, TEST_ENTRY_FLOW);
    }

    #[test]
    fn parses_nested_skill_manifest() {
        let manifest = parse_manifest(&format!(
            r#"
[skill]
skill_id = "{TEST_SKILL_ID}"
version = "{TEST_SKILL_VERSION}"
entry_flow = "{TEST_ENTRY_FLOW}"
"#
        ))
        .expect("manifest");

        assert_eq!(manifest.skill_id, TEST_SKILL_ID);
        assert_eq!(manifest.version, TEST_SKILL_VERSION);
        assert_eq!(manifest.entry_flow, TEST_ENTRY_FLOW);
    }

    #[test]
    fn load_pack_info_extracts_pack_metadata() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let pack_dir = tmp.path().join("obra-superpowers-brainstorming");
        let skill_dir = pack_dir
            .join("skills")
            .join("obra-superpowers-brainstorming");
        fs::create_dir_all(&skill_dir).expect("mkdir");
        fs::write(
            pack_dir.join(PACK_FILE),
            r#"
pack_id = "obra-superpowers-brainstorming"
version = "0.0.1"
skill = "obra-superpowers-brainstorming"

[source]
kind = "port"
upstream = "https://github.com/obra/superpowers"
"#,
        )
        .expect("write pack.toml");

        let info = load_pack_info(&skill_dir).expect("pack info");
        assert_eq!(info.pack_id, "obra-superpowers-brainstorming");
        assert_eq!(info.pack_version.as_deref(), Some("0.0.1"));
        assert_eq!(info.source_kind.as_deref(), Some("port"));
        assert_eq!(
            info.source_upstream.as_deref(),
            Some("https://github.com/obra/superpowers")
        );
    }

    #[test]
    fn load_pack_info_returns_none_for_legacy_layout() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let skill_dir = tmp.path().join("legacy-skill");
        fs::create_dir_all(&skill_dir).expect("mkdir");
        assert!(load_pack_info(&skill_dir).is_none());
    }

    use crate::checkpoints::CheckpointStore;
    use crate::executions::ExecutionStore;
    use crate::skills::SkillLibrary;
    use crate::state::AppState;
    use crate::tasks::TaskQueueManager;
    use apxm_runtime::capability::executor::EchoCapability;
    use apxm_runtime::{Runtime, RuntimeConfig};
    use dashmap::DashMap;
    use std::sync::Arc;
    use std::time::SystemTime;

    fn prompt_only_prepared(
        required_capabilities: Vec<String>,
        allowed_tools: Vec<String>,
    ) -> PreparedPromptOnlyExecution {
        PreparedPromptOnlyExecution {
            skill_md_body: "You are a helper.".to_string(),
            args: vec!["hello".to_string()],
            session_id: "test-session".to_string(),
            session_dir: "/tmp".to_string(),
            timeout_ms: None,
            execution_id: "test-exec".to_string(),
            skill_id: TEST_SKILL_ID.to_string(),
            skill_version: TEST_SKILL_VERSION.to_string(),
            required_capabilities,
            allowed_tools,
        }
    }

    async fn test_state_with_echo() -> AppState {
        let runtime = Runtime::new(RuntimeConfig::in_memory())
            .await
            .expect("test runtime");
        runtime
            .capability_system()
            .register(Arc::new(EchoCapability::new()))
            .expect("register echo capability");
        AppState {
            runtime: Arc::new(runtime),
            agent_registry: Arc::new(DashMap::new()),
            task_manager: TaskQueueManager::new(),
            checkpoint_store: CheckpointStore::new(),
            start_time: SystemTime::now(),
            a2a_tasks: Arc::new(DashMap::new()),
            skill_library: SkillLibrary::default(),
            execution_store: ExecutionStore::new(),
            run_event_bus: crate::runs::RunEventBus::new(),
            webhook_dispatcher: None,
            rollout_paths: Arc::new(apxm_rollout::RolloutPaths::new({
                let dir = tempfile::tempdir().expect("rollout home");
                let path = dir.path().to_path_buf();
                std::mem::forget(dir);
                path
            })),
            rollout_index: Arc::new(tokio::sync::Mutex::new(
                apxm_rollout::IndexDb::open_in_memory().expect("rollout index"),
            )),
            rollout_registry: crate::rollout::RolloutRegistry::new(),
            inference_limiter: crate::state::InferenceLimiter::unlimited_for_tests(),
            server_config: apxm_driver::ServerConfig::default(),
            cancel_registry: Arc::new(DashMap::new()),
        }
    }

    #[tokio::test]
    async fn prompt_only_skill_filters_tool_surface_to_declared_capabilities() {
        let state = test_state_with_echo().await;
        let prepared = prompt_only_prepared(vec!["echo".to_string()], vec![]);

        let request = build_prompt_only_request(&state, &prepared);

        let tools = request
            .tools
            .expect("tools present when capability declared");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        // ToolChoice::Auto is the only legal value when the surface is non-empty.
        assert!(matches!(request.tool_choice, Some(ToolChoice::Auto)));
    }

    #[tokio::test]
    async fn prompt_only_skill_filters_out_unregistered_capabilities() {
        let state = test_state_with_echo().await;
        // `nonexistent_tool` is declared but never registered — must NOT
        // leak into the surface.
        let prepared = prompt_only_prepared(
            vec!["echo".to_string(), "nonexistent_tool".to_string()],
            vec![],
        );

        let request = build_prompt_only_request(&state, &prepared);

        let tools = request.tools.expect("tools present");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
    }

    #[tokio::test]
    async fn prompt_only_skill_prefers_allowed_tools_over_required_capabilities() {
        let state = test_state_with_echo().await;
        // When `allowed_tools` is set, it's the authoritative surface;
        // `required_capabilities` does NOT widen it.
        let prepared = prompt_only_prepared(vec!["echo".to_string()], vec!["echo".to_string()]);

        let request = build_prompt_only_request(&state, &prepared);

        let tools = request.tools.expect("tools present");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
    }

    #[tokio::test]
    async fn prompt_only_skill_with_no_required_capabilities_disables_tool_use() {
        let state = test_state_with_echo().await;
        let prepared = prompt_only_prepared(vec![], vec![]);

        let request = build_prompt_only_request(&state, &prepared);

        assert!(
            request.tools.is_none(),
            "empty ACL must not advertise any tools"
        );
        assert!(
            request.tool_choice.is_none(),
            "empty ACL must leave tool_choice unset"
        );
    }
}
