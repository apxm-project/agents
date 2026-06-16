//! Validate, analyze, explain commands.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::Result;
use colored::Colorize;

#[cfg(feature = "driver")]
use super::compile::air_graph_from_source;
use super::implementations::{
    Status, category_str, find_op_spec, op_latency_ms, print_section_header, print_status_line,
};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, ApxmPathFormat};

pub fn validate_command(
    input: PathBuf,
    json_output: bool,
    _no_check_resources: bool,
) -> Result<()> {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if !ApxmPathFormat::from_path(&input).is_air_source() {
        errors.push(
            "workflow source must be canonical .air; JSON is reserved for structured data outputs"
                .to_string(),
        );
    } else {
        match load_air_graph_for_analysis(&input) {
            Ok(graph) => {
                if let Err(err) = graph.validate() {
                    errors.push(err.to_string());
                }
            }
            Err(err) => errors.push(err.to_string()),
        }
    }

    if warnings.is_empty() && errors.is_empty() {
        if let Ok(graph) = load_air_graph_for_analysis(&input)
            && graph.nodes.is_empty()
        {
            warnings.push("compiled AIR contains no executable nodes".to_string());
        }
    }

    let valid = errors.is_empty();

    if json_output {
        let result = serde_json::json!({
            "file": input.display().to_string(),
            "valid": valid,
            "errors": errors,
            "warnings": warnings,
        });
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        if !valid {
            return Err(super::output_already_emitted());
        }
    } else if valid {
        print_status_line(&input.display().to_string(), Status::Ok, "valid");
        if !warnings.is_empty() {
            for w in &warnings {
                println!(
                    "  {} {}",
                    apxm_core::constants::ui::icons::CAUTION.yellow(),
                    w
                );
            }
        }
    } else {
        print_status_line(&input.display().to_string(), Status::Error, "invalid");
        for e in &errors {
            println!("  {} {}", apxm_core::constants::ui::icons::FAILED.red(), e);
        }
        for w in &warnings {
            println!(
                "  {} {}",
                apxm_core::constants::ui::icons::CAUTION.yellow(),
                w
            );
        }
        return Err(anyhow::anyhow!("{} error(s) found", errors.len()));
    }

    Ok(())
}

fn load_air_graph_for_analysis(input: &PathBuf) -> Result<apxm_compiler::AirModule> {
    #[cfg(feature = "driver")]
    {
        return air_graph_from_source(input);
    }

    #[cfg(not(feature = "driver"))]
    {
        let _ = input;
        Err(anyhow::anyhow!(
            "AIR validation and analysis require the APXM driver feature"
        ))
    }
}

/// Parsed graph topology for analyze and explain commands.
pub(crate) struct GraphAnalysis<'a> {
    pub(crate) graph: &'a apxm_compiler::AirModule,
    pub(crate) node_index: HashMap<u64, usize>,
    pub(crate) edge_count: usize,
    pub(crate) node_ids: HashSet<u64>,
    pub(crate) successors: HashMap<u64, Vec<u64>>,
    pub(crate) predecessors: HashMap<u64, Vec<u64>>,
    pub(crate) entry_nodes: Vec<u64>,
    pub(crate) exit_nodes: Vec<u64>,
    pub(crate) phases: Vec<Vec<u64>>,
}

