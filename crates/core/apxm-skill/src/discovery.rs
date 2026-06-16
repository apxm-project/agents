//! Scope-aware, description-based skill discovery.
//!
//! `rank` filters the catalogue to the agent's visible set (shared tier ∪
//! resolved imports) and ranks survivors against a natural-language request by
//! `description` / `when_to_use` / `tags` (lexical term overlap). This is the
//! deterministic core behind the `search_skills` capability.

use std::collections::BTreeSet;

/// Minimal discovery metadata for one skill — a row of the catalogue.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillCard {
    pub skill_id: String,
    /// Owning library / pack id, if any (used for `lib` and `lib::skill` imports
    /// and for same-library visibility).
    pub library: Option<String>,
    pub description: String,
    pub when_to_use: String,
    pub tags: Vec<String>,
    /// Part of the global shared tier (visible without an import).
    pub shared: bool,
}

/// The set of libraries / skills an agent has imported, plus the implicit
/// global (shared) tier. Decides which cards are visible.
#[derive(Debug, Clone, Default)]
pub struct VisibleSet {
    imports: BTreeSet<String>,
}

impl VisibleSet {
    /// Build from an agent's `imports` list. Entries may be library ids
    /// (`apxm-app-github`), namespaced skill ids (`apxm-app-github::triage`),
    /// or bare skill ids (`triage`).
    pub fn from_imports<I, S>(imports: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            imports: imports.into_iter().map(Into::into).collect(),
        }
    }

    /// A card is visible iff it is shared (global tier) or explicitly imported.
    pub fn sees(&self, card: &SkillCard) -> bool {
        if card.shared {
            return true; // global tier — ambient to every agent
        }
        // whole-library import
        if let Some(lib) = &card.library {
            if self.imports.contains(lib) {
                return true;
            }
            // namespaced skill import: lib::skill
            if self.imports.contains(&format!("{lib}::{}", card.skill_id)) {
                return true;
            }
        }
        // bare skill-id import
        self.imports.contains(&card.skill_id)
    }
}

/// A ranked discovery hit. Bodies are never returned — only enough to let the
/// agent decide which skill to load/call next (progressive disclosure).
#[derive(Debug, Clone, PartialEq)]
pub struct SkillMatch {
    pub skill_id: String,
    pub library: Option<String>,
    pub description: String,
    pub when_to_use: String,
    pub score: f32,
}

/// Rank the catalogue against `query`, restricted to the `visible` set, and
/// return up to `k` matches with score > 0, best first.
pub fn rank(query: &str, cards: &[SkillCard], visible: &VisibleSet, k: usize) -> Vec<SkillMatch> {
    let q_terms = tokenize(query);
    let mut scored: Vec<SkillMatch> = cards
        .iter()
        .filter(|c| visible.sees(c))
        .map(|c| SkillMatch {
            skill_id: c.skill_id.clone(),
            library: c.library.clone(),
            description: c.description.clone(),
            when_to_use: c.when_to_use.clone(),
            score: lexical_score(&q_terms, query, c),
        })
        .filter(|m| m.score > 0.0)
        .collect();

    // Stable sort by score desc, then skill_id asc for determinism.
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.skill_id.cmp(&b.skill_id))
    });
    scored.truncate(k);
    scored
}

/// Field weights: a hit in "use when" or a tag is a stronger signal than a hit
/// in the prose description, which beats a hit in the bare id.
const W_WHEN: f32 = 3.0;
const W_TAG: f32 = 3.0;
const W_DESC: f32 = 2.0;
const W_ID: f32 = 1.0;

/// Lexical relevance of one card to the query terms. Coverage of query terms,
/// each scored by the strongest field it appears in, plus a small phrase boost.
fn lexical_score(q_terms: &[String], raw_query: &str, card: &SkillCard) -> f32 {
    if q_terms.is_empty() {
        return 0.0;
    }
    let id_terms: BTreeSet<String> = tokenize(&card.skill_id).into_iter().collect();
    let desc_terms: BTreeSet<String> = tokenize(&card.description).into_iter().collect();
    let when_terms: BTreeSet<String> = tokenize(&card.when_to_use).into_iter().collect();
    let tag_terms: BTreeSet<String> = card.tags.iter().flat_map(|t| tokenize(t)).collect();

    let mut total = 0.0f32;
    for q in q_terms {
        let mut best = 0.0f32;
        if when_terms.contains(q) {
            best = best.max(W_WHEN);
        }
        if tag_terms.contains(q) {
            best = best.max(W_TAG);
        }
        if desc_terms.contains(q) {
            best = best.max(W_DESC);
        }
        if id_terms.contains(q) {
            best = best.max(W_ID);
        }
        total += best;
    }
    // Normalize by query length so longer queries don't dominate, then add a
    // small boost when the whole query phrase appears verbatim.
    let mut score = total / q_terms.len() as f32;
    let hay = format!(
        "{} {} {}",
        card.description.to_lowercase(),
        card.when_to_use.to_lowercase(),
        card.skill_id.to_lowercase()
    );
    let ql = raw_query.trim().to_lowercase();
    if ql.len() >= 3 && hay.contains(&ql) {
        score += 1.0;
    }
    score
}

