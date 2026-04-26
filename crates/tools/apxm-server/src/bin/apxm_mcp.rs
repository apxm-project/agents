//! APXM MCP Server -- exposes the APXM compiler as MCP tools over stdio.
//!
//! Implements JSON-RPC 2.0 over stdin/stdout per the Model Context Protocol
//! (2024-11-05) so that external agents (Claude Code, Codex, etc.) can
//! validate, compile, and execute APXM AIR.
//!
//! # Tools
//!
//! - `apxm_validate`      -- validate AIR against the AIS contract
//! - `apxm_compile`       -- compile AIR to an optimized artifact
//! - `apxm_execute`       -- compile + execute AIR in one shot
//! - `apxm_get_contract`  -- return the full AIS contract (ops, attrs, types)
//!
//! # Running
//!
//! ```bash
//! apxm-mcp-server          # reads JSON-RPC lines from stdin, writes to stdout
//! ```

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::time::Instant;

use apxm_artifact::Artifact;
use apxm_compiler::{Context as CompilerContext, Pipeline as CompilerPipeline};
use apxm_core::constants::jsonrpc;
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::{AIS_OPERATIONS, OptimizationLevel};
use serde_json::{Value, json};

const MCP_PROTOCOL_VERSION: &str = apxm_core::constants::protocols::MCP_VERSION;
const SERVER_NAME: &str = "apxm-mcp-server";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

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

    let result = match method {
        "initialize" => handle_initialize(),
        "tools/list" => handle_tools_list(),
        "tools/call" => handle_tools_call(params),
        "ping" => Ok(json!({})),
        "" => Err(rpc_error(PARSE_ERROR, "missing method")),
        _ => Err(rpc_error(
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
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "serverInfo": {
            "name": SERVER_NAME,
            "version": SERVER_VERSION,
        },
        "capabilities": {
            "tools": { "listChanged": false }
        }
    }))
}

fn handle_tools_list() -> Result<Value, Value> {
    let tools = vec![
        json!({
            "name": "apxm_validate",
            "description": "Validate canonical APXM AIR against the AIS contract.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "air": {
                        "type": "string",
                        "description": "Canonical APXM AIR text"
                    }
                },
                "required": ["air"]
            }
        }),
        json!({
            "name": "apxm_compile",
            "description": "Compile canonical APXM AIR to an optimized APXM artifact (.apxmobj). Returns the artifact path and compilation stats.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "air": {
                        "type": "string",
                        "description": "Canonical APXM AIR text"
                    },
                    "opt_level": {
                        "type": "integer",
                        "description": "Optimization level (0-3). 0=none, 1=basic, 2=standard, 3=aggressive. Default: 2",
                        "minimum": 0,
                        "maximum": 3
                    }
                },
                "required": ["air"]
            }
        }),
        json!({
            "name": "apxm_execute",
            "description": "Compile and execute APXM AIR in one shot. Requires APXM runtime environment.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "air": {
                        "type": "string",
                        "description": "Canonical APXM AIR text"
                    },
                    "parameters": {
                        "type": "object",
                        "description": "Runtime parameters to pass to the graph entry flow",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["air"]
            }
        }),
        json!({
            "name": "apxm_get_contract",
            "description": "Return the full AIS contract: all valid operations with required attributes, valid dependency types, parameter types, and AIR input contract.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
        json!({
            "name": "apxm_analyze",
            "description": "Analyze APXM AIR to extract parallelism opportunities, critical path, and execution phases.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "air": {
                        "type": "string",
                        "description": "Canonical APXM AIR text"
                    }
                },
                "required": ["air"]
            }
        }),
    ];
    Ok(json!({ "tools": tools }))
}

fn handle_tools_call(params: Value) -> Result<Value, Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_error(INVALID_PARAMS, "tools/call missing params.name"))?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let result = match name {
        "apxm_validate" => tool_validate(args),
        "apxm_compile" => tool_compile(args),
        "apxm_execute" => tool_execute(args),
        "apxm_get_contract" => tool_get_contract(),
        "apxm_analyze" => tool_analyze(args),
        _ => Err(format!("unknown tool: {name}")),
    };

    match result {
        Ok(output) => Ok(json!({
            "content": [{ "type": "text", "text": output }],
            "isError": false,
        })),
        Err(error) => Ok(json!({
            "content": [{ "type": "text", "text": error }],
            "isError": true,
        })),
    }
}

