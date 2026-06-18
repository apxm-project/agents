//! APXM MCP Server -- exposes the APXM compiler as MCP tools over stdio.
//!
//! Implements JSON-RPC 2.0 over stdin/stdout per the Model Context Protocol
//! (2024-11-05) so that external agents (Claude Code, Codex, etc.) can
//! validate and compile APXM AIR. Raw AIR execution is hidden and disabled by
//! default, and is exposed only when `APXM_MCP_ENABLE_RAW_EXECUTE` is set.
//!
//! # Tools
//!
//! - `validate`      -- validate AIR against the AIS contract
//! - `compile`       -- compile AIR to an optimized artifact
//! - `execute`       -- compile + execute AIR only when explicitly enabled
//! - `get_contract`  -- return the full AIS contract (ops, attrs, types)
//! - `resources/list`     -- enumerate bundled/user APXM skill resources
//! - `resources/read`     -- read `skill://...` resources
//! - `prompt_as_workflow` -- emit, validate, compile, and optionally execute a workflow
//! - query tools          -- fetch traces, AAM memory, evidence, and capabilities
//!
//! # Running
//!
//! ```bash
//! apxm-mcp-server          # reads JSON-RPC lines from stdin, writes to stdout
//! ```

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::time::Instant;

use apxm_artifact::Artifact;
use apxm_compiler::{Context as CompilerContext, Pipeline as CompilerPipeline};
use apxm_core::constants::jsonrpc;
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::{AIS_OPERATIONS, OptimizationLevel};
use serde_json::{Value, json};

#[path = "../config_layers.rs"]
mod config_layers;
#[path = "../mcp_protocol.rs"]
mod mcp_protocol;
#[path = "../mcp_tools.rs"]
mod mcp_tools;
#[allow(dead_code)]
#[path = "../remote_runner.rs"]
mod remote_runner;
#[path = "../runtime_setup.rs"]
mod runtime_setup;
#[path = "../skill_resources.rs"]
mod skill_resources;
#[path = "../workflow_source.rs"]
mod workflow_source;

use mcp_protocol::{
    ContentKind, McpMethod, OperationCategoryWire, StdioTool, Tier3Tool, args as mcp_args,
    contract_value, fields as mcp_fields, operation_latency_estimate_ms, schema_type, server_name,
    tool_description, tool_result,
};

const MCP_PROTOCOL_VERSION: &str = apxm_core::constants::protocols::MCP_VERSION;
const SERVER_NAME: &str = server_name::STDIO;
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const RAW_EXECUTE_ENV: &str = "APXM_MCP_ENABLE_RAW_EXECUTE";
const RAW_EXECUTE_DISABLED_MESSAGE: &str = "execute is disabled by default in the stdio MCP server. Use the HTTP MCP skill_call tool for server-owned skills, or set APXM_MCP_ENABLE_RAW_EXECUTE=1 for explicit developer/debug raw AIR execution.";
// JSON-RPC error codes
const PARSE_ERROR: i64 = apxm_core::constants::jsonrpc::error_codes::PARSE_ERROR;
const METHOD_NOT_FOUND: i64 = apxm_core::constants::jsonrpc::error_codes::METHOD_NOT_FOUND;
const INVALID_PARAMS: i64 = apxm_core::constants::jsonrpc::error_codes::INVALID_PARAMS;
const _INTERNAL_ERROR: i64 = apxm_core::constants::jsonrpc::error_codes::INTERNAL_ERROR;

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let resp = json!({
                    jsonrpc::JSONRPC: jsonrpc::VERSION,
                    jsonrpc::ID: null,
                    jsonrpc::ERROR: { "code": PARSE_ERROR, "message": format!("invalid JSON: {e}") }
                });
                write_response(&mut stdout, &resp);
                continue;
            }
        };

        let response = handle_request(request);
        if response != Value::Null {
            write_response(&mut stdout, &response);
        }
    }
}

fn write_response(stdout: &mut io::Stdout, response: &Value) {
    let line = serde_json::to_string(response).expect("serialize response");
    let _ = writeln!(stdout, "{line}");
    let _ = stdout.flush();
}

