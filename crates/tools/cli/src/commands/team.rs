//! Agent team management.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use colored::Colorize;
use serde::{Deserialize, Serialize};

use super::cli::*;
use super::implementations::print_section_header;

pub fn team_command(action: TeamAction, json_output: bool) -> Result<()> {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TeamMember {
    role: String,
    profile: String,
    #[serde(default)]
    system_prompt: Option<String>,
}

/// A team document declares its roster under exactly one key: `members`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamDefinition {
    name: String,
    description: String,
    #[serde(default)]
    members: Vec<TeamMember>,
}

struct TeamRegistry {
    teams: HashMap<String, TeamDefinition>,
}

impl TeamRegistry {
    fn load_from_default_path() -> Self {
        Self::user_config_path().map_or_else(Self::empty, |path| Self::load_from_path(&path))
    }

    fn load_from_path(path: &Path) -> Self {
        let mut teams = HashMap::new();

        if let Ok(content) = std::fs::read_to_string(path)
            && let Ok(file) = toml::from_str::<TeamsFile>(&content)
        {
            for team in file.team {
                teams.insert(team.name.clone(), team);
            }
        }

        Self { teams }
    }

    fn empty() -> Self {
        Self {
            teams: HashMap::new(),
        }
    }

    fn get(&self, name: &str) -> Option<&TeamDefinition> {
        self.teams.get(name)
    }

    fn list(&self) -> Vec<&TeamDefinition> {
        self.teams.values().collect()
    }

    fn add_member(
        &mut self,
        team_name: &str,
        role: String,
        profile: String,
        system_prompt: Option<String>,
    ) -> Result<(), std::io::Error> {
        let path = Self::user_config_path()
            .ok_or_else(|| std::io::Error::other("cannot determine home directory"))?;
        self.add_member_to_path(team_name, role, profile, system_prompt, &path)
    }

    fn add_member_to_path(
        &mut self,
        team_name: &str,
        role: String,
        profile: String,
        system_prompt: Option<String>,
        path: &Path,
    ) -> Result<(), std::io::Error> {
        let team = self
            .teams
            .get_mut(team_name)
            .ok_or_else(|| std::io::Error::other(format!("Team '{team_name}' not found")))?;

        if team.members.iter().any(|member| member.role == role) {
            return Err(std::io::Error::other(format!(
                "Role '{role}' already exists in team '{team_name}'"
            )));
        }

        team.members.push(TeamMember {
            role,
            profile,
            system_prompt,
        });

        self.persist_to_path(path)
    }

    fn user_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|home| home.join(".apxm").join("teams.toml"))
    }

    fn persist_to_path(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut teams: Vec<TeamDefinition> = self.teams.values().cloned().collect();
        teams.sort_by(|a, b| a.name.cmp(&b.name));

        let file = TeamsFile { team: teams };
        let content = toml::to_string_pretty(&file)
            .map_err(|err| std::io::Error::other(format!("serialize: {err}")))?;

        std::fs::write(path, format!("# APXM Team Definitions\n\n{content}"))?;
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TeamsFile {
    #[serde(default)]
    team: Vec<TeamDefinition>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_registry_reads_members_from_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("teams.toml");
        std::fs::write(
            &path,
            r#"
[[team]]
name = "core"
description = "Core team"

[[team.members]]
role = "driver"
profile = "canonical-driver"
"#,
        )
        .expect("write teams");

        let registry = TeamRegistry::load_from_path(&path);
        let team = registry.get("core").expect("team");

        assert_eq!(team.members.len(), 1);
        assert_eq!(team.members[0].role, "driver");
        assert_eq!(team.members[0].profile, "canonical-driver");
    }

    #[test]
    fn team_registry_rejects_singular_member_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("teams.toml");
        std::fs::write(
            &path,
            r#"
[[team]]
name = "core"
description = "Core team"

[[team.member]]
role = "driver"
profile = "canonical-driver"
"#,
        )
        .expect("write teams");

        let registry = TeamRegistry::load_from_path(&path);

        assert!(
            registry.get("core").is_none(),
            "`member` is not a roster key; only `members` declares a team roster"
        );
    }

    #[test]
    fn team_registry_add_member_persists_to_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("teams.toml");
        std::fs::write(
            &path,
            r#"
[[team]]
name = "core"
description = "Core team"
"#,
        )
        .expect("write teams");
        let mut registry = TeamRegistry::load_from_path(&path);

        registry
            .add_member_to_path(
                "core",
                "runtime".to_string(),
                "canonical-runtime".to_string(),
                Some("Run canonical checks".to_string()),
                &path,
            )
            .expect("add");
        let reloaded = TeamRegistry::load_from_path(&path);
        let team = reloaded.get("core").expect("team");

        assert_eq!(team.members.len(), 1);
        assert_eq!(team.members[0].role, "runtime");
        assert_eq!(
            team.members[0].system_prompt.as_deref(),
            Some("Run canonical checks")
        );
    }
}
