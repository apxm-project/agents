use std::sync::Arc;

use apxm_core::events::EventEmitter;
use apxm_runtime::RuntimeExecutionResult;
use axum::Json;
use serde_json::Value as JsonValue;

use crate::execute::to_execute_response;
use crate::executions::{ExecutionRecordingEmitter, ExecutionStore};
use crate::helpers::mcp_tool_result;
use crate::mcp_protocol::{args as mcp_args, plan_skill};
use crate::mcp_tools::{self, PlanExecutionHandle, PlanExecutionRecorder, PlanExecutionStart};
use crate::skills::{SkillExecuteRequest, SkillLookupError, execute_skill_by_id};
use crate::state::AppState;

use super::schema::{
    MCP_TOOL_APXM_AAM_RECALL, MCP_TOOL_APXM_CAPABILITY_LIST, MCP_TOOL_APXM_EVIDENCE_LOOKUP,
    MCP_TOOL_APXM_PLAN_AS_GRAPH, MCP_TOOL_APXM_SKILL_CALL, MCP_TOOL_APXM_SKILL_GET,
    MCP_TOOL_APXM_SKILL_VALIDATE, MCP_TOOL_APXM_SKILLS_LIST, MCP_TOOL_APXM_TRACE_FETCH,
    MCP_TOOL_ARG_ARGS, MCP_TOOL_ARG_ID, MCP_TOOL_ARG_SESSION_ID,
};

pub(crate) async fn call_skill_tool(
    state: &AppState,
    id: &JsonValue,
    tool_name: &str,
    tool_args: &JsonValue,
) -> Option<Json<JsonValue>> {
    match tool_name {
        MCP_TOOL_APXM_SKILLS_LIST => {
            Some(mcp_json_tool_result(id.clone(), state.skill_library.scan()))
        }
        MCP_TOOL_APXM_SKILL_GET | MCP_TOOL_APXM_SKILL_VALIDATE => {
            let Some(skill_id) = tool_args.get(MCP_TOOL_ARG_ID).and_then(JsonValue::as_str) else {
                return Some(mcp_tool_result(
                    id.clone(),
                    format!("missing required argument: {MCP_TOOL_ARG_ID}"),
                    true,
                ));
            };
            Some(match state.skill_library.find(skill_id) {
                Ok(record) => mcp_json_tool_result(id.clone(), record),
                Err(error) => mcp_tool_result(id.clone(), skill_lookup_message(error), true),
            })
        }
        MCP_TOOL_APXM_SKILL_CALL => {
            let Some(skill_id) = tool_args.get(MCP_TOOL_ARG_ID).and_then(JsonValue::as_str) else {
                return Some(mcp_tool_result(
                    id.clone(),
                    format!("missing required argument: {MCP_TOOL_ARG_ID}"),
                    true,
                ));
            };
            let args = match parse_string_array_arg(tool_args, MCP_TOOL_ARG_ARGS) {
                Ok(args) => args,
                Err(message) => return Some(mcp_tool_result(id.clone(), message, true)),
            };
            let session_id = match parse_optional_string_arg(tool_args, MCP_TOOL_ARG_SESSION_ID) {
                Ok(session_id) => session_id,
                Err(message) => return Some(mcp_tool_result(id.clone(), message, true)),
            };
            let request = SkillExecuteRequest {
                args,
                session_id,
                sandbox_hint: None,
            };
            Some(match execute_skill_by_id(state, skill_id, request).await {
                Ok(response) => mcp_json_tool_result(id.clone(), response),
                Err(error) => mcp_tool_result(id.clone(), error.message, true),
            })
        }
        MCP_TOOL_APXM_PLAN_AS_GRAPH => Some(
            match mcp_tools::plan_as_graph_with_recorder(
                &state.runtime,
                tool_args.clone(),
                Some(Arc::new(HttpPlanExecutionRecorder::new(
                    state.execution_store.clone(),
                ))),
                &state.server_config.mcp,
            )
            .await
            {
                Ok(response) => mcp_json_tool_result(id.clone(), response),
                Err(error) => mcp_tool_result(id.clone(), error, true),
            },
        ),
        MCP_TOOL_APXM_TRACE_FETCH => {
            let Some(trace_id) = tool_args
                .get(mcp_args::TRACE_ID)
                .and_then(JsonValue::as_str)
            else {
                return Some(mcp_tool_result(
                    id.clone(),
                    format!("missing required argument: {}", mcp_args::TRACE_ID),
                    true,
                ));
            };
            let execution_record = state
                .execution_store
                .get(trace_id)
                .and_then(|record| serde_json::to_value(record).ok());
            Some(
                match mcp_tools::trace_fetch_with_config(
                    Some(&state.runtime),
                    execution_record,
                    tool_args.clone(),
                    &state.server_config.mcp,
                )
                .await
                {
                    Ok(response) => mcp_json_tool_result(id.clone(), response),
                    Err(error) => mcp_tool_result(id.clone(), error, true),
                },
            )
        }
        MCP_TOOL_APXM_AAM_RECALL => Some(
            match mcp_tools::aam_recall_with_config(
                &state.runtime,
                tool_args.clone(),
                &state.server_config.mcp,
            )
            .await
            {
                Ok(response) => mcp_json_tool_result(id.clone(), response),
                Err(error) => mcp_tool_result(id.clone(), error, true),
            },
        ),
        MCP_TOOL_APXM_EVIDENCE_LOOKUP => Some(
            match mcp_tools::evidence_lookup_with_config(
                tool_args.clone(),
                &state.server_config.mcp,
            ) {
                Ok(response) => mcp_json_tool_result(id.clone(), response),
                Err(error) => mcp_tool_result(id.clone(), error, true),
            },
        ),
        MCP_TOOL_APXM_CAPABILITY_LIST => Some(mcp_json_tool_result(
            id.clone(),
            mcp_tools::capability_list_with_config(
                &state.runtime,
                tool_args.clone(),
                &state.server_config.mcp,
            ),
        )),
        _ => None,
    }
}