fn handle_request(request: Value) -> Value {
    let id = request.get(jsonrpc::ID).cloned().unwrap_or(Value::Null);
    let method = request
        .get(jsonrpc::METHOD)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = request.get(jsonrpc::PARAMS).cloned().unwrap_or(Value::Null);

    // Notifications (no "id" field) -- handle silently
    if request.get(jsonrpc::ID).is_none() {
        // notifications/initialized, notifications/cancelled, etc.
        return Value::Null;
    }

    let result = match McpMethod::from_str(method) {
        Some(McpMethod::Initialize) => handle_initialize(),
        Some(McpMethod::ToolsList) => handle_tools_list(raw_execute_enabled()),
        Some(McpMethod::ToolsCall) => handle_tools_call(params, raw_execute_enabled()),
        Some(McpMethod::ResourcesList) => handle_resources_list(),
        Some(McpMethod::ResourcesRead) => handle_resources_read(params),
        Some(McpMethod::Ping) => Ok(json!({})),
        None if method.is_empty() => Err(rpc_error(PARSE_ERROR, "missing method")),
        None => Err(rpc_error(
            METHOD_NOT_FOUND,
            format!("unknown method: {method}"),
        )),
    };

    match result {
        Ok(result) => json!({
            jsonrpc::JSONRPC: jsonrpc::VERSION,
            jsonrpc::ID: id,
            jsonrpc::RESULT: result,
        }),
        Err(error) => json!({
            jsonrpc::JSONRPC: jsonrpc::VERSION,
            jsonrpc::ID: id,
            jsonrpc::ERROR: error,
        }),
    }
}

fn handle_initialize() -> Result<Value, Value> {
    Ok(json!({
        (mcp_fields::PROTOCOL_VERSION): MCP_PROTOCOL_VERSION,
        (mcp_fields::SERVER_INFO): {
            (mcp_fields::NAME): SERVER_NAME,
            (mcp_fields::VERSION): SERVER_VERSION,
        },
        (mcp_fields::CAPABILITIES): {
            (mcp_fields::TOOLS): { (mcp_fields::LIST_CHANGED): false },
            (mcp_fields::RESOURCES): {
                (mcp_fields::LIST_CHANGED): false,
                (mcp_fields::SUBSCRIBE): false,
            }
        }
    }))
}

fn raw_execute_enabled() -> bool {
    std::env::var(RAW_EXECUTE_ENV)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
}

