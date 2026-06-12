use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use apxm_artifact::Artifact;
use apxm_backends::LLMRequest;
use apxm_compiler::{Context as CompilerContext, Pipeline as CompilerPipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::{memory as memory_const, session::files as session_files};
use apxm_core::events::{EventEmitter, EventSource, SkillEventProvenance};
use apxm_core::paths::ApxmPaths;
use apxm_core::types::Value as RuntimeValue;
use apxm_core::types::{AISOperationType, OptimizationLevel};
use apxm_driver::ServerMcpConfig;
use apxm_runtime::capability::CapabilitySandboxPreflight;
use apxm_runtime::{
    EmitterAdapter, ExecutionEventEmitter, MemorySpace, Runtime, RuntimeExecutionResult,
};
use serde_json::{Value as JsonValue, json};

use crate::mcp_protocol::{
    admission_error, args as mcp_args, defaults as mcp_defaults, evidence_path,
    status as mcp_status, tool_result, workflow_skill,
};

const WORKFLOW_PROMPT: &str = include_str!("../skills/prompt-as-workflow/prompt.md");
const WORKFLOW_REPAIR_FEEDBACK_HEADING: &str = "Compiler feedback to repair AIR:";
const WORKFLOW_REPAIR_FEEDBACK_PREFIX: &str =
    "The previous response could not be compiled as executable APXM AIR";
const WORKFLOW_REPAIRED_INVALID_PREFIX: &str = "repaired AIR still invalid";
const WORKFLOW_CAPABILITY_GUIDANCE_HEADING: &str = "Registered APXM capabilities:";
const WORKFLOW_CAPABILITY_NONE_GUIDANCE: &str = "none. Do not emit ais.inv_tool; use ais.ask, ais.think, ais.wait_all, and func.return instead.";
const WORKFLOW_CAPABILITY_TRUNCATED_GUIDANCE: &str = "- additional capabilities omitted";
#[allow(dead_code)]
pub(crate) struct WorkflowExecutionStart {
    pub(crate) execution_id: String,
    pub(crate) session_id: String,
    pub(crate) session_dir: String,
    pub(crate) air_hash: String,
    pub(crate) artifact_hash: String,
}

pub(crate) struct WorkflowExecutionHandle {
    pub(crate) execution_id: String,
    pub(crate) emitter: Arc<dyn EventEmitter>,
}

pub(crate) trait WorkflowExecutionRecorder: Send + Sync {
    fn start(&self, start: WorkflowExecutionStart) -> WorkflowExecutionHandle;
    fn complete_success(
        &self,
        execution_id: &str,
        result: &RuntimeExecutionResult,
        session_dir: &str,
    );
    fn complete_failure(&self, execution_id: &str, error: String);
}

struct CompiledWorkflowAir {
    air: String,
    artifact: Artifact,
    artifact_hash: String,
    air_hash: String,
    compile_ms: u128,
}

enum WorkflowCandidateError {
    Emission(String),
    Timeout(String),
    InvalidCandidate(String),
}

impl WorkflowCandidateError {
    fn into_message(self) -> String {
        match self {
            Self::Emission(message) | Self::Timeout(message) | Self::InvalidCandidate(message) => {
                message
            }
        }
    }
}

#[allow(dead_code)]
pub(crate) async fn prompt_as_workflow(
    runtime: &Runtime,
    args: JsonValue,
) -> Result<JsonValue, String> {
    prompt_as_workflow_with_recorder(runtime, args, None, &ServerMcpConfig::default()).await
}

pub(crate) async fn prompt_as_workflow_with_recorder(
    runtime: &Runtime,
    args: JsonValue,
    recorder: Option<Arc<dyn WorkflowExecutionRecorder>>,
    config: &ServerMcpConfig,
) -> Result<JsonValue, String> {
    let task = required_string_arg(&args, mcp_args::TASK)?;
    let context = optional_string_arg(&args, mcp_args::CONTEXT)?;
    let constraints = args.get(mcp_args::CONSTRAINTS).cloned();
    let parameters = args
        .get(mcp_args::PARAMETERS)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let should_execute = args
        .get(mcp_args::EXECUTE)
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    let trace_id = args
        .get(mcp_args::TRACE_ID)
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("{}-{}", workflow_skill::TRACE_PREFIX, uuid::Uuid::new_v4()));
    validate_trace_id(&trace_id)?;

    let emission_start = Instant::now();
    let compiled = match emit_air_candidate(
        runtime,
        &task,
        context.as_deref(),
        constraints.as_ref(),
        None,
        &trace_id,
        config,
    )
    .await
    {
        Ok(candidate) => match build_compiled_workflow_air(candidate, Some(runtime)) {
            Ok(compiled) => compiled,
            Err(error) => {
                repair_air_candidate(
                    runtime,
                    &task,
                    context.as_deref(),
                    constraints.as_ref(),
                    &trace_id,
                    &error,
                    config,
                )
                .await?
            }
        },
        Err(WorkflowCandidateError::InvalidCandidate(error)) => {
            repair_air_candidate(
                runtime,
                &task,
                context.as_deref(),
                constraints.as_ref(),
                &trace_id,
                &error,
                config,
            )
            .await?
        }
        Err(error @ WorkflowCandidateError::Emission(_))
        | Err(error @ WorkflowCandidateError::Timeout(_)) => return Err(error.into_message()),
    };
    let emission_ms = emission_start.elapsed().as_millis();
    let session_dir = workflow_session_dir(&trace_id)?;
    let air_path = write_generated_air(&session_dir, &compiled.air)?;

    let mut output = json!({
        (tool_result::STATUS): mcp_status::COMPILED,
        (tool_result::TRACE_ID): trace_id,
        (tool_result::SESSION_DIR): session_dir,
        (tool_result::AIR_PATH): air_path,
        (tool_result::AIR_TEXT): compiled.air.clone(),
        (tool_result::AIR_HASH): compiled.air_hash.clone(),
        (tool_result::ARTIFACT_HASH): compiled.artifact_hash.clone(),
        (tool_result::STATS): compile_stats(&compiled.artifact, compiled.compile_ms, emission_ms),
    });

    if should_execute {
        validate_generated_workflow_admission(&compiled.artifact, runtime)?;
        let runtime_args = runtime_args_for_artifact(&compiled.artifact, &parameters);
        let session_id = output[tool_result::TRACE_ID]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let session_dir = output[tool_result::SESSION_DIR]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let execution_handle = recorder.as_ref().map(|recorder| {
            recorder.start(WorkflowExecutionStart {
                execution_id: session_id.clone(),
                session_id: session_id.clone(),
                session_dir: session_dir.clone(),
                air_hash: output[tool_result::AIR_HASH]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                artifact_hash: output[tool_result::ARTIFACT_HASH]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            })
        });
        let event_emitter: Option<Arc<dyn ExecutionEventEmitter>> =
            execution_handle.as_ref().map(|handle| {
                Arc::new(
                    EmitterAdapter::new(
                        Arc::clone(&handle.emitter),
                        EventSource::Runtime,
                        &session_id,
                    )
                    .with_skill_provenance(SkillEventProvenance {
                        skill_id: workflow_skill::ID.to_string(),
                        skill_version: workflow_skill::VERSION.to_string(),
                        parent_skill_id: None,
                        parent_execution_id: None,
                        flow_name: Some(workflow_skill::ENTRY_FLOW.to_string()),
                    }),
                ) as Arc<dyn ExecutionEventEmitter>
            });
        let execute_start = Instant::now();
        let execution = match runtime
            .execute_artifact_with_session_and_emitter(
                compiled.artifact,
                runtime_args,
                Some(session_id),
                event_emitter,
                Some(session_dir.clone()),
            )
            .await
        {
            Ok(execution) => execution,
            Err(error) => {
                if let Some(handle) = execution_handle {
                    let message = format!("workflow execution failed: {error}");
                    if let Some(recorder) = recorder.as_ref() {
                        recorder.complete_failure(&handle.execution_id, message.clone());
                    }
                    return Err(message);
                }
                return Err(format!("workflow execution failed: {error}"));
            }
        };
        if let (Some(recorder), Some(handle)) = (recorder.as_ref(), execution_handle.as_ref()) {
            recorder.complete_success(&handle.execution_id, &execution, &session_dir);
            output[tool_result::EXECUTION_ID] = JsonValue::String(handle.execution_id.clone());
        }
        output[tool_result::STATUS] = JsonValue::String(mcp_status::EXECUTED.to_string());
        output[tool_result::SUMMARY] =
            execution_summary(&execution, execute_start.elapsed().as_millis());
    } else {
        output[tool_result::SUMMARY] = json!({
            (tool_result::EXECUTED_NODES): 0,
            (tool_result::FAILED_NODES): 0,
            (tool_result::EXECUTE_MS): 0,
        });
    }

    let _ = runtime
        .memory()
        .record_episodic_event(
            output[tool_result::TRACE_ID]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            workflow_skill::ID,
            RuntimeValue::try_from(output.clone()).unwrap_or(RuntimeValue::Null),
            None,
            None,
        )
        .await;

    Ok(output)
}

