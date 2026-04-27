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

pub fn estimate_tokens(text: &str) -> usize {
    bpe_openai::o200k_base().count(text)
}

pub fn truncate_to_budget(text: &str, max_tokens: usize) -> (String, bool) {
    let tokenizer = bpe_openai::o200k_base();
    let tokens = tokenizer.encode(text);

    if tokens.len() <= max_tokens {
        return (text.to_string(), false);
    }

    let dropped_tokens = tokens.len().saturating_sub(max_tokens);
    let mut keep = max_tokens.min(tokens.len());
    let prefix = loop {
        if let Some(decoded) = tokenizer.decode(&tokens[..keep]) {
            break decoded;
        }
        if keep == 0 {
            break String::new();
        }
        keep -= 1;
    };
    (
        format!("{}\n... [truncated {} tokens]", prefix, dropped_tokens),
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
