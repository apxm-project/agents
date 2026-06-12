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