#[allow(dead_code)]
pub(crate) async fn trace_fetch(
    runtime: Option<&Runtime>,
    execution_record: Option<JsonValue>,
    args: JsonValue,
) -> Result<JsonValue, String> {
    trace_fetch_with_config(runtime, execution_record, args, &ServerMcpConfig::default()).await
}

pub(crate) async fn trace_fetch_with_config(
    runtime: Option<&Runtime>,
    execution_record: Option<JsonValue>,
    args: JsonValue,
    config: &ServerMcpConfig,
) -> Result<JsonValue, String> {
    let trace_id = required_string_arg(&args, mcp_args::TRACE_ID)?;
    let full = args
        .get(mcp_args::FULL)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let node_id = args.get(mcp_args::NODE_ID).and_then(JsonValue::as_u64);

    let mut trace = json!({
        (tool_result::TRACE_ID): trace_id,
        (tool_result::STATUS): mcp_status::NOT_FOUND,
    });

    if let Some(record) = execution_record {
        trace[tool_result::STATUS] = JsonValue::String(mcp_status::FOUND.to_string());
        trace[tool_result::EXECUTION] = summarize_execution_record(record, node_id, full);
    }

    if let Some(runtime) = runtime {
        match runtime.memory().query_episodes(&trace_id).await {
            Ok(episodes) if !episodes.is_empty() => {
                trace[tool_result::STATUS] = JsonValue::String(mcp_status::FOUND.to_string());
                let values = episodes
                    .into_iter()
                    .take(if full {
                        usize::MAX
                    } else {
                        config.default_trace_event_limit.max(1)
                    })
                    .map(|episode| serde_json::to_value(episode).unwrap_or(JsonValue::Null))
                    .collect::<Vec<_>>();
                trace[tool_result::EPISODES] = JsonValue::Array(values);
            }
            Ok(_) => {}
            Err(error) => {
                trace[tool_result::WARNINGS] =
                    json!([format!("episodic memory query failed: {error}")]);
            }
        }
    }

    if trace[tool_result::STATUS] == mcp_status::NOT_FOUND {
        let files = lookup_trace_files(&trace_id, full, config)?;
        if !files.is_empty() {
            trace[tool_result::STATUS] = JsonValue::String(mcp_status::FOUND.to_string());
            trace[tool_result::FILES] = JsonValue::Array(files);
        }
    }

    Ok(trace)
}

