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

