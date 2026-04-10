use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub role: String,
    pub profile: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamDefinition {
    pub name: String,
    pub description: String,
    #[serde(alias = "member")]
    pub members: Vec<TeamMember>,
}

pub struct TeamRegistry {
    teams: HashMap<String, TeamDefinition>,
}

impl TeamRegistry {
    pub fn load_from_default_path() -> Self {
        let mut teams = HashMap::new();

        if let Some(path) = Self::user_config_path() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(file) = toml::from_str::<TeamsFile>(&content) {
                    for team in file.team {
                        teams.insert(team.name.clone(), team);
                    }
                } else {
                    tracing::warn!(path = %path.display(), "failed to parse teams.toml");
                }
            }
        }

        Self { teams }
    }

    pub fn get(&self, name: &str) -> Option<&TeamDefinition> {
        self.teams.get(name)
    }

    pub fn list(&self) -> Vec<&TeamDefinition> {
        self.teams.values().collect()
    }

    pub fn add_member(
        &mut self,
        team_name: &str,
        role: String,
        profile: String,
        system_prompt: Option<String>,
    ) -> Result<(), std::io::Error> {
        let team = self
            .teams
            .get_mut(team_name)
            .ok_or_else(|| std::io::Error::other(format!("Team '{}' not found", team_name)))?;

        if team.members.iter().any(|m| m.role == role) {
            return Err(std::io::Error::other(format!(
                "Role '{}' already exists in team '{}'",
                role, team_name
            )));
        }

        team.members.push(TeamMember {
            role,
            profile,
            system_prompt,
        });

        self.persist_user_entries()
    }

    fn user_config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".apxm").join("teams.toml"))
    }

    fn persist_user_entries(&self) -> Result<(), std::io::Error> {
        let path = Self::user_config_path()
            .ok_or_else(|| std::io::Error::other("cannot determine home directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut teams: Vec<TeamDefinition> = self.teams.values().cloned().collect();
        teams.sort_by(|a, b| a.name.cmp(&b.name));

        let file = TeamsFile { team: teams };
        let content = toml::to_string_pretty(&file)
            .map_err(|e| std::io::Error::other(format!("serialize: {e}")))?;

        std::fs::write(&path, format!("# APXM Team Definitions\n\n{content}"))?;
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

    fn sample_team() -> TeamDefinition {
        TeamDefinition {
            name: "ultrathink".to_string(),
            description: "3-phase parallel planning team".to_string(),
            members: vec![
                TeamMember {
                    role: "architect".to_string(),
                    profile: "claude".to_string(),
                    system_prompt: Some("You are a software architect...".to_string()),
                },
                TeamMember {
                    role: "adversary".to_string(),
                    profile: "claude".to_string(),
                    system_prompt: Some("You are an adversarial reviewer...".to_string()),
                },
                TeamMember {
                    role: "impl_expert".to_string(),
                    profile: "codex".to_string(),
                    system_prompt: None,
                },
            ],
        }
    }

    #[test]
    fn team_definition_roundtrip() {
        let team = sample_team();
        let toml_str = toml::to_string_pretty(&team).unwrap();
        let parsed: TeamDefinition = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.name, "ultrathink");
        assert_eq!(parsed.members.len(), 3);
        assert_eq!(parsed.members[0].role, "architect");
    }

    #[test]
    fn teams_file_roundtrip() {
        let file = TeamsFile {
            team: vec![sample_team()],
        };
        let toml_str = toml::to_string_pretty(&file).unwrap();
        let parsed: TeamsFile = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.team.len(), 1);
        assert_eq!(parsed.team[0].name, "ultrathink");
    }

    #[test]
    fn teams_file_with_member_alias() {
        let toml_str = r#"
            name = "test"
            description = "test team"

            [[member]]
            role = "dev"
            profile = "claude"
        "#;
        let parsed: TeamDefinition = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.members.len(), 1);
        assert_eq!(parsed.members[0].role, "dev");
    }

    #[test]
    fn empty_registry_returns_none() {
        let registry = TeamRegistry {
            teams: HashMap::new(),
        };
        assert!(registry.get("nonexistent").is_none());
        assert!(registry.list().is_empty());
    }

    #[test]
    fn registry_lookup() {
        let mut registry = TeamRegistry {
            teams: HashMap::new(),
        };
        registry
            .teams
            .insert("ultrathink".to_string(), sample_team());
        let team = registry.get("ultrathink").unwrap();
        assert_eq!(team.name, "ultrathink");
        assert_eq!(team.members.len(), 3);
    }
}
