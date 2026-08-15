//! Validate, analyze, explain commands for canonical `apxm.air`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use colored::Colorize;

use super::air_ops::{category_str, find_op_spec, op_latency_ms};
use super::implementations::{Status, print_section_header, print_status_line};
use apxm_core::types::ApxmPathFormat;
use apxm_program::air::{AirModule, SemanticOp, StructuralNode};

pub fn validate_command(
    input: PathBuf,
    json_output: bool,
    _no_check_resources: bool,
) -> Result<()> {
    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    if ApxmPathFormat::from_path(&input).is_air_source() {
        match load_air_module_for_analysis(&input) {
            Ok(module) => {
                let verdict = module.verify();
                if !verdict.is_accepted() {
                    errors.extend(
                        verdict
                            .into_diagnostics()
                            .into_iter()
                            .map(|diagnostic| diagnostic.message),
                    );
                }
                if module.semantic_operations.is_empty() {
                    warnings.push("canonical AIR contains no semantic operations".to_string());
                }
            }
            Err(err) => errors.push(err.to_string()),
        }
    } else {
        errors.push(
            "workflow source must be canonical .air JSON; other JSON is reserved for structured data outputs"
                .to_string(),
        );
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
        for warning in &warnings {
            println!(
                "  {} {}",
                apxm_core::constants::ui::icons::CAUTION.yellow(),
                warning
            );
        }
    } else {
        print_status_line(&input.display().to_string(), Status::Error, "invalid");
        for error in &errors {
            println!(
                "  {} {}",
                apxm_core::constants::ui::icons::FAILED.red(),
                error
            );
        }
        for warning in &warnings {
            println!(
                "  {} {}",
                apxm_core::constants::ui::icons::CAUTION.yellow(),
                warning
            );
        }
        return Err(anyhow::anyhow!("{} error(s) found", errors.len()));
    }

    Ok(())
}

fn load_air_module_for_analysis(input: &PathBuf) -> Result<AirModule> {
    let text = std::fs::read_to_string(input)
        .with_context(|| format!("failed to read canonical AIR from {}", input.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("{} must contain canonical apxm.air JSON", input.display()))
}

/// Parsed canonical AIR sequence for analyze and explain commands.
pub(crate) struct GraphAnalysis<'a> {
    pub(crate) graph: &'a AirModule,
    pub(crate) phases: Vec<Vec<usize>>,
}

impl<'a> GraphAnalysis<'a> {
    pub(crate) fn from_graph(graph: &'a AirModule) -> Self {
        let phases = graph
            .semantic_operations
            .iter()
            .enumerate()
            .map(|(index, _)| vec![index])
            .collect();
        Self { graph, phases }
    }

    fn op_by_index(&self, index: usize) -> Option<&SemanticOp> {
        self.graph.semantic_operations.get(index)
    }

    pub(crate) fn node_op(&self, index: usize) -> String {
        self.op_by_index(index)
            .map_or_else(|| "?".to_string(), |op| op.op.wire().to_string())
    }

    pub(crate) fn node_id(&self, index: usize) -> &str {
        self.op_by_index(index)
            .map_or("?", |op| op.node_id.as_str())
    }

    fn node_latency_ms(&self, index: usize) -> u64 {
        op_latency_ms(&self.node_op(index))
    }

    pub(crate) fn max_parallelism(&self) -> usize {
        self.phases.iter().map(Vec::len).max().unwrap_or(1)
    }

    pub(crate) fn parallel_ms(&self) -> u64 {
        self.phases
            .iter()
            .map(|layer| {
                layer
                    .iter()
                    .map(|&index| self.node_latency_ms(index))
                    .max()
                    .unwrap_or(0)
            })
            .sum()
    }

    pub(crate) fn sequential_ms(&self) -> u64 {
        (0..self.graph.semantic_operations.len())
            .map(|index| self.node_latency_ms(index))
            .sum()
    }

    pub(crate) fn speedup(&self) -> f64 {
        let parallel = self.parallel_ms();
        if parallel > 0 {
            self.sequential_ms() as f64 / parallel as f64
        } else {
            1.0
        }
    }

    pub(crate) fn critical_path(&self) -> (Vec<String>, u64) {
        let path: Vec<String> = self
            .graph
            .semantic_operations
            .iter()
            .map(|op| op.node_id.clone())
            .collect();
        let ms = self.sequential_ms();
        (path, ms)
    }

    fn structural_nodes(&self) -> &[StructuralNode] {
        &self.graph.structural_ir
    }
}

