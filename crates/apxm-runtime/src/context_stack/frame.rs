//! File-loading and truncation helpers for ContextStack frames.

use std::path::Path;

use apxm_core::constants::session;
use apxm_core::paths::session_node_dir_name;

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

pub fn estimate_tokens(text: &str) -> usize {
    let char_count = text.chars().count();
    if char_count == 0 {
        0
    } else {
        char_count.div_ceil(4)
    }
}

fn byte_index_for_char_count(text: &str, max_chars: usize) -> usize {
    if max_chars == 0 {
        return 0;
    }
    text.char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}

pub fn truncate_to_budget(text: &str, max_tokens: usize) -> (String, bool) {
    let max_chars = max_tokens.saturating_mul(4);
    let total_chars = text.chars().count();

    if total_chars <= max_chars {
        return (text.to_string(), false);
    }

    let truncated_chars = total_chars.saturating_sub(max_chars);
    let prefix = &text[..byte_index_for_char_count(text, max_chars)];
    (
        format!("{}\n... [truncated {} chars]", prefix, truncated_chars),
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::constants::session;
    use apxm_core::paths::session_node_dir_name;
    use tempfile::tempdir;

    #[test]
    fn load_node_output_reads_session_workspace() {
        let dir = tempdir().expect("tempdir");
        let node_dir = dir
            .path()
            .join(session::files::NODES_DIR)
            .join(session_node_dir_name(1, "seed"));
        std::fs::create_dir_all(&node_dir).expect("node dir");
        std::fs::write(node_dir.join(session::node::OUTPUT_JSON), "hello").expect("output");

        assert_eq!(
            load_node_output(dir.path(), 1, "seed").as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn truncate_within_budget_keeps_text() {
        let (result, truncated) = truncate_to_budget("hello", 100);
        assert_eq!(result, "hello");
        assert!(!truncated);
    }

    #[test]
    fn truncate_exceeds_budget_marks_output() {
        let long_text = "a".repeat(1000);
        let (result, truncated) = truncate_to_budget(&long_text, 100);

        assert!(truncated);
        assert!(result.contains("truncated"));
        assert!(result.len() < long_text.len());
    }
}
