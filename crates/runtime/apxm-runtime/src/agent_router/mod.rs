//! AgentRouter - runtime-owned agent route selection.
//!
//! APXM hosts discover concrete agent profiles, but routing policy belongs in
//! the runtime so goals, workflow orchestration, and future APXM OS callers use
//! one decision model.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Abstract capabilities used by APXM agent routing.
pub const AGENT_ROUTE_CAPABILITIES: &[&str] =
    &["read", "write", "execute", "critique", "workflow_author"];

/// Candidate route discovered by a host from its agent registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRouteCandidate {
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
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
    pub preferred_profiles: Vec<String>,
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
    pub required_capabilities: Vec<String>,
    pub preferred_profiles: Vec<String>,
    pub eligible_profiles: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentRoutingError {
    #[error("agent routing requires at least one candidate for target '{target_id}'")]
    NoCandidates { target_id: String },
    #[error("agent routing target '{target_id}' requested unknown profile '{profile}'")]
    UnknownProfile { target_id: String, profile: String },
    #[error(
        "agent routing target '{target_id}' requested profile '{profile}' without required capabilities: {}",
        required_capabilities.join(", ")
    )]
    ProfileCapabilityMismatch {
        target_id: String,
        profile: String,
        required_capabilities: Vec<String>,
        candidate_capabilities: Vec<String>,
    },
    #[error(
        "agent routing found no candidate for target '{target_id}' with required capabilities: {}",
        required_capabilities.join(", ")
    )]
    NoMatchingCandidates {
        target_id: String,
        required_capabilities: Vec<String>,
        candidate_count: usize,
    },
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

    /// Route targets with an explainable deterministic selector.
    ///
    /// The selector preserves explicit profiles, filters automatic candidates by
    /// required capabilities, then chooses the least-used candidate, the
    /// closest capability fit, preferred profiles, and finally registry order.
    pub fn route_targets(
        &self,
        targets: &[AgentRouteTarget],
        require_agents: bool,
    ) -> Result<Vec<AgentRouteDecision>, AgentRoutingError> {
        self.route_targets_with_counts(targets, require_agents, &HashMap::new())
    }

    /// Route targets using caller-provided initial selection counts.
    ///
    /// Batch planners normally start from zero. Runtime callers such as
    /// `SPAWN_AGENT` seed this with active process counts so standalone
    /// spawns do not repeatedly choose the same eligible profile.
    pub fn route_targets_with_counts(
        &self,
        targets: &[AgentRouteTarget],
        require_agents: bool,
        initial_selected_counts: &HashMap<String, usize>,
    ) -> Result<Vec<AgentRouteDecision>, AgentRoutingError> {
        let normalized_candidates = self
            .candidates
            .iter()
            .enumerate()
            .map(NormalizedCandidate::new)
            .collect::<Vec<_>>();
        let mut selected_counts = initial_selected_counts.clone();
        let mut decisions = Vec::with_capacity(targets.len());
        for target in targets {
            let required_capabilities = normalize_capabilities(&target.required_capabilities);
            let preferred_profiles = normalize_profiles(&target.preferred_profiles);
            if let Some(profile) = target.profile.as_ref() {
                let candidate = normalized_candidates
                    .iter()
                    .find(|candidate| &candidate.profile == profile)
                    .ok_or_else(|| AgentRoutingError::UnknownProfile {
                        target_id: target.id.clone(),
                        profile: profile.clone(),
                    })?;
                if !candidate.matches_required_capabilities(&required_capabilities) {
                    return Err(AgentRoutingError::ProfileCapabilityMismatch {
                        target_id: target.id.clone(),
                        profile: profile.clone(),
                        required_capabilities,
                        candidate_capabilities: candidate.capability_list.clone(),
                    });
                }
                *selected_counts.entry(profile.clone()).or_insert(0) += 1;
                decisions.push(AgentRouteDecision {
                    id: target.id.clone(),
                    profile: target.profile.clone(),
                    mode: target
                        .mode
                        .clone()
                        .or_else(|| candidate.default_mode.clone()),
                    model: target
                        .model
                        .clone()
                        .or_else(|| candidate.default_model.clone()),
                    source: AgentRouteSource::Explicit,
                    required_capabilities,
                    preferred_profiles,
                    eligible_profiles: vec![profile.clone()],
                    reason: "caller supplied an explicit eligible profile".to_string(),
                });
                continue;
            }

            let eligible = matching_candidates(&normalized_candidates, &required_capabilities);
            if eligible.is_empty() {
                if require_agents {
                    if self.candidates.is_empty() {
                        return Err(AgentRoutingError::NoCandidates {
                            target_id: target.id.clone(),
                        });
                    }
                    return Err(AgentRoutingError::NoMatchingCandidates {
                        target_id: target.id.clone(),
                        required_capabilities,
                        candidate_count: self.candidates.len(),
                    });
                }
                decisions.push(AgentRouteDecision {
                    id: target.id.clone(),
                    profile: None,
                    mode: target.mode.clone(),
                    model: target.model.clone(),
                    source: AgentRouteSource::Deterministic,
                    required_capabilities,
                    preferred_profiles,
                    eligible_profiles: Vec::new(),
                    reason: "no APXM agent profile matched; deterministic execution allowed"
                        .to_string(),
                });
                continue;
            };
            let eligible_profiles = eligible
                .iter()
                .map(|candidate| candidate.profile.clone())
                .collect::<Vec<_>>();
            let candidate = select_candidate(
                &eligible,
                required_capabilities.len(),
                &preferred_profiles,
                &selected_counts,
            );
            let fit_score = capability_fit_score(candidate, required_capabilities.len());
            let preference = preferred_profiles
                .iter()
                .position(|profile| profile == &candidate.profile);
            *selected_counts
                .entry(candidate.profile.clone())
                .or_insert(0) += 1;

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
                required_capabilities,
                preferred_profiles,
                eligible_profiles,
                reason: format!(
                    "selected {}eligible profile '{}' with capability fit score {}",
                    if preference.is_some() {
                        "preferred "
                    } else {
                        "least-used "
                    },
                    candidate.profile,
                    fit_score
                ),
            });
        }
        Ok(decisions)
    }
}

