//! AgentRouter - runtime-owned agent route selection.
//!
//! APXM hosts discover concrete agent profiles, but routing policy belongs in
//! the runtime so goals, workflow orchestration, and future APXM OS callers use
//! one decision model.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Candidate route discovered by a host from its agent registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRouteCandidate {
    pub profile: String,
    pub executable: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
}

/// Work item that needs an optional agent binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRouteTarget {
    pub id: String,
    pub profile: Option<String>,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub required_capabilities: Vec<String>,
}

/// Why a target received its route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRouteSource {
    Explicit,
    Selected,
    Deterministic,
}

impl AgentRouteSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Selected => "selected",
            Self::Deterministic => "deterministic",
        }
    }
}

/// Runtime route decision for one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRouteDecision {
    pub id: String,
    pub profile: Option<String>,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub source: AgentRouteSource,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentRoutingError {
    #[error("agent routing requires at least one candidate for target '{target_id}'")]
    NoCandidates { target_id: String },
}

#[derive(Debug, Clone)]
pub struct AgentRouter {
    candidates: Vec<AgentRouteCandidate>,
}

impl AgentRouter {
    pub fn new(candidates: Vec<AgentRouteCandidate>) -> Self {
        Self { candidates }
    }

    pub fn candidates(&self) -> &[AgentRouteCandidate] {
        &self.candidates
    }

    /// Route targets in a stable round-robin order while preserving explicit
    /// profiles. Capability matching is enforced before round-robin selection.
    pub fn route_targets(
        &self,
        targets: &[AgentRouteTarget],
        require_agents: bool,
    ) -> Result<Vec<AgentRouteDecision>, AgentRoutingError> {
        let mut cursor = 0usize;
        let mut decisions = Vec::with_capacity(targets.len());
        for target in targets {
            if target.profile.is_some() {
                decisions.push(AgentRouteDecision {
                    id: target.id.clone(),
                    profile: target.profile.clone(),
                    mode: target.mode.clone(),
                    model: target.model.clone(),
                    source: AgentRouteSource::Explicit,
                });
                continue;
            }

            let candidates = self.matching_candidates(&target.required_capabilities);
            let Some(candidate) = candidates.get(cursor % candidates.len().max(1)) else {
                if require_agents {
                    return Err(AgentRoutingError::NoCandidates {
                        target_id: target.id.clone(),
                    });
                }
                decisions.push(AgentRouteDecision {
                    id: target.id.clone(),
                    profile: None,
                    mode: target.mode.clone(),
                    model: target.model.clone(),
                    source: AgentRouteSource::Deterministic,
                });
                continue;
            };

            decisions.push(AgentRouteDecision {
                id: target.id.clone(),
                profile: Some(candidate.profile.clone()),
                mode: target
                    .mode
                    .clone()
                    .or_else(|| candidate.default_mode.clone()),
                model: target
                    .model
                    .clone()
                    .or_else(|| candidate.default_model.clone()),
                source: AgentRouteSource::Selected,
            });
            cursor += 1;
        }
        Ok(decisions)
    }

    fn matching_candidates(&self, required_capabilities: &[String]) -> Vec<&AgentRouteCandidate> {
        self.candidates
            .iter()
            .filter(|candidate| {
                required_capabilities.iter().all(|required| {
                    candidate
                        .capabilities
                        .iter()
                        .any(|capability| capability == required)
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(id: &str, profile: Option<&str>) -> AgentRouteTarget {
        AgentRouteTarget {
            id: id.to_string(),
            profile: profile.map(str::to_string),
            mode: None,
            model: None,
            required_capabilities: Vec::new(),
        }
    }

    fn candidate(profile: &str, mode: Option<&str>, model: Option<&str>) -> AgentRouteCandidate {
        AgentRouteCandidate {
            profile: profile.to_string(),
            executable: profile.to_string(),
            capabilities: Vec::new(),
            default_mode: mode.map(str::to_string),
            default_model: model.map(str::to_string),
        }
    }

    #[test]
    fn preserves_explicit_profiles_and_selects_missing_targets() {
        let router = AgentRouter::new(vec![
            candidate("agent-a", Some("architect"), Some("model-a")),
            candidate("agent-b", None, Some("model-b")),
        ]);
        let decisions = router
            .route_targets(
                &[
                    target("planner", None),
                    target("executor", Some("explicit")),
                    target("verifier", None),
                ],
                true,
            )
            .expect("route");

        assert_eq!(decisions[0].profile.as_deref(), Some("agent-a"));
        assert_eq!(decisions[0].mode.as_deref(), Some("architect"));
        assert_eq!(decisions[0].model.as_deref(), Some("model-a"));
        assert_eq!(decisions[0].source, AgentRouteSource::Selected);
        assert_eq!(decisions[1].profile.as_deref(), Some("explicit"));
        assert_eq!(decisions[1].source, AgentRouteSource::Explicit);
        assert_eq!(decisions[2].profile.as_deref(), Some("agent-b"));
        assert_eq!(decisions[2].model.as_deref(), Some("model-b"));
    }

    #[test]
    fn optional_routing_keeps_deterministic_targets_without_candidates() {
        let router = AgentRouter::new(Vec::new());
        let decisions = router
            .route_targets(&[target("planner", None)], false)
            .expect("optional route");

        assert_eq!(decisions[0].profile, None);
        assert_eq!(decisions[0].source, AgentRouteSource::Deterministic);
    }

    #[test]
    fn required_routing_fails_without_candidates() {
        let router = AgentRouter::new(Vec::new());
        let error = router
            .route_targets(&[target("planner", None)], true)
            .expect_err("missing candidates");

        assert_eq!(
            error,
            AgentRoutingError::NoCandidates {
                target_id: "planner".to_string()
            }
        );
    }

    #[test]
    fn filters_candidates_by_required_capabilities() {
        let mut read_agent = candidate("read-agent", None, None);
        read_agent.capabilities = vec!["read".to_string()];
        let mut execute_agent = candidate("execute-agent", None, None);
        execute_agent.capabilities = vec!["execute".to_string()];
        let router = AgentRouter::new(vec![read_agent, execute_agent]);

        let decisions = router
            .route_targets(
                &[AgentRouteTarget {
                    id: "executor".to_string(),
                    profile: None,
                    mode: None,
                    model: None,
                    required_capabilities: vec!["execute".to_string()],
                }],
                true,
            )
            .expect("route by capability");

        assert_eq!(decisions[0].profile.as_deref(), Some("execute-agent"));
    }
}