#[allow(dead_code)]
pub(crate) async fn aam_recall(runtime: &Runtime, args: JsonValue) -> Result<JsonValue, String> {
    aam_recall_with_config(runtime, args, &ServerMcpConfig::default()).await
}

pub(crate) async fn aam_recall_with_config(
    runtime: &Runtime,
    args: JsonValue,
    config: &ServerMcpConfig,
) -> Result<JsonValue, String> {
    let query = args
        .get(mcp_args::QUERY)
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let top_k = bounded_usize_arg(
        &args,
        mcp_args::TOP_K,
        config.default_top_k,
        1,
        config.max_top_k,
    )?;

    let beliefs = runtime
        .aam()
        .beliefs()
        .into_iter()
        .filter(|(key, value)| matches_query(&query, key, value))
        .take(top_k)
        .map(|(key, value)| json!({ (tool_result::NAME): key, (tool_result::VALUE): value }))
        .collect::<Vec<_>>();

    let goals = runtime
        .aam()
        .goals()
        .into_iter()
        .filter(|goal| {
            query.is_empty()
                || serde_json::to_string(goal)
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(&query)
        })
        .take(top_k)
        .map(|goal| serde_json::to_value(goal).unwrap_or(JsonValue::Null))
        .collect::<Vec<_>>();

    let transitions = runtime
        .aam()
        .recent_transitions(top_k)
        .into_iter()
        .filter(|transition| {
            query.is_empty()
                || format!("{transition:?}")
                    .to_ascii_lowercase()
                    .contains(&query)
        })
        .map(|transition| {
            json!({
                (tool_result::TIMESTAMP): transition.timestamp.to_rfc3339(),
                (tool_result::LABEL): serde_json::to_value(&transition.label).unwrap_or(JsonValue::Null),
                (tool_result::BELIEF_CHANGES): format!("{:?}", transition.belief_changes),
                (tool_result::GOAL_CHANGES): format!("{:?}", transition.goal_changes),
                (tool_result::CAPABILITY_CHANGES): format!("{:?}", transition.capability_changes),
            })
        })
        .collect::<Vec<_>>();

    let mut memory = Vec::new();
    if !query.is_empty() {
        for space in [MemorySpace::Stm, MemorySpace::Ltm, MemorySpace::Episodic] {
            if let Ok(results) = runtime.memory().search(space, &query, top_k).await {
                memory.extend(results.into_iter().map(|result| {
                    json!({
                        (mcp_args::SPACE): memory_space_name(space),
                        (tool_result::KEY): result.key,
                        (tool_result::SCORE): result.score,
                        (tool_result::VALUE): result.value,
                    })
                }));
            }
        }
        memory.truncate(top_k);
    }

    Ok(json!({
        (tool_result::QUERY): query,
        (tool_result::AAM): {
            (tool_result::BELIEFS): beliefs,
            (tool_result::GOALS): goals,
            (tool_result::TRANSITIONS): transitions,
        },
        (tool_result::MEMORY): memory,
    }))
}

#[allow(dead_code)]
pub(crate) fn capability_list(runtime: &Runtime, args: JsonValue) -> JsonValue {
    capability_list_with_config(runtime, args, &ServerMcpConfig::default())
}

pub(crate) fn capability_list_with_config(
    runtime: &Runtime,
    args: JsonValue,
    config: &ServerMcpConfig,
) -> JsonValue {
    let query = args
        .get(mcp_args::QUERY)
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let query_terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>();
    let top_k = args
        .get(mcp_args::TOP_K)
        .and_then(JsonValue::as_u64)
        .map(|value| value as usize)
        .filter(|value| *value > 0)
        .unwrap_or(config.default_top_k)
        .clamp(1, config.max_top_k.max(1));
    let mut capabilities: Vec<(JsonValue, usize)> = runtime
        .capability_system()
        .list_capabilities()
        .into_iter()
        .map(|capability| {
            let value = serde_json::to_value(capability).unwrap_or(JsonValue::Null);
            let score = capability_query_score(&query_terms, &value);
            (value, score)
        })
        .collect();
    if !query_terms.is_empty() {
        capabilities.retain(|(_, score)| *score > 0);
    }
    capabilities.sort_by(|(left, left_score), (right, right_score)| {
        right_score.cmp(left_score).then_with(|| {
            capability_name(left)
                .unwrap_or_default()
                .cmp(capability_name(right).unwrap_or_default())
        })
    });
    let capabilities = capabilities
        .into_iter()
        .take(top_k)
        .map(|(value, _)| value)
        .collect::<Vec<_>>();

    let backends = runtime.llm_registry().backend_names();
    let health = runtime
        .model_router()
        .map(|router| serde_json::to_value(router.all_health()).unwrap_or(JsonValue::Null))
        .unwrap_or(JsonValue::Null);

    json!({
        (tool_result::CAPABILITIES): capabilities,
        (tool_result::BACKENDS): backends,
        (tool_result::HEALTH): health,
    })
}

fn capability_query_score(query_terms: &[&str], capability: &JsonValue) -> usize {
    if query_terms.is_empty() {
        return 1;
    }
    let haystack = serde_json::to_string(capability)
        .unwrap_or_default()
        .to_ascii_lowercase();
    query_terms
        .iter()
        .filter(|term| haystack.contains(**term))
        .count()
}

