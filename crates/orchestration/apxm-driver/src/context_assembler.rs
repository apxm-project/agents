//! Agent context assembly for per-node workspaces.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use apxm_core::agent_profile::AgentProfileRegistry;
use apxm_core::constants;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::paths::session_node_dir_name;
use apxm_core::types::{AISOperationType, Number, Value};
use apxm_runtime::memory::MemorySystem;

#[derive(Clone, Debug)]
pub struct WorkspaceNodeMetadata {
    pub name: String,
    pub op_type: AISOperationType,
    pub attributes: HashMap<String, Value>,
}

pub struct ContextAssembler {
    session_dir: PathBuf,
    execution_id: String,
    project_root: PathBuf,
    node_metadata: Arc<HashMap<u64, WorkspaceNodeMetadata>>,
    graph_edges: Arc<Vec<(u64, u64)>>,
    memory: Option<Arc<MemorySystem>>,
}

impl ContextAssembler {
    pub fn new(
        session_dir: PathBuf,
        execution_id: String,
        project_root: PathBuf,
        node_metadata: Arc<HashMap<u64, WorkspaceNodeMetadata>>,
        graph_edges: Arc<Vec<(u64, u64)>>,
    ) -> Self {
        Self {
            session_dir,
            execution_id,
            project_root,
            node_metadata,
            graph_edges,
            memory: None,
        }
    }

    pub fn with_memory(mut self, memory: Arc<MemorySystem>) -> Self {
        self.memory = Some(memory);
        self
    }

    pub fn assemble_claude_md(
        &self,
        node_id: u64,
        meta: &WorkspaceNodeMetadata,
        skill_names: &[String],
    ) -> io::Result<String> {
        Ok(self.render_agent_doc("Claude", node_id, meta, skill_names))
    }

    pub fn assemble_agents_md(
        &self,
        node_id: u64,
        meta: &WorkspaceNodeMetadata,
        skill_names: &[String],
    ) -> io::Result<String> {
        Ok(self.render_agent_doc("Codex", node_id, meta, skill_names))
    }

    fn render_agent_doc(
        &self,
        agent_label: &str,
        node_id: u64,
        meta: &WorkspaceNodeMetadata,
        skill_names: &[String],
    ) -> String {
        let upstream = self.load_upstream_outputs(node_id);
        let profile = meta
            .attributes
            .get(graph_attrs::PROFILE)
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let reg = AgentProfileRegistry::new();
        let agent_profile = reg.resolve(profile);
        let role_desc = agent_profile
            .map(|p| p.description.as_str())
            .unwrap_or("Work within the provided workspace.");
        let constraints: Vec<String> = agent_profile
            .map(|p| p.constraints.clone())
            .unwrap_or_default();
        let skills_to_show: &[String] = agent_profile
            .map(|p| p.allowed_skills.as_slice())
            .filter(|s| !s.is_empty())
            .unwrap_or(skill_names);
        let task = self.extract_task(meta, &upstream);

        let mut doc = String::new();
        doc.push_str(&format!("# {} Context\n\n", agent_label));
        doc.push_str("## Task\n");
        doc.push_str(&task);
        doc.push_str("\n\n## Role\n");
        doc.push_str(role_desc);
        doc.push_str("\n\n");

        if !upstream.is_empty() {
            doc.push_str("## Upstream Outputs\n");
            for (name, output) in upstream {
                doc.push_str(&format!("### {}\n{}\n\n", name, truncate(&output, 2000)));
            }
        }

        if let Some(history) = self.load_episodic_history(profile) {
            doc.push_str("## Agent History\n");
            doc.push_str(&history);
            doc.push_str("\n\n");
        }

        if !skills_to_show.is_empty() {
            doc.push_str("## Available Skills\n");
            for skill in skills_to_show {
                doc.push_str(&format!("- {}: see skills/{}/SKILL.md\n", skill, skill));
            }
            doc.push('\n');
        }

        doc.push_str("## Session\n");
        doc.push_str(&format!("- Execution: {}\n", self.execution_id));
        doc.push_str(&format!(
            "- Project root: {}\n",
            self.project_root.display()
        ));
        doc.push_str("\n## Constraints\n");
        if constraints.is_empty() {
            doc.push_str("- Work within the provided workspace.\n");
        } else {
            for c in &constraints {
                doc.push_str(&format!("- {}\n", c));
            }
        }
        doc.push('\n');
        doc
    }