fn handle_tools_list(raw_execute_enabled: bool) -> Result<Value, Value> {
    let mut tools = vec![
        json!({
            (mcp_fields::NAME): StdioTool::Validate.as_str(),
            (mcp_fields::DESCRIPTION): "Validate canonical APXM AIR against the AIS contract.",
            (mcp_fields::INPUT_SCHEMA): {
                (mcp_fields::TYPE): schema_type::OBJECT,
                (mcp_fields::PROPERTIES): {
                    (mcp_args::AIR): {
                        (mcp_fields::TYPE): schema_type::STRING,
                        (mcp_fields::DESCRIPTION): "Canonical APXM AIR text"
                    },
                    (mcp_args::PATH): {
                        (mcp_fields::TYPE): schema_type::STRING,
                        (mcp_fields::DESCRIPTION): "Path to a .air file or Python frontend file that emits AIR"
                    }
                },
                (mcp_fields::REQUIRED): []
            }
        }),
        json!({
            (mcp_fields::NAME): StdioTool::Compile.as_str(),
            (mcp_fields::DESCRIPTION): "Compile canonical APXM AIR to an optimized APXM artifact (.apxmobj). Returns the artifact path and compilation stats.",
            (mcp_fields::INPUT_SCHEMA): {
                (mcp_fields::TYPE): schema_type::OBJECT,
                (mcp_fields::PROPERTIES): {
                    (mcp_args::AIR): {
                        (mcp_fields::TYPE): schema_type::STRING,
                        (mcp_fields::DESCRIPTION): "Canonical APXM AIR text"
                    },
                    (mcp_args::PATH): {
                        (mcp_fields::TYPE): schema_type::STRING,
                        (mcp_fields::DESCRIPTION): "Path to a .air file or Python frontend file that emits AIR"
                    },
                    (mcp_args::OPT_LEVEL): {
                        (mcp_fields::TYPE): schema_type::INTEGER,
                        (mcp_fields::DESCRIPTION): "Optimization level (0-3). 0=none, 1=basic, 2=standard, 3=aggressive. Default: 2",
                        (mcp_fields::MINIMUM): 0,
                        (mcp_fields::MAXIMUM): 3
                    }
                },
                (mcp_fields::REQUIRED): []
            }
        }),
        json!({
            (mcp_fields::NAME): StdioTool::GetContract.as_str(),
            (mcp_fields::DESCRIPTION): "Return the full AIS contract: all valid operations with required attributes, valid dependency types, parameter types, and AIR input contract.",
            (mcp_fields::INPUT_SCHEMA): {
                (mcp_fields::TYPE): schema_type::OBJECT,
                (mcp_fields::PROPERTIES): {},
                (mcp_fields::REQUIRED): []
            }
        }),
        json!({
            (mcp_fields::NAME): StdioTool::Analyze.as_str(),
            (mcp_fields::DESCRIPTION): "Analyze APXM AIR to extract parallelism opportunities, critical path, and execution phases.",
            (mcp_fields::INPUT_SCHEMA): {
                (mcp_fields::TYPE): schema_type::OBJECT,
                (mcp_fields::PROPERTIES): {
                    (mcp_args::AIR): {
                        (mcp_fields::TYPE): schema_type::STRING,
                        (mcp_fields::DESCRIPTION): "Canonical APXM AIR text"
                    },
                    (mcp_args::PATH): {
                        (mcp_fields::TYPE): schema_type::STRING,
                        (mcp_fields::DESCRIPTION): "Path to a .air file or Python frontend file that emits AIR"
                    }
                },
                (mcp_fields::REQUIRED): []
            }
        }),
        json!({
            (mcp_fields::NAME): Tier3Tool::PromptAsWorkflow.as_str(),
            (mcp_fields::DESCRIPTION): tool_description::tier3(Tier3Tool::PromptAsWorkflow),
            (mcp_fields::INPUT_SCHEMA): {
                (mcp_fields::TYPE): schema_type::OBJECT,
                (mcp_fields::PROPERTIES): {
                    (mcp_args::TASK): {
                        (mcp_fields::TYPE): schema_type::STRING,
                        (mcp_fields::DESCRIPTION): "Natural-language task to convert into an APXM execution workflow"
                    },
                    (mcp_args::CONTEXT): {
                        (mcp_fields::TYPE): schema_type::STRING,
                        (mcp_fields::DESCRIPTION): "Optional context that should shape the workflow"
                    },
                    (mcp_args::CONSTRAINTS): {
                        (mcp_fields::TYPE): schema_type::OBJECT,
                        (mcp_fields::DESCRIPTION): "Optional structured constraints for the workflow emitter"
                    },
                    (mcp_args::PARAMETERS): {
                        (mcp_fields::TYPE): schema_type::OBJECT,
                        (mcp_fields::DESCRIPTION): "Optional runtime parameter values keyed by emitted parameter name"
                    },
                    (mcp_args::EXECUTE): {
                        (mcp_fields::TYPE): schema_type::BOOLEAN,
                        (mcp_fields::DESCRIPTION): "Whether to execute after successful compile. Default: true"
                    },
                    (mcp_args::TRACE_ID): {
                        (mcp_fields::TYPE): schema_type::STRING,
                        (mcp_fields::DESCRIPTION): "Optional caller-provided trace id"
                    }
                },
                (mcp_fields::REQUIRED): [mcp_args::TASK]
            }
        }),
        json!({
            (mcp_fields::NAME): Tier3Tool::TraceFetch.as_str(),
            (mcp_fields::DESCRIPTION): tool_description::tier3(Tier3Tool::TraceFetch),
            (mcp_fields::INPUT_SCHEMA): {
                (mcp_fields::TYPE): schema_type::OBJECT,
                (mcp_fields::PROPERTIES): {
                    (mcp_args::TRACE_ID): {
                        (mcp_fields::TYPE): schema_type::STRING,
                        (mcp_fields::DESCRIPTION): "Execution trace id returned by prompt_as_workflow or skill execution"
                    },
                    (mcp_args::NODE_ID): {
                        (mcp_fields::TYPE): schema_type::INTEGER,
                        (mcp_fields::MINIMUM): 1
                    },
                    (mcp_args::FULL): {
                        (mcp_fields::TYPE): schema_type::BOOLEAN,
                        (mcp_fields::DESCRIPTION): "Return the full execution record instead of a compact summary"
                    }
                },
                (mcp_fields::REQUIRED): [mcp_args::TRACE_ID]
            }
        }),
        json!({
            (mcp_fields::NAME): Tier3Tool::AamRecall.as_str(),
            (mcp_fields::DESCRIPTION): tool_description::tier3(Tier3Tool::AamRecall),
            (mcp_fields::INPUT_SCHEMA): query_tool_schema()
        }),
        json!({
            (mcp_fields::NAME): Tier3Tool::EvidenceLookup.as_str(),
            (mcp_fields::DESCRIPTION): tool_description::tier3(Tier3Tool::EvidenceLookup),
            (mcp_fields::INPUT_SCHEMA): {
                (mcp_fields::TYPE): schema_type::OBJECT,
                (mcp_fields::PROPERTIES): {
                    (mcp_args::QUERY): { (mcp_fields::TYPE): schema_type::STRING },
                    (mcp_args::CLAIM_ID): { (mcp_fields::TYPE): schema_type::STRING },
                    (mcp_args::PATH): { (mcp_fields::TYPE): schema_type::STRING },
                    (mcp_args::LIMIT): {
                        (mcp_fields::TYPE): schema_type::INTEGER,
                        (mcp_fields::MINIMUM): 1,
                        (mcp_fields::MAXIMUM): 100
                    }
                },
                (mcp_fields::REQUIRED): []
            }
        }),
        json!({
            (mcp_fields::NAME): Tier3Tool::CapabilityList.as_str(),
            (mcp_fields::DESCRIPTION): tool_description::tier3(Tier3Tool::CapabilityList),
            (mcp_fields::INPUT_SCHEMA): query_tool_schema()
        }),
    ];
    if raw_execute_enabled {
        tools.insert(
            2,
            json!({
                (mcp_fields::NAME): StdioTool::Execute.as_str(),
                (mcp_fields::DESCRIPTION): "Developer/debug only: compile and execute raw APXM AIR in one shot. Safe skill clients should use HTTP MCP skill_call.",
                (mcp_fields::INPUT_SCHEMA): {
                    (mcp_fields::TYPE): schema_type::OBJECT,
                    (mcp_fields::PROPERTIES): {
                        (mcp_args::AIR): {
                            (mcp_fields::TYPE): schema_type::STRING,
                            (mcp_fields::DESCRIPTION): "Canonical APXM AIR text"
                        },
                        (mcp_args::PATH): {
                            (mcp_fields::TYPE): schema_type::STRING,
                            (mcp_fields::DESCRIPTION): "Path to a .air file or Python frontend file that emits AIR"
                        },
                        (mcp_args::PARAMETERS): {
                            (mcp_fields::TYPE): schema_type::OBJECT,
                            (mcp_fields::DESCRIPTION): "Runtime parameters to pass to the workflow entry flow",
                            (mcp_fields::ADDITIONAL_PROPERTIES): { (mcp_fields::TYPE): schema_type::STRING }
                        }
                    },
                    (mcp_fields::REQUIRED): []
                }
            }),
        );
    }
    Ok(json!({ (mcp_fields::TOOLS): tools }))
}