fn capability_name(capability: &JsonValue) -> Option<&str> {
    capability
        .get(tool_result::NAME)
        .and_then(JsonValue::as_str)
}

#[allow(dead_code)]
pub(crate) fn evidence_lookup(args: JsonValue) -> Result<JsonValue, String> {
    evidence_lookup_with_config(args, &ServerMcpConfig::default())
}

pub(crate) fn evidence_lookup_with_config(
    args: JsonValue,
    config: &ServerMcpConfig,
) -> Result<JsonValue, String> {
    let query = args
        .get(mcp_args::QUERY)
        .or_else(|| args.get(mcp_args::CLAIM_ID))
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let explicit_path = args.get(mcp_args::PATH).and_then(JsonValue::as_str);
    let limit = bounded_usize_arg(
        &args,
        mcp_args::LIMIT,
        config.default_evidence_limit,
        1,
        config.max_evidence_limit,
    )?;
    let roots = evidence_roots()?;
    let matches = if let Some(path) = explicit_path {
        lookup_explicit_evidence_path(path, &roots, &query, config)?
    } else {
        scan_evidence_roots(&roots, &query, limit, config)?
    };

    Ok(json!({
        (tool_result::QUERY): query,
        (tool_result::MATCHES): matches,
    }))
}

async fn repair_air_candidate(
    runtime: &Runtime,
    task: &str,
    context: Option<&str>,
    constraints: Option<&JsonValue>,
    trace_id: &str,
    first_error: &str,
    config: &ServerMcpConfig,
) -> Result<CompiledWorkflowAir, String> {
    let mut last_error = first_error.to_string();
    for _ in 0..config.workflow_repair_attempts.max(1) {
        let feedback = format!("{WORKFLOW_REPAIR_FEEDBACK_PREFIX}: {last_error}");
        let repaired = match emit_air_candidate(
            runtime,
            task,
            context,
            constraints,
            Some(&feedback),
            trace_id,
            config,
        )
        .await
        {
            Ok(candidate) => candidate,
            Err(WorkflowCandidateError::InvalidCandidate(error)) => {
                last_error = error;
                continue;
            }
            Err(error @ WorkflowCandidateError::Emission(_)) => {
                return Err(error.into_message());
            }
            Err(error @ WorkflowCandidateError::Timeout(_)) => return Err(error.into_message()),
        };
        match build_compiled_workflow_air(repaired, Some(runtime)) {
            Ok(compiled) => return Ok(compiled),
            Err(error) => last_error = error,
        }
    }
    Err(format!("{WORKFLOW_REPAIRED_INVALID_PREFIX}: {last_error}"))
}

fn build_compiled_workflow_air(
    air: String,
    runtime: Option<&Runtime>,
) -> Result<CompiledWorkflowAir, String> {
    let compile_start = Instant::now();
    let artifact = compile_air_to_artifact(&air)?;
    let compile_ms = compile_start.elapsed().as_millis();
    if let Some(runtime) = runtime {
        validate_generated_workflow_admission(&artifact, runtime)?;
    }
    let artifact_bytes = artifact
        .to_bytes()
        .map_err(|error| format!("artifact encode failed: {error}"))?;
    let artifact_hash = format!("blake3:{}", blake3::hash(&artifact_bytes).to_hex());
    let air_hash = format!("blake3:{}", blake3::hash(air.as_bytes()).to_hex());

    Ok(CompiledWorkflowAir {
        air,
        artifact,
        artifact_hash,
        air_hash,
        compile_ms,
    })
}

/// Admission rules for a side-effecting capability invoked by a generated workflow:
/// the capability must be registered, and either read-only or sandbox-preflight
/// clean (a `Direct` side effect is rejected).
fn check_generated_capability_admission(
    capability: &str,
    args: impl FnOnce() -> Result<HashMap<String, RuntimeValue>, String>,
    runtime: &Runtime,
) -> Result<(), String> {
    let capability_system = runtime.capability_system();

    if !capability_system.has_capability(capability) {
        return Err(admission_error::generated_capability_not_registered(
            capability,
        ));
    }

    if capability_system.is_read_only(capability) {
        return Ok(());
    }

    let args = args()?;
    match capability_system.sandbox_preflight(capability, &args) {
        Ok(CapabilitySandboxPreflight::Sandboxed { .. }) => Ok(()),
        Ok(CapabilitySandboxPreflight::Direct) => Err(
            admission_error::generated_capability_direct_side_effect(capability),
        ),
        Err(error) => Err(admission_error::generated_capability_sandbox_preflight(
            capability,
            &error.to_string(),
        )),
    }
}

fn emit_prompt(
    task: &str,
    context: Option<&str>,
    constraints: Option<&JsonValue>,
    feedback: Option<&str>,
    capability_guidance: &str,
) -> String {
    let mut prompt = String::from(WORKFLOW_PROMPT);
    prompt.push_str("\n\n");
    prompt.push_str(capability_guidance);
    prompt.push_str("\n\nTask:\n");
    prompt.push_str(task);
    if let Some(context) = context.filter(|value| !value.trim().is_empty()) {
        prompt.push_str("\n\nContext:\n");
        prompt.push_str(context);
    }
    if let Some(constraints) = constraints {
        prompt.push_str("\n\nConstraints:\n");
        prompt.push_str(
            &serde_json::to_string_pretty(constraints).unwrap_or_else(|_| constraints.to_string()),
        );
    }
    if let Some(feedback) = feedback {
        prompt.push_str("\n\n");
        prompt.push_str(WORKFLOW_REPAIR_FEEDBACK_HEADING);
        prompt.push('\n');
        prompt.push_str(feedback);
    }
    prompt
}