impl<'a> GraphAnalysis<'a> {
    pub(crate) fn from_graph(graph: &'a apxm_compiler::AirModule) -> Self {
        let node_index: HashMap<u64, usize> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i))
            .collect();
        let node_ids: HashSet<u64> = graph.nodes.iter().map(|n| n.id).collect();
        let edge_count = graph.edges.len();

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

        let mut phases: Vec<Vec<u64>> = Vec::new();
        let mut remaining_in: HashMap<u64, usize> = in_degree.clone();
        let mut current_layer: Vec<u64> = entry_nodes.clone();
        current_layer.sort();

        while !current_layer.is_empty() {
            phases.push(current_layer.clone());
            let mut next_layer = Vec::new();
            for &nid in &current_layer {
                if let Some(succs) = successors.get(&nid) {
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

        Self {
            graph,
            node_index,
            edge_count,
            node_ids,
            successors,
            predecessors,
            entry_nodes,
            exit_nodes,
            phases,
        }
    }

    fn node_by_id(&self, id: u64) -> Option<&apxm_compiler::AirNode> {
        self.node_index.get(&id).map(|&i| &self.graph.nodes[i])
    }

    pub(crate) fn node_op(&self, id: u64) -> String {
        self.node_by_id(id)
            .map(|n| n.op.to_string())
            .unwrap_or_else(|| "?".to_string())
    }

    fn node_latency_op(&self, id: u64) -> String {
        let Some(node) = self.node_by_id(id) else {
            return "?".to_string();
        };
        node.attributes
            .get(graph_attrs::LLM_OPERATION)
            .and_then(|value| value.as_str())
            .and_then(|value| value.parse::<AISOperationType>().ok())
            .map(|operation| operation.to_string())
            .unwrap_or_else(|| node.op.to_string())
    }

    pub(crate) fn node_name(&self, id: u64) -> &str {
        self.node_by_id(id).map(|n| n.name.as_str()).unwrap_or("?")
    }

    fn node_latency_ms(&self, id: u64) -> u64 {
        op_latency_ms(&self.node_latency_op(id))
    }

    pub(crate) fn max_parallelism(&self) -> usize {
        self.phases.iter().map(|p| p.len()).max().unwrap_or(1)
    }

    pub(crate) fn parallel_ms(&self) -> u64 {
        self.phases
            .iter()
            .map(|layer| {
                layer
                    .iter()
                    .map(|&id| self.node_latency_ms(id))
                    .max()
                    .unwrap_or(0)
            })
            .sum()
    }

    pub(crate) fn sequential_ms(&self) -> u64 {
        self.node_ids
            .iter()
            .map(|&id| self.node_latency_ms(id))
            .sum()
    }

    pub(crate) fn speedup(&self) -> f64 {
        let par = self.parallel_ms();
        if par > 0 {
            self.sequential_ms() as f64 / par as f64
        } else {
            1.0
        }
    }

    pub(crate) fn critical_path(&self) -> (Vec<u64>, u64) {
        let mut dist: HashMap<u64, u64> = HashMap::new();
        let mut prev: HashMap<u64, u64> = HashMap::new();
        for phase in &self.phases {
            for &nid in phase {
                let lat = self.node_latency_ms(nid);
                let max_pred = self
                    .predecessors
                    .get(&nid)
                    .and_then(|preds| preds.iter().filter_map(|&p| dist.get(&p)).max().copied())
                    .unwrap_or(0);
                dist.insert(nid, max_pred + lat);
                if let Some(preds) = self.predecessors.get(&nid)
                    && let Some(&best) = preds.iter().max_by_key(|&&p| dist.get(&p).unwrap_or(&0))
                {
                    prev.insert(nid, best);
                }
            }
        }

        let critical_end = dist.iter().max_by_key(|&(_, &d)| d).map(|(&id, _)| id);
        let mut path = Vec::new();
        if let Some(mut node) = critical_end {
            path.push(node);
            while let Some(&p) = prev.get(&node) {
                path.push(p);
                node = p;
            }
            path.reverse();
        }
        let ms = path.iter().map(|&id| self.node_latency_ms(id)).sum();
        (path, ms)
    }
}

pub fn analyze_command(input: PathBuf, json_output: bool) -> Result<()> {
    if !ApxmPathFormat::from_path(&input).is_air_source() {
        return Err(anyhow::anyhow!(
            "Analyze accepts canonical .air workflow source. JSON is reserved for structured data outputs."
        ));
    }
    let graph = load_air_graph_for_analysis(&input)?;

    let ga = GraphAnalysis::from_graph(&graph);
    let (critical_path, critical_ms) = ga.critical_path();
    let sequential_ms = ga.sequential_ms();
    let parallel_ms = ga.parallel_ms();
    let max_parallelism = ga.max_parallelism();
    let speedup = ga.speedup();

    // Build suggestions shared by JSON and human-readable output.
    let mut suggestions: Vec<String> = Vec::new();
    if max_parallelism > 1 {
        let parallel_phases: Vec<usize> = ga
            .phases
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
    if critical_path.len() >= 3
        && let Some(&bn) = critical_path
            .iter()
            .max_by_key(|&&id| ga.node_latency_ms(id))
    {
        suggestions.push(format!(
            "Critical path bottleneck: node {} ('{}', op={})",
            bn,
            ga.node_name(bn),
            ga.node_op(bn)
        ));
    }

    if json_output {
        let phase_json: Vec<serde_json::Value> = ga.phases.iter().enumerate().map(|(i, layer)| {
            let max_lat = layer.iter().map(|&id| ga.node_latency_ms(id)).max().unwrap_or(0);
            let node_details: Vec<serde_json::Value> = layer.iter().map(|&id| {
                serde_json::json!({"id": id, "name": ga.node_name(id), "op": ga.node_op(id), "latency_ms": ga.node_latency_ms(id)})
            }).collect();
            serde_json::json!({
                "phase": i + 1,
                "parallel": layer.len() > 1,
                "parallelism_degree": layer.len(),
                "estimated_ms": max_lat,
                "nodes": node_details,
            })
        }).collect();

        let result = serde_json::json!({
            "file": input.display().to_string(),
            "workflow_name": ga.graph.name,
            "node_count": ga.graph.nodes.len(),
            "edge_count": ga.edge_count,
            "entry_nodes": ga.entry_nodes,
            "exit_nodes": ga.exit_nodes,
            "depth": ga.phases.len(),
            "max_parallelism": max_parallelism,
            "execution_phases": phase_json,
            "critical_path": {
                "nodes": critical_path,
                "length": critical_path.len(),
                "estimated_ms": critical_ms,
            },
            "speedup": {
                "sequential_ms": sequential_ms,
                "parallel_ms": parallel_ms,
                "estimated_speedup": format!("{:.2}x", speedup),
            },
            "suggestions": suggestions,
        });
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        print_section_header(&format!("Analysis: {}", ga.graph.name));
        println!(
            "  {} nodes, {} edges, {} phases, max parallelism {}",
            ga.graph.nodes.len(),
            ga.edge_count,
            ga.phases.len(),
            max_parallelism
        );
        println!();

        for (i, layer) in ga.phases.iter().enumerate() {
            let tag = if layer.len() > 1 {
                format!("({}x parallel)", layer.len()).green().to_string()
            } else {
                "(sequential)".dimmed().to_string()
            };
            println!(
                "  {} Phase {} {}",
                apxm_core::constants::ui::icons::STARTED.cyan(),
                i + 1,
                tag
            );
            for &id in layer {
                println!(
                    "    {} {} {} [{}ms]",
                    format!("#{id}").dimmed(),
                    ga.node_name(id).bold(),
                    ga.node_op(id).cyan(),
                    ga.node_latency_ms(id)
                );
            }
        }

        println!();
        println!(
            "  {} Critical path: {} nodes, ~{}ms",
            apxm_core::constants::ui::icons::LIGHTNING.yellow(),
            critical_path.len(),
            critical_ms
        );
        println!(
            "  {} Speedup: {:.2}x (sequential {}ms {} parallel {}ms)",
            apxm_core::constants::ui::icons::ROCKET,
            speedup,
            sequential_ms,
            apxm_core::constants::ui::icons::ARROW_RIGHT,
            parallel_ms
        );

        if !suggestions.is_empty() {
            println!();
            println!("  {}", "Suggestions:".bold());
            for s in &suggestions {
                println!(
                    "    {} {s}",
                    apxm_core::constants::ui::icons::BULLET.dimmed()
                );
            }
        }
    }

    Ok(())
}

pub fn explain_command(target: &str, json_output: bool) -> Result<()> {
    // Check if target looks like an error code (e.g., E511, e511, 511)
    let trimmed = target.trim();
    let is_error_code = trimmed.starts_with('E')
        || trimmed.starts_with('e')
        || trimmed.chars().all(|c| c.is_ascii_digit());

    if is_error_code {
        // Extract the numeric part
        let code_str = if trimmed.starts_with('E') || trimmed.starts_with('e') {
            &trimmed[1..]
        } else {
            trimmed
        };

        let code_num: u32 = code_str
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid error code: {}", target))?;

        let error_code = apxm_core::error::ErrorCode::from_u32(code_num)
            .ok_or_else(|| anyhow::anyhow!("Unknown error code: E{}", code_num))?;

        if json_output {
            let output = serde_json::json!({
                "code": error_code.as_str(),
                "component": error_code.component(),
                "is_warning": error_code.is_warning(),
                "documentation_url": error_code.documentation_url(),
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
        } else {
            println!("{}", "=".repeat(70).bright_cyan());
            println!(
                "{} {}",
                "Error Code:".bright_yellow(),
                error_code.as_str().bright_white().bold()
            );
            println!(
                "{} {}",
                "Component:".bright_yellow(),
                error_code.component().bright_white()
            );
            println!(
                "{} {}",
                "Severity:".bright_yellow(),
                if error_code.is_warning() {
                    "Warning".bright_yellow()
                } else {
                    "Error".bright_red()
                }
            );
            println!("{}", "=".repeat(70).bright_cyan());
            println!();

            // Print description based on the error code
            match error_code {
                apxm_core::error::ErrorCode::DeadNode => {
                    println!("{}", "Description:".bright_blue().bold());
                    println!("  A node produces output that is never used (no path to return).");
                    println!();
                    println!("{}", "How to fix:".bright_green().bold());
                    println!("  - Remove the node if it's not needed, OR");
                    println!("  - Connect it to the graph's output path");
                }
                apxm_core::error::ErrorCode::MissingReturnValue => {
                    println!("{}", "Description:".bright_blue().bold());
                    println!("  Graph has no exit node (all nodes have outgoing edges).");
                    println!();
                    println!("{}", "How to fix:".bright_green().bold());
                    println!(
                        "  Ensure at least one node has zero outgoing edges to serve as the return value."
                    );
                }
                apxm_core::error::ErrorCode::CommunicateBeforeSpawn => {
                    println!("{}", "Description:".bright_blue().bold());
                    println!("  COMMUNICATE node does not consume the matching SPAWN_AGENT token.");
                    println!();
                    println!("{}", "Why this is a problem:".bright_blue().bold());
                    println!("  COMMUNICATE is the session transport. It needs a structural Data");
                    println!(
                        "  dependency from SPAWN_AGENT so the spawned session token is an operand."
                    );
                    println!();
                    println!("{}", "How to fix:".bright_green().bold());
                    println!("  Use the agent handle API, or add a Data dependency from the");
                    println!(
                        "  SPAWN_AGENT chain to COMMUNICATE. Control edges are not sufficient."
                    );
                }
                apxm_core::error::ErrorCode::EmptyTemplate => {
                    println!("{}", "Description:".bright_blue().bold());
                    println!(
                        "  ASK/THINK/REASON node has an empty or whitespace-only template_str."
                    );
                    println!();
                    println!("{}", "How to fix:".bright_green().bold());
                    println!("  Provide a non-empty template string.");
                }
                _ => {
                    println!("{}", "Description:".bright_blue().bold());
                    println!("  See documentation for details.");
                }
            }
            println!();
            println!(
                "{} {}",
                "Documentation:".bright_yellow(),
                error_code.documentation_url()
            );
        }

        return Ok(());
    }

    // Otherwise, treat as a workflow file path
    let file = PathBuf::from(target);
    if !ApxmPathFormat::from_path(&file).is_air_source() {
        return Err(anyhow::anyhow!(
            "Explain accepts an error code or canonical .air workflow source. JSON is reserved for structured data outputs."
        ));
    }
    let graph = load_air_graph_for_analysis(&file)?;

    let ga = GraphAnalysis::from_graph(&graph);
    let critical_ms = ga.parallel_ms();
    let max_parallelism = ga.max_parallelism();
    let depth = ga.phases.len();
    let parallelizable = max_parallelism > 1;

    let notable_attrs = |id: u64| -> Vec<(String, String)> {
        let mut attrs = Vec::new();
        if let Some(node) = ga.node_by_id(id) {
            let interesting = [
                "template_str",
                "capability",
                "claim",
                "evidence",
                "token",
                "value",
                "true_label",
                "false_label",
                "key",
                "tokens",
                "flow_name",
                "budget",
                "store",
                "namespace",
                "target_task",
            ];
            for &key in &interesting {
                if let Some(val) = node.attributes.get(key) {
                    let display = match val {
                        apxm_core::types::Value::String(s) => {
                            if s.len() > 60 {
                                format!("{}...", &s[..57])
                            } else {
                                s.clone()
                            }
                        }
                        other => other.to_string(),
                    };
                    attrs.push((key.to_string(), display));
                }
            }
        }
        attrs
    };

    if json_output {
        let phase_json: Vec<serde_json::Value> = ga
            .phases
            .iter()
            .enumerate()
            .map(|(i, layer)| {
                let node_details: Vec<serde_json::Value> = layer
                    .iter()
                    .map(|&id| {
                        let op = ga.node_op(id);
                        let spec = find_op_spec(&op);
                        let required_attrs: Vec<String> = spec
                            .map(|s| {
                                s.fields
                                    .iter()
                                    .filter(|f| f.required)
                                    .map(|f| f.name.to_string())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let feeds: Vec<u64> = ga.successors.get(&id).cloned().unwrap_or_default();
                        let depends_on: Vec<u64> =
                            ga.predecessors.get(&id).cloned().unwrap_or_default();

                        serde_json::json!({
                            "id": id,
                            "name": ga.node_name(id),
                            "op": op,
                            "category": spec.map(|s| category_str(s.category)).unwrap_or("unknown"),
                            "description": spec.map(|s| s.description).unwrap_or(""),
                            "latency": spec.map(|s| s.latency.as_str()).unwrap_or("unknown"),
                            "latency_ms": ga.node_latency_ms(id),
                            "produces_output": spec.map(|s| s.produces_output).unwrap_or(false),
                            "required_attributes": required_attrs,
                            "feeds": feeds,
                            "depends_on": depends_on,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "phase": i + 1,
                    "parallel": layer.len() > 1,
                    "nodes": node_details,
                })
            })
            .collect();

        let result = serde_json::json!({
            "file": file.display().to_string(),
            "workflow_name": ga.graph.name,
            "node_count": ga.graph.nodes.len(),
            "edge_count": ga.edge_count,
            "depth": depth,
            "execution_flow": phase_json,
            "summary": {
                "max_parallelism": max_parallelism,
                "critical_path_steps": depth,
                "estimated_ms": critical_ms,
            },
        });
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        println!();
        println!("  {} {}", "Workflow:".bold().cyan(), ga.graph.name.bold(),);
        println!(
            "  Nodes: {} | Edges: {} | Depth: {}",
            ga.graph.nodes.len(),
            ga.edge_count,
            depth,
        );
        println!();
        println!("  {}", "Execution Flow:".bold().cyan());
        println!(
            "  {}",
            apxm_core::constants::ui::icons::HRULE_DOUBLE
                .repeat(15)
                .dimmed()
        );

        for (i, layer) in ga.phases.iter().enumerate() {
            println!();
            if layer.len() > 1 {
                println!(
                    "  {} ({} parallel):",
                    format!("Phase {}", i + 1).bold(),
                    layer.len(),
                );
            } else {
                println!("  {}:", format!("Phase {}", i + 1).bold());
            }

            for &id in layer {
                let op = ga.node_op(id);
                let spec = find_op_spec(&op);
                let cat = spec.map(|s| category_str(s.category)).unwrap_or("unknown");
                let lat_val = ga.node_latency_ms(id);
                let desc = spec.map(|s| s.description).unwrap_or("");

                println!();
                println!(
                    "    {} \"{}\" {} {} ({}, ~{}ms)",
                    format!("[{}]", id).dimmed(),
                    ga.node_name(id).bold(),
                    apxm_core::constants::ui::icons::EM_DASH.dimmed(),
                    op.cyan().bold(),
                    cat,
                    lat_val,
                );
                println!("        {}", desc.dimmed());

                // Show notable attributes
                for (key, val) in notable_attrs(id) {
                    let label = key.chars().next().unwrap_or(' ').to_uppercase().to_string()
                        + &key[1..].replace('_', " ");
                    println!("        {}: \"{}\"", label.bold(), val);
                }

                // Dependency info
                let deps: Vec<u64> = ga.predecessors.get(&id).cloned().unwrap_or_default();
                let feeds: Vec<u64> = ga.successors.get(&id).cloned().unwrap_or_default();
                if !deps.is_empty() {
                    let dep_strs: Vec<String> = deps.iter().map(|d| d.to_string()).collect();
                    println!(
                        "        {} depends on: [{}]",
                        apxm_core::constants::ui::icons::ARROW_LEFT.dimmed(),
                        dep_strs.join(", "),
                    );
                }
                if !feeds.is_empty() {
                    let feed_strs: Vec<String> = feeds.iter().map(|f| f.to_string()).collect();
                    println!(
                        "        {} feeds: [{}]",
                        apxm_core::constants::ui::icons::ARROW_RIGHT.dimmed(),
                        feed_strs.join(", "),
                    );
                }
            }
        }

        println!();
        println!("  {}", "Summary:".bold().cyan());
        println!(
            "    Max parallelism: {}{}",
            max_parallelism,
            if parallelizable {
                format!(
                    " (phase {})",
                    ga.phases
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| p.len() > 1)
                        .map(|(i, _)| (i + 1).to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            } else {
                String::new()
            },
        );
        println!("    Critical path: {} steps (~{}ms)", depth, critical_ms);
        println!(
            "    Parallelizable: {}",
            if parallelizable { "yes" } else { "no" }
        );
    }

    Ok(())
}
