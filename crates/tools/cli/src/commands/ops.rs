//! Browse AIS operations.

use std::{collections::HashMap, io, path::Path};

use anyhow::Result;
use colored::Colorize;

use super::cli::*;
use super::implementations::category_str;

pub fn ops_command(action: OpsAction, json_output: bool) -> Result<()> {
    use apxm_core::types::{AIS_OPERATIONS, OperationCategory};

    fn parse_category(s: &str) -> Option<OperationCategory> {
        match s.to_lowercase().as_str() {
            "metadata" => Some(OperationCategory::Metadata),
            "memory" => Some(OperationCategory::Memory),
            "reasoning" | "llm" => Some(OperationCategory::Reasoning),
            "tools" | "tool" => Some(OperationCategory::Tools),
            "control_flow" | "controlflow" | "control" => Some(OperationCategory::ControlFlow),
            "synchronization" | "sync" => Some(OperationCategory::Synchronization),
            "error_handling" | "error" | "errorhandling" => Some(OperationCategory::ErrorHandling),
            "communication" | "comm" => Some(OperationCategory::Communication),
            "internal" => Some(OperationCategory::Internal),
            _ => None,
        }
    }

    match action {
        OpsAction::List { category } => {
            let cat_filter = match &category {
                Some(c) => Some(parse_category(c).ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unknown category '{}'. Valid: metadata, memory, reasoning, tools, \
                         control_flow, synchronization, error_handling, communication, internal",
                        c
                    )
                })?),
                None => None,
            };

            let ops: Vec<_> = AIS_OPERATIONS
                .iter()
                .filter(|s| cat_filter.is_none_or(|c| s.category == c))
                .collect();

            if json_output {
                let json_ops: Vec<serde_json::Value> = ops
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "op": s.op_type.to_string(),
                            "name": s.name,
                            "category": category_str(s.category),
                            "description": s.description,
                            "latency": s.latency.as_str(),
                            "produces_output": s.produces_output,
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&json_ops).unwrap());
            } else {
                let mut current_cat = None;
                for spec in &ops {
                    if current_cat != Some(spec.category) {
                        current_cat = Some(spec.category);
                        println!();
                        println!(
                            "  {}",
                            category_str(spec.category).to_uppercase().bold().cyan()
                        );
                        println!(
                            "  {}",
                            apxm_core::constants::ui::icons::HRULE.repeat(40).dimmed()
                        );
                    }
                    println!(
                        "  {:<18} {}  {}",
                        spec.op_type.to_string().bold(),
                        format!("[{}]", spec.latency.as_str()).dimmed(),
                        spec.description
                    );
                }
                println!();
                println!("  {} operations total", ops.len());
                println!("  Use {} for details", "apxm ops show <OP>".bold());
            }
        }
        OpsAction::Show { name } => {
            let name_upper = name.to_uppercase();
            let spec = AIS_OPERATIONS
                .iter()
                .find(|s| s.op_type.to_string() == name_upper || s.name.eq_ignore_ascii_case(&name))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unknown operation '{}'. Run 'apxm ops list' to see all operations.",
                        name
                    )
                })?;

            if json_output {
                let required: Vec<serde_json::Value> = spec
                    .fields
                    .iter()
                    .filter(|f| f.required)
                    .map(|f| serde_json::json!({"name": f.name, "description": f.description}))
                    .collect();
                let optional: Vec<serde_json::Value> = spec
                    .fields
                    .iter()
                    .filter(|f| !f.required)
                    .map(|f| serde_json::json!({"name": f.name, "description": f.description}))
                    .collect();

                let mut result = serde_json::json!({
                    "op": spec.op_type.to_string(),
                    "name": spec.name,
                    "category": category_str(spec.category),
                    "description": spec.description,
                    "long_description": spec.long_description,
                    "latency": spec.latency.as_str(),
                    "required_fields": required,
                    "optional_fields": optional,
                    "produces_output": spec.produces_output,
                    "needs_submission": spec.needs_submission,
                    "min_inputs": spec.min_inputs,
                });

                if let Some(example) = spec.example_json {
                    result["example"] = serde_json::Value::String(example.to_string());
                }

                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else {
                println!();
                println!(
                    "  {} {}",
                    spec.op_type.to_string().bold().cyan(),
                    spec.name.dimmed()
                );
                println!(
                    "  {}",
                    apxm_core::constants::ui::icons::HRULE.repeat(50).dimmed()
                );
                println!("  {}", spec.description);
                println!();
                println!("  {}", spec.long_description);
                println!();
                println!(
                    "  {} {}  {} {}  {} {}",
                    "Category:".bold(),
                    category_str(spec.category),
                    "Latency:".bold(),
                    spec.latency.as_str(),
                    "Output:".bold(),
                    if spec.produces_output { "yes" } else { "no" }
                );

                let required_fields: Vec<_> = spec.fields.iter().filter(|f| f.required).collect();
                let optional_fields: Vec<_> = spec.fields.iter().filter(|f| !f.required).collect();

                if !required_fields.is_empty() {
                    println!();
                    println!("  {}", "Required Fields:".bold());
                    for f in &required_fields {
                        println!("    {} {}", f.name.green().bold(), f.description.dimmed());
                    }
                }

                if !optional_fields.is_empty() {
                    println!();
                    println!("  {}", "Optional Fields:".bold());
                    for f in &optional_fields {
                        println!("    {} {}", f.name.yellow(), f.description.dimmed());
                    }
                }

                if let Some(example) = spec.example_json {
                    println!();
                    println!("  {}", "Example Node JSON:".bold());
                    println!("  {}", example);
                }
                println!();
            }
        }
        OpsAction::Usage => {
            let paths = apxm_core::paths::ApxmPaths::discover()
                .map_err(|e| anyhow::anyhow!("Failed to resolve APXM paths: {e}"))?;
            let usage = read_persisted_operation_usage(&paths)
                .map_err(|e| anyhow::anyhow!("Failed to read op-usage stats: {e}"))?;

            let total_ops = AIS_OPERATIONS.len();
            let mut rows: Vec<(String, u64)> = usage.into_iter().collect();
            rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let used_count = rows.len();

            if json_output {
                let json_rows: Vec<serde_json::Value> = rows
                    .iter()
                    .map(|(op, count)| serde_json::json!({"op": op, "count": count}))
                    .collect();
                let output = serde_json::json!({
                    "total_operations_defined": total_ops,
                    "operations_used": used_count,
                    "operations_never_used": total_ops.saturating_sub(used_count),
                    "usage": json_rows,
                });
                println!("{}", serde_json::to_string_pretty(&output)?);
            } else {
                println!();
                println!(
                    "  {} of {} defined operations have recorded dispatches",
                    used_count.to_string().bold(),
                    total_ops
                );
                println!(
                    "  {}",
                    apxm_core::constants::ui::icons::HRULE.repeat(40).dimmed()
                );
                if rows.is_empty() {
                    println!(
                        "  No usage recorded yet. Run {} or {} to populate this.",
                        "apxm execute".bold(),
                        "apxm run".bold()
                    );
                } else {
                    for (op, count) in &rows {
                        println!("  {:<20} {}", op.bold(), count);
                    }
                }
                println!();
                println!(
                    "  Use {} to see the full defined operation catalog.",
                    "apxm ops list".bold()
                );
            }
        }
    }
    Ok(())
}

const OP_USAGE_FILE_NAME: &str = "op-usage.json";

fn read_persisted_operation_usage(
    paths: &apxm_core::paths::ApxmPaths,
) -> io::Result<HashMap<String, u64>> {
    let dir = paths.cache_component_dir("op-usage")?;
    read_persisted_operation_usage_from_dir(&dir)
}

fn read_persisted_operation_usage_from_dir(dir: &Path) -> io::Result<HashMap<String, u64>> {
    let path = dir.join(OP_USAGE_FILE_NAME);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_operation_usage_missing_file_is_empty() {
        let temp = tempfile::tempdir().expect("tempdir");

        let usage = read_persisted_operation_usage_from_dir(temp.path()).expect("usage");

        assert!(usage.is_empty());
    }

    #[test]
    fn persisted_operation_usage_rejects_invalid_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("op-usage.json"), "not json").expect("write");

        let err = read_persisted_operation_usage_from_dir(temp.path()).expect_err("invalid json");

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
