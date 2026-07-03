//! Shared candidate-scoring engine (RTG-10, routing.md decision 6).
//!
//! `AgentRouter` (ACP profile selection, `agent_router.rs`) and — once org
//! members exist as a candidate set — org topic/role resolution both need
//! the same deterministic, explainable tie-break: candidates are filtered to
//! those matching a set of required capabilities, then the best one is
//! chosen by least-used count, capability fit, declared preference, and
//! finally declaration order. This module is that one scoring engine: it
//! knows nothing about ACP profiles or org members, only about scoreable
//! candidates that expose a stable id, a capability set, and a declaration
//! index.
//!
//! Extraction note: the tie-break precedence implemented here — **least-used
//! → capability fit → preferred → order** — is the precedence `AgentRouter`
//! already shipped with (`selected_count` compared before
//! `capability_fit_score` in the old `select_candidate` min-key tuple).
//! `docs/plans/routing.md` (decision 6 / RTG-10) describes the target
//! precedence in prose as "capability fit → least-used → preferred → order".
//! This extraction is intentionally **behavior-preserving** for the existing
//! ACP profile consumer (routing.md's own RTG-10 wording), so the
//! already-implemented least-used-first order is kept rather than reordered
//! to match the prose; reordering would change today's ACP profile selection
//! results and is left for a follow-up if the prose order is what's wanted
//! once org-member resolution actually lands.

use std::collections::{HashMap, HashSet};

/// Minimal shape the scoring engine needs from a candidate, regardless of
/// what the candidate represents (an ACP profile today, an org member once
/// ORG lands).
pub trait ScoringCandidate {
    /// Stable identity used for usage counts and preference lookups (an ACP
    /// profile name, an org member id, ...).
    fn scoring_id(&self) -> &str;
    /// Normalized (lowercase, deduped) capability set.
    fn scoring_capabilities(&self) -> &HashSet<String>;
    /// Declaration/registry order — the final, deterministic tie-break.
    fn scoring_index(&self) -> usize;
}

/// Per-candidate scoring evidence, independent of the candidate's domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreEntry {
    pub id: String,
    pub eligible: bool,
    pub matched_capabilities: Vec<String>,
    pub missing_capabilities: Vec<String>,
    pub usage_count: usize,
    pub capability_fit_score: usize,
    pub preference_rank: Option<usize>,
    pub order_index: usize,
    pub reason: String,
}

/// A candidate rejected before final selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedCandidate {
    pub id: String,
    pub missing_capabilities: Vec<String>,
    pub reason: String,
}

/// Normalize a list of free-form strings (capabilities, preferred ids): trim,
/// lowercase, drop empties, dedup, sort. Used for required-capability lists.
pub fn normalize_capabilities(values: &[String]) -> Vec<String> {
    let mut normalized = values
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

/// Normalize a preference-ordered list: trim, drop empties, dedup, but
/// **preserve order** — preference rank depends on position.
pub fn normalize_preferences(values: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        normalized.push(value.to_string());
    }
    normalized
}

/// Whether a candidate's capabilities are a superset of the required set.
pub fn matches_required_capabilities(
    capabilities: &HashSet<String>,
    required_capabilities: &[String],
) -> bool {
    required_capabilities
        .iter()
        .all(|required| capabilities.contains(required))
}

/// Candidates whose capabilities are a superset of the required set.
pub fn matching_candidates<'a, C: ScoringCandidate>(
    candidates: &'a [C],
    required_capabilities: &[String],
) -> Vec<&'a C> {
    candidates
        .iter()
        .filter(|candidate| {
            matches_required_capabilities(candidate.scoring_capabilities(), required_capabilities)
        })
        .collect()
}

/// How closely a candidate's capability set fits the requirement — lower is
/// tighter. Extra capabilities beyond what's required count against fit.
pub fn capability_fit_score(capabilities: &HashSet<String>, required_count: usize) -> usize {
    capabilities.len().saturating_sub(required_count)
}