    fn extract_task(&self, meta: &WorkspaceNodeMetadata, upstream: &[(String, String)]) -> String {
        for key in [
            graph_attrs::TASK_SPEC,
            graph_attrs::PROMPT,
            graph_attrs::MESSAGE,
            graph_attrs::GOAL,
            graph_attrs::QUERY,
            graph_attrs::CONDITION,
        ] {
            if let Some(text) = meta.attributes.get(key).and_then(|v| v.as_str())
                && !text.trim().is_empty()
            {
                return text.to_string();
            }
        }

        if let Some((_, text)) = upstream.first()
            && !text.trim().is_empty()
        {
            return text.clone();
        }

        format!("Complete the work assigned to the '{}' node.", meta.name)
    }

    fn load_upstream_outputs(&self, node_id: u64) -> Vec<(String, String)> {
        let mut outputs = Vec::new();
        for (from, to) in self.graph_edges.iter() {
            if *to != node_id {
                continue;
            }
            let Some(meta) = self.node_metadata.get(from) else {
                continue;
            };
            let output_path = self
                .session_dir
                .join(constants::session::files::NODES_DIR)
                .join(session_node_dir_name(*from, &meta.name))
                .join(constants::session::node::OUTPUT_JSON);
            if let Ok(content) = fs::read_to_string(&output_path) {
                outputs.push((meta.name.clone(), content));
            }
        }
        outputs
    }

    fn load_episodic_history(&self, _profile: &str) -> Option<String> {
        let memory = self.memory.as_ref()?;

        // Query episodic memory for recent entries from this execution
        // Use block_in_place to avoid nested runtime panic when called from async context
        let handle = tokio::runtime::Handle::try_current().ok()?;
        let entries = tokio::task::block_in_place(|| {
            handle.block_on(memory.query_episodes(&self.execution_id))
        })
        .ok()?;

        if entries.is_empty() {
            return None;
        }

        let mut history = String::new();
        history.push_str("Previous execution patterns for this session:\n");

        // Aggregate statistics by event type
        let mut event_counts: HashMap<String, usize> = HashMap::new();
        let mut operation_stats: HashMap<String, Vec<f64>> = HashMap::new();

        for entry in &entries {
            *event_counts.entry(entry.event_type.clone()).or_insert(0) += 1;

            // Extract operation timing if available
            if entry.event_type.starts_with("operation_completed:") {
                if let Value::Object(ref map) = entry.payload {
                    if let Some(Value::Number(Number::Float(duration))) = map.get("duration_ms") {
                        operation_stats
                            .entry(entry.event_type.clone())
                            .or_default()
                            .push(*duration);
                    }
                }
            }
        }

        // Report top event types
        let mut event_vec: Vec<_> = event_counts.into_iter().collect();
        event_vec.sort_by(|a, b| b.1.cmp(&a.1));

        for (event_type, count) in event_vec.iter().take(5) {
            history.push_str(&format!("  - {}: {} occurrences\n", event_type, count));

            // Add average duration if available
            if let Some(durations) = operation_stats.get(event_type) {
                if !durations.is_empty() {
                    let avg = durations.iter().sum::<f64>() / durations.len() as f64;
                    history.push_str(&format!("    avg response time: {:.1}ms\n", avg));
                }
            }
        }

        Some(history)
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        text.to_string()
    } else {
        format!(
            "{}\n[truncated {} chars]",
            &text[..max_chars],
            text.len() - max_chars
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn assembles_agents_doc_with_upstream_output() {
        let dir = tempdir().expect("tempdir");
        let session_dir = dir.path().join("session");
        let upstream_dir = session_dir
            .join(constants::session::files::NODES_DIR)
            .join(session_node_dir_name(1, "seed"));
        fs::create_dir_all(&upstream_dir).expect("upstream dir");
        fs::write(
            upstream_dir.join(constants::session::node::OUTPUT_JSON),
            "{\"result\":\"design\"}",
        )
        .expect("output");

        let mut metadata = HashMap::new();
        metadata.insert(
            1,
            WorkspaceNodeMetadata {
                name: "seed".to_string(),
                op_type: AISOperationType::ConstStr,
                attributes: HashMap::new(),
            },
        );
        metadata.insert(
            2,
            WorkspaceNodeMetadata {
                name: "implement".to_string(),
                op_type: AISOperationType::SpawnAgent,
                attributes: HashMap::from([(
                    graph_attrs::PROFILE.to_string(),
                    Value::String("codex".to_string()),
                )]),
            },
        );

        let assembler = ContextAssembler::new(
            session_dir,
            "exec-123".to_string(),
            dir.path().to_path_buf(),
            Arc::new(metadata),
            Arc::new(vec![(1, 2)]),
        );
        let meta = assembler.node_metadata.get(&2).unwrap();
        let doc = assembler
            .assemble_agents_md(2, meta, &["extend".to_string()])
            .expect("doc");

        assert!(doc.contains("Codex Context"));
        assert!(doc.contains("seed"));
        assert!(doc.contains("extend"));
        assert!(doc.contains("exec-123"));
    }
}