fn workflow_capability_guidance(runtime: &Runtime, config: &ServerMcpConfig) -> String {
    let mut capabilities = runtime.capability_system().list_capabilities();
    capabilities.sort_by(|left, right| left.name.cmp(&right.name));
    let limit = config.workflow_capability_guidance_limit.max(1);

    let mut guidance = String::from(WORKFLOW_CAPABILITY_GUIDANCE_HEADING);
    guidance.push('\n');
    if capabilities.is_empty() {
        guidance.push_str(WORKFLOW_CAPABILITY_NONE_GUIDANCE);
        return guidance;
    }

    for capability in capabilities.iter().take(limit) {
        guidance.push_str("- ");
        guidance.push_str(&capability.name);
        guidance.push_str(" (read_only=");
        guidance.push_str(if capability.read_only {
            "true"
        } else {
            "false"
        });
        if !capability.groups.is_empty() {
            guidance.push_str(", groups=");
            guidance.push_str(&capability.groups.join(","));
        }
        guidance.push(')');
        if !capability.description.trim().is_empty() {
            guidance.push_str(": ");
            guidance.push_str(capability.description.trim());
        }
        guidance.push('\n');
    }
    if capabilities.len() > limit {
        guidance.push_str(WORKFLOW_CAPABILITY_TRUNCATED_GUIDANCE);
    }
    guidance
}

async fn emit_air_candidate(
    runtime: &Runtime,
    task: &str,
    context: Option<&str>,
    constraints: Option<&JsonValue>,
    feedback: Option<&str>,
    trace_id: &str,
    config: &ServerMcpConfig,
) -> Result<String, WorkflowCandidateError> {
    let router = runtime
        .model_router()
        .ok_or_else(|| WorkflowCandidateError::Emission("model router unavailable; initialize APXM server runtime with ModelRouter before calling prompt_as_workflow".to_string()))?;
    let capability_guidance = workflow_capability_guidance(runtime, config);
    let request = LLMRequest::new(emit_prompt(
        task,
        context,
        constraints,
        feedback,
        &capability_guidance,
    ))
    .with_max_tokens(config.workflow_max_tokens.max(1))
    .with_temperature(config.workflow_temperature.clamp(0.0, 2.0))
    .with_operation_type(AISOperationType::Plan)
    .with_metadata_value(
        workflow_skill::REQUEST_CAPABILITY_KEY,
        json!(workflow_skill::EMISSION_CAPABILITY),
    )
    .with_trace_id(trace_id.to_string());
    let timeout_ms = config.workflow_emit_timeout_ms.max(1);
    let response =
        match tokio::time::timeout(Duration::from_millis(timeout_ms), router.generate(request))
            .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                return Err(WorkflowCandidateError::Emission(format!(
                    "model-router AIR emission failed: {error}"
                )));
            }
            Err(_) => {
                return Err(WorkflowCandidateError::Timeout(format!(
                    "model-router AIR emission timed out after {timeout_ms}ms"
                )));
            }
        };
    extract_air_document(&response.content).map_err(WorkflowCandidateError::InvalidCandidate)
}

fn runtime_args_for_artifact(
    artifact: &Artifact,
    parameters: &serde_json::Map<String, JsonValue>,
) -> Vec<String> {
    artifact
        .entry_dag()
        .map(|dag| dag.metadata.parameters.as_slice())
        .unwrap_or(&[])
        .iter()
        .map(|parameter| {
            parameters
                .get(&parameter.name)
                .map(json_arg_to_string)
                .unwrap_or_default()
        })
        .collect()
}

fn validate_generated_workflow_admission(
    artifact: &Artifact,
    runtime: &Runtime,
) -> Result<(), String> {
    if artifact
        .sections()
        .iter()
        .any(|section| section.kind == apxm_runtime::python_tools::CAPABILITY_NAME)
    {
        return Err(admission_error::GENERATED_PYTHON_TOOL_SECTIONS.to_string());
    }

    for dag in artifact.dags() {
        for node in &dag.nodes {
            if node.attributes.contains_key(graph_attrs::PYTHON_HANDLER_ID) {
                return Err(admission_error::GENERATED_PYTHON_TOOL_HANDLERS.to_string());
            }

            match node.op_type {
                AISOperationType::InvTool => validate_generated_inv_tool_node(node, runtime)?,
                AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason => {
                    validate_generated_llm_tool_exposure(node, runtime)?;
                }
                AISOperationType::SpawnAgent | AISOperationType::SpawnTeam => {
                    return Err(admission_error::generated_process_spawn_not_allowed(
                        node.op_type,
                    ));
                }
                AISOperationType::WorkflowSpawn => validate_generated_workflow_spawn_node(node)?,
                _ => {}
            }
        }
    }

    Ok(())
}