/// Score every candidate against a requirement + preference + usage context.
/// Explanations are populated for both eligible and ineligible candidates.
pub fn score_candidates<C: ScoringCandidate>(
    candidates: &[C],
    required_capabilities: &[String],
    preferred_ids: &[String],
    usage_counts: &HashMap<String, usize>,
) -> Vec<ScoreEntry> {
    candidates
        .iter()
        .map(|candidate| {
            let capabilities = candidate.scoring_capabilities();
            let mut matched_capabilities = Vec::new();
            let mut missing_capabilities = Vec::new();
            for required in required_capabilities {
                if capabilities.contains(required) {
                    matched_capabilities.push(required.clone());
                } else {
                    missing_capabilities.push(required.clone());
                }
            }
            let eligible = missing_capabilities.is_empty();
            let preference_rank = preferred_ids
                .iter()
                .position(|id| id == candidate.scoring_id());
            let usage_count = usage_counts
                .get(candidate.scoring_id())
                .copied()
                .unwrap_or(0);
            let capability_fit_score = capability_fit_score(capabilities, required_capabilities.len());
            let reason = if eligible {
                format!(
                    "eligible: selected_count={usage_count}, capability_fit_score={capability_fit_score}"
                )
            } else {
                format!(
                    "missing required capabilities [{}]",
                    missing_capabilities.join(", ")
                )
            };
            ScoreEntry {
                id: candidate.scoring_id().to_string(),
                eligible,
                matched_capabilities,
                missing_capabilities,
                usage_count,
                capability_fit_score,
                preference_rank,
                order_index: candidate.scoring_index(),
                reason,
            }
        })
        .collect()
}

/// Ineligible candidates, carrying their rejection reason forward.
pub fn rejected_candidates(scores: &[ScoreEntry]) -> Vec<RejectedCandidate> {
    scores
        .iter()
        .filter(|score| !score.eligible)
        .map(|score| RejectedCandidate {
            id: score.id.clone(),
            missing_capabilities: score.missing_capabilities.clone(),
            reason: score.reason.clone(),
        })
        .collect()
}