struct HttpPlanExecutionRecorder {
    execution_store: ExecutionStore,
}

impl HttpPlanExecutionRecorder {
    fn new(execution_store: ExecutionStore) -> Self {
        Self { execution_store }
    }
}

impl PlanExecutionRecorder for HttpPlanExecutionRecorder {
    fn start(&self, start: PlanExecutionStart) -> PlanExecutionHandle {
        let record = self
            .execution_store
            .start_skill_execution_with_provenance_and_execution_id(
                start.execution_id,
                apxm_skill::SkillExecutionProvenance {
                    skill_id: plan_skill::ID.to_string(),
                    skill_version: plan_skill::VERSION.to_string(),
                    entry_flow: Some(plan_skill::ENTRY_FLOW.to_string()),
                    source_hash: None,
                    air_hash: Some(start.air_hash),
                    artifact_hash: Some(start.artifact_hash),
                    parent_execution_id: None,
                    parent_skill_id: None,
                    parent_skill_version: None,
                    scope_id: None,
                },
                &start.session_id,
                &start.session_dir,
            );
        PlanExecutionHandle {
            execution_id: record.execution_id.clone(),
            emitter: Arc::new(ExecutionRecordingEmitter::new(
                self.execution_store.clone(),
                record.execution_id,
            )) as Arc<dyn EventEmitter>,
        }
    }

    fn complete_success(
        &self,
        execution_id: &str,
        result: &RuntimeExecutionResult,
        session_dir: &str,
    ) {
        let response = to_execute_response(result.clone(), Some(session_dir.to_string()));
        self.execution_store
            .complete_success(execution_id, response);
    }

    fn complete_failure(&self, execution_id: &str, error: String) {
        self.execution_store.complete_failure(execution_id, error);
    }
}

fn parse_string_array_arg(args: &JsonValue, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let Some(values) = value.as_array() else {
        return Err(format!("{key} must be an array of strings"));
    };
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(ToString::to_string)
                .ok_or_else(|| format!("{key} must be an array of strings"))
        })
        .collect()
}

fn parse_optional_string_arg(args: &JsonValue, key: &str) -> Result<Option<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.to_string()))
        .ok_or_else(|| format!("{key} must be a string"))
}

fn mcp_json_tool_result<T: serde::Serialize>(id: JsonValue, value: T) -> Json<JsonValue> {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|error| {
        serde_json::json!({ "error": format!("failed to serialize MCP result: {error}") })
            .to_string()
    });
    mcp_tool_result(id, text, false)
}

fn skill_lookup_message(error: SkillLookupError) -> String {
    match error {
        SkillLookupError::NotFound(id) => format!("skill not found: {id}"),
        SkillLookupError::Ambiguous(id) => {
            format!("skill id has multiple versions; request {id}@<version>")
        }
    }
}
