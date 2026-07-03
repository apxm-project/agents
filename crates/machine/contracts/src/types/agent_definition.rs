//! Canonical agent definition (`apxm.agent-definition.v1`).
//!
//! Mirrors `workspace/contracts/schemas/agent-definition.v1.json`, the
//! normative schema for "what an agent is": identity, entry point, prompts,
//! declared capabilities (by reference to [`super::capability::CapabilityDefinition`]
//! ids), a hierarchy edge (parent/permitted-children), and trigger
//! subscriptions (the cue kinds this agent listens for: webhook, cron,
//! channel, a2a, mcp_in, watch, process).
//!
//! This is the single cross-repo source of truth for an agent's declared
//! shape. Downstream repos project it into their own execution-specific
//! views (e.g. the runtime flow-owner [`super::execution::Agent`] via
//! `From<AgentDefinition> for Agent`) rather than inventing a private notion
//! of what an agent is.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::execution::{Agent, AgentMetadata, CapabilityDeclaration};

pub const AGENT_DEFINITION_SCHEMA_V1: &str = "apxm.agent-definition.v1";

/// Canonical agent definition. See module docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentDefinition {
    pub schema_version: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub entry: AgentEntry,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prompts: BTreeMap<String, String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<AgentHierarchy>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<AgentTrigger>,
}

/// The agent's package entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEntry {
    /// Path/reference to the entry flow (e.g. an AIR file).
    pub flow: String,
    /// Optional named loop/driver within the entry flow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#loop: Option<String>,
}

/// Hierarchy edge: this agent's parent and the children it is permitted to
/// delegate to/spawn. One hierarchy model shared by every projection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentHierarchy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permitted_children: Vec<String>,
}

/// A trigger subscription: a cue kind this agent listens for.
///
/// `kind` is the cue concept's kind vocabulary: `webhook | cron | channel |
/// a2a | mcp_in | watch | process` (kept as an open string here so contracts
/// does not have to track every OS-side cue extension; OS validates against
/// its own `CueKind` enum when merging this into `agents.d`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTrigger {
    pub id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentDefinitionError {
    #[error("invalid schema_version: expected {expected}, got {actual}")]
    InvalidSchemaVersion {
        expected: &'static str,
        actual: String,
    },
    #[error("id must not be empty")]
    EmptyId,
    #[error("entry.flow must not be empty")]
    EmptyEntryFlow,
    #[error("trigger id must not be empty")]
    EmptyTriggerId,
    #[error("trigger kind must not be empty")]
    EmptyTriggerKind,
}

impl AgentDefinition {
    pub fn validate(&self) -> Result<(), AgentDefinitionError> {
        if self.schema_version != AGENT_DEFINITION_SCHEMA_V1 {
            return Err(AgentDefinitionError::InvalidSchemaVersion {
                expected: AGENT_DEFINITION_SCHEMA_V1,
                actual: self.schema_version.clone(),
            });
        }
        if self.id.trim().is_empty() {
            return Err(AgentDefinitionError::EmptyId);
        }
        if self.entry.flow.trim().is_empty() {
            return Err(AgentDefinitionError::EmptyEntryFlow);
        }
        for trigger in &self.triggers {
            if trigger.id.trim().is_empty() {
                return Err(AgentDefinitionError::EmptyTriggerId);
            }
            if trigger.kind.trim().is_empty() {
                return Err(AgentDefinitionError::EmptyTriggerKind);
            }
        }
        Ok(())
    }
}

/// Project a contract-level agent definition into the runtime flow-owner
/// execution subset. Flows themselves are populated separately by the
/// compiler/loader from the entry artifact; this projection carries the
/// identity, capability, and descriptive fields that the runtime `Agent`
/// exposes today.
impl From<AgentDefinition> for Agent {
    fn from(def: AgentDefinition) -> Self {
        let capabilities = def
            .capabilities
            .into_iter()
            .map(|name| CapabilityDeclaration {
                name,
                description: None,
            })
            .collect();
        Agent {
            name: def.id,
            metadata: AgentMetadata {
                capabilities,
                context: def.description,
                ..AgentMetadata::default()
            },
            flows: std::collections::HashMap::new(),
        }
    }
}

