//! Graph template browsing.

use anyhow::Result;
use colored::Colorize;

use super::cli::*;
use super::implementations::print_section_header;

pub fn template_command(action: TemplateAction, json_output: bool) -> Result<()> {
    struct Template {
        name: &'static str,
        description: &'static str,
        graph_json: &'static str,
    }

    let templates = &[
        Template {
            name: "ask",
            description: "Single LLM call — the simplest possible graph",
            graph_json: r#"{
  "name": "simple-ask",
  "nodes": [
    {"id": 1, "name": "prompt", "op": "ASK", "attributes": {"template_str": "Explain quantum computing in one sentence"}}
  ],
  "edges": [],
  "parameters": [],
  "metadata": {}
}"#,
        },
        Template {
            name: "pipeline",
            description: "Sequential chain — each step feeds the next (draft → review → refine)",
            graph_json: r#"{
  "name": "pipeline",
  "nodes": [
    {"id": 1, "name": "draft", "op": "ASK", "attributes": {"template_str": "Write a short blog post about Rust"}},
    {"id": 2, "name": "review", "op": "THINK", "attributes": {"template_str": "Review this draft for clarity and accuracy: {draft}", "input_names": ["draft"]}},
    {"id": 3, "name": "refine", "op": "ASK", "attributes": {"template_str": "Improve the draft based on this review feedback: {review}", "input_names": ["review"]}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}"#,
        },
        Template {
            name: "fan-out",
            description: "Parallel execution — multiple independent tasks run concurrently then synchronize",
            graph_json: r#"{
  "name": "fan-out",
  "nodes": [
    {"id": 1, "name": "research-a", "op": "ASK", "attributes": {"template_str": "Research topic A"}},
    {"id": 2, "name": "research-b", "op": "ASK", "attributes": {"template_str": "Research topic B"}},
    {"id": 3, "name": "research-c", "op": "ASK", "attributes": {"template_str": "Research topic C"}},
    {"id": 4, "name": "merge", "op": "WAIT_ALL", "attributes": {}}
  ],
  "edges": [
    {"from": 1, "to": 4, "dependency": "Data"},
    {"from": 2, "to": 4, "dependency": "Data"},
    {"from": 3, "to": 4, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}"#,
        },
        Template {
            name: "map-reduce",
            description: "Fan-out then synthesize — parallel work followed by aggregation",
            graph_json: r#"{
  "name": "map-reduce",
  "nodes": [
    {"id": 1, "name": "analyze-1", "op": "ASK", "attributes": {"template_str": "Analyze aspect 1 of the problem"}},
    {"id": 2, "name": "analyze-2", "op": "ASK", "attributes": {"template_str": "Analyze aspect 2 of the problem"}},
    {"id": 3, "name": "analyze-3", "op": "ASK", "attributes": {"template_str": "Analyze aspect 3 of the problem"}},
    {"id": 4, "name": "sync", "op": "WAIT_ALL", "attributes": {}},
    {"id": 5, "name": "synthesize", "op": "ASK", "attributes": {"template_str": "Synthesize all analyses into a final report: {sync}", "input_names": ["sync"]}}
  ],
  "edges": [
    {"from": 1, "to": 4, "dependency": "Data"},
    {"from": 2, "to": 4, "dependency": "Data"},
    {"from": 3, "to": 4, "dependency": "Data"},
    {"from": 4, "to": 5, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}"#,
        },
        Template {
            name: "verify",
            description: "Claim + verification — generate then fact-check",
            graph_json: r#"{
  "name": "verify",
  "nodes": [
    {"id": 1, "name": "generate-evidence", "op": "ASK", "attributes": {"template_str": "How many planets are in the solar system? Answer with a short factual sentence."}},
    {"id": 2, "name": "check", "op": "VERIFY", "attributes": {"claim": "The solar system has eight planets."}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}"#,
        },
        Template {
            name: "conditional",
            description: "Branch on a condition — route to different paths based on comparison",
            graph_json: r#"{
  "name": "conditional",
  "nodes": [
    {"id": 1, "name": "classify", "op": "ASK", "attributes": {"template_str": "Is this a technical question? Answer only 'yes' or 'no'"}},
    {"id": 2, "name": "branch", "op": "BRANCH_ON_VALUE", "attributes": {"value": "yes", "true_label": "3", "false_label": "4"}},
    {"id": 3, "name": "technical-path", "op": "ASK", "attributes": {"template_str": "Give a detailed technical answer"}},
    {"id": 4, "name": "general-path", "op": "ASK", "attributes": {"template_str": "Give a friendly general answer"}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Control"},
    {"from": 2, "to": 4, "dependency": "Control"}
  ],
  "parameters": [],
  "metadata": {}
}"#,
        },
    ];

    let user_templates = load_user_templates();

    match action {
        TemplateAction::List => {
            if json_output {
                let mut items: Vec<serde_json::Value> = templates
                    .iter()
                    .map(|t| serde_json::json!({"name": t.name, "description": t.description}))
                    .collect();
                for ut in &user_templates {
                    items.push(serde_json::json!({"name": ut.name, "description": ut.description, "source": "user"}));
                }
                println!("{}", serde_json::to_string_pretty(&items).unwrap());
            } else {
                print_section_header("Graph Templates");
                for t in templates {
                    println!("  {:<16} {}", t.name.bold(), t.description);
                }
                if !user_templates.is_empty() {
                    println!();
                    println!(
                        "  {} User templates (from ~/.apxm/templates.json):",
                        "~".dimmed()
                    );
                    for ut in &user_templates {
                        println!("  {:<16} {}", ut.name.bold(), ut.description);
                    }
                }
                println!();
                println!(
                    "  Use {} for the full graph JSON",
                    "apxm template show <name>".bold()
                );
            }
        }
        TemplateAction::Show { name } => {
            if let Some(tpl) = templates
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&name))
            {
                if json_output {
                    println!("{}", tpl.graph_json);
                } else {
                    print_section_header(&format!("Template: {}", tpl.name));
                    println!("  {}", tpl.description);
                    println!();
                    println!("{}", tpl.graph_json);
                    println!();
                    println!(
                        "  {} pipe to validate: {} | apxm validate /dev/stdin",
                        apxm_core::constants::ui::icons::INFO.cyan(),
                        format!("apxm template show {} --json", tpl.name).dimmed()
                    );
                }
            } else if let Some(ut) = user_templates
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&name))
            {
                if json_output {
                    println!("{}", ut.graph_json);
                } else {
                    print_section_header(&format!("Template: {} (user)", ut.name));
                    println!("  {}", ut.description);
                    println!();
                    println!("{}", ut.graph_json);
                }
            } else {
                return Err(anyhow::anyhow!(
                    "Unknown template '{}'. Run 'apxm template list' to see available templates.",
                    name
                ));
            }
        }
    }

    Ok(())
}

#[derive(serde::Deserialize)]
struct UserTemplateEntry {
    name: String,
    description: String,
    graph_json: String,
}

fn load_user_templates() -> Vec<UserTemplateEntry> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let templates_path = home.join(".apxm").join("templates.json");
    if !templates_path.exists() {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(&templates_path) else {
        return Vec::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}
