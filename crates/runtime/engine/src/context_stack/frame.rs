//! File-loading and truncation helpers for ContextStack frames.

use std::path::Path;

use apxm_core::constants::session;
use apxm_core::paths::session_node_dir_name;
use serde::{Deserialize, Serialize};

/// Tokenizer selected by the context-planning policy.
///
/// Context assembly records this identity with every plan so token estimates
/// and truncation can be interpreted against the configured implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextTokenizer {
    Cl100kBase,
    O200kBase,
}

impl ContextTokenizer {
    /// Stable identity emitted with context-planning evidence.
    pub const fn identity(self) -> &'static str {
        match self {
            Self::Cl100kBase => "cl100k_base",
            Self::O200kBase => "o200k_base",
        }
    }

    fn tokenizer(self) -> &'static bpe_openai::Tokenizer {
        match self {
            Self::Cl100kBase => bpe_openai::cl100k_base(),
            Self::O200kBase => bpe_openai::o200k_base(),
        }
    }
}

pub fn load_node_output(session_dir: &Path, node_id: u64, node_name: &str) -> Option<String> {
    let output_path = session_dir
        .join(session::files::NODES_DIR)
        .join(session_node_dir_name(node_id, node_name))
        .join(session::node::OUTPUT_JSON);
    std::fs::read_to_string(output_path).ok()
}

pub fn load_node_prompt(session_dir: &Path, node_id: u64, node_name: &str) -> Option<String> {
    let prompt_path = session_dir
        .join(session::files::NODES_DIR)
        .join(session_node_dir_name(node_id, node_name))
        .join(session::node::PROMPT_TXT);
    std::fs::read_to_string(prompt_path).ok()
}

pub fn load_node_status(session_dir: &Path, node_id: u64, node_name: &str) -> Option<String> {
    let status_path = session_dir
        .join(session::files::NODES_DIR)
        .join(session_node_dir_name(node_id, node_name))
        .join(session::node::STATUS_JSON);
    std::fs::read_to_string(status_path).ok()
}

pub fn load_graph_summary(session_dir: &Path) -> Option<String> {
    let summary_path = session_dir.join("graph_summary.json");
    std::fs::read_to_string(summary_path).ok()
}

/// Count tokens with the explicitly selected tokenizer.
pub fn estimate_tokens_with(tokenizer: ContextTokenizer, text: &str) -> usize {
    tokenizer.tokenizer().count(text)
}

/// Estimate tokens with the runtime's canonical default tokenizer.
pub fn estimate_tokens(text: &str) -> usize {
    estimate_tokens_with(ContextTokenizer::O200kBase, text)
}

/// Truncate with the explicitly selected tokenizer.
pub fn truncate_to_budget_with(
    tokenizer: ContextTokenizer,
    text: &str,
    max_tokens: usize,
) -> (String, bool) {
    let tokenizer = tokenizer.tokenizer();
    let tokens = tokenizer.encode(text);

    if tokens.len() <= max_tokens {
        return (text.to_string(), false);
    }

    let mut keep = max_tokens.min(tokens.len());
    let content = loop {
        if let Some(decoded) = tokenizer.decode(&tokens[..keep]) {
            let dropped_tokens = tokens.len().saturating_sub(keep);
            let candidate = format!("{}\n... [truncated {} tokens]", decoded, dropped_tokens);
            if tokenizer.count(&candidate) <= max_tokens {
                break candidate;
            }
        }
        if keep == 0 {
            break String::new();
        }
        keep -= 1;
    };
    (content, true)
}

/// Truncate with the runtime's canonical default tokenizer.
pub fn truncate_to_budget(text: &str, max_tokens: usize) -> (String, bool) {
    truncate_to_budget_with(ContextTokenizer::O200kBase, text, max_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed, checked-in tool-result fixture: a synthetic oversized
    /// `http_get`-shaped payload —
    /// deterministic content, no timestamps/random ids, so a byte-diff
    /// across runs can only come from a non-deterministic trimmer.
    const TOOL_RESULT_FIXTURE: &str = include_str!("./testdata/tool_result_fixture.txt");

    /// **Deterministic tool-result trimming:** trimming the SAME fixed payload
    /// at the SAME token budget N
    /// times in a row must produce byte-identical output every time — no
    /// wall-clock/model-call/hash-order dependence. This is the
    /// correctness argument for reusing `truncate_to_budget` (already
    /// deterministic — a pure function over a real bpe tokenizer) instead of
    /// re-deriving a chars/4 heuristic (`chat.rs`'s retired duplicate).
    #[test]
    fn truncate_to_budget_is_byte_identical_across_repeated_runs() {
        let budget = 64;
        let (first, first_truncated) =
            truncate_to_budget_with(ContextTokenizer::O200kBase, TOOL_RESULT_FIXTURE, budget);
        assert!(first_truncated, "fixture must exceed the trim budget");

        for _ in 0..50 {
            let (again, truncated_again) =
                truncate_to_budget_with(ContextTokenizer::O200kBase, TOOL_RESULT_FIXTURE, budget);
            assert_eq!(again, first, "trim output must be byte-identical every run");
            assert_eq!(truncated_again, first_truncated);
        }
    }

    /// The exact truncation marker and the head boundary are stable, not
    /// just "some" deterministic output — pins the literal marker text and
    /// proves the kept content is a genuine PREFIX of the original (head
    /// kept, tail dropped), matching the doc comment's "deterministic
    /// prefix-keep" claim.
    #[test]
    fn truncate_to_budget_marker_and_head_boundary_are_stable() {
        let (trimmed, was_truncated) =
            truncate_to_budget_with(ContextTokenizer::O200kBase, TOOL_RESULT_FIXTURE, 64);
        assert!(was_truncated);
        assert!(
            estimate_tokens_with(ContextTokenizer::O200kBase, &trimmed) <= 64,
            "truncation marker must remain inside the requested budget"
        );

        let (head, marker) = trimmed
            .rsplit_once("\n... [truncated ")
            .expect("exact truncation marker must be present");
        assert!(marker.ends_with(" tokens]"));
        assert!(
            TOOL_RESULT_FIXTURE.starts_with(head),
            "kept content must be a byte-for-byte prefix of the original"
        );
        assert!(!head.is_empty());

        // Re-trimming at the same budget reproduces the exact same boundary.
        let (trimmed_again, _) =
            truncate_to_budget_with(ContextTokenizer::O200kBase, TOOL_RESULT_FIXTURE, 64);
        assert_eq!(trimmed_again, trimmed);
    }

    /// A payload already within budget is returned byte-identical and
    /// unmarked — trimming is a no-op below the threshold, not a
    /// mandatory rewrite.
    #[test]
    fn truncate_to_budget_is_a_no_op_under_budget() {
        let small = "short tool result";
        let (out, truncated) = truncate_to_budget_with(ContextTokenizer::O200kBase, small, 1_000);
        assert_eq!(out, small);
        assert!(!truncated);
    }

    #[test]
    fn tokenizer_identity_is_selected_by_policy() {
        assert_eq!(ContextTokenizer::Cl100kBase.identity(), "cl100k_base");
        assert_eq!(ContextTokenizer::O200kBase.identity(), "o200k_base");
    }
}