fn handle_tools_call(params: Value, raw_execute_enabled: bool) -> Result<Value, Value> {
    let name = params
        .get(mcp_fields::NAME)
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_error(INVALID_PARAMS, "tools/call missing params.name"))?;
    let args = params
        .get(mcp_fields::ARGUMENTS)
        .cloned()
        .unwrap_or_else(|| json!({}));

    let result = match StdioTool::from_str(name) {
        Some(StdioTool::Validate) => tool_validate(args),
        Some(StdioTool::Compile) => tool_compile(args),
        Some(StdioTool::Execute) if raw_execute_enabled => tool_execute(args),
        Some(StdioTool::Execute) => Err(RAW_EXECUTE_DISABLED_MESSAGE.to_string()),
        Some(StdioTool::GetContract) => tool_get_contract(),
        Some(StdioTool::Analyze) => tool_analyze(args),
        None => match Tier3Tool::from_str(name) {
            Some(Tier3Tool::PromptAsWorkflow) => tool_prompt_as_workflow(args),
            Some(Tier3Tool::TraceFetch) => tool_trace_fetch(args),
            Some(Tier3Tool::AamRecall) => tool_aam_recall(args),
            Some(Tier3Tool::EvidenceLookup) => tool_evidence_lookup(args),
            Some(Tier3Tool::CapabilityList) => tool_capability_list(args),
            None => Err(format!("unknown tool: {name}")),
        },
    };

    match result {
        Ok(output) => Ok(json!({
            (mcp_fields::CONTENT): [{
                (mcp_fields::TYPE): ContentKind::Text.as_str(),
                (mcp_fields::TEXT): output,
            }],
            (mcp_fields::IS_ERROR): false,
        })),
        Err(error) => Ok(json!({
            (mcp_fields::CONTENT): [{
                (mcp_fields::TYPE): ContentKind::Text.as_str(),
                (mcp_fields::TEXT): error,
            }],
            (mcp_fields::IS_ERROR): true,
        })),
    }
}

fn query_tool_schema() -> Value {
    json!({
        (mcp_fields::TYPE): schema_type::OBJECT,
        (mcp_fields::PROPERTIES): {
            (mcp_args::QUERY): {
                (mcp_fields::TYPE): schema_type::STRING,
                (mcp_fields::DESCRIPTION): "Optional substring query"
            },
            (mcp_args::TOP_K): {
                (mcp_fields::TYPE): schema_type::INTEGER,
                (mcp_fields::MINIMUM): 1,
                (mcp_fields::MAXIMUM): 100
            }
        },
        (mcp_fields::REQUIRED): []
    })
}

fn handle_resources_list() -> Result<Value, Value> {
    handle_resources_list_with_roots(&discovered_skill_roots())
}

fn handle_resources_read(params: Value) -> Result<Value, Value> {
    handle_resources_read_with_roots(params, &discovered_skill_roots())
}

fn discovered_skill_roots() -> Vec<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    skill_resources::prepend_builtin_skill_root(skill_resources::parse_skill_roots(&args))
}

fn handle_resources_list_with_roots(roots: &[PathBuf]) -> Result<Value, Value> {
    let packages = skill_resources::scan_resource_packages(roots);
    let resources = skill_resources::list_skill_resources(&packages);
    Ok(json!({ (mcp_fields::RESOURCES): resources }))
}