// ---------------------------------------------------------------------------
// Tool: apxm_validate
// ---------------------------------------------------------------------------

fn tool_validate(args: Value) -> Result<String, String> {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let air = get_air_arg(&args)?;
    match compile_air_to_artifact(air, OptimizationLevel::O1) {
        Ok(artifact) => {
            if artifact.entry_dag().is_none() {
                warnings.push("AIR compiled but produced no entry DAG".to_string());
            }
        }
        Err(err) => errors.push(err),
    }

    let valid = errors.is_empty();
    let result = json!({
        "valid": valid,
        "errors": errors,
        "warnings": warnings,
    });
    Ok(serde_json::to_string_pretty(&result).unwrap())
}

// ---------------------------------------------------------------------------
// Tool: apxm_compile
// ---------------------------------------------------------------------------

fn tool_compile(args: Value) -> Result<String, String> {
    let air = get_air_arg(&args)?;
    let opt_level_num = args.get("opt_level").and_then(Value::as_u64).unwrap_or(2);
    let opt_level = match opt_level_num {
        0 => OptimizationLevel::O0,
        1 => OptimizationLevel::O1,
        2 => OptimizationLevel::O2,
        3 => OptimizationLevel::O3,
        _ => return Err(format!("opt_level must be 0-3, got {opt_level_num}")),
    };

    let start = Instant::now();
    let artifact = compile_air_to_artifact(air, opt_level)?;
    let compile_ms = start.elapsed().as_millis();
    let artifact_bytes = artifact
        .to_bytes()
        .map_err(|e| format!("artifact encode failed: {e}"))?;
    let dag = artifact.entry_dag();
    let node_count = dag.map(|d| d.nodes.len()).unwrap_or(0);
    let edge_count = dag.map(|d| d.edges.len()).unwrap_or(0);
    let graph_name = dag
        .and_then(|d| d.metadata.name.as_deref())
        .unwrap_or("artifact");

    let artifact_path = std::env::temp_dir().join(format!("{graph_name}.apxmobj"));
    artifact
        .write_to_path(&artifact_path)
        .map_err(|e| format!("failed to write artifact: {e}"))?;

    let result = json!({
        "artifact_path": artifact_path.display().to_string(),
        "stats": {
            "nodes": node_count,
            "edges": edge_count,
            "artifact_bytes": artifact_bytes.len(),
            "compile_ms": compile_ms,
            "opt_level": opt_level_num,
        }
    });
    Ok(serde_json::to_string_pretty(&result).unwrap())
}

// ---------------------------------------------------------------------------
// Tool: apxm_execute
// ---------------------------------------------------------------------------

fn tool_execute(args: Value) -> Result<String, String> {
    let air = get_air_arg(&args)?;

    let parameters = args
        .get("parameters")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let compile_start = Instant::now();
    let artifact = compile_air_to_artifact(air, OptimizationLevel::O1)?;
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
        use apxm_runtime::{Runtime, RuntimeConfig};
        let runtime = Runtime::new(RuntimeConfig::default())
            .await
            .map_err(|e| format!("runtime init failed: {e}"))?;
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
        "result": content.unwrap_or_default(),
        "results": results_map,
        "stats": {
            "duration_ms": compile_ms as u64 + exec_ms as u64,
            "compile_ms": compile_ms,
            "execute_ms": exec_ms,
            "executed_nodes": execution.stats.executed_nodes,
            "failed_nodes": execution.stats.failed_nodes,
        },
        "llm_usage": {
            "input_tokens": execution.llm_metrics.total_input_tokens,
            "output_tokens": execution.llm_metrics.total_output_tokens,
            "total_requests": execution.llm_metrics.total_requests,
        }
    });
    Ok(serde_json::to_string_pretty(&result).unwrap())
}

// ---------------------------------------------------------------------------
// Tool: apxm_get_contract
// ---------------------------------------------------------------------------

