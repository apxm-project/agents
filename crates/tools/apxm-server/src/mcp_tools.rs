use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use apxm_artifact::Artifact;
use apxm_backends::LLMRequest;
use apxm_compiler::{
    AirEdge, AirModule, AirNode, AirParam, Context as CompilerContext, Pipeline as CompilerPipeline,
};
use apxm_core::constants::graph::{attrs as graph_attrs, metadata as graph_meta};
use apxm_core::constants::{memory as memory_const, session::files as session_files};
use apxm_core::events::{EventEmitter, EventSource, SkillEventProvenance};
use apxm_core::paths::ApxmPaths;
use apxm_core::types::Value as RuntimeValue;
use apxm_core::types::{AISOperationType, DependencyType, OptimizationLevel};
use apxm_runtime::capability::CapabilitySandboxPreflight;
use apxm_runtime::{
    EmitterAdapter, ExecutionEventEmitter, MemorySpace, Runtime, RuntimeExecutionResult,
};
use serde::Deserialize;
use serde_json::{Map, Value as JsonValue, json};

use crate::mcp_protocol::{
    admission_error, args as mcp_args, defaults as mcp_defaults, evidence_path, plan_field,
    plan_skill, status as mcp_status, tool_result,
};

const PLAN_PROMPT: &str = include_str!("../skills/apxm-plan-as-graph/prompt.md");
const PLAN_SCHEMA: &str = include_str!("../skills/apxm-plan-as-graph/schema.json");
const PLAN_MAX_TOKENS: usize = 8192;
const PLAN_TEMPERATURE: f64 = 0.0;
// Three attempts cover the observed worst case where the first emission
// violates the schema one way (e.g. an unknown wrapper field) and the
// second-turn repair introduces a different violation (e.g. omits a
// required top-level key); two attempts can exhaust before the second
// class of error is corrected.
const PLAN_REPAIR_ATTEMPTS: usize = 3;
const PLAN_REPAIR_FEEDBACK_HEADING: &str = "Compiler or validation feedback to repair:";
const PLAN_REPAIR_FEEDBACK_PREFIX: &str =
    "The previous response could not be converted into executable APXM AIR";
const PLAN_REPAIRED_INVALID_PREFIX: &str = "repaired plan still invalid";
const PLAN_CAPABILITY_GUIDANCE_HEADING: &str = "Registered APXM capabilities:";
const PLAN_CAPABILITY_GUIDANCE_LIMIT: usize = 32;
const PLAN_CAPABILITY_NONE_GUIDANCE: &str =
    "none. Do not emit inv_tool nodes; use ask, think, wait_all, or yield nodes instead.";
const PLAN_CAPABILITY_TRUNCATED_GUIDANCE: &str = "- additional capabilities omitted";
const PLAN_INV_TOOL_CONTRACT: &str = "inv_tool nodes require a non-empty capability from the registered capability list and args as an object";
const PLAN_PROMPT_CONTRACT: &str = "agent, ask, think, and yield nodes require a non-empty prompt";
const PLAN_DEFAULT_NAME: &str = "generated_plan";
const PLAN_DEFAULT_ENTRY: &str = "plan";
const PLAN_DEFAULT_NODE_NAME_PREFIX: &str = "node";
const PLAN_NODE_ATTRIBUTE_ALIASES: &[&str] = &["attr", "attrs", "attributes"];
const PLAN_NODE_ALLOWED_FIELDS: &[&str] = &[
    plan_field::ID,
    plan_field::NAME,
    plan_field::OP,
    plan_field::PROMPT,
    plan_field::AGENT,
    plan_field::CAPABILITY,
    plan_field::ARGS,
    plan_field::DEPENDS_ON,
];
const PLAN_NODE_NESTED_ATTRIBUTE_FIELDS: &[&str] = &[
    plan_field::PROMPT,
    plan_field::AGENT,
    plan_field::CAPABILITY,
    plan_field::ARGS,
];
// Top-level (graph) fields tolerated by serde for PlanGraph. Any other key the
// LLM volunteered (e.g. `attr`, `description`, `metadata`) is stripped during
// normalization so a small schema slip does not waste a repair attempt. At
// graph scope every allowed field is also lift-eligible, so the alias-lift
// pass passes this same slice as both `allowed` and `nestable`.
const PLAN_GRAPH_ALLOWED_FIELDS: &[&str] = &[
    plan_field::NAME,
    plan_field::ENTRY,
    plan_field::PARAMETERS,
    plan_field::NODES,
];
const EVIDENCE_MAX_FILE_BYTES: u64 = 128 * 1024;
const EVIDENCE_MAX_SCAN_FILES: usize = 4_096;
const TRACE_MAX_SCAN_FILES: usize = 4_096;
const DEFAULT_TOP_K: usize = 10;
const DEFAULT_EVIDENCE_LIMIT: usize = 10;
const DEFAULT_TRACE_EVENT_LIMIT: usize = 64;