fn handle_resources_read_with_roots(params: Value, roots: &[PathBuf]) -> Result<Value, Value> {
    let uri = params
        .get(mcp_fields::URI)
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_error(INVALID_PARAMS, "resources/read missing params.uri"))?;
    let packages = skill_resources::scan_resource_packages(roots);
    let content = skill_resources::resolve_skill_uri(&packages, uri)
        .map_err(|error| rpc_error(INVALID_PARAMS, error.to_string()))?;
    Ok(json!({ (mcp_fields::CONTENTS): [content] }))
}

// ---------------------------------------------------------------------------
// Tool: validate
// ---------------------------------------------------------------------------

fn tool_validate(args: Value) -> Result<String, String> {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let air = get_air_arg(&args)?;
    match compile_air_to_artifact(&air, OptimizationLevel::O1) {
        Ok(artifact) => {
            if artifact.entry_dag().is_none() {
                warnings.push("AIR compiled but produced no entry DAG".to_string());
            }
        }
        Err(err) => errors.push(err),
    }

    let valid = errors.is_empty();
    let result = json!({
        (tool_result::VALID): valid,
        (tool_result::ERRORS): errors,
        (tool_result::WARNINGS): warnings,
    });
    Ok(serde_json::to_string_pretty(&result).unwrap())
}

// ---------------------------------------------------------------------------
// Tool: compile
// ---------------------------------------------------------------------------

fn tool_compile(args: Value) -> Result<String, String> {
    let air = get_air_arg(&args)?;
    let opt_level_num = args
        .get(mcp_args::OPT_LEVEL)
        .and_then(Value::as_u64)
        .unwrap_or(2);
    let opt_level = match opt_level_num {
        0 => OptimizationLevel::O0,
        1 => OptimizationLevel::O1,
        2 => OptimizationLevel::O2,
        3 => OptimizationLevel::O3,
        _ => return Err(format!("opt_level must be 0-3, got {opt_level_num}")),
    };

    let start = Instant::now();
    let artifact = compile_air_to_artifact(&air, opt_level)?;
    let compile_ms = start.elapsed().as_millis();
    let artifact_bytes = artifact
        .to_bytes()
        .map_err(|e| format!("artifact encode failed: {e}"))?;
    let dag = artifact.entry_dag();
    let node_count = dag.map(|d| d.nodes.len()).unwrap_or(0);
    let edge_count = dag.map(|d| d.edges.len()).unwrap_or(0);
    let workflow_name = dag
        .and_then(|d| d.metadata.name.as_deref())
        .unwrap_or("artifact");

    let artifact_path = std::env::temp_dir().join(format!("{workflow_name}.apxmobj"));
    artifact
        .write_to_path(&artifact_path)
        .map_err(|e| format!("failed to write artifact: {e}"))?;

    let result = json!({
        (tool_result::ARTIFACT_PATH): artifact_path.display().to_string(),
        (tool_result::STATS): {
            (tool_result::NODES): node_count,
            (tool_result::EDGES): edge_count,
            (tool_result::ARTIFACT_BYTES): artifact_bytes.len(),
            (tool_result::COMPILE_MS): compile_ms,
            (tool_result::OPT_LEVEL): opt_level_num,
        }
    });
    Ok(serde_json::to_string_pretty(&result).unwrap())
}

// ---------------------------------------------------------------------------
// Tool: execute (hidden and disabled unless APXM_MCP_ENABLE_RAW_EXECUTE is set)
// ---------------------------------------------------------------------------

fn tool_execute(args: Value) -> Result<String, String> {
    let air = get_air_arg(&args)?;

    let parameters = args
        .get(mcp_args::PARAMETERS)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let compile_start = Instant::now();
    let artifact = compile_air_to_artifact(&air, OptimizationLevel::O1)?;
    let compile_ms = compile_start.elapsed().as_millis();

    // Build args from parameters map
    let entry_dag = artifact.entry_dag();
    let args: Vec<String> = if let Some(dag) = entry_dag {
        dag.metadata
            .parameters
            .iter()
            .map(|p| {
                parameters
                    .get(&p.name)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            })
            .collect()
    } else {
        Vec::new()
    };

    // Execute via the runtime (requires tokio)
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("tokio runtime init failed: {e}"))?;

    let exec_start = Instant::now();
    let execution = rt.block_on(async {
        use apxm_driver::runtime::sandbox::configure_sandbox_registry;
        use apxm_runtime::{Runtime, RuntimeConfig};
        let mut runtime = Runtime::new(RuntimeConfig::default())
            .await
            .map_err(|e| format!("runtime init failed: {e}"))?;
        runtime.set_sandbox_registry(configure_sandbox_registry());
        runtime
            .execute_artifact_with_args(artifact, args)
            .await
            .map_err(|e| format!("execution failed: {e}"))
    })?;
    let exec_ms = exec_start.elapsed().as_millis();

    // Format results
    let mut results_map = serde_json::Map::new();
    let mut content: Option<String> = None;
    for (token, value) in &execution.results {
        let json_val = value
            .to_json()
            .unwrap_or_else(|_| Value::String(value.to_string()));
        if content.is_none()
            && let Some(text) = json_val.as_str()
        {
            content = Some(text.to_string());
        }
        results_map.insert(token.to_string(), json_val);
    }

    let result = json!({
        (tool_result::RESULT): content.unwrap_or_default(),
        (tool_result::RESULTS): results_map,
        (tool_result::STATS): {
            (tool_result::DURATION_MS): compile_ms as u64 + exec_ms as u64,
            (tool_result::COMPILE_MS): compile_ms,
            (tool_result::EXECUTE_MS): exec_ms,
            (tool_result::EXECUTED_NODES): execution.stats.executed_nodes,
            (tool_result::FAILED_NODES): execution.stats.failed_nodes,
        },
        (tool_result::LLM_USAGE): {
            (tool_result::INPUT_TOKENS): execution.llm_metrics.total_input_tokens,
            (tool_result::OUTPUT_TOKENS): execution.llm_metrics.total_output_tokens,
            (tool_result::TOTAL_REQUESTS): execution.llm_metrics.total_requests,
        }
    });
    Ok(serde_json::to_string_pretty(&result).unwrap())
}

