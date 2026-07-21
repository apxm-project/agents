//! External tool/capability registration.

use std::path::PathBuf;

use anyhow::Result;
use colored::Colorize;

use super::cli::*;
use super::dekk_hints;
use super::implementations::{Status, print_section_header, print_status_line};

fn tools_path() -> PathBuf {
    let mut path = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push(".apxm");
    path.push("tools.json");
    path
}

fn load_tools() -> Result<ToolsFile> {
    let path = tools_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ToolsFile::default());
        }
        Err(error) => return Err(anyhow::anyhow!("Failed to read {}: {error}", path.display())),
    };
    let tools_file: ToolsFile = serde_json::from_str(&content)
        .map_err(|error| anyhow::anyhow!("Failed to parse {}: {error}", path.display()))?;
    Ok(tools_file)
}

fn save_tools(tools_file: &ToolsFile) -> Result<()> {
    let path = tools_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| anyhow::anyhow!("Failed to create {}: {error}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(tools_file)
        .map_err(|error| anyhow::anyhow!("Failed to serialize tools: {error}"))?;
    std::fs::write(&path, content)
        .map_err(|error| anyhow::anyhow!("Failed to write {}: {error}", path.display()))?;
    Ok(())
}

pub fn tool_command(action: ToolAction, json_output: bool) -> Result<()> {
    match action {
        ToolAction::List => {
            let tf = load_tools()?;
            if json_output {
                let json = serde_json::to_string_pretty(&tf.tools)
                    .map_err(|e| anyhow::anyhow!("JSON serialization failed: {e}"))?;
                println!("{json}");
                return Ok(());
            }
            if tf.tools.is_empty() {
                println!("No tools registered.");
                println!("Add one with: {}", dekk_hints::TOOL_ADD_WITH_DESCRIPTION);
                return Ok(());
            }
            print_section_header("Registered Tools");
            let max_name = tf.tools.iter().map(|t| t.name.len()).max().unwrap_or(8);
            for tool in &tf.tools {
                println!(
                    "  {:<width$}    {}",
                    tool.name.bold(),
                    tool.description,
                    width = max_name
                );
            }
            println!();
            println!(
                "{} tool{} registered",
                tf.tools.len(),
                if tf.tools.len() == 1 { "" } else { "s" }
            );
        }
        ToolAction::Add {
            name,
            description,
            schema,
        } => {
            let mut tf = load_tools()?;
            if tf.tools.iter().any(|t| t.name == name) {
                return Err(anyhow::anyhow!(
                    "Tool '{}' is already registered. Remove it first with: apxm tool remove {}",
                    name,
                    name
                ));
            }
            tf.tools.push(ToolEntry {
                name: name.clone(),
                description: description.clone(),
                schema,
            });
            save_tools(&tf)?;
            print_section_header("Tool Registered");
            print_status_line(&name, Status::Ok, &description);
        }
        ToolAction::Remove { name } => {
            let mut tf = load_tools()?;
            let before = tf.tools.len();
            tf.tools.retain(|t| t.name != name);
            if tf.tools.len() == before {
                return Err(anyhow::anyhow!("Tool '{}' not found", name));
            }
            save_tools(&tf)?;
            print_section_header("Tool Removed");
            print_status_line(&name, Status::Ok, "removed");
        }
    }
    Ok(())
}
