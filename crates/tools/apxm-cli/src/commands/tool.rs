//! External tool/capability registration.

use anyhow::Result;
use colored::Colorize;

use super::agent::{load_tools, save_tools};
use super::cli::*;
use super::implementations::{Status, print_section_header, print_status_line};

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
                println!("Add one with: dekk apxm tool add <name> --description \"...\"");
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