// ---------------------------------------------------------------------------
// Tool: get_contract
// ---------------------------------------------------------------------------

fn tool_get_contract() -> Result<String, String> {
    let mut operations = serde_json::Map::new();
    for spec in AIS_OPERATIONS {
        let required_attrs: Vec<&str> = spec
            .fields
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();

        let optional_attrs: Vec<Value> = spec
            .fields
            .iter()
            .filter(|f| !f.required)
            .map(|f| {
                json!({
                    (tool_result::NAME): f.name,
                    (tool_result::DESCRIPTION): f.description
                })
            })
            .collect();

        let mut op_json = json!({
            (tool_result::DESCRIPTION): spec.description,
            (tool_result::LONG_DESCRIPTION): spec.long_description,
            (tool_result::CATEGORY): OperationCategoryWire::from(spec.category).as_str(),
            (tool_result::LATENCY): spec.latency.as_str(),
            (tool_result::REQUIRED_ATTRIBUTES): required_attrs,
            (tool_result::OPTIONAL_ATTRIBUTES): optional_attrs,
            (tool_result::PRODUCES_OUTPUT): spec.produces_output,
        });

        if let Some(example) = spec.example_json {
            op_json[tool_result::EXAMPLE] = Value::String(example.to_string());
        }

        operations.insert(spec.op_type.to_string(), op_json);
    }

    let result = json!({
        (tool_result::OPERATIONS): Value::Object(operations),
        (tool_result::DEPENDENCY_TYPES): contract_value::DEPENDENCY_TYPES,
        (tool_result::PARAMETER_TYPES): contract_value::PARAMETER_TYPES,
        (tool_result::AIR_CONTRACT): {
            (tool_result::REQUIRED_ARGUMENT): mcp_args::AIR,
            (tool_result::DESCRIPTION): "Canonical APXM workflow source as AIR text, or path to .air/.py",
        }
    });
    Ok(serde_json::to_string_pretty(&result).unwrap())
}

// ---------------------------------------------------------------------------
// Tool: analyze
// ---------------------------------------------------------------------------