#[derive(Debug, Clone)]
struct NormalizedCandidate {
    profile: String,
    default_mode: Option<String>,
    default_model: Option<String>,
    capability_list: Vec<String>,
    capabilities: HashSet<String>,
    index: usize,
}

impl NormalizedCandidate {
    fn new((index, candidate): (usize, &AgentRouteCandidate)) -> Self {
        let capability_list = normalize_capabilities(&candidate.capabilities);
        let capabilities = capability_list.iter().cloned().collect::<HashSet<_>>();
        Self {
            profile: candidate.profile.clone(),
            default_mode: candidate.default_mode.clone(),
            default_model: candidate.default_model.clone(),
            capability_list,
            capabilities,
            index,
        }
    }

    fn matches_required_capabilities(&self, required_capabilities: &[String]) -> bool {
        required_capabilities
            .iter()
            .all(|required| self.capabilities.contains(required))
    }
}

fn matching_candidates<'a>(
    candidates: &'a [NormalizedCandidate],
    required_capabilities: &[String],
) -> Vec<&'a NormalizedCandidate> {
    candidates
        .iter()
        .filter(|candidate| candidate.matches_required_capabilities(required_capabilities))
        .collect()
}

fn select_candidate<'a>(
    eligible: &'a [&'a NormalizedCandidate],
    required_count: usize,
    preferred_profiles: &[String],
    selected_counts: &HashMap<String, usize>,
) -> &'a NormalizedCandidate {
    eligible
        .iter()
        .copied()
        .min_by_key(|candidate| {
            let preference_rank = preferred_profiles
                .iter()
                .position(|profile| profile == &candidate.profile)
                .unwrap_or(usize::MAX);
            (
                selected_counts
                    .get(&candidate.profile)
                    .copied()
                    .unwrap_or(0),
                capability_fit_score(candidate, required_count),
                preference_rank,
                candidate.index,
            )
        })
        .expect("eligible candidates are non-empty")
}

fn capability_fit_score(candidate: &NormalizedCandidate, required_count: usize) -> usize {
    candidate.capabilities.len().saturating_sub(required_count)
}