fn tool_get_contract() -> Result<String, String> {
    use apxm_core::types::OperationCategory;

    fn category_str(cat: OperationCategory) -> &'static str {
        match cat {
            OperationCategory::Metadata => "metadata",
            OperationCategory::Memory => "memory",
            OperationCategory::Reasoning => "reasoning",
            OperationCategory::Tools => "tools",
            OperationCategory::ControlFlow => "control_flow",
            OperationCategory::Synchronization => "synchronization",
            OperationCategory::ErrorHandling => "error_handling",
            OperationCategory::Communication => "communication",
            OperationCategory::Internal => "internal",
            OperationCategory::Coordination => "coordination",
            OperationCategory::Identity => "identity",
        }
    }

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
            .map(|f| json!({"name": f.name, "description": f.description}))
            .collect();

        let mut op_json = json!({
            "description": spec.description,
            "long_description": spec.long_description,
            "category": category_str(spec.category),
            "latency": spec.latency.as_str(),
            "required_attributes": required_attrs,
            "optional_attributes": optional_attrs,
            "produces_output": spec.produces_output,
        });

        if let Some(example) = spec.example_json {
            op_json["example"] = Value::String(example.to_string());
        }

        operations.insert(spec.op_type.to_string(), op_json);
    }

    let result = json!({
        "operations": Value::Object(operations),
        "dependency_types": ["Data", "Control", "Effect"],
        "parameter_types": ["str", "int", "float", "bool", "json"],
        "air_contract": {
            "required_argument": "air",
            "description": "Canonical APXM graph source as AIR text",
        }
    });
    Ok(serde_json::to_string_pretty(&result).unwrap())
}

// ---------------------------------------------------------------------------
// Tool: apxm_analyze
// ---------------------------------------------------------------------------

fn tool_analyze(args: Value) -> Result<String, String> {
    let air = get_air_arg(&args)?;
    let artifact = compile_air_to_artifact(air, OptimizationLevel::O1)?;
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
                use apxm_core::types::OperationLatency;
                for spec in AIS_OPERATIONS {
                    if spec.op_type == n.op_type {
                        return Some(match spec.latency {
                            OperationLatency::None => 10,
                            OperationLatency::Low => 100,
                            OperationLatency::Medium => 1000,
                            OperationLatency::High => 5000,
                        });
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
                    json!({"id": id, "name": name, "op": op, "latency_ms": node_latency(id)})
                })
                .collect();
            json!({
                "phase": i + 1,
                "parallel": layer.len() > 1,
                "parallelism_degree": layer.len(),
                "estimated_ms": max_latency,
                "nodes": node_details,
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
        "graph_name": graph.metadata.name.as_deref().unwrap_or("artifact"),
        "node_count": graph.nodes.len(),
        "edge_count": graph.edges.len(),
        "entry_nodes": entry_nodes,
        "exit_nodes": exit_nodes,
        "depth": phases.len(),
        "max_parallelism": max_parallelism,
        "execution_phases": phase_json,
        "critical_path": {
            "nodes": critical_path,
            "length": critical_path.len(),
            "estimated_ms": critical_path_latency,
        },
        "speedup": {
            "sequential_ms": sequential_latency,
            "parallel_ms": parallel_latency,
            "estimated_speedup": format!("{:.2}x", speedup),
        },
        "suggestions": build_suggestions(&phases, max_parallelism, speedup, &critical_path, &graph),
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
        suggestions.push("Graph is fully sequential — no parallelism opportunities".to_string());
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
                            return match spec.latency {
                                apxm_core::types::OperationLatency::High => 5000u64,
                                apxm_core::types::OperationLatency::Medium => 1000,
                                apxm_core::types::OperationLatency::Low => 100,
                                apxm_core::types::OperationLatency::None => 10,
                            };
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
// Helpers
// ---------------------------------------------------------------------------

fn get_air_arg(args: &Value) -> Result<&str, String> {
    let air = args
        .get("air")
        .and_then(Value::as_str)
        .ok_or("missing required argument: air")?;
    if air.trim().is_empty() {
        return Err("air must not be empty".to_string());
    }
    Ok(air)
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
