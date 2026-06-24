//! AgentRouter - runtime-owned agent route selection.
//!
//! APXM hosts discover concrete agent profiles, but routing policy belongs in
//! the runtime so goals, workflow orchestration, and future APXM OS callers use
//! one decision model.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Abstract capabilities for APXM agent routing.
pub const AGENT_ROUTE_CAPABILITIES: &[&str] = &[
    "read",
    "write",
    "execute",
    "critique",
    "workflow_author",
    // Role-level worker capabilities used by APXM goal/council routing.
    "planner",
    "reviewer",
    "critic",
    "verifier",
    "synthesizer",
];
pub const AGENT_ROUTE_SELECTOR_DETERMINISTIC: &str = "deterministic";

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

/// Runtime-level request for a spawned agent profile binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRouteRequest {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_profiles: Vec<String>,
    pub require_agent: bool,
}

impl AgentRouteRequest {
    pub fn spawn_agent(
        id: String,
        profile: Option<String>,
        mode: Option<String>,
        model: Option<String>,
        required_capabilities: Vec<String>,
        preferred_profiles: Vec<String>,
        require_agent: bool,
    ) -> Self {
        Self {
            id,
            profile,
            mode,
            model,
            required_capabilities,
            preferred_profiles,
            require_agent,
        }
    }
}

/// Per-candidate evidence for the deterministic selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRouteScore {
    pub profile: String,
    pub eligible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_capabilities: Vec<String>,
    pub selected_count: usize,
    pub capability_fit_score: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preference_rank: Option<usize>,
    pub registry_index: usize,
    pub reason: String,
}

/// Candidate rejected before final route selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRouteRejection {
    pub profile: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_capabilities: Vec<String>,
    pub reason: String,
}

/// Why a request received its route.
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

