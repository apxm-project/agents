//! AIR template browsing.

use anyhow::Result;
use colored::Colorize;

use super::cli::*;
use super::implementations::print_section_header;

pub fn template_command(action: TemplateAction, json_output: bool) -> Result<()> {
    struct Template {
        name: &'static str,
        description: &'static str,
        air: &'static str,
    }

    let templates = &[
        Template {
            name: "ask",
            description: "Single LLM call",
            air: r#"module {
  func.func @simple_ask() -> !ais.token attributes {ais.entry} {
    %answer = ais.ask "Explain quantum computing in one sentence" : !ais.token
    func.return %answer : !ais.token
  }
}
"#,
        },
        Template {
            name: "pipeline",
            description: "Sequential chain feeding draft into review and refine",
            air: r#"module {
  func.func @pipeline() -> !ais.token attributes {ais.entry} {
    %draft = ais.ask "Write a short blog post about Rust" : !ais.token
    %review = ais.think "Review this draft for clarity and accuracy: {draft}" [%draft : !ais.token] {input_names = ["draft"]} : !ais.token
    %refine = ais.ask "Improve the draft based on this review feedback: {review}" [%review : !ais.token] {input_names = ["review"]} : !ais.token
    func.return %refine : !ais.token
  }
}
"#,
        },
        Template {
            name: "fan-out",
            description: "Independent work branches synchronized with WAIT_ALL",
            air: r#"module {
  func.func @fan_out() -> !ais.token attributes {ais.entry} {
    %research_a = ais.ask "Research topic A" : !ais.token
    %research_b = ais.ask "Research topic B" : !ais.token
    %research_c = ais.ask "Research topic C" : !ais.token
    %merge = ais.wait_all %research_a, %research_b, %research_c : !ais.token, !ais.token, !ais.token -> !ais.token
    func.return %merge : !ais.token
  }
}
"#,
        },
        Template {
            name: "map-reduce",
            description: "Parallel analysis followed by synthesis",
            air: r#"module {
  func.func @map_reduce() -> !ais.token attributes {ais.entry} {
    %aspect_1 = ais.ask "Analyze aspect 1 of the problem" : !ais.token
    %aspect_2 = ais.ask "Analyze aspect 2 of the problem" : !ais.token
    %aspect_3 = ais.ask "Analyze aspect 3 of the problem" : !ais.token
    %sync = ais.wait_all %aspect_1, %aspect_2, %aspect_3 : !ais.token, !ais.token, !ais.token -> !ais.token
    %report = ais.ask "Synthesize all analyses into a final report: {sync}" [%sync : !ais.token] {input_names = ["sync"]} : !ais.token
    func.return %report : !ais.token
  }
}
"#,
        },
        Template {
            name: "verify",
            description: "Generate evidence and verify a claim",
            air: r#"module {
  func.func @verify_claim() -> !ais.token attributes {ais.entry} {
    %evidence = ais.ask "How many planets are in the solar system? Answer with a short factual sentence." : !ais.token
    %check = ais.verify %evidence : !ais.token vs %evidence : !ais.token with "The solar system has eight planets." : !ais.token
    func.return %check : !ais.token
  }
}
"#,
        },
    ];

    match action {
        TemplateAction::List => {
            if json_output {
                let items: Vec<serde_json::Value> = templates
                    .iter()
                    .map(|t| serde_json::json!({"name": t.name, "description": t.description}))
                    .collect();
                println!("{}", serde_json::to_string_pretty(&items).unwrap());
            } else {
                print_section_header("AIR Templates");
                for t in templates {
                    println!("  {:<16} {}", t.name.bold(), t.description);
                }
                println!();
                println!(
                    "  Use {} to print canonical AIR",
                    "apxm template show <name>".bold()
                );
            }
        }
        TemplateAction::Show { name } => {
            let tpl = templates
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&name))
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Unknown template '{}'. Run 'apxm template list' to see available templates.",
                        name
                    )
                })?;
            if json_output {
                let output = serde_json::json!({
                    "name": tpl.name,
                    "description": tpl.description,
                    "air": tpl.air,
                });
                println!("{}", serde_json::to_string_pretty(&output).unwrap());
            } else {
                print_section_header(&format!("Template: {}", tpl.name));
                println!("  {}", tpl.description);
                println!();
                println!("{}", tpl.air);
            }
        }
    }

    Ok(())
}