/// Lowercase alphanumeric tokenizer with a tiny stop-word filter.
fn tokenize(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_lowercase())
        .filter(|t| !is_stopword(t))
        .collect()
}

fn is_stopword(t: &str) -> bool {
    matches!(
        t,
        "the"
            | "and"
            | "for"
            | "with"
            | "how"
            | "use"
            | "when"
            | "that"
            | "this"
            | "you"
            | "your"
            | "are"
            | "from"
            | "into"
            | "via"
    )
}

#[cfg(test)]
mod tests {
    use super::{SkillCard, VisibleSet, rank};

    fn card(
        skill_id: &str,
        library: Option<&str>,
        description: &str,
        when_to_use: &str,
        tags: &[&str],
        shared: bool,
    ) -> SkillCard {
        SkillCard {
            skill_id: skill_id.to_string(),
            library: library.map(str::to_string),
            description: description.to_string(),
            when_to_use: when_to_use.to_string(),
            tags: tags.iter().map(|tag| tag.to_string()).collect(),
            shared,
        }
    }

    fn catalogue() -> Vec<SkillCard> {
        vec![
            card(
                "ticket_triage",
                Some("support"),
                "Classify incoming bug reports by product area, urgency, and owner.",
                "Use when prioritizing GitHub issues, support tickets, incidents, and bug reports.",
                &["github", "issues", "triage", "severity"],
                false,
            ),
            card(
                "architecture_review",
                Some("engineering"),
                "Review software architecture boundaries, module dependencies, and implementation structure.",
                "Use for frontend composability, backend scalability, hooks, constants, enums, and module ownership.",
                &["architecture", "frontend", "backend", "composability"],
                false,
            ),
            card(
                "security_scan",
                Some("security"),
                "Find vulnerabilities, unsafe inputs, secret leakage, and risky dependencies.",
                "Use when reviewing authentication, authorization, dependency, sandbox, or data exposure risks.",
                &["security", "vulnerability", "secrets", "dependencies"],
                true,
            ),
            card(
                "release_notes",
                Some("docs"),
                "Draft release notes, changelogs, and upgrade summaries from merged changes.",
                "Use when summarizing pull requests, commits, milestones, and version changes for a release.",
                &["release", "changelog", "commits", "summary"],
                false,
            ),
            card(
                "data_viz",
                Some("analysis"),
                "Create charts, graphs, tables, and visual explanations for numeric results.",
                "Use when visualizing benchmark data, measurements, trends, latency, or evaluation outcomes.",
                &["charts", "graphs", "benchmarks", "metrics"],
                false,
            ),
        ]
    }

    #[test]
    fn visible_set_includes_shared_and_imported_skills() {
        let cards = catalogue();
        let visible = VisibleSet::from_imports(["engineering", "docs::release_notes"]);

        assert!(visible.sees(&cards[1]));
        assert!(visible.sees(&cards[2]));
        assert!(visible.sees(&cards[3]));
        assert!(!visible.sees(&cards[0]));
        assert!(!visible.sees(&cards[4]));
    }

    #[test]
    fn rank_is_deterministic_and_scope_aware() {
        let cards = catalogue();
        let visible = VisibleSet::from_imports(["support", "engineering", "docs"]);

        let matches = rank("review frontend hooks and module boundaries", &cards, &visible, 3);

        assert_eq!(matches[0].skill_id, "architecture_review");
        assert!(matches.iter().all(|m| m.skill_id != "data_viz"));
    }

    #[test]
    fn representative_requests_select_expected_skill_by_description() {
        let cards = catalogue();
        let visible = VisibleSet::from_imports(["support", "engineering", "docs", "analysis"]);
        let cases = [
            ("prioritize incoming bug report by severity", "ticket_triage"),
            ("review module boundaries and frontend composability", "architecture_review"),
            ("look for vulnerabilities and unsafe inputs", "security_scan"),
            ("write a changelog from merged commits", "release_notes"),
            ("make a chart of latency measurements", "data_viz"),
            ("check hooks composition and implementation structure", "architecture_review"),
            ("classify GitHub issues into owner buckets", "ticket_triage"),
            ("scan dependencies for secret handling risks", "security_scan"),
            ("summarize pull requests for a release", "release_notes"),
            ("visualize benchmark results as graphs", "data_viz"),
        ];

        let correct = cases
            .iter()
            .filter(|(query, expected)| {
                rank(query, &cards, &visible, 1)
                    .first()
                    .map(|m| m.skill_id.as_str() == *expected)
                    .unwrap_or(false)
            })
            .count();

        assert!(
            correct >= 8,
            "expected at least 8/10 representative selections, got {correct}/10"
        );
    }
}
