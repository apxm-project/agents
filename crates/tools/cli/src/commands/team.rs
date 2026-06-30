//! Agent team management.

use anyhow::Result;
use colored::Colorize;

use super::cli::*;
use super::implementations::print_section_header;

pub fn team_command(action: TeamAction, json_output: bool) -> Result<()> {
    use apxm_runtime::team::TeamRegistry;

    match action {
        TeamAction::List => {
            let registry = TeamRegistry::load_from_default_path();
            let teams = registry.list();

            if json_output {
                let team_list: Vec<serde_json::Value> = teams
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "name": t.name,
                            "description": t.description,
                            "members": t.members.len(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&team_list)?);
                return Ok(());
            }

            if teams.is_empty() {
                println!("No teams defined in ~/.apxm/teams.toml.");
                println!("Copy docs/examples/teams.toml to ~/.apxm/teams.toml to get started.");
                return Ok(());
            }

            print_section_header("Agent Teams");
            for team in &teams {
                println!("  {} - {}", team.name.bold(), team.description);
                println!("    Members: {}", team.members.len());
                for member in &team.members {
                    println!("      • {} ({})", member.role, member.profile);
                }
                println!();
            }
            println!("Use 'apxm team show <name>' to view full team configuration.");
            Ok(())
        }

        TeamAction::Show { name } => {
            let registry = TeamRegistry::load_from_default_path();
            let team = registry.get(&name).ok_or_else(|| {
                anyhow::anyhow!(
                    "Team '{}' not found. Use 'apxm team list' to see available teams.",
                    name
                )
            })?;

            if json_output {
                println!("{}", serde_json::to_string_pretty(&team)?);
                return Ok(());
            }

            print_section_header(&format!("Team: {}", team.name));
            println!("  Description: {}", team.description);
            println!("\n  Members:");
            for member in &team.members {
                println!("\n    Role:    {}", member.role.bold());
                println!("    Profile: {}", member.profile);
                if let Some(ref prompt) = member.system_prompt {
                    let line_count = prompt.lines().count();
                    println!("    Prompt:  {}", prompt.lines().next().unwrap_or(""));
                    if line_count > 1 {
                        println!("             (+ {} more lines)", line_count - 1);
                    }
                }
            }
            println!();
            Ok(())
        }

        TeamAction::Add {
            team,
            role,
            profile,
            system_prompt,
        } => {
            let mut registry = TeamRegistry::load_from_default_path();
            registry.add_member(&team, role.clone(), profile.clone(), system_prompt)?;

            println!(
                "Added member '{}' (profile: {}) to team '{}'.",
                role, profile, team
            );
            println!("Team definition saved to ~/.apxm/teams.toml");
            Ok(())
        }
    }
}
