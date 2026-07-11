//! Canonical agent definition (`apxm.agent-definition.v1`).
//!
//! Implements `workspace/contracts/schemas/agent-definition.v1.json`, the
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
    /// `[runtime]` from `agent.toml` — the single source of truth for every
    /// `ConversationalAgent` knob: loop/memory_space/session_prefix.
    /// When `entry.flow` is empty, `runtime.loop` is what lets a
    /// pure-declarative package be instantiated with no Python entry file
    /// (see [`AgentEntry::flow`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<AgentRuntime>,
    /// `[[hooks]]` from `agent.toml`: manifest-declared lifecycle
    /// hooks, lowered the same way whether the package has a custom entry
    /// (where entry code may add MORE hooks, but must never contradict
    /// these — a lint check, not a runtime one) or no entry at all (where
    /// these are the agent's only hooks).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<AgentHook>,
}

/// `[runtime]` `extra` key an `agent.toml` author uses to declare the
/// conversation-compaction policy for a pure-declarative package (no Python
/// entry file, so no `CompactionPolicy(...)` object to read). The value is
/// the same JSON shape the Python frontend stamps into `ApxmGraph.metadata`
/// (the runtime compaction policy): `{"keep_recent", "compact_at_tokens",
/// "strategy", "summary_key"}`. `extra` is already an open, stringly-typed
/// bag ([`AgentRuntime::extra`]) — this constant just names the
/// well-known key so every producer/consumer (this crate's loader, the
/// server's typed-sidecar loader) agrees on it instead of each side
/// inventing its own literal.
pub const RUNTIME_EXTRA_COMPACTION_POLICY_KEY: &str = "compaction_policy";

/// `[runtime]` — every `ConversationalAgent` knob declarable in the
/// manifest. Kept an open shape (`extra`) so knobs added later don't
/// need a schema break. Extra
/// values are stringly-typed to keep this derive-`Eq`-able (unlike
/// `toml::Value`/`serde_json::Value`, which carry floats).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#loop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_prefix: Option<String>,
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, String>,
}

/// One `[[hooks]]` entry: a manifest-declared binding of a lifecycle event to
/// a handler in `capabilities/handlers/`. `event`/`mode` are kept as
/// open strings here (not the Python frontend's `LifecycleEvent`/`HookMode`
/// enums) so contracts does not have to track the frontend's vocabulary —
/// the package toolchain's lint validates the closed set at author time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentHook {
    pub event: String,
    #[serde(default = "default_hook_match")]
    pub r#match: String,
    pub mode: String,
    pub handler: String,
}

fn default_hook_match() -> String {
    "*".to_string()
}

/// The agent's package entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentEntry {
    /// Path/reference to the entry flow (e.g. an AIR file). May be empty
    /// for a **pure-declarative** package with no custom Python
    /// entry — the loader then instantiates a `ConversationalAgent` straight
    /// from `AgentDefinition::runtime` + `AgentDefinition::hooks` instead.
    /// [`AgentDefinition::validate`] requires `runtime.loop` to be set
    /// whenever `flow` is empty, so there is always an unambiguous loop mode
    /// to build.
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
    #[error("entry.flow must not be empty unless runtime.loop is set ( pure-declarative agent)")]
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
        // an empty entry.flow is only valid for a pure-declarative
        // agent — one whose [runtime].loop is set, so the loader has an
        // unambiguous loop mode to build a ConversationalAgent from (no
        // Python entry file needed).
        let is_pure_declarative = self
            .runtime
            .as_ref()
            .and_then(|r| r.r#loop.as_deref())
            .is_some();
        if self.entry.flow.trim().is_empty() && !is_pure_declarative {
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
            runtime: None,
            hooks: Vec::new(),
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
            runtime: Some(AgentRuntime {
                r#loop: Some("in_graph".to_string()),
                memory_space: Some("stm".to_string()),
                session_prefix: Some("demo".to_string()),
                extra: BTreeMap::new(),
            }),
            hooks: vec![AgentHook {
                event: "pre_cap".to_string(),
                r#match: "*".to_string(),
                mode: "gate".to_string(),
                handler: "capabilities/handlers/guard.py:check".to_string(),
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
    fn empty_entry_flow_is_valid_for_pure_declarative_agent_with_runtime_loop() {
        // a package with no entry file at all is valid as long as
        // [runtime].loop tells the loader which ConversationalAgent loop to
        // build — nothing ambiguous is left for it to guess.
        let mut def = sample();
        def.entry.flow = String::new();
        def.validate()
            .expect("empty entry.flow is valid when runtime.loop is set");
    }

    #[test]
    fn rejects_empty_entry_flow_without_runtime_loop() {
        let mut def = sample();
        def.entry.flow = String::new();
        def.runtime = None;
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

    /// **Cross-plane parity surface:** a pure-declarative package's
    /// `[runtime]` extra bag round-trips a `compaction_policy` knob
    /// unchanged through serde — the same `extra: BTreeMap<String, String>`
    /// open shape every other runtime knob already uses, so no schema break
    /// was needed to add compaction parity to the declarative surface.
    #[test]
    fn runtime_extra_round_trips_compaction_policy_knob() {
        let mut def = sample();
        let policy_json = r#"{"keep_recent":2,"compact_at_tokens":300,"strategy":"summarize","summary_key":"conversation:summary"}"#;
        def.runtime = Some(AgentRuntime {
            r#loop: Some("in_graph".to_string()),
            memory_space: Some("stm".to_string()),
            session_prefix: None,
            extra: BTreeMap::from([(
                RUNTIME_EXTRA_COMPACTION_POLICY_KEY.to_string(),
                policy_json.to_string(),
            )]),
        });

        let json = serde_json::to_string(&def).expect("serialize");
        let back: AgentDefinition = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(def, back);
        assert_eq!(
            back.runtime
                .as_ref()
                .unwrap()
                .extra
                .get(RUNTIME_EXTRA_COMPACTION_POLICY_KEY)
                .map(String::as_str),
            Some(policy_json),
        );
    }
}