pub fn analyze_command(input: PathBuf, json_output: bool) -> Result<()> {
    if !ApxmPathFormat::from_path(&input).is_air_source() {
        return Err(anyhow::anyhow!(
            "Analyze accepts canonical .air JSON. Other JSON is reserved for structured data outputs."
        ));
    }
    let graph = load_air_module_for_analysis(&input)?;
    let ga = GraphAnalysis::from_graph(&graph);
    let (critical_path, critical_ms) = ga.critical_path();
    let sequential_ms = ga.sequential_ms();
    let parallel_ms = ga.parallel_ms();
    let max_parallelism = ga.max_parallelism();
    let speedup = ga.speedup();
    let suggestions = vec![
        "Canonical apxm.air analysis is derived from semantic_operations and structural_ir; legacy ExecutionDag analysis is not used."
            .to_string(),
    ];

    if json_output {
        let phase_json: Vec<serde_json::Value> = ga
            .phases
            .iter()
            .enumerate()
            .map(|(i, layer)| {
                let node_details: Vec<serde_json::Value> = layer
                    .iter()
                    .map(|&index| {
                        serde_json::json!({
                            "id": ga.node_id(index),
                            "op": ga.node_op(index),
                            "latency_ms": ga.node_latency_ms(index),
                        })
                    })
                    .collect();
                serde_json::json!({
                    "phase": i + 1,
                    "parallel": layer.len() > 1,
                    "parallelism_degree": layer.len(),
                    "estimated_ms": layer.iter().map(|&index| ga.node_latency_ms(index)).max().unwrap_or(0),
                    "nodes": node_details,
                })
            })
            .collect();

        let result = serde_json::json!({
            "file": input.display().to_string(),
            "schema_version": "apxm.air",
            "semantic_operation_count": ga.graph.semantic_operations.len(),
            "structural_ir_count": ga.graph.structural_ir.len(),
            "depth": ga.phases.len(),
            "max_parallelism": max_parallelism,
            "execution_phases": phase_json,
            "structural_ir": ga.structural_nodes().iter().map(|node| {
                serde_json::json!({"region_id": node.region_id, "kind": node.kind.wire()})
            }).collect::<Vec<_>>(),
            "critical_path": {
                "nodes": critical_path,
                "length": ga.graph.semantic_operations.len(),
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
        print_section_header("Canonical AIR Analysis");
        println!(
            "  {} semantic operation(s), {} structural region(s), {} phase(s)",
            ga.graph.semantic_operations.len(),
            ga.graph.structural_ir.len(),
            ga.phases.len(),
        );
        println!();

        for (i, layer) in ga.phases.iter().enumerate() {
            println!(
                "  {} Phase {} {}",
                apxm_core::constants::ui::icons::STARTED.cyan(),
                i + 1,
                "(semantic operation)".dimmed()
            );
            for &index in layer {
                println!(
                    "    {} {} [{}ms]",
                    ga.node_id(index).bold(),
                    ga.node_op(index).cyan(),
                    ga.node_latency_ms(index)
                );
            }
        }

        if !ga.graph.structural_ir.is_empty() {
            println!();
            println!("  {}", "Structural IR:".bold());
            for node in ga.structural_nodes() {
                println!(
                    "    {} {}",
                    node.region_id.as_str().bold(),
                    node.kind.wire().cyan()
                );
            }
        }

        println!();
        println!(
            "  {} Critical path: {} operation(s), ~{}ms",
            apxm_core::constants::ui::icons::LIGHTNING.yellow(),
            ga.graph.semantic_operations.len(),
            critical_ms
        );
    }

    Ok(())
}

pub fn explain_command(target: &str, json_output: bool) -> Result<()> {
    let trimmed = target.trim();
    let is_error_code = trimmed.starts_with('E')
        || trimmed.starts_with('e')
        || trimmed.chars().all(|c| c.is_ascii_digit());

    if is_error_code {
        return explain_error_code(target, json_output);
    }

    let file = PathBuf::from(target);
    if !ApxmPathFormat::from_path(&file).is_air_source() {
        return Err(anyhow::anyhow!(
            "Explain accepts an error code or canonical .air JSON."
        ));
    }
    let graph = load_air_module_for_analysis(&file)?;
    let ga = GraphAnalysis::from_graph(&graph);

    if json_output {
        let operations: Vec<serde_json::Value> = graph
            .semantic_operations
            .iter()
            .map(|op| {
                serde_json::json!({
                    "node_id": op.node_id,
                    "op": op.op.wire(),
                    "operands": op.operands,
                })
            })
            .collect();
        let structural_ir: Vec<serde_json::Value> = graph
            .structural_ir
            .iter()
            .map(|node| {
                serde_json::json!({
                    "region_id": node.region_id,
                    "kind": node.kind.wire(),
                })
            })
            .collect();
        let result = serde_json::json!({
            "file": file.display().to_string(),
            "schema_version": "apxm.air",
            "semantic_operations": operations,
            "structural_ir": structural_ir,
            "summary": {
                "semantic_operation_count": graph.semantic_operations.len(),
                "structural_ir_count": graph.structural_ir.len(),
                "estimated_ms": ga.parallel_ms(),
            },
        });
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        println!();
        println!("  {} {}", "AIR:".bold().cyan(), "apxm.air".bold());
        println!(
            "  Semantic operations: {} | Structural regions: {}",
            graph.semantic_operations.len(),
            graph.structural_ir.len(),
        );
        println!();
        println!("  {}", "Execution Flow:".bold().cyan());

        for (index, op) in graph.semantic_operations.iter().enumerate() {
            let op_name = op.op.wire();
            let spec = find_op_spec(op_name);
            let category = spec.map_or("unknown", |s| category_str(s.category));
            let desc = spec.map_or("", |s| s.description);
            println!();
            println!(
                "    {} \"{}\" {} {} ({}, ~{}ms)",
                format!("[{}]", index + 1).dimmed(),
                op.node_id.bold(),
                apxm_core::constants::ui::icons::EM_DASH.dimmed(),
                op_name.cyan().bold(),
                category,
                ga.node_latency_ms(index),
            );
            if !desc.is_empty() {
                println!("        {}", desc.dimmed());
            }
            for operand in &op.operands {
                println!(
                    "        {}: {} ({})",
                    operand.slot.bold(),
                    operand.value_id,
                    operand.type_ref,
                );
            }
        }

        if !graph.structural_ir.is_empty() {
            println!();
            println!("  {}", "Structural IR:".bold().cyan());
            for node in &graph.structural_ir {
                println!(
                    "    {} {}",
                    node.region_id.as_str().bold(),
                    node.kind.wire().cyan()
                );
            }
        }
    }

    Ok(())
}

fn explain_error_code(target: &str, json_output: bool) -> Result<()> {
    let trimmed = target.trim();
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
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

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
    Ok(())
}