fn validate_generated_workflow_spawn_node(
    node: &apxm_core::types::execution::Node,
) -> Result<(), String> {
    if node.attributes.contains_key(graph_attrs::SESSION_ROOT) {
        return Err(
            "WORKFLOW_SPAWN session_root is server-controlled and may not be supplied by a generated workflow"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_generated_inv_tool_node(
    node: &apxm_core::types::execution::Node,
    runtime: &Runtime,
) -> Result<(), String> {
    let capability = node
        .attributes
        .get(graph_attrs::CAPABILITY)
        .and_then(|value| value.as_string())
        .ok_or_else(|| admission_error::INV_TOOL_MISSING_CAPABILITY.to_string())?;
    check_generated_capability_admission(capability, || inv_tool_static_args(node), runtime)
}

fn validate_generated_llm_tool_exposure(
    node: &apxm_core::types::execution::Node,
    runtime: &Runtime,
) -> Result<(), String> {
    let Some(requested_tools) = parse_string_array_attr(node, graph_attrs::TOOLS) else {
        return validate_generated_ask_group_or_all_tools(node, runtime);
    };
    if requested_tools.is_empty() {
        return validate_generated_ask_group_or_all_tools(node, runtime);
    }
    validate_read_only_tool_names(&requested_tools, runtime)
}

fn validate_generated_ask_group_or_all_tools(
    node: &apxm_core::types::execution::Node,
    runtime: &Runtime,
) -> Result<(), String> {
    let tools_enabled = node
        .attributes
        .get(graph_attrs::TOOLS_ENABLED)
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    if !tools_enabled {
        return Ok(());
    }

    if let Some(groups) = parse_string_array_attr(node, graph_attrs::TOOL_GROUPS)
        && !groups.is_empty()
    {
        let grouped_tools = runtime
            .capability_system()
            .list_capabilities_by_groups(&groups);
        for metadata in grouped_tools {
            if !metadata.read_only {
                return Err(admission_error::ask_group_capability_not_read_only(
                    &metadata.name,
                ));
            }
        }
        return Ok(());
    }

    for metadata in runtime.capability_system().list_capabilities() {
        if !metadata.read_only {
            return Err(admission_error::ask_all_tools_exposes_non_read_only(
                &metadata.name,
            ));
        }
    }
    Ok(())
}

fn validate_read_only_tool_names(tool_names: &[String], runtime: &Runtime) -> Result<(), String> {
    let capability_system = runtime.capability_system();
    for tool_name in tool_names {
        let Some(metadata) = capability_system.get_metadata(tool_name) else {
            return Err(admission_error::generated_capability_not_registered(
                tool_name,
            ));
        };
        if !metadata.read_only {
            return Err(admission_error::ask_named_tool_not_read_only(tool_name));
        }
    }
    Ok(())
}

fn parse_string_array_attr(
    node: &apxm_core::types::execution::Node,
    attr_name: &str,
) -> Option<Vec<String>> {
    node.attributes
        .get(attr_name)
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_string().map(ToString::to_string))
                .collect()
        })
}

fn inv_tool_static_args(
    node: &apxm_core::types::execution::Node,
) -> Result<HashMap<String, RuntimeValue>, String> {
    let mut args = HashMap::new();
    let Some(params_json) = node
        .attributes
        .get(graph_attrs::PARAMS_JSON)
        .and_then(|value| value.as_string())
    else {
        return Ok(args);
    };

    let parsed: JsonValue = serde_json::from_str(params_json)
        .map_err(|error| format!("invalid INV_TOOL params_json: {error}"))?;
    let Some(object) = parsed.as_object() else {
        return Err(admission_error::INV_TOOL_PARAMS_NOT_OBJECT.to_string());
    };

    for (key, value) in object {
        let value = RuntimeValue::try_from(value.clone())
            .map_err(|error| format!("invalid INV_TOOL arg '{key}': {error}"))?;
        args.insert(key.clone(), value);
    }

    Ok(args)
}

fn compile_air_to_artifact(air: &str) -> Result<Artifact, String> {
    let context =
        CompilerContext::new().map_err(|error| format!("compiler context init failed: {error}"))?;
    let pipeline = CompilerPipeline::with_opt_level(&context, OptimizationLevel::O1);
    let module = pipeline
        .compile(air)
        .map_err(|error| format!("compilation failed: {error}"))?;
    let artifact_bytes = module
        .generate_artifact_bytes()
        .map_err(|error| format!("artifact generation failed: {error}"))?;
    Artifact::from_bytes(&artifact_bytes)
        .map_err(|error| format!("artifact decode failed: {error}"))
}

fn compile_stats(artifact: &Artifact, compile_ms: u128, emission_ms: u128) -> JsonValue {
    let dag = artifact.entry_dag();
    json!({
        (tool_result::WORKFLOW_NAME): dag.and_then(|dag| dag.metadata.name.as_deref()).unwrap_or(mcp_defaults::ARTIFACT_WORKFLOW_NAME),
        (tool_result::NODE_COUNT): dag.map(|dag| dag.nodes.len()).unwrap_or(0),
        (tool_result::EDGE_COUNT): dag.map(|dag| dag.edges.len()).unwrap_or(0),
        (tool_result::COMPILE_MS): compile_ms,
        (tool_result::EMISSION_MS): emission_ms,
    })
}

fn execution_summary(
    execution: &apxm_runtime::RuntimeExecutionResult,
    execute_ms: u128,
) -> JsonValue {
    let mut result_preview = JsonValue::Null;
    for value in execution.results.values() {
        let json_value = value
            .to_json()
            .unwrap_or_else(|_| JsonValue::String(value.to_string()));
        if !json_value.is_null() {
            result_preview = json_value;
            break;
        }
    }
    json!({
        (tool_result::RESULT): result_preview,
        (tool_result::EXECUTE_MS): execute_ms,
        (tool_result::EXECUTED_NODES): execution.stats.executed_nodes,
        (tool_result::FAILED_NODES): execution.stats.failed_nodes,
        (tool_result::DURATION_MS): execution.stats.duration_ms,
        (tool_result::LLM_USAGE): {
            (tool_result::INPUT_TOKENS): execution.llm_metrics.total_input_tokens,
            (tool_result::OUTPUT_TOKENS): execution.llm_metrics.total_output_tokens,
            (tool_result::TOTAL_REQUESTS): execution.llm_metrics.total_requests,
        },
    })
}

