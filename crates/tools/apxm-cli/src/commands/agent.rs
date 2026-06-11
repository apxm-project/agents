//! Agent profile management (ACP).

use std::path::PathBuf;

use anyhow::Result;
use colored::Colorize;

use super::cli::*;
use super::implementations::{Status, print_section_header, print_status_line};

pub fn tools_path() -> PathBuf {
    let mut p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push(".apxm");
    p.push("tools.json");
    p
}
pub(super) fn load_tools() -> Result<ToolsFile> {
    let path = tools_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ToolsFile::default()),
        Err(e) => return Err(anyhow::anyhow!("Failed to read {}: {e}", path.display())),
    };
    let tf: ToolsFile = serde_json::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse {}: {e}", path.display()))?;
    Ok(tf)
}

pub(super) fn save_tools(tf: &ToolsFile) -> Result<()> {
    let path = tools_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| anyhow::anyhow!("Failed to create {}: {e}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(tf)
        .map_err(|e| anyhow::anyhow!("Failed to serialize tools: {e}"))?;
    std::fs::write(&path, content)
        .map_err(|e| anyhow::anyhow!("Failed to write {}: {e}", path.display()))?;
    Ok(())
}
pub async fn agent_command(action: AgentAction, json_output: bool) -> Result<()> {
    use apxm_acp::constants::registry::{json_keys, sources};
    match action {
        AgentAction::List => {
            let reg = apxm_acp::AgentRegistry::load();
            let list = reg.list();
            if json_output {
                let entries: Vec<serde_json::Value> = list
                    .iter()
                    .map(|(name, profile, from_template)| {
                        serde_json::json!({
                            (json_keys::NAME): name,
                            (json_keys::COMMAND): profile.command,
                            (json_keys::DESCRIPTION): profile.description,
                            (json_keys::ROUTE_CAPABILITIES): profile.route_capabilities,
                            (json_keys::DEFAULT_MODE): profile.default_mode,
                            (json_keys::DEFAULT_MODEL): profile.default_model,
                            (json_keys::SOURCE): if *from_template { sources::TEMPLATE } else { sources::USER_PROFILE },
                            (json_keys::CLOSE_GRACE_MS): profile.close_grace_ms,
                            (json_keys::SESSION_CREATE_TIMEOUT_MS): profile.session_create_timeout_ms,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&entries)
                        .map_err(|e| anyhow::anyhow!("JSON: {e}"))?
                );
                return Ok(());
            }

            if list.is_empty() {
                println!(
                    "No ACP agent profiles available. Run 'apxm agent templates' or add a profile with 'apxm agent add <agent>'."
                );
                return Ok(());
            }

            print_section_header("Available ACP Agent Profiles");
            let max_name = list.iter().map(|(n, _, _)| n.len()).max().unwrap_or(8);
            let max_source = sources::USER_PROFILE.len();
            println!(
                "  {:<name_w$}  {:<src_w$}  {}",
                "AGENT",
                "SOURCE",
                "COMMAND",
                name_w = max_name,
                src_w = max_source,
            );
            for (name, profile, from_template) in &list {
                let source = if *from_template {
                    sources::TEMPLATE
                } else {
                    sources::USER_PROFILE
                };
                println!(
                    "  {:<name_w$}  {:<src_w$}  {}",
                    name.bold(),
                    source.dimmed(),
                    profile.command,
                    name_w = max_name,
                    src_w = max_source,
                );
            }
            println!();
            println!(
                "{} profile{} available",
                list.len(),
                if list.len() == 1 { "" } else { "s" }
            );
        }
        AgentAction::Add {
            name,
            command,
            permissions,
            close_grace_ms,
            sandbox,
            no_test,
        } => {
            use apxm_acp::constants::timeouts as acp_timeouts;

            let mut reg = apxm_acp::AgentRegistry::load();

            let profile = match command {
                Some(cmd) => apxm_acp::AcpAgentProfile {
                    command: cmd,
                    description: None,
                    close_grace_ms: close_grace_ms.unwrap_or(acp_timeouts::DEFAULT_CLOSE_GRACE_MS),
                    session_create_timeout_ms: acp_timeouts::DEFAULT_SESSION_TIMEOUT_MS,
                    permission_mode: permissions,
                    env: Default::default(),
                    default_mode: None,
                    default_model: None,
                    route_capabilities: apxm_acp::default_route_capabilities(),
                    system_prompt: None,
                    skip_preamble: false,
                    capabilities: Vec::new(),
                    sandbox,
                },
                None => {
                    let mut profile = reg
                        .get_template(&name)
                        .ok_or_else(|| {
                            anyhow::anyhow!("Unknown template '{name}'. Run: apxm agent templates")
                        })?
                        .clone();
                    // Apply overrides
                    profile.permission_mode = permissions;
                    if let Some(grace) = close_grace_ms {
                        profile.close_grace_ms = grace;
                    }
                    profile.sandbox = sandbox;
                    profile
                }
            };

            // Spawn test: verify the agent is reachable before persisting
            if !no_test {
                println!("Testing agent '{}'...", name.bold());
                println!("  Command: {}", profile.command);
                let cwd = std::env::current_dir().unwrap_or_default();
                let start = std::time::Instant::now();
                let aam_context = apxm_core::types::aam::AamContext::default();
                match apxm_acp::AcpSession::spawn(&name, &profile, &cwd, &aam_context, None).await {
                    Ok(session) => {
                        let elapsed = start.elapsed();
                        println!(
                            "  {}",
                            format!("Connected in {:.1}s", elapsed.as_secs_f64()).green()
                        );
                        session.close().await;
                    }
                    Err(e) => {
                        print_status_line(&name, Status::Error, &format!("{e}"));
                        return Err(anyhow::anyhow!(
                            "Agent '{}' is not reachable. Is the tool installed?\n\
                             Use --no-test to register without testing.",
                            name
                        ));
                    }
                }
            }

            let display_cmd = profile.command.clone();
            reg.add(name.clone(), profile)
                .map_err(|e| anyhow::anyhow!("Failed to save agent profile: {e}"))?;
            print_section_header("Agent Profile Saved");
            print_status_line(&name, Status::Ok, &display_cmd);
        }
        AgentAction::Remove { name } => {
            let mut reg = apxm_acp::AgentRegistry::load();
            match reg.remove(&name) {
                Ok(true) => {
                    print_section_header("Agent Removed");
                    print_status_line(&name, Status::Ok, "removed");
                }
                Ok(false) => {
                    return Err(anyhow::anyhow!(
                        "Agent '{name}' not found. Run: apxm agent list"
                    ));
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to remove agent: {e}"));
                }
            }
        }
        AgentAction::Test { name } => {
            let reg = apxm_acp::AgentRegistry::load();
            let profile = reg.get(&name).ok_or_else(|| {
                anyhow::anyhow!("Agent profile '{name}' is not available. Run: apxm agent list")
            })?;
            println!("Testing agent '{}'...", name.bold());
            println!("  Command: {}", profile.command);
            let cwd = std::env::current_dir().unwrap_or_default();
            let start = std::time::Instant::now();
            let aam_context = apxm_core::types::aam::AamContext::default();
            match apxm_acp::AcpSession::spawn(&name, profile, &cwd, &aam_context, None).await {
                Ok(session) => {
                    let elapsed = start.elapsed();
                    println!("  Session ID: {}", session.session_id());
                    if let Some(agent_sid) = session.agent_session_id() {
                        println!("  Agent Session ID: {agent_sid}");
                    }
                    println!(
                        "  {}",
                        format!("Connected in {:.1}s", elapsed.as_secs_f64()).green()
                    );
                    session.close().await;
                    print_status_line(&name, Status::Ok, "agent reachable");
                }
                Err(e) => {
                    print_status_line(&name, Status::Error, &format!("{e}"));
                    return Err(anyhow::anyhow!("Agent test failed: {e}"));
                }
            }
        }
        AgentAction::Templates => {
            let reg = apxm_acp::AgentRegistry::load();
            let templates = reg.list_templates();
            if json_output {
                let entries: Vec<serde_json::Value> = templates
                    .iter()
                    .map(|(name, profile)| {
                        serde_json::json!({
                            (json_keys::NAME): name,
                            (json_keys::COMMAND): profile.command,
                            (json_keys::DESCRIPTION): profile.description,
                            (json_keys::ROUTE_CAPABILITIES): profile.route_capabilities,
                            (json_keys::DEFAULT_MODE): profile.default_mode,
                            (json_keys::DEFAULT_MODEL): profile.default_model,
                            (json_keys::SOURCE): sources::TEMPLATE,
                            (json_keys::CLOSE_GRACE_MS): profile.close_grace_ms,
                            (json_keys::SESSION_CREATE_TIMEOUT_MS): profile.session_create_timeout_ms,
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&entries)
                        .map_err(|e| anyhow::anyhow!("JSON: {e}"))?
                );
                return Ok(());
            }

            print_section_header("Built-in Agent Templates");
            let max_name = templates.iter().map(|(n, _)| n.len()).max().unwrap_or(8);
            println!(
                "  {:<name_w$}  {:<8}  {}",
                "TEMPLATE",
                "TIMEOUT",
                "COMMAND",
                name_w = max_name,
            );
            for (name, profile) in &templates {
                println!(
                    "  {:<name_w$}  {:<8}  {}",
                    name.bold(),
                    format!("{}ms", profile.session_create_timeout_ms).dimmed(),
                    profile.command,
                    name_w = max_name,
                );
            }
            println!();
            println!(
                "{} templates available. Save a user profile with: apxm agent add <name>",
                templates.len()
            );
        }
    }
    Ok(())
}