fn tool_analyze(args: Value) -> Result<String, String> {
    let air = get_air_arg(&args)?;
    let artifact = compile_air_to_artifact(&air, OptimizationLevel::O1)?;
    let graph = artifact
        .entry_dag()
        .ok_or_else(|| "compiled AIR produced no entry DAG".to_string())?;

    // Build adjacency and reverse-adjacency maps
    let node_ids: HashSet<u64> = graph.nodes.iter().map(|n| n.id).collect();
    let mut successors: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut predecessors: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut in_degree: HashMap<u64, usize> = node_ids.iter().map(|&id| (id, 0)).collect();

    for edge in &graph.edges {
        successors.entry(edge.from).or_default().push(edge.to);
        predecessors.entry(edge.to).or_default().push(edge.from);
        *in_degree.entry(edge.to).or_insert(0) += 1;
    }

    let entry_nodes: Vec<u64> = in_degree
        .iter()
        .filter_map(|(&id, &deg)| if deg == 0 { Some(id) } else { None })
        .collect();

    let exit_nodes: Vec<u64> = node_ids
        .iter()
        .filter(|&&id| successors.get(&id).is_none_or(|s| s.is_empty()))
        .copied()
        .collect();

    // Compute execution phases via BFS layering (topological levels)
    let mut phases: Vec<Vec<u64>> = Vec::new();
    let mut remaining_in: HashMap<u64, usize> = in_degree.clone();
    let mut current_layer: Vec<u64> = entry_nodes.clone();
    current_layer.sort();

    while !current_layer.is_empty() {
        phases.push(current_layer.clone());
        let mut next_layer = Vec::new();
        for &node_id in &current_layer {
            if let Some(succs) = successors.get(&node_id) {
                for &succ in succs {
                    if let Some(deg) = remaining_in.get_mut(&succ) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 {
                            next_layer.push(succ);
                        }
                    }
                }
            }
        }
        next_layer.sort();
        next_layer.dedup();
        current_layer = next_layer;
    }

    // Compute critical path via longest-path DAG algorithm
    // Use latency estimates from OperationSpec
    let node_latency = |node_id: u64| -> u64 {
        graph
            .nodes
            .iter()
            .find(|n| n.id == node_id)
            .and_then(|n| {
                for spec in AIS_OPERATIONS {
                    if spec.op_type == n.op_type {
                        return Some(operation_latency_estimate_ms(spec.latency));
                    }
                }
                None
            })
            .unwrap_or(100)
    };

    // Compute longest path from each entry to each exit
    let mut dist: HashMap<u64, u64> = HashMap::new();
    let mut prev: HashMap<u64, u64> = HashMap::new();
    // Process nodes in topological order (phase order)
    for phase in &phases {
        for &node_id in phase {
            let latency = node_latency(node_id);
            let max_pred_dist = predecessors
                .get(&node_id)
                .and_then(|preds| preds.iter().filter_map(|&p| dist.get(&p)).max().copied())
                .unwrap_or(0);
            let d = max_pred_dist + latency;
            dist.insert(node_id, d);
            // Track which predecessor gave the max
            if let Some(preds) = predecessors.get(&node_id)
                && let Some(&best_pred) = preds.iter().max_by_key(|&&p| dist.get(&p).unwrap_or(&0))
            {
                prev.insert(node_id, best_pred);
            }
        }
    }

    // Find the node with maximum distance (end of critical path)
    let critical_end = dist.iter().max_by_key(|&(_, &d)| d).map(|(&id, _)| id);
    let mut critical_path = Vec::new();
    if let Some(mut node) = critical_end {
        critical_path.push(node);
        while let Some(&p) = prev.get(&node) {
            critical_path.push(p);
            node = p;
        }
        critical_path.reverse();
    }

    let critical_path_latency: u64 = critical_path.iter().map(|&id| node_latency(id)).sum();
    let sequential_latency: u64 = graph.nodes.iter().map(|n| node_latency(n.id)).sum();

    // Build phase estimates
    let phase_json: Vec<Value> = phases
        .iter()
        .enumerate()
        .map(|(i, layer)| {
            let max_latency = layer.iter().map(|&id| node_latency(id)).max().unwrap_or(0);
            let node_details: Vec<Value> = layer
                .iter()
                .map(|&id| {
                    let node = graph.nodes.iter().find(|n| n.id == id);
                    let op = node
                        .map(|n| n.op_type.to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let name = node.and_then(|n| n.metadata.name.as_deref()).unwrap_or("?");
                    json!({
                        (tool_result::ID): id,
                        (tool_result::NAME): name,
                        (tool_result::OP): op,
                        (tool_result::LATENCY_MS): node_latency(id)
                    })
                })
                .collect();
            json!({
                (tool_result::PHASE): i + 1,
                (tool_result::PARALLEL): layer.len() > 1,
                (tool_result::PARALLELISM_DEGREE): layer.len(),
                (tool_result::ESTIMATED_MS): max_latency,
                (tool_result::NODES): node_details,
            })
        })
        .collect();

    let parallel_latency: u64 = phases
        .iter()
        .map(|layer| layer.iter().map(|&id| node_latency(id)).max().unwrap_or(0))
        .sum();

    let speedup = if parallel_latency > 0 {
        sequential_latency as f64 / parallel_latency as f64
    } else {
        1.0
    };

    let max_parallelism = phases.iter().map(|p| p.len()).max().unwrap_or(1);

    let result = json!({
        (tool_result::WORKFLOW_NAME): graph.metadata.name.as_deref().unwrap_or("artifact"),
        (tool_result::NODE_COUNT): graph.nodes.len(),
        (tool_result::EDGE_COUNT): graph.edges.len(),
        (tool_result::ENTRY_NODES): entry_nodes,
        (tool_result::EXIT_NODES): exit_nodes,
        (tool_result::DEPTH): phases.len(),
        (tool_result::MAX_PARALLELISM): max_parallelism,
        (tool_result::EXECUTION_PHASES): phase_json,
        (tool_result::CRITICAL_PATH): {
            (tool_result::NODES): critical_path,
            (tool_result::LENGTH): critical_path.len(),
            (tool_result::ESTIMATED_MS): critical_path_latency,
        },
        (tool_result::SPEEDUP): {
            (tool_result::SEQUENTIAL_MS): sequential_latency,
            (tool_result::PARALLEL_MS): parallel_latency,
            (tool_result::ESTIMATED_SPEEDUP): format!("{:.2}x", speedup),
        },
        (tool_result::SUGGESTIONS): build_suggestions(&phases, max_parallelism, speedup, &critical_path, &graph),
    });

    Ok(serde_json::to_string_pretty(&result).unwrap())
}