/// Reconstruct the subset of an [`AgentDefinition`] that [`Agent`] carries.
/// Used by round-trip tests to confirm the projection loses no data for the
/// fields it is responsible for (identity, capability ids, description).
/// Fields the runtime `Agent` does not carry (entry, prompts, hierarchy,
/// triggers) are intentionally absent here — they are not part of the
/// execution subset.
impl From<&Agent> for AgentDefinition {
    fn from(agent: &Agent) -> Self {
        AgentDefinition {
            schema_version: AGENT_DEFINITION_SCHEMA_V1.to_string(),
            id: agent.name.clone(),
            name: None,
            description: agent.metadata.context.clone(),
            entry: AgentEntry {
                flow: String::new(),
                r#loop: None,
            },
            prompts: BTreeMap::new(),
            capabilities: agent
                .metadata
                .capabilities
                .iter()
                .map(|c| c.name.clone())
                .collect(),
            hierarchy: None,
            triggers: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AgentDefinition {
        AgentDefinition {
            schema_version: AGENT_DEFINITION_SCHEMA_V1.to_string(),
            id: "demo.agent".to_string(),
            name: Some("Demo Agent".to_string()),
            description: Some("does demo things".to_string()),
            entry: AgentEntry {
                flow: "main.air".to_string(),
                r#loop: None,
            },
            prompts: BTreeMap::from([("system".to_string(), "be helpful".to_string())]),
            capabilities: vec!["web.search".to_string(), "files.read".to_string()],
            hierarchy: Some(AgentHierarchy {
                parent: Some("team.supervisor".to_string()),
                permitted_children: vec!["team.worker".to_string()],
            }),
            triggers: vec![AgentTrigger {
                id: "on-issue".to_string(),
                kind: "webhook".to_string(),
                config: None,
            }],
        }
    }

    #[test]
    fn valid_definition_passes_validation() {
        sample().validate().expect("sample definition is valid");
    }

    #[test]
    fn rejects_wrong_schema_version() {
        let mut def = sample();
        def.schema_version = "apxm.agent-definition.v0".to_string();
        assert_eq!(
            def.validate(),
            Err(AgentDefinitionError::InvalidSchemaVersion {
                expected: AGENT_DEFINITION_SCHEMA_V1,
                actual: "apxm.agent-definition.v0".to_string(),
            })
        );
    }

    #[test]
    fn rejects_empty_entry_flow() {
        let mut def = sample();
        def.entry.flow = String::new();
        assert_eq!(def.validate(), Err(AgentDefinitionError::EmptyEntryFlow));
    }

    #[test]
    fn rejects_trigger_missing_kind() {
        let mut def = sample();
        def.triggers.push(AgentTrigger {
            id: "no-kind".to_string(),
            kind: String::new(),
            config: None,
        });
        assert_eq!(def.validate(), Err(AgentDefinitionError::EmptyTriggerKind));
    }

    #[test]
    fn round_trips_identity_capabilities_and_description_through_agent() {
        let def = sample();
        let agent: Agent = def.clone().into();

        assert_eq!(agent.name, def.id);
        assert_eq!(agent.metadata.context, def.description);
        assert_eq!(
            agent
                .metadata
                .capabilities
                .iter()
                .map(|c| c.name.clone())
                .collect::<Vec<_>>(),
            def.capabilities
        );

        // Project back: fields the runtime Agent is responsible for carrying
        // must survive the round trip with no data loss.
        let back: AgentDefinition = (&agent).into();
        assert_eq!(back.id, def.id);
        assert_eq!(back.description, def.description);
        assert_eq!(back.capabilities, def.capabilities);

        // Fields outside the execution subset (entry, prompts, hierarchy,
        // triggers, display name) are not carried by Agent and are not
        // expected to survive — confirm they come back empty rather than
        // silently retaining stale/incorrect data.
        assert_eq!(back.entry.flow, "");
        assert!(back.prompts.is_empty());
        assert!(back.hierarchy.is_none());
        assert!(back.triggers.is_empty());
    }

    #[test]
    fn serde_round_trips_full_definition() {
        let def = sample();
        let json = serde_json::to_string(&def).expect("serialize");
        let back: AgentDefinition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(def, back);
    }
}