/// Runtime route decision for one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRouteDecision {
    pub id: String,
    pub profile: Option<String>,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub source: AgentRouteSource,
    pub required_capabilities: Vec<String>,
    pub preferred_profiles: Vec<String>,
    pub eligible_profiles: Vec<String>,
    pub candidate_scores: Vec<AgentRouteScore>,
    pub rejected_candidates: Vec<AgentRouteRejection>,
    pub candidate_snapshot_hash: String,
    pub reason: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgentRoutingError {
    #[error("agent routing requires at least one candidate for request '{request_id}'")]
    NoCandidates { request_id: String },
    #[error("agent routing request '{request_id}' requested unknown profile '{profile}'")]
    UnknownProfile { request_id: String, profile: String },
    #[error(
        "agent routing request '{request_id}' requested profile '{profile}' without required capabilities: {}",
        required_capabilities.join(", ")
    )]
    ProfileCapabilityMismatch {
        request_id: String,
        profile: String,
        required_capabilities: Vec<String>,
        candidate_capabilities: Vec<String>,
    },
    #[error(
        "agent routing found no candidate for request '{request_id}' with required capabilities: {}",
        required_capabilities.join(", ")
    )]
    NoMatchingCandidates {
        request_id: String,
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

    /// Route runtime requests with an explainable deterministic selector.
    ///
    /// The selector preserves explicit profiles, filters automatic candidates by
    /// required capabilities, then chooses the least-used candidate, the
    /// closest capability fit, preferred profiles, and finally registry order.
    pub fn route_requests(
        &self,
        requests: &[AgentRouteRequest],
    ) -> Result<Vec<AgentRouteDecision>, AgentRoutingError> {
        self.route_requests_with_counts(requests, &HashMap::new())
    }

    /// Route runtime-level agent requests using caller-provided initial counts.
    pub fn route_requests_with_counts(
        &self,
        requests: &[AgentRouteRequest],
        initial_selected_counts: &HashMap<String, usize>,
    ) -> Result<Vec<AgentRouteDecision>, AgentRoutingError> {
        let normalized_candidates = self
            .candidates
            .iter()
            .enumerate()
            .map(NormalizedCandidate::new)
            .collect::<Vec<_>>();
        let candidate_snapshot_hash = candidate_snapshot_hash(&self.candidates);
        let mut selected_counts = initial_selected_counts.clone();
        let mut decisions = Vec::with_capacity(requests.len());
        for request in requests {
            let required_capabilities = normalize_capabilities(&request.required_capabilities);
            let preferred_profiles = normalize_profiles(&request.preferred_profiles);
            let scores = score_candidates(
                &normalized_candidates,
                &required_capabilities,
                &preferred_profiles,
                &selected_counts,
            );
            let rejected_candidates = rejected_candidates(&scores);
            if let Some(profile) = request.profile.as_ref() {
                let candidate = normalized_candidates
                    .iter()
                    .find(|candidate| &candidate.profile == profile)
                    .ok_or_else(|| AgentRoutingError::UnknownProfile {
                        request_id: request.id.clone(),
                        profile: profile.clone(),
                    })?;
                if !candidate.matches_required_capabilities(&required_capabilities) {
                    return Err(AgentRoutingError::ProfileCapabilityMismatch {
                        request_id: request.id.clone(),
                        profile: profile.clone(),
                        required_capabilities,
                        candidate_capabilities: candidate.capability_names.clone(),
                    });
                }
                *selected_counts.entry(profile.clone()).or_insert(0) += 1;
                decisions.push(AgentRouteDecision {
                    id: request.id.clone(),
                    profile: request.profile.clone(),
                    mode: request
                        .mode
                        .clone()
                        .or_else(|| candidate.default_mode.clone()),
                    model: request
                        .model
                        .clone()
                        .or_else(|| candidate.default_model.clone()),
                    source: AgentRouteSource::Explicit,
                    required_capabilities,
                    preferred_profiles,
                    eligible_profiles: vec![profile.clone()],
                    candidate_scores: scores,
                    rejected_candidates,
                    candidate_snapshot_hash: candidate_snapshot_hash.clone(),
                    reason: "caller supplied an explicit eligible profile".to_string(),
                });
                continue;
            }

            let eligible = matching_candidates(&normalized_candidates, &required_capabilities);
            if eligible.is_empty() {
                if request.require_agent {
                    if self.candidates.is_empty() {
                        return Err(AgentRoutingError::NoCandidates {
                            request_id: request.id.clone(),
                        });
                    }
                    return Err(AgentRoutingError::NoMatchingCandidates {
                        request_id: request.id.clone(),
                        required_capabilities,
                        candidate_count: self.candidates.len(),
                    });
                }
                decisions.push(AgentRouteDecision {
                    id: request.id.clone(),
                    profile: None,
                    mode: request.mode.clone(),
                    model: request.model.clone(),
                    source: AgentRouteSource::Deterministic,
                    required_capabilities,
                    preferred_profiles,
                    eligible_profiles: Vec::new(),
                    candidate_scores: scores,
                    rejected_candidates,
                    candidate_snapshot_hash: candidate_snapshot_hash.clone(),
                    reason: "no APXM agent profile matched; deterministic execution allowed"
                        .to_string(),
                });
                continue;
            }
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
                id: request.id.clone(),
                profile: Some(candidate.profile.clone()),
                mode: request
                    .mode
                    .clone()
                    .or_else(|| candidate.default_mode.clone()),
                model: request
                    .model
                    .clone()
                    .or_else(|| candidate.default_model.clone()),
                source: AgentRouteSource::Selected,
                required_capabilities,
                preferred_profiles,
                eligible_profiles,
                candidate_scores: scores,
                rejected_candidates,
                candidate_snapshot_hash: candidate_snapshot_hash.clone(),
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
    capability_names: Vec<String>,
    capabilities: HashSet<String>,
    index: usize,
}

impl NormalizedCandidate {
    fn new((index, candidate): (usize, &AgentRouteCandidate)) -> Self {
        let capability_names = normalize_capabilities(&candidate.capabilities);
        let capabilities = capability_names.iter().cloned().collect::<HashSet<_>>();
        Self {
            profile: candidate.profile.clone(),
            default_mode: candidate.default_mode.clone(),
            default_model: candidate.default_model.clone(),
            capability_names,
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

fn score_candidates(
    candidates: &[NormalizedCandidate],
    required_capabilities: &[String],
    preferred_profiles: &[String],
    selected_counts: &HashMap<String, usize>,
) -> Vec<AgentRouteScore> {
    candidates
        .iter()
        .map(|candidate| {
            let mut matched_capabilities = Vec::new();
            let mut missing_capabilities = Vec::new();
            for required in required_capabilities {
                if candidate.capabilities.contains(required) {
                    matched_capabilities.push(required.clone());
                } else {
                    missing_capabilities.push(required.clone());
                }
            }
            let eligible = missing_capabilities.is_empty();
            let preference_rank = preferred_profiles
                .iter()
                .position(|profile| profile == &candidate.profile);
            let selected_count = selected_counts
                .get(&candidate.profile)
                .copied()
                .unwrap_or(0);
            let capability_fit_score = capability_fit_score(candidate, required_capabilities.len());
            let reason = if eligible {
                format!(
                    "eligible: selected_count={selected_count}, capability_fit_score={capability_fit_score}"
                )
            } else {
                format!(
                    "missing required capabilities [{}]",
                    missing_capabilities.join(", ")
                )
            };
            AgentRouteScore {
                profile: candidate.profile.clone(),
                eligible,
                matched_capabilities,
                missing_capabilities,
                selected_count,
                capability_fit_score,
                preference_rank,
                registry_index: candidate.index,
                reason,
            }
        })
        .collect()
}

fn rejected_candidates(scores: &[AgentRouteScore]) -> Vec<AgentRouteRejection> {
    scores
        .iter()
        .filter(|score| !score.eligible)
        .map(|score| AgentRouteRejection {
            profile: score.profile.clone(),
            missing_capabilities: score.missing_capabilities.clone(),
            reason: score.reason.clone(),
        })
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

fn candidate_snapshot_hash(candidates: &[AgentRouteCandidate]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for candidate in candidates {
        hash_str(&mut hash, &candidate.profile);
        hash_str(&mut hash, candidate.source.as_deref().unwrap_or(""));
        hash_str(&mut hash, &candidate.executable);
        for capability in normalize_capabilities(&candidate.capabilities) {
            hash_str(&mut hash, &capability);
        }
        hash_str(&mut hash, candidate.default_mode.as_deref().unwrap_or(""));
        hash_str(&mut hash, candidate.default_model.as_deref().unwrap_or(""));
    }
    format!("fnv1a64:{hash:016x}")
}

fn hash_str(hash: &mut u64, value: &str) {
    for byte in value.as_bytes() {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x100000001b3);
    }
    *hash ^= 0xff;
    *hash = hash.wrapping_mul(0x100000001b3);
}
