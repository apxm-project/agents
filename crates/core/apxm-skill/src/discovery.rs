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
    use super::*;

    fn card(
        id: &str,
        lib: Option<&str>,
        desc: &str,
        when: &str,
        tags: &[&str],
        shared: bool,
    ) -> SkillCard {
        SkillCard {
            skill_id: id.to_string(),
            library: lib.map(|s| s.to_string()),
            description: desc.to_string(),
            when_to_use: when.to_string(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            shared,
        }
    }

    fn catalogue() -> Vec<SkillCard> {
        vec![
            card(
                "pdf_extract",
                Some("docs"),
                "Extract text and tables from PDF files",
                "Use when the user mentions PDFs or document extraction",
                &["pdf", "document"],
                false,
            ),
            card(
                "web_search",
                None,
                "Search the web for current information",
                "Use when the user needs fresh facts",
                &["search", "web"],
                true,
            ),
            card(
                "deploy",
                Some("ops"),
                "Deploy a service to production",
                "Use when the user asks to ship or release",
                &["deploy", "release"],
                false,
            ),
        ]
    }

    #[test]
    fn shared_tier_is_visible_without_import() {
        let v = VisibleSet::default(); // imports nothing
        let cat = catalogue();
        assert!(v.sees(&cat[1])); // web_search is shared
        assert!(!v.sees(&cat[0])); // pdf_extract is scoped, not imported
        assert!(!v.sees(&cat[2])); // deploy is scoped, not imported
    }

    #[test]
    fn whole_library_import_makes_its_skills_visible() {
        let v = VisibleSet::from_imports(["docs"]);
        let cat = catalogue();
        assert!(v.sees(&cat[0])); // docs::pdf_extract now visible
        assert!(!v.sees(&cat[2])); // ops not imported
    }

    #[test]
    fn namespaced_and_bare_imports_work() {
        let cat = catalogue();
        assert!(VisibleSet::from_imports(["ops::deploy"]).sees(&cat[2]));
        assert!(VisibleSet::from_imports(["deploy"]).sees(&cat[2]));
        assert!(!VisibleSet::from_imports(["ops::other"]).sees(&cat[2]));
    }

    #[test]
    fn rank_only_returns_visible_skills() {
        // Query matches pdf_extract, but it is not imported and not shared.
        let v = VisibleSet::default();
        let hits = rank("extract tables from a pdf", &catalogue(), &v, 5);
        assert!(
            hits.iter().all(|m| m.skill_id != "pdf_extract"),
            "scoped skill must not leak into discovery: {hits:?}"
        );
    }

    #[test]
    fn rank_orders_by_relevance_within_visible_set() {
        let v = VisibleSet::from_imports(["docs", "ops"]);
        let hits = rank(
            "I need to extract a table from a PDF document",
            &catalogue(),
            &v,
            5,
        );
        assert_eq!(
            hits.first().map(|m| m.skill_id.as_str()),
            Some("pdf_extract"),
            "most relevant visible skill should rank first: {hits:?}"
        );
    }

    #[test]
    fn no_match_returns_empty_not_everything() {
        let v = VisibleSet::from_imports(["docs", "ops"]);
        let hits = rank("xyzzy quux nonsense", &catalogue(), &v, 5);
        assert!(
            hits.is_empty(),
            "irrelevant query must not return skills: {hits:?}"
        );
    }
}