fn extract_air_document(content: &str) -> Result<String, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err("model response contained no AIR".to_string());
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Err("model response used JSON; expected canonical APXM AIR text".to_string());
    }
    if trimmed.starts_with("```") {
        return Err("model response used a markdown fence; expected raw APXM AIR text".to_string());
    }

    let first_line = trimmed
        .lines()
        .map(str::trim_start)
        .find(|line| !line.is_empty());
    if !first_line.is_some_and(|line| line.starts_with("module")) {
        return Err("model response must start with a canonical AIR module".to_string());
    }
    Ok(format!("{}\n", trimmed.trim_end()))
}

fn summarize_execution_record(record: JsonValue, node_id: Option<u64>, full: bool) -> JsonValue {
    if full {
        return record;
    }
    let mut summary = json!({
        (tool_result::EXECUTION_ID): record.get(tool_result::EXECUTION_ID).cloned().unwrap_or(JsonValue::Null),
        (tool_result::STATUS): record.get(tool_result::STATUS).cloned().unwrap_or(JsonValue::Null),
        (tool_result::SKILL_ID): record.get(tool_result::SKILL_ID).cloned().unwrap_or(JsonValue::Null),
        (tool_result::SKILL_VERSION): record.get(tool_result::SKILL_VERSION).cloned().unwrap_or(JsonValue::Null),
        (tool_result::SESSION_ID): record.get(tool_result::SESSION_ID).cloned().unwrap_or(JsonValue::Null),
        (tool_result::SESSION_DIR): record.get(tool_result::SESSION_DIR).cloned().unwrap_or(JsonValue::Null),
        (tool_result::RESULT): record.get(tool_result::RESULT).cloned().unwrap_or(JsonValue::Null),
    });
    if let Some(node_id) = node_id {
        let outputs = record
            .get(tool_result::NODE_OUTPUTS)
            .and_then(JsonValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| {
                        item.get(tool_result::NODE_ID).and_then(JsonValue::as_u64) == Some(node_id)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let metrics = record
            .get(tool_result::NODE_METRICS)
            .and_then(JsonValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|item| {
                        item.get(tool_result::NODE_ID).and_then(JsonValue::as_u64) == Some(node_id)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        summary[tool_result::NODES] = json!({
            (tool_result::OUTPUTS): outputs,
            (tool_result::METRICS): metrics,
        });
    }
    summary
}

fn lookup_trace_files(
    trace_id: &str,
    full: bool,
    config: &ServerMcpConfig,
) -> Result<Vec<JsonValue>, String> {
    let paths =
        ApxmPaths::discover().map_err(|error| format!("failed to discover APXM paths: {error}"))?;
    let mut roots = paths.session_lookup_dirs();
    roots.retain(|root| root.is_dir());
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    let max_scan_files = config.trace_max_scan_files.max(1);
    let default_event_limit = config.default_trace_event_limit.max(1);
    for root in roots {
        let direct = root.join(trace_id);
        if direct.is_dir() {
            collect_trace_dir(&direct, full, default_event_limit, &mut matches);
        }
        scan_for_trace_files(
            &root,
            trace_id,
            full,
            default_event_limit,
            max_scan_files,
            &mut scanned,
            &mut matches,
        );
        if scanned >= max_scan_files {
            break;
        }
    }
    Ok(matches)
}

fn scan_for_trace_files(
    root: &Path,
    trace_id: &str,
    full: bool,
    default_event_limit: usize,
    max_scan_files: usize,
    scanned: &mut usize,
    matches: &mut Vec<JsonValue>,
) {
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        if *scanned >= max_scan_files {
            return;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            *scanned += 1;
            if path.is_dir() {
                if path.file_name().and_then(|name| name.to_str()) == Some(trace_id) {
                    collect_trace_dir(&path, full, default_event_limit, matches);
                }
                queue.push_back(path);
                continue;
            }
            let execution_file_name = format!("{trace_id}.json");
            if path.file_name().and_then(|name| name.to_str()) == Some(execution_file_name.as_str())
                && let Ok(text) = fs::read_to_string(&path)
                && let Ok(value) = serde_json::from_str::<JsonValue>(&text)
            {
                matches.push(json!({
                    (mcp_args::PATH): display_path(&path),
                    (tool_result::RECORD): if full { value } else { summarize_execution_record(value, None, false) },
                }));
            }
        }
    }
}

fn collect_trace_dir(
    dir: &Path,
    full: bool,
    default_event_limit: usize,
    matches: &mut Vec<JsonValue>,
) {
    let trace_path = dir.join(session_files::TRACE);
    let manifest_path = dir.join(session_files::MANIFEST);
    let trace = if trace_path.is_file() {
        read_lines_json(
            &trace_path,
            if full {
                usize::MAX
            } else {
                default_event_limit
            },
        )
    } else {
        Vec::new()
    };
    let manifest = if manifest_path.is_file() {
        fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|text| serde_json::from_str::<JsonValue>(&text).ok())
            .unwrap_or(JsonValue::Null)
    } else {
        JsonValue::Null
    };
    matches.push(json!({
        (mcp_args::PATH): display_path(dir),
        (tool_result::MANIFEST): manifest,
        (tool_result::TRACE): trace,
    }));
}

fn read_lines_json(path: &Path, limit: usize) -> Vec<JsonValue> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .take(limit)
        .filter_map(|line| serde_json::from_str::<JsonValue>(line).ok())
        .collect()
}

fn evidence_roots() -> Result<Vec<PathBuf>, String> {
    let paths =
        ApxmPaths::discover().map_err(|error| format!("failed to discover APXM paths: {error}"))?;
    let project = paths.project_dir();
    Ok([
        project
            .join(evidence_path::DOCS)
            .join(evidence_path::CLAIMS),
        project
            .join(evidence_path::DOCS)
            .join(evidence_path::EVALUATION),
        project
            .join(evidence_path::BENCHMARKS)
            .join(evidence_path::RESULTS),
        project.join(evidence_path::EVALUATION),
    ]
    .into_iter()
    .filter_map(|path| path.canonicalize().ok())
    .collect())
}

fn lookup_explicit_evidence_path(
    path: &str,
    roots: &[PathBuf],
    query: &str,
    config: &ServerMcpConfig,
) -> Result<Vec<JsonValue>, String> {
    let requested = PathBuf::from(path);
    let absolute = if requested.is_absolute() {
        requested
    } else {
        std::env::current_dir()
            .map_err(|error| format!("failed to resolve current directory: {error}"))?
            .join(requested)
    };
    let canonical = absolute.canonicalize().map_err(|error| {
        format!(
            "failed to resolve evidence path '{}': {error}",
            absolute.display()
        )
    })?;
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err("path is outside allowed APXM evidence roots".to_string());
    }
    Ok(read_evidence_candidate(&canonical, query, config)
        .into_iter()
        .collect())
}