fn build_suggestions(
    phases: &[Vec<u64>],
    max_parallelism: usize,
    speedup: f64,
    critical_path: &[u64],
    graph: &ExecutionDag,
) -> Vec<String> {
    let mut suggestions = Vec::new();

    if max_parallelism > 1 {
        let parallel_phases: Vec<usize> = phases
            .iter()
            .enumerate()
            .filter(|(_, p)| p.len() > 1)
            .map(|(i, _)| i + 1)
            .collect();
        suggestions.push(format!(
            "Phases {:?} can execute in parallel (up to {} concurrent operations)",
            parallel_phases, max_parallelism
        ));
    } else {
        suggestions.push("Workflow is fully sequential; no parallelism opportunities".to_string());
    }

    if speedup > 1.2 {
        suggestions.push(format!(
            "Estimated {:.1}x speedup from parallel execution vs sequential",
            speedup
        ));
    }

    if critical_path.len() >= 3 {
        // Find bottleneck node on critical path
        let bottleneck = critical_path.iter().max_by_key(|&&id| {
            graph
                .nodes
                .iter()
                .find(|n| n.id == id)
                .map(|n| {
                    for spec in AIS_OPERATIONS {
                        if spec.op_type == n.op_type {
                            return operation_latency_estimate_ms(spec.latency);
                        }
                    }
                    100
                })
                .unwrap_or(100)
        });
        if let Some(&bn) = bottleneck
            && let Some(node) = graph.nodes.iter().find(|n| n.id == bn)
        {
            let node_name = node.metadata.name.as_deref().unwrap_or("?");
            suggestions.push(format!(
                "Critical path bottleneck: node {} ('{}', op={})",
                bn, node_name, node.op_type
            ));
        }
    }

    suggestions
}

// ---------------------------------------------------------------------------
// Tier-3 dispatch/query tools
// ---------------------------------------------------------------------------

fn tool_prompt_as_workflow(args: Value) -> Result<String, String> {
    run_with_stdio_runtime(|runtime, server_config| async move {
        mcp_tools::prompt_as_workflow_with_recorder(&runtime, args, None, &server_config.mcp).await
    })
}

fn tool_trace_fetch(args: Value) -> Result<String, String> {
    run_with_stdio_runtime(|runtime, _server_config| async move {
        mcp_tools::trace_fetch(Some(&runtime), None, args).await
    })
}

fn tool_aam_recall(args: Value) -> Result<String, String> {
    run_with_stdio_runtime(|runtime, _server_config| async move {
        mcp_tools::aam_recall(&runtime, args).await
    })
}

fn tool_evidence_lookup(args: Value) -> Result<String, String> {
    let output = mcp_tools::evidence_lookup(args)?;
    serde_json::to_string_pretty(&output).map_err(|error| error.to_string())
}

fn tool_capability_list(args: Value) -> Result<String, String> {
    run_with_stdio_runtime(|runtime, _server_config| async move {
        Ok(mcp_tools::capability_list(&runtime, args))
    })
}

fn run_with_stdio_runtime<F, Fut>(f: F) -> Result<String, String>
where
    F: FnOnce(apxm_runtime::Runtime, apxm_driver::ServerConfig) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("tokio runtime init failed: {error}"))?;
    let output = rt.block_on(async {
        let server_config = config_layers::server_config_from_layers()
            .map_err(|error| format!("server config init failed: {error}"))?;
        let runtime =
            runtime_setup::build_runtime_with_router(apxm_runtime::RuntimeConfig::default(), None)
                .await
                .map_err(|error| format!("runtime init failed: {error}"))?;
        f(runtime, server_config).await
    })?;
    serde_json::to_string_pretty(&output).map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_air_arg(args: &Value) -> Result<String, String> {
    workflow_source::air_from_args(args)
}

fn compile_air_to_artifact(air: &str, opt_level: OptimizationLevel) -> Result<Artifact, String> {
    let context =
        CompilerContext::new().map_err(|e| format!("compiler context init failed: {e}"))?;
    let pipeline = CompilerPipeline::with_opt_level(&context, opt_level);
    let module = pipeline
        .compile(air)
        .map_err(|e| format!("compilation failed: {e}"))?;
    let artifact_bytes = module
        .generate_artifact_bytes()
        .map_err(|e| format!("artifact generation failed: {e}"))?;
    Artifact::from_bytes(&artifact_bytes).map_err(|e| format!("artifact decode failed: {e}"))
}

fn rpc_error(code: i64, message: impl Into<String>) -> Value {
    json!({
        "code": code,
        "message": message.into(),
    })
}