#[allow(dead_code)]
pub(crate) struct PlanExecutionStart {
    pub(crate) execution_id: String,
    pub(crate) session_id: String,
    pub(crate) session_dir: String,
    pub(crate) air_hash: String,
    pub(crate) artifact_hash: String,
}

pub(crate) struct PlanExecutionHandle {
    pub(crate) execution_id: String,
    pub(crate) emitter: Arc<dyn EventEmitter>,
}

pub(crate) trait PlanExecutionRecorder: Send + Sync {
    fn start(&self, start: PlanExecutionStart) -> PlanExecutionHandle;
    fn complete_success(
        &self,
        execution_id: &str,
        result: &RuntimeExecutionResult,
        session_dir: &str,
    );
    fn complete_failure(&self, execution_id: &str, error: String);
}

struct CompiledPlanGraph {
    plan: PlanGraph,
    normalized_plan: JsonValue,
    artifact: Artifact,
    artifact_hash: String,
    air_hash: String,
    compile_ms: u128,
}

enum PlanCandidateError {
    Emission(String),
    InvalidCandidate(String),
}

impl PlanCandidateError {
    fn into_message(self) -> String {
        match self {
            Self::Emission(message) | Self::InvalidCandidate(message) => message,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanGraph {
    name: String,
    entry: String,
    #[serde(default)]
    parameters: Vec<PlanParameter>,
    nodes: Vec<PlanNode>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanParameter {
    name: String,
    #[serde(rename = "type")]
    type_name: String,
    #[serde(default)]
    required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanNode {
    id: u64,
    name: String,
    op: PlanNodeOp,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    capability: Option<String>,
    #[serde(default)]
    args: Option<JsonValue>,
    #[serde(default)]
    depends_on: Vec<PlanDependency>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanDependency {
    node: u64,
    dependency: DependencyType,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PlanNodeOp {
    Agent,
    Ask,
    Think,
    InvTool,
    WaitAll,
    Yield,
}

impl PlanNodeOp {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Ask => "ask",
            Self::Think => "think",
            Self::InvTool => "inv_tool",
            Self::WaitAll => "wait_all",
            Self::Yield => "yield",
        }
    }

    const fn requires_prompt(self) -> bool {
        matches!(self, Self::Agent | Self::Ask | Self::Think | Self::Yield)
    }

    fn ais(self) -> AISOperationType {
        match self {
            Self::Agent => AISOperationType::Agent,
            Self::Ask => AISOperationType::Ask,
            Self::Think => AISOperationType::Think,
            Self::InvTool => AISOperationType::InvTool,
            Self::WaitAll => AISOperationType::WaitAll,
            Self::Yield => AISOperationType::Yield,
        }
    }
}

#[allow(dead_code)]
pub(crate) async fn plan_as_graph(runtime: &Runtime, args: JsonValue) -> Result<JsonValue, String> {
    plan_as_graph_with_recorder(runtime, args, None).await
}

pub(crate) async fn plan_as_graph_with_recorder(
    runtime: &Runtime,
    args: JsonValue,
    recorder: Option<Arc<dyn PlanExecutionRecorder>>,
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
        .unwrap_or_else(|| format!("{}-{}", plan_skill::TRACE_PREFIX, uuid::Uuid::new_v4()));
    validate_trace_id(&trace_id)?;

    let schema: JsonValue = serde_json::from_str(PLAN_SCHEMA)
        .map_err(|error| format!("bundled plan schema is invalid JSON: {error}"))?;
    let emission_start = Instant::now();
    let compiled = match emit_plan_candidate(
        runtime,
        &task,
        context.as_deref(),
        constraints.as_ref(),
        None,
        &schema,
        &trace_id,
    )
    .await
    {
        Ok(candidate) => match build_compiled_plan_graph(candidate) {
            Ok(compiled) => compiled,
            Err(error) => {
                repair_plan_candidate(
                    runtime,
                    &task,
                    context.as_deref(),
                    constraints.as_ref(),
                    &schema,
                    &trace_id,
                    &error,
                )
                .await?
            }
        },
        Err(PlanCandidateError::InvalidCandidate(error)) => {
            repair_plan_candidate(
                runtime,
                &task,
                context.as_deref(),
                constraints.as_ref(),
                &schema,
                &trace_id,
                &error,
            )
            .await?
        }
        Err(error @ PlanCandidateError::Emission(_)) => return Err(error.into_message()),
    };
    let emission_ms = emission_start.elapsed().as_millis();

    let mut output = json!({
        (tool_result::STATUS): mcp_status::COMPILED,
        (tool_result::TRACE_ID): trace_id,
        (tool_result::PLAN): compiled.normalized_plan,
        (tool_result::AIR_HASH): compiled.air_hash,
        (tool_result::ARTIFACT_HASH): compiled.artifact_hash,
        (tool_result::STATS): compile_stats(&compiled.artifact, compiled.compile_ms, emission_ms),
    });

    if should_execute {
        validate_generated_plan_admission(&compiled.artifact, runtime)?;
        let runtime_args = runtime_args_for_plan(&compiled.plan, &parameters);
        let session_id = output[tool_result::TRACE_ID]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let session_dir = plan_session_dir(&session_id)?;
        let execution_handle = recorder.as_ref().map(|recorder| {
            recorder.start(PlanExecutionStart {
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
                        skill_id: plan_skill::ID.to_string(),
                        skill_version: plan_skill::VERSION.to_string(),
                        parent_skill_id: None,
                        parent_execution_id: None,
                        flow_name: Some(plan_skill::ENTRY_FLOW.to_string()),
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
                    let message = format!("plan execution failed: {error}");
                    if let Some(recorder) = recorder.as_ref() {
                        recorder.complete_failure(&handle.execution_id, message.clone());
                    }
                    return Err(message);
                }
                return Err(format!("plan execution failed: {error}"));
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
            plan_skill::ID,
            RuntimeValue::try_from(output.clone()).unwrap_or(RuntimeValue::Null),
            None,
            None,
        )
        .await;

    Ok(output)
}

pub(crate) async fn trace_fetch(
    runtime: Option<&Runtime>,
    execution_record: Option<JsonValue>,
    args: JsonValue,
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
                        DEFAULT_TRACE_EVENT_LIMIT
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
        let files = lookup_trace_files(&trace_id, full)?;
        if !files.is_empty() {
            trace[tool_result::STATUS] = JsonValue::String(mcp_status::FOUND.to_string());
            trace[tool_result::FILES] = JsonValue::Array(files);
        }
    }

    Ok(trace)
}

pub(crate) async fn aam_recall(runtime: &Runtime, args: JsonValue) -> Result<JsonValue, String> {
    let query = args
        .get(mcp_args::QUERY)
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let top_k = bounded_usize_arg(&args, mcp_args::TOP_K, DEFAULT_TOP_K, 1, 100)?;

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

pub(crate) fn capability_list(runtime: &Runtime, args: JsonValue) -> JsonValue {
    let query = args
        .get(mcp_args::QUERY)
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut capabilities: Vec<JsonValue> = runtime
        .capability_system()
        .list_capabilities()
        .into_iter()
        .map(|capability| serde_json::to_value(capability).unwrap_or(JsonValue::Null))
        .collect();
    if !query.is_empty() {
        capabilities.retain(|value| {
            serde_json::to_string(value)
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains(&query)
        });
    }

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

pub(crate) fn evidence_lookup(args: JsonValue) -> Result<JsonValue, String> {
    let query = args
        .get(mcp_args::QUERY)
        .or_else(|| args.get(mcp_args::CLAIM_ID))
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let explicit_path = args.get(mcp_args::PATH).and_then(JsonValue::as_str);
    let limit = bounded_usize_arg(&args, mcp_args::LIMIT, DEFAULT_EVIDENCE_LIMIT, 1, 100)?;
    let roots = evidence_roots()?;
    let matches = if let Some(path) = explicit_path {
        lookup_explicit_evidence_path(path, &roots, &query)?
    } else {
        scan_evidence_roots(&roots, &query, limit)?
    };

    Ok(json!({
        (tool_result::QUERY): query,
        (tool_result::MATCHES): matches,
    }))
}

async fn repair_plan_candidate(
    runtime: &Runtime,
    task: &str,
    context: Option<&str>,
    constraints: Option<&JsonValue>,
    schema: &JsonValue,
    trace_id: &str,
    first_error: &str,
) -> Result<CompiledPlanGraph, String> {
    let mut last_error = first_error.to_string();
    for _ in 0..PLAN_REPAIR_ATTEMPTS {
        let feedback = format!("{PLAN_REPAIR_FEEDBACK_PREFIX}: {last_error}");
        let repaired = match emit_plan_candidate(
            runtime,
            task,
            context,
            constraints,
            Some(&feedback),
            schema,
            trace_id,
        )
        .await
        {
            Ok(candidate) => candidate,
            Err(PlanCandidateError::InvalidCandidate(error)) => {
                last_error = error;
                continue;
            }
            Err(error @ PlanCandidateError::Emission(_)) => {
                return Err(error.into_message());
            }
        };
        match build_compiled_plan_graph(repaired) {
            Ok(compiled) => return Ok(compiled),
            Err(error) => last_error = error,
        }
    }
    Err(format!("{PLAN_REPAIRED_INVALID_PREFIX}: {last_error}"))
}

fn build_compiled_plan_graph(value: JsonValue) -> Result<CompiledPlanGraph, String> {
    let plan_value = normalize_plan_value(decode_plan_graph(&value)?)?;
    let (plan, normalized_plan) = parse_plan_graph(plan_value)?;
    let module = lower_plan_to_air_module(&plan)?;
    let air = module
        .to_air()
        .map_err(|error| format!("AIR emission failed: {error}"))?;

    let compile_start = Instant::now();
    let artifact = compile_air_to_artifact(&air)?;
    let compile_ms = compile_start.elapsed().as_millis();
    let artifact_bytes = artifact
        .to_bytes()
        .map_err(|error| format!("artifact encode failed: {error}"))?;
    let artifact_hash = format!("blake3:{}", blake3::hash(&artifact_bytes).to_hex());
    let air_hash = format!("blake3:{}", blake3::hash(air.as_bytes()).to_hex());

    Ok(CompiledPlanGraph {
        plan,
        normalized_plan,
        artifact,
        artifact_hash,
        air_hash,
        compile_ms,
    })
}

fn normalize_plan_value(mut plan: JsonValue) -> Result<JsonValue, String> {
    normalize_plan_top_level_attribute_aliases(&mut plan);
    normalize_plan_top_level_defaults(&mut plan);
    normalize_plan_node_attribute_aliases(&mut plan);
    let node_id_aliases = normalize_plan_node_ids(&mut plan)?;
    normalize_plan_node_names(&mut plan);

    let node_refs = {
        let Some(nodes) = plan.get(plan_field::NODES).and_then(JsonValue::as_array) else {
            return Ok(plan);
        };
        let mut refs = node_id_aliases;
        for node in nodes {
            let Some(id) = node.get(plan_field::ID).and_then(JsonValue::as_u64) else {
                continue;
            };
            refs.insert(id.to_string(), id);
            if let Some(name) = node.get(plan_field::NAME).and_then(JsonValue::as_str) {
                refs.insert(name.to_string(), id);
                refs.insert(sanitize_input_name(name), id);
            }
        }
        refs
    };

    let Some(nodes) = plan
        .get_mut(plan_field::NODES)
        .and_then(JsonValue::as_array_mut)
    else {
        return Ok(plan);
    };
    for node in nodes {
        let Some(depends_on) = node
            .get_mut(plan_field::DEPENDS_ON)
            .and_then(JsonValue::as_array_mut)
        else {
            continue;
        };
        for dependency in depends_on {
            match dependency {
                JsonValue::Object(object) => {
                    let Some(node_ref) = object.get_mut(plan_field::NODE) else {
                        continue;
                    };
                    if let Some(name_ref) = node_ref.as_str() {
                        let resolved = resolve_plan_node_ref(name_ref, &node_refs)?;
                        *node_ref = JsonValue::Number(serde_json::Number::from(resolved));
                    }
                }
                JsonValue::String(_) | JsonValue::Number(_) => {
                    let resolved = resolve_plan_dependency_value(dependency, &node_refs)?;
                    *dependency = json!({
                        (plan_field::NODE): resolved,
                        (plan_field::DEPENDENCY): DependencyType::Data,
                    });
                }
                _ => {}
            }
        }
    }

    Ok(plan)
}

/// Strip non-schema top-level fields and lift nested `attr`/`attrs`/
/// `attributes` wrappers. Mirrors `normalize_plan_node_attribute_aliases`
/// at the graph surface so both layers tolerate the same shape drift.
fn normalize_plan_top_level_attribute_aliases(plan: &mut JsonValue) {
    let Some(object) = plan.as_object_mut() else {
        return;
    };
    lift_aliases_into_object(object, PLAN_GRAPH_ALLOWED_FIELDS, PLAN_GRAPH_ALLOWED_FIELDS);
}

/// Move fields out of an `attr`/`attrs`/`attributes` wrapper into the parent
/// object, then drop any sibling key that is not in `allowed_fields`. Existing
/// canonical fields take priority over wrapper-supplied values.
fn lift_aliases_into_object(
    object: &mut Map<String, JsonValue>,
    nestable_fields: &[&str],
    allowed_fields: &[&str],
) {
    for alias in PLAN_NODE_ATTRIBUTE_ALIASES {
        let Some(alias_value) = object.remove(*alias) else {
            continue;
        };
        let Some(alias_object) = alias_value.as_object() else {
            continue;
        };
        for field in nestable_fields {
            if !object.contains_key(*field)
                && let Some(value) = alias_object.get(*field)
            {
                object.insert((*field).to_string(), value.clone());
            }
        }
    }
    object.retain(|field, _| allowed_fields.contains(&field.as_str()));
}

fn normalize_plan_top_level_defaults(plan: &mut JsonValue) {
    let Some(object) = plan.as_object_mut() else {
        return;
    };
    if object
        .get(plan_field::NAME)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        object.insert(
            plan_field::NAME.to_string(),
            JsonValue::String(PLAN_DEFAULT_NAME.to_string()),
        );
    }
    if object
        .get(plan_field::ENTRY)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        object.insert(
            plan_field::ENTRY.to_string(),
            JsonValue::String(PLAN_DEFAULT_ENTRY.to_string()),
        );
    }
}

fn normalize_plan_node_attribute_aliases(plan: &mut JsonValue) {
    let Some(nodes) = plan
        .get_mut(plan_field::NODES)
        .and_then(JsonValue::as_array_mut)
    else {
        return;
    };
    for node in nodes {
        if let Some(object) = node.as_object_mut() {
            lift_aliases_into_object(
                object,
                PLAN_NODE_NESTED_ATTRIBUTE_FIELDS,
                PLAN_NODE_ALLOWED_FIELDS,
            );
        }
    }
}

fn normalize_plan_node_names(plan: &mut JsonValue) {
    let Some(nodes) = plan
        .get_mut(plan_field::NODES)
        .and_then(JsonValue::as_array_mut)
    else {
        return;
    };
    for (index, node) in nodes.iter_mut().enumerate() {
        let Some(object) = node.as_object_mut() else {
            continue;
        };
        if object
            .get(plan_field::NAME)
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
        {
            continue;
        }
        let id = object
            .get(plan_field::ID)
            .and_then(JsonValue::as_u64)
            .unwrap_or((index + 1) as u64);
        object.insert(
            plan_field::NAME.to_string(),
            JsonValue::String(format!("{PLAN_DEFAULT_NODE_NAME_PREFIX}_{id}")),
        );
    }
}

fn normalize_plan_node_ids(plan: &mut JsonValue) -> Result<HashMap<String, u64>, String> {
    let Some(nodes) = plan
        .get_mut(plan_field::NODES)
        .and_then(JsonValue::as_array_mut)
    else {
        return Ok(HashMap::new());
    };

    let mut used_ids = HashSet::new();
    for node in nodes.iter() {
        let Some(id_value) = node.get(plan_field::ID) else {
            continue;
        };
        if let Some(id) = id_value.as_u64().or_else(|| {
            id_value
                .as_str()
                .and_then(|value| value.trim().parse().ok())
        }) && id > 0
        {
            used_ids.insert(id);
        }
    }

    let mut aliases = HashMap::new();
    let mut next_id = 1u64;
    for node in nodes {
        let Some(id_value) = node.get_mut(plan_field::ID) else {
            continue;
        };
        if let Some(id_ref) = id_value.as_str() {
            let id = match parse_plan_node_id_ref(id_ref) {
                Ok(id) => id,
                Err(_) => {
                    while used_ids.contains(&next_id) {
                        next_id += 1;
                    }
                    let assigned = next_id;
                    used_ids.insert(assigned);
                    aliases.insert(id_ref.trim().to_string(), assigned);
                    assigned
                }
            };
            *id_value = JsonValue::Number(serde_json::Number::from(id));
        }
    }
    Ok(aliases)
}

fn resolve_plan_dependency_value(
    value: &JsonValue,
    node_refs: &HashMap<String, u64>,
) -> Result<u64, String> {
    if let Some(id) = value.as_u64() {
        if id == 0 {
            return Err("node id reference must be >= 1".to_string());
        }
        return Ok(id);
    }
    if let Some(reference) = value.as_str() {
        return resolve_plan_node_ref(reference, node_refs);
    }
    Err(format!(
        "{} entries must be dependency objects or node references",
        plan_field::DEPENDS_ON
    ))
}

fn resolve_plan_node_ref(reference: &str, node_refs: &HashMap<String, u64>) -> Result<u64, String> {
    let trimmed = reference.trim();
    if let Ok(id) = parse_plan_node_id_ref(trimmed) {
        return Ok(id);
    }
    node_refs
        .get(trimmed)
        .or_else(|| node_refs.get(&sanitize_input_name(trimmed)))
        .copied()
        .ok_or_else(|| format!("dependency references unknown node '{trimmed}'"))
}

fn parse_plan_node_id_ref(reference: &str) -> Result<u64, String> {
    let trimmed = reference.trim();
    let id = trimmed
        .parse::<u64>()
        .map_err(|_| format!("node id reference '{trimmed}' is not a positive integer"))?;
    if id == 0 {
        return Err("node id reference must be >= 1".to_string());
    }
    Ok(id)
}

fn emit_prompt(
    task: &str,
    context: Option<&str>,
    constraints: Option<&JsonValue>,
    feedback: Option<&str>,
    capability_guidance: &str,
) -> String {
    let mut prompt = String::from(PLAN_PROMPT);
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
        prompt.push_str(PLAN_REPAIR_FEEDBACK_HEADING);
        prompt.push('\n');
        prompt.push_str(feedback);
    }
    prompt
}

fn plan_capability_guidance(runtime: &Runtime) -> String {
    let mut capabilities = runtime.capability_system().list_capabilities();
    capabilities.sort_by(|left, right| left.name.cmp(&right.name));

    let mut guidance = String::from(PLAN_CAPABILITY_GUIDANCE_HEADING);
    guidance.push('\n');
    if capabilities.is_empty() {
        guidance.push_str(PLAN_CAPABILITY_NONE_GUIDANCE);
        return guidance;
    }

    for capability in capabilities.iter().take(PLAN_CAPABILITY_GUIDANCE_LIMIT) {
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
    if capabilities.len() > PLAN_CAPABILITY_GUIDANCE_LIMIT {
        guidance.push_str(PLAN_CAPABILITY_TRUNCATED_GUIDANCE);
    }
    guidance
}

async fn emit_plan_candidate(
    runtime: &Runtime,
    task: &str,
    context: Option<&str>,
    constraints: Option<&JsonValue>,
    feedback: Option<&str>,
    schema: &JsonValue,
    trace_id: &str,
) -> Result<JsonValue, PlanCandidateError> {
    let router = runtime
        .model_router()
        .ok_or_else(|| PlanCandidateError::Emission("model router unavailable; initialize APXM server runtime with ModelRouter before calling apxm_plan_as_graph".to_string()))?;
    let capability_guidance = plan_capability_guidance(runtime);
    let request = LLMRequest::new(emit_prompt(
        task,
        context,
        constraints,
        feedback,
        &capability_guidance,
    ))
    .with_output_schema(schema.clone())
    .with_max_tokens(PLAN_MAX_TOKENS)
    .with_temperature(PLAN_TEMPERATURE)
    .with_operation_type(AISOperationType::Plan)
    .with_metadata_value(
        plan_skill::REQUEST_CAPABILITY_KEY,
        json!(plan_skill::EMISSION_CAPABILITY),
    )
    .with_trace_id(trace_id.to_string());
    let response = router.generate(request).await.map_err(|error| {
        PlanCandidateError::Emission(format!("model-router plan emission failed: {error}"))
    })?;
    extract_json_document(&response.content).map_err(PlanCandidateError::InvalidCandidate)
}

fn decode_plan_graph(value: &JsonValue) -> Result<JsonValue, String> {
    if value.get(plan_field::NODES).is_some() {
        return Ok(value.clone());
    }
    if let Some(graph) = value.get(plan_field::GRAPH) {
        return Ok(graph.clone());
    }
    // gpt-oss-120b occasionally wraps the entire plan inside one of the
    // PLAN_NODE_ATTRIBUTE_ALIASES keys (`attr`/`attrs`/`attributes`). If that
    // wrapper exposes `nodes` or `graph`, treat it as the real plan envelope
    // rather than burning a repair attempt rejecting an unknown top-level key.
    for alias in PLAN_NODE_ATTRIBUTE_ALIASES {
        let Some(wrapped) = value.get(*alias) else {
            continue;
        };
        if wrapped.get(plan_field::NODES).is_some() {
            return Ok(wrapped.clone());
        }
        if let Some(graph) = wrapped.get(plan_field::GRAPH) {
            return Ok(graph.clone());
        }
    }
    Err(format!(
        "expected top-level '{}' or '{}'",
        plan_field::NODES,
        plan_field::GRAPH
    ))
}

fn parse_plan_graph(value: JsonValue) -> Result<(PlanGraph, JsonValue), String> {
    let plan: PlanGraph = serde_json::from_value(value.clone())
        .map_err(|error| format!("plan JSON does not satisfy schema: {error}"))?;
    validate_plan_graph(&plan)?;
    Ok((plan, value))
}

fn validate_plan_graph(plan: &PlanGraph) -> Result<(), String> {
    if plan.name.trim().is_empty() {
        return Err(format!("{} must not be empty", plan_field::NAME));
    }
    if plan.entry.trim().is_empty() {
        return Err(format!("{} must not be empty", plan_field::ENTRY));
    }
    if plan.nodes.is_empty() {
        return Err(format!(
            "{} must contain at least one node",
            plan_field::NODES
        ));
    }
    let mut ids = HashSet::new();
    for node in &plan.nodes {
        if node.id == 0 {
            return Err(format!("{} must be >= 1", plan_field::ID));
        }
        if !ids.insert(node.id) {
            return Err(format!("duplicate node id {}", node.id));
        }
        if node.name.trim().is_empty() {
            return Err(format!("node {} has empty {}", node.id, plan_field::NAME));
        }
        match node.op {
            PlanNodeOp::InvTool => {
                if node
                    .capability
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
                {
                    return Err(format!(
                        "node '{}' (id={}, op={}) is missing required attribute '{}'; {}",
                        node.name,
                        node.id,
                        node.op.as_str(),
                        plan_field::CAPABILITY,
                        PLAN_INV_TOOL_CONTRACT
                    ));
                }
                if !node
                    .args
                    .as_ref()
                    .is_some_and(|value| value.as_object().is_some())
                {
                    return Err(format!(
                        "node '{}' (id={}, op={}) is missing required object attribute '{}'; {}",
                        node.name,
                        node.id,
                        node.op.as_str(),
                        plan_field::ARGS,
                        PLAN_INV_TOOL_CONTRACT
                    ));
                }
            }
            op if op.requires_prompt()
                && node
                    .prompt
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none() =>
            {
                return Err(format!(
                    "node '{}' (id={}, op={}) is missing required attribute '{}'; {}",
                    node.name,
                    node.id,
                    node.op.as_str(),
                    plan_field::PROMPT,
                    PLAN_PROMPT_CONTRACT
                ));
            }
            _ => {}
        }
    }
    for node in &plan.nodes {
        for dep in &node.depends_on {
            if !ids.contains(&dep.node) {
                return Err(format!(
                    "node {} depends on unknown node {}",
                    node.id, dep.node
                ));
            }
        }
    }
    Ok(())
}

fn lower_plan_to_air_module(plan: &PlanGraph) -> Result<AirModule, String> {
    let source_names = plan
        .nodes
        .iter()
        .map(|node| (node.id, sanitize_input_name(&node.name)))
        .collect::<HashMap<_, _>>();
    let nodes = plan
        .nodes
        .iter()
        .map(|node| lower_plan_node(node, &source_names))
        .collect::<Result<Vec<_>, _>>()?;
    let edges = plan
        .nodes
        .iter()
        .flat_map(|node| {
            node.depends_on.iter().map(|dep| AirEdge {
                from: dep.node,
                to: node.id,
                dependency: dep.dependency.clone(),
            })
        })
        .collect::<Vec<_>>();
    let parameters = plan
        .parameters
        .iter()
        .map(|param| AirParam {
            name: param.name.clone(),
            type_name: param.type_name.clone(),
        })
        .collect::<Vec<_>>();
    let module = AirModule {
        name: plan.name.clone(),
        nodes,
        edges,
        parameters,
        metadata: HashMap::from([(graph_meta::IS_ENTRY.to_string(), RuntimeValue::Bool(true))]),
    };
    module
        .validate()
        .map_err(|error| format!("AIR graph validation failed: {error}"))?;
    Ok(module)
}

fn lower_plan_node(
    node: &PlanNode,
    source_names: &HashMap<u64, String>,
) -> Result<AirNode, String> {
    let mut attributes = HashMap::new();
    if let Some(prompt) = &node.prompt {
        attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            RuntimeValue::String(prompt.clone()),
        );
    }
    if let Some(agent) = &node.agent {
        attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            RuntimeValue::String(agent.clone()),
        );
    }
    if let Some(capability) = &node.capability {
        attributes.insert(
            graph_attrs::CAPABILITY.to_string(),
            RuntimeValue::String(capability.clone()),
        );
    }
    if let Some(args) = &node.args {
        attributes.insert(
            graph_attrs::PARAMS_JSON.to_string(),
            RuntimeValue::String(args.to_string()),
        );
    }

    let data_input_names = node
        .depends_on
        .iter()
        .filter(|dep| matches!(dep.dependency, DependencyType::Data))
        .filter_map(|dep| source_names.get(&dep.node).cloned())
        .map(RuntimeValue::String)
        .collect::<Vec<_>>();
    if !data_input_names.is_empty() {
        attributes.insert(
            graph_attrs::INPUT_NAMES.to_string(),
            RuntimeValue::Array(data_input_names),
        );
    }

    Ok(AirNode {
        id: node.id,
        name: node.name.clone(),
        op: node.op.ais(),
        attributes,
    })
}

fn runtime_args_for_plan(
    plan: &PlanGraph,
    parameters: &serde_json::Map<String, JsonValue>,
) -> Vec<String> {
    plan.parameters
        .iter()
        .map(|parameter| {
            parameters
                .get(&parameter.name)
                .map(json_arg_to_string)
                .or_else(|| {
                    if parameter.required {
                        Some(String::new())
                    } else {
                        None
                    }
                })
                .unwrap_or_default()
        })
        .collect()
}

fn validate_generated_plan_admission(artifact: &Artifact, runtime: &Runtime) -> Result<(), String> {
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
                _ => {}
            }
        }
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
    let capability_system = runtime.capability_system();

    if !capability_system.has_capability(capability) {
        return Err(admission_error::generated_capability_not_registered(
            capability,
        ));
    }

    if capability_system.is_read_only(capability) {
        return Ok(());
    }

    let args = inv_tool_static_args(node)?;
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
        (tool_result::GRAPH_NAME): dag.and_then(|dag| dag.metadata.name.as_deref()).unwrap_or(mcp_defaults::ARTIFACT_GRAPH_NAME),
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

fn extract_json_document(content: &str) -> Result<JsonValue, String> {
    let trimmed = content.trim();
    if let Ok(value) = serde_json::from_str::<JsonValue>(trimmed) {
        return Ok(value);
    }

    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    if let Ok(value) = serde_json::from_str::<JsonValue>(unfenced) {
        return Ok(value);
    }

    let Some(start) = unfenced.find('{') else {
        return Err("model response contained no JSON object".to_string());
    };
    let Some(end) = unfenced.rfind('}') else {
        return Err("model response contained an unterminated JSON object".to_string());
    };
    serde_json::from_str(&unfenced[start..=end])
        .map_err(|error| format!("failed to parse JSON object from model response: {error}"))
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

fn lookup_trace_files(trace_id: &str, full: bool) -> Result<Vec<JsonValue>, String> {
    let paths =
        ApxmPaths::discover().map_err(|error| format!("failed to discover APXM paths: {error}"))?;
    let mut roots = paths.session_lookup_dirs();
    roots.retain(|root| root.is_dir());
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    for root in roots {
        let direct = root.join(trace_id);
        if direct.is_dir() {
            collect_trace_dir(&direct, full, &mut matches);
        }
        scan_for_trace_files(&root, trace_id, full, &mut scanned, &mut matches);
        if scanned >= TRACE_MAX_SCAN_FILES {
            break;
        }
    }
    Ok(matches)
}

fn scan_for_trace_files(
    root: &Path,
    trace_id: &str,
    full: bool,
    scanned: &mut usize,
    matches: &mut Vec<JsonValue>,
) {
    let mut queue = VecDeque::from([root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        if *scanned >= TRACE_MAX_SCAN_FILES {
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
                    collect_trace_dir(&path, full, matches);
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

fn collect_trace_dir(dir: &Path, full: bool, matches: &mut Vec<JsonValue>) {
    let trace_path = dir.join(session_files::TRACE);
    let manifest_path = dir.join(session_files::MANIFEST);
    let trace = if trace_path.is_file() {
        read_lines_json(
            &trace_path,
            if full {
                usize::MAX
            } else {
                DEFAULT_TRACE_EVENT_LIMIT
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
    Ok(read_evidence_candidate(&canonical, query)
        .into_iter()
        .collect())
}

fn scan_evidence_roots(
    roots: &[PathBuf],
    query: &str,
    limit: usize,
) -> Result<Vec<JsonValue>, String> {
    let mut matches = Vec::new();
    let mut scanned = 0usize;
    for root in roots {
        let mut queue = VecDeque::from([root.clone()]);
        while let Some(dir) = queue.pop_front() {
            if scanned >= EVIDENCE_MAX_SCAN_FILES || matches.len() >= limit {
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
                if let Some(item) = read_evidence_candidate(&path, query) {
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

fn read_evidence_candidate(path: &Path, query: &str) -> Option<JsonValue> {
    if !is_evidence_file(path) {
        return None;
    }
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > EVIDENCE_MAX_FILE_BYTES {
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

fn plan_session_dir(session_id: &str) -> Result<String, String> {
    validate_trace_id(session_id)?;
    let base = ApxmPaths::discover()
        .map_err(|error| format!("failed to discover APXM paths: {error}"))?
        .sessions_dir()
        .map_err(|error| format!("failed to resolve sessions dir: {error}"))?;
    let session_dir = base
        .join(plan_skill::SESSION_DIR_KIND)
        .join(plan_skill::ID)
        .join(session_id);
    fs::create_dir_all(&session_dir)
        .map_err(|error| format!("failed to create plan session dir: {error}"))?;
    Ok(session_dir.to_string_lossy().to_string())
}

fn json_arg_to_string(value: &JsonValue) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| value.to_string())
}

fn sanitize_input_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_separator = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_separator = false;
        } else if !last_separator {
            out.push('_');
            last_separator = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        mcp_defaults::INPUT_NAME.to_string()
    } else {
        trimmed.to_string()
    }
}

fn display_path(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