fn scan_evidence_roots(
    roots: &[PathBuf],
    query: &str,
    limit: usize,
    config: &ServerMcpConfig,
) -> Result<Vec<JsonValue>, String> {
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    let max_scan_files = config.evidence_max_scan_files.max(1);
    for root in roots {
        let mut queue = VecDeque::from([root.clone()]);
        while let Some(dir) = queue.pop_front() {
            if scanned >= max_scan_files || matches.len() >= limit {
                return Ok(matches);
            }
            let entries = fs::read_dir(&dir).map_err(|error| {
                format!("failed to read evidence dir '{}': {error}", dir.display())
            })?;
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                scanned += 1;
                if path.is_dir() {
                    queue.push_back(path);
                    continue;
                }
                if let Some(item) = read_evidence_candidate(&path, query, config) {
                    matches.push(item);
                    if matches.len() >= limit {
                        return Ok(matches);
                    }
                }
            }
        }
    }
    Ok(matches)
}

fn read_evidence_candidate(
    path: &Path,
    query: &str,
    config: &ServerMcpConfig,
) -> Option<JsonValue> {
    if !is_evidence_file(path) {
        return None;
    }
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > config.evidence_max_file_bytes.max(1) {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let haystack = format!("{} {}", path.display(), text).to_ascii_lowercase();
    if !query.is_empty() && !haystack.contains(query) {
        return None;
    }
    Some(json!({
        (mcp_args::PATH): display_path(path),
        (tool_result::PREVIEW): preview_text(&text),
    }))
}

fn is_evidence_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| evidence_path::FILE_EXTENSIONS.contains(&ext))
}

fn preview_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("\n")
        .chars()
        .take(1_000)
        .collect()
}

fn matches_query(query: &str, key: &str, value: &RuntimeValue) -> bool {
    query.is_empty()
        || key.to_ascii_lowercase().contains(query)
        || value.to_string().to_ascii_lowercase().contains(query)
}

fn memory_space_name(space: MemorySpace) -> &'static str {
    match space {
        MemorySpace::Stm => memory_const::STM,
        MemorySpace::Ltm => memory_const::LTM,
        MemorySpace::Episodic => memory_const::EPISODIC,
    }
}

fn required_string_arg(args: &JsonValue, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| format!("missing required argument: {key}"))
}

fn optional_string_arg(args: &JsonValue, key: &str) -> Result<Option<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| format!("{key} must be a string"))
}

fn bounded_usize_arg(
    args: &JsonValue,
    key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> Result<usize, String> {
    let max = max.max(min);
    let default = default.clamp(min, max);
    let value = args
        .get(key)
        .and_then(JsonValue::as_u64)
        .map(|value| value as usize)
        .unwrap_or(default);
    if value < min || value > max {
        return Err(format!("{key} must be between {min} and {max}"));
    }
    Ok(value)
}

fn validate_trace_id(trace_id: &str) -> Result<(), String> {
    if trace_id.is_empty()
        || trace_id == "."
        || trace_id == ".."
        || !trace_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(admission_error::TRACE_ID_UNSAFE.to_string());
    }
    Ok(())
}

fn workflow_session_dir(session_id: &str) -> Result<String, String> {
    validate_trace_id(session_id)?;
    let base = ApxmPaths::discover()
        .map_err(|error| format!("failed to discover APXM paths: {error}"))?
        .sessions_dir()
        .map_err(|error| format!("failed to resolve sessions dir: {error}"))?;
    let session_dir = base
        .join(workflow_skill::SESSION_DIR_KIND)
        .join(workflow_skill::ID)
        .join(session_id);
    fs::create_dir_all(&session_dir)
        .map_err(|error| format!("failed to create workflow session dir: {error}"))?;
    Ok(session_dir.to_string_lossy().to_string())
}

fn write_generated_air(session_dir: &str, air: &str) -> Result<String, String> {
    let path = Path::new(session_dir).join("workflow.air");
    fs::write(&path, air).map_err(|error| {
        format!(
            "failed to write generated AIR '{}': {error}",
            path.display()
        )
    })?;
    Ok(path.to_string_lossy().to_string())
}

fn json_arg_to_string(value: &JsonValue) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn display_path(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

