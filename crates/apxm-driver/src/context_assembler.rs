//! Agent context assembly for per-node workspaces.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use apxm_core::constants;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::paths::session_node_dir_name;
use apxm_core::types::{AISOperationType, Value};

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceNodeMetadata {
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
        }
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
        let role = infer_role(profile, &meta.name);
        let task = self.extract_task(meta, &upstream);

        let mut doc = String::new();
        doc.push_str(&format!("# {} Context\n\n", agent_label));
        doc.push_str("## Task\n");
        doc.push_str(&task);
        doc.push_str("\n\n## Role\n");
        doc.push_str(role_description(agent_label, role));
        doc.push_str("\n\n");

        if !upstream.is_empty() {
            doc.push_str("## Upstream Outputs\n");
            for (name, output) in upstream {
                doc.push_str(&format!("### {}\n{}\n\n", name, truncate(&output, 2000)));
            }
        }

        if !skill_names.is_empty() {
            doc.push_str("## Available Skills\n");
            for skill in skill_names {
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
        doc.push_str(profile_constraints(role));
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
}

fn infer_role(profile: &str, node_name: &str) -> &'static str {
    let lowered = node_name.to_ascii_lowercase();
    if lowered.contains("architect") {
        "Architect"
    } else if lowered.contains("review") {
        "Reviewer"
    } else if lowered.contains("implement") || lowered.contains("coder") {
        "Implementer"
    } else if profile.eq_ignore_ascii_case("claude") || profile.eq_ignore_ascii_case("codex") {
        "Agent"
    } else {
        "Worker"
    }
}

fn role_description(agent_label: &str, role: &str) -> &'static str {
    match (agent_label, role) {
        ("Claude", "Architect") => {
            "You are the design agent. Focus on architecture, interfaces, and execution strategy."
        }
        ("Codex", "Implementer") => {
            "You are the implementation agent. Make concrete code changes that satisfy the provided design."
        }
        (_, "Reviewer") => {
            "You are the review agent. Look for correctness issues, regressions, and missing coverage."
        }
        ("Claude", _) => "You are Claude operating inside an APXM node workspace.",
        ("Codex", _) => "You are Codex operating inside an APXM node workspace.",
        _ => "You are an APXM agent operating inside a node workspace.",
    }
}

fn profile_constraints(role: &str) -> &'static str {
    match role {
        "Architect" => {
            "- Stay at the design/interface level when the task is explicitly architectural.\n- Use upstream outputs as the source of truth for requirements."
        }
        "Reviewer" => {
            "- Prioritize bugs, regressions, and missing tests.\n- Keep summaries brief after the findings."
        }
        _ => {
            "- Work within the provided workspace.\n- Preserve existing project conventions unless the task requires a change."
        }
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