/// Pick the best eligible candidate. Tie-break precedence, in order:
/// **least-used → capability fit (tightest) → preferred (earliest) →
/// declaration order (earliest)**. See the module doc for why this order —
/// not the "capability fit first" prose in routing.md — is what ships today.
pub fn select_best<'a, C: ScoringCandidate>(
    eligible: &[&'a C],
    required_count: usize,
    preferred_ids: &[String],
    usage_counts: &HashMap<String, usize>,
) -> &'a C {
    eligible
        .iter()
        .copied()
        .min_by_key(|candidate| {
            let preference_rank = preferred_ids
                .iter()
                .position(|id| id == candidate.scoring_id())
                .unwrap_or(usize::MAX);
            (
                usage_counts
                    .get(candidate.scoring_id())
                    .copied()
                    .unwrap_or(0),
                capability_fit_score(candidate.scoring_capabilities(), required_count),
                preference_rank,
                candidate.scoring_index(),
            )
        })
        .expect("eligible candidates are non-empty")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestCandidate {
        id: &'static str,
        capabilities: HashSet<String>,
        index: usize,
    }

    impl TestCandidate {
        fn new(id: &'static str, capabilities: &[&str], index: usize) -> Self {
            Self {
                id,
                capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
                index,
            }
        }
    }

    impl ScoringCandidate for TestCandidate {
        fn scoring_id(&self) -> &str {
            self.id
        }

        fn scoring_capabilities(&self) -> &HashSet<String> {
            &self.capabilities
        }

        fn scoring_index(&self) -> usize {
            self.index
        }
    }

    fn ids(required: &[&str]) -> Vec<String> {
        required.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn matches_required_capabilities_checks_superset() {
        let caps: HashSet<String> = ["read", "write"].iter().map(|s| s.to_string()).collect();
        assert!(matches_required_capabilities(&caps, &ids(&["read"])));
        assert!(!matches_required_capabilities(&caps, &ids(&["execute"])));
    }

    #[test]
    fn capability_fit_score_counts_extra_capabilities() {
        let caps: HashSet<String> = ["read", "write", "execute"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(capability_fit_score(&caps, 1), 2);
        assert_eq!(capability_fit_score(&caps, 3), 0);
        // saturating: never goes negative even if required > capabilities.
        assert_eq!(capability_fit_score(&caps, 10), 0);
    }

    #[test]
    fn score_candidates_reports_matched_and_missing() {
        let candidates = vec![
            TestCandidate::new("a", &["read", "write"], 0),
            TestCandidate::new("b", &["read"], 1),
        ];
        let required = ids(&["read", "write"]);
        let scores = score_candidates(&candidates, &required, &[], &HashMap::new());
        assert!(scores[0].eligible);
        assert_eq!(scores[0].missing_capabilities, Vec::<String>::new());
        assert!(!scores[1].eligible);
        assert_eq!(scores[1].missing_capabilities, vec!["write".to_string()]);

        let rejected = rejected_candidates(&scores);
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].id, "b");
        assert_eq!(rejected[0].missing_capabilities, vec!["write".to_string()]);
    }

    /// Tie-break level 1: when capability fit, preference, and order would
    /// all pick differently, least-used wins.
    #[test]
    fn select_best_prefers_least_used_first() {
        let candidates = vec![
            // tighter fit, preferred, earlier order... but heavily used.
            TestCandidate::new("busy", &["read"], 0),
            // looser fit, not preferred, later order, but never used.
            TestCandidate::new("idle", &["read", "write", "execute"], 1),
        ];
        let refs: Vec<&TestCandidate> = candidates.iter().collect();
        let preferred = ids(&["busy"]);
        let mut usage = HashMap::new();
        usage.insert("busy".to_string(), 5);
        usage.insert("idle".to_string(), 0);

        let chosen = select_best(&refs, 1, &preferred, &usage);
        assert_eq!(chosen.id, "idle");
    }

    /// Tie-break level 2: usage tied, so tightest capability fit wins.
    #[test]
    fn select_best_falls_back_to_capability_fit() {
        let candidates = vec![
            TestCandidate::new("loose", &["read", "write", "execute"], 0),
            TestCandidate::new("tight", &["read"], 1),
        ];
        let refs: Vec<&TestCandidate> = candidates.iter().collect();
        let usage = HashMap::new();

        let chosen = select_best(&refs, 1, &[], &usage);
        assert_eq!(chosen.id, "tight");
    }

    /// Tie-break level 3: usage and fit tied, so declared preference wins.
    #[test]
    fn select_best_falls_back_to_preference() {
        let candidates = vec![
            TestCandidate::new("a", &["read"], 0),
            TestCandidate::new("b", &["read"], 1),
        ];
        let refs: Vec<&TestCandidate> = candidates.iter().collect();
        let preferred = ids(&["b"]);
        let usage = HashMap::new();

        let chosen = select_best(&refs, 1, &preferred, &usage);
        assert_eq!(chosen.id, "b");
    }

    /// Tie-break level 4: usage, fit, and preference all tied, so
    /// declaration order wins (earliest index).
    #[test]
    fn select_best_falls_back_to_declaration_order() {
        let candidates = vec![
            TestCandidate::new("second", &["read"], 1),
            TestCandidate::new("first", &["read"], 0),
        ];
        let refs: Vec<&TestCandidate> = candidates.iter().collect();
        let usage = HashMap::new();

        let chosen = select_best(&refs, 1, &[], &usage);
        assert_eq!(chosen.id, "first");
    }

    #[test]
    fn normalize_capabilities_trims_lowercases_dedups_sorts() {
        let values = vec![
            " Read ".to_string(),
            "WRITE".to_string(),
            "read".to_string(),
            "".to_string(),
        ];
        assert_eq!(normalize_capabilities(&values), vec!["read", "write"]);
    }

    #[test]
    fn normalize_preferences_preserves_order_dedups() {
        let values = vec![
            " b ".to_string(),
            "a".to_string(),
            "b".to_string(),
            "".to_string(),
        ];
        assert_eq!(normalize_preferences(&values), vec!["b", "a"]);
    }
}