fn normalize_capabilities(capabilities: &[String]) -> Vec<String> {
    let mut normalized = capabilities
        .iter()
        .map(|capability| capability.trim().to_ascii_lowercase())
        .filter(|capability| !capability.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_profiles(profiles: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for profile in profiles {
        let profile = profile.trim();
        if profile.is_empty() || !seen.insert(profile.to_string()) {
            continue;
        }
        normalized.push(profile.to_string());
    }
    normalized
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
            preferred_profiles: Vec::new(),
        }
    }

    fn candidate(profile: &str, mode: Option<&str>, model: Option<&str>) -> AgentRouteCandidate {
        AgentRouteCandidate {
            profile: profile.to_string(),
            description: None,
            source: None,
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
            candidate("explicit", None, None),
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
                    preferred_profiles: Vec::new(),
                }],
                true,
            )
            .expect("route by capability");

        assert_eq!(decisions[0].profile.as_deref(), Some("execute-agent"));
    }

    #[test]
    fn required_routing_reports_no_matching_candidates() {
        let mut read_agent = candidate("read-agent", None, None);
        read_agent.capabilities = vec!["read".to_string()];
        let router = AgentRouter::new(vec![read_agent]);

        let error = router
            .route_targets(
                &[AgentRouteTarget {
                    id: "executor".to_string(),
                    profile: None,
                    mode: None,
                    model: None,
                    required_capabilities: vec!["execute".to_string()],
                    preferred_profiles: Vec::new(),
                }],
                true,
            )
            .expect_err("missing matching candidate");

        assert_eq!(
            error,
            AgentRoutingError::NoMatchingCandidates {
                target_id: "executor".to_string(),
                required_capabilities: vec!["execute".to_string()],
                candidate_count: 1,
            }
        );
    }

    #[test]
    fn explicit_profile_must_be_discovered() {
        let router = AgentRouter::new(vec![candidate("agent", None, None)]);

        let error = router
            .route_targets(&[target("executor", Some("missing"))], true)
            .expect_err("missing explicit profile");

        assert_eq!(
            error,
            AgentRoutingError::UnknownProfile {
                target_id: "executor".to_string(),
                profile: "missing".to_string(),
            }
        );
    }

    #[test]
    fn explicit_profile_must_match_required_capabilities() {
        let mut reader = candidate("reader", None, None);
        reader.capabilities = vec!["read".to_string()];
        let router = AgentRouter::new(vec![reader]);

        let error = router
            .route_targets(
                &[AgentRouteTarget {
                    id: "executor".to_string(),
                    profile: Some("reader".to_string()),
                    mode: None,
                    model: None,
                    required_capabilities: vec!["execute".to_string()],
                    preferred_profiles: Vec::new(),
                }],
                true,
            )
            .expect_err("explicit profile does not satisfy capabilities");

        assert_eq!(
            error,
            AgentRoutingError::ProfileCapabilityMismatch {
                target_id: "executor".to_string(),
                profile: "reader".to_string(),
                required_capabilities: vec!["execute".to_string()],
                candidate_capabilities: vec!["read".to_string()],
            }
        );
    }

    #[test]
    fn explicit_profiles_use_defaults_and_count_for_balancing() {
        let reader = candidate("reader", Some("review"), Some("model-reader"));
        let mut writer = candidate("writer", Some("code"), Some("model-writer"));
        writer.capabilities = vec!["read".to_string(), "write".to_string()];
        let router = AgentRouter::new(vec![reader, writer]);

        let decisions = router
            .route_targets(
                &[
                    AgentRouteTarget {
                        id: "explicit-reader".to_string(),
                        profile: Some("reader".to_string()),
                        mode: None,
                        model: None,
                        required_capabilities: Vec::new(),
                        preferred_profiles: Vec::new(),
                    },
                    target("auto-reader", None),
                ],
                true,
            )
            .expect("route");

        assert_eq!(decisions[0].profile.as_deref(), Some("reader"));
        assert_eq!(decisions[0].mode.as_deref(), Some("review"));
        assert_eq!(decisions[0].model.as_deref(), Some("model-reader"));
        assert_eq!(decisions[1].profile.as_deref(), Some("writer"));
    }

    #[test]
    fn balances_by_least_used_then_capability_fit() {
        let mut reader = candidate("reader", None, None);
        reader.capabilities = vec!["read".to_string()];
        let mut executor = candidate("executor", None, None);
        executor.capabilities = vec!["read".to_string(), "execute".to_string()];
        let router = AgentRouter::new(vec![reader, executor]);

        let decisions = router
            .route_targets(
                &[
                    AgentRouteTarget {
                        id: "execute".to_string(),
                        profile: None,
                        mode: None,
                        model: None,
                        required_capabilities: vec!["execute".to_string()],
                        preferred_profiles: Vec::new(),
                    },
                    AgentRouteTarget {
                        id: "read".to_string(),
                        profile: None,
                        mode: None,
                        model: None,
                        required_capabilities: vec!["read".to_string()],
                        preferred_profiles: Vec::new(),
                    },
                ],
                true,
            )
            .expect("route");

        assert_eq!(decisions[0].profile.as_deref(), Some("executor"));
        assert_eq!(decisions[1].profile.as_deref(), Some("reader"));
        assert_eq!(decisions[1].eligible_profiles, vec!["reader", "executor"]);
    }

    #[test]
    fn normalizes_capability_names_before_matching() {
        let mut agent = candidate("agent", None, None);
        agent.capabilities = vec![" Execute ".to_string(), "execute".to_string()];
        let router = AgentRouter::new(vec![agent]);

        let decisions = router
            .route_targets(
                &[AgentRouteTarget {
                    id: "executor".to_string(),
                    profile: None,
                    mode: None,
                    model: None,
                    required_capabilities: vec!["execute".to_string()],
                    preferred_profiles: Vec::new(),
                }],
                true,
            )
            .expect("route");

        assert_eq!(decisions[0].profile.as_deref(), Some("agent"));
        assert_eq!(
            decisions[0].required_capabilities,
            vec!["execute".to_string()]
        );
    }

    #[test]
    fn preferred_profiles_break_eligible_ties() {
        let mut first = candidate("first", None, None);
        first.capabilities = vec!["execute".to_string()];
        let mut second = candidate("second", None, None);
        second.capabilities = vec!["execute".to_string()];
        let router = AgentRouter::new(vec![first, second]);

        let decisions = router
            .route_targets(
                &[AgentRouteTarget {
                    id: "executor".to_string(),
                    profile: None,
                    mode: None,
                    model: None,
                    required_capabilities: vec!["execute".to_string()],
                    preferred_profiles: vec!["second".to_string()],
                }],
                true,
            )
            .expect("route");

        assert_eq!(decisions[0].profile.as_deref(), Some("second"));
        assert_eq!(decisions[0].preferred_profiles, vec!["second"]);
        assert!(decisions[0].reason.contains("preferred eligible"));
    }

    #[test]
    fn preferred_profiles_do_not_override_load_or_capability_fit() {
        let mut reader = candidate("reader", None, None);
        reader.capabilities = vec!["read".to_string()];
        let mut broader = candidate("broader", None, None);
        broader.capabilities = vec!["read".to_string(), "write".to_string()];
        let router = AgentRouter::new(vec![reader, broader]);

        let decisions = router
            .route_targets(
                &[
                    target("explicit-reader", Some("reader")),
                    AgentRouteTarget {
                        id: "auto-read".to_string(),
                        profile: None,
                        mode: None,
                        model: None,
                        required_capabilities: vec!["read".to_string()],
                        preferred_profiles: vec!["reader".to_string()],
                    },
                ],
                true,
            )
            .expect("route");

        assert_eq!(decisions[1].profile.as_deref(), Some("broader"));
        assert!(decisions[1].reason.contains("least-used eligible"));
    }

    #[test]
    fn seeded_counts_affect_standalone_selection() {
        let mut first = candidate("first", None, None);
        first.capabilities = vec!["read".to_string()];
        let mut second = candidate("second", None, None);
        second.capabilities = vec!["read".to_string()];
        let router = AgentRouter::new(vec![first, second]);
        let counts = HashMap::from([("first".to_string(), 1usize)]);

        let decisions = router
            .route_targets_with_counts(
                &[AgentRouteTarget {
                    id: "reader".to_string(),
                    profile: None,
                    mode: None,
                    model: None,
                    required_capabilities: vec!["read".to_string()],
                    preferred_profiles: Vec::new(),
                }],
                true,
                &counts,
            )
            .expect("route");

        assert_eq!(decisions[0].profile.as_deref(), Some("second"));
    }
}
