//! Demand-paged prompt context assembled from session node workspaces.

mod budget;
mod frame;
mod policy;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use apxm_core::types::operations::AISOperationType;
use serde::{Deserialize, Serialize};

pub use budget::BudgetAllocator;
pub use frame::{
    estimate_tokens, load_graph_summary, load_node_output, load_node_prompt, load_node_status,
    truncate_to_budget,
};
pub use policy::ScopeRules;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextStackConfig {
    pub session_dir: PathBuf,
    #[serde(default)]
    pub node_metadata: HashMap<u64, NodeMetadata>,
    #[serde(default)]
    pub graph_edges: Vec<(u64, u64)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeMetadata {
    pub name: String,
    pub op_type: AISOperationType,
}

#[derive(Clone)]
pub struct ContextStack {
    session_dir: PathBuf,
    node_metadata: Arc<HashMap<u64, NodeMetadata>>,
    graph_edges: Arc<Vec<(u64, u64)>>,
    memory: Option<Arc<crate::memory::MemorySystem>>,
    execution_id: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ContextAssembly {
    pub frames: Vec<ContextFrame>,
    pub total_estimated_tokens: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug)]
pub struct ContextFrame {
    pub scope: ContextScope,
    pub node_id: Option<u64>,
    pub node_name: Option<String>,
    pub content: String,
    pub token_estimate: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContextScope {
    Session,
    Upstream(u64),
    Local,
}

impl ContextStack {
    pub fn new(
        session_dir: PathBuf,
        node_metadata: Arc<HashMap<u64, NodeMetadata>>,
        graph_edges: Arc<Vec<(u64, u64)>>,
    ) -> Self {
        Self {
            session_dir,
            node_metadata,
            graph_edges,
            memory: None,
            execution_id: None,
        }
    }

    pub fn with_memory(
        mut self,
        memory: Arc<crate::memory::MemorySystem>,
        execution_id: String,
    ) -> Self {
        self.memory = Some(memory);
        self.execution_id = Some(execution_id);
        self
    }

    pub fn from_config(config: &ContextStackConfig) -> Self {
        Self::new(
            config.session_dir.clone(),
            Arc::new(config.node_metadata.clone()),
            Arc::new(config.graph_edges.clone()),
        )
    }

    pub fn assemble(&self, node_id: u64, profile: &str, token_budget: usize) -> ContextAssembly {
        let rules = ScopeRules::for_profile(profile);
        let mut allocator = BudgetAllocator::new(token_budget);
        let mut frames = Vec::new();
        let mut truncated = false;

        let session_content = self.session_frame_content();
        self.push_frame(
            &mut frames,
            &mut allocator,
            ContextScope::Session,
            None,
            None,
            session_content,
            rules.session_frame_budget,
            &mut truncated,
        );

        if let Some(local_content) = self.local_frame_content(node_id, profile) {
            let local_budget = allocator.remaining().min(estimate_tokens(&local_content));
            self.push_frame(
                &mut frames,
                &mut allocator,
                ContextScope::Local,
                Some(node_id),
                self.node_metadata
                    .get(&node_id)
                    .map(|meta| meta.name.clone()),
                local_content,
                local_budget,
                &mut truncated,
            );
        }

        let upstream_chain = self.upstream_chain(node_id, rules.upstream_depth);
        for upstream_id in upstream_chain {
            let Some(meta) = self.node_metadata.get(&upstream_id) else {
                continue;
            };
            let Some(content) = self.upstream_frame_content(upstream_id, meta, &rules) else {
                continue;
            };

            if !self.push_frame(
                &mut frames,
                &mut allocator,
                ContextScope::Upstream(upstream_id),
                Some(upstream_id),
                Some(meta.name.clone()),
                content,
                rules.upstream_frame_budget,
                &mut truncated,
            ) {
                break;
            }
        }

        ContextAssembly {
            total_estimated_tokens: frames.iter().map(|frame| frame.token_estimate).sum(),
            frames,
            truncated,
        }
    }

    fn push_frame(
        &self,
        frames: &mut Vec<ContextFrame>,
        allocator: &mut BudgetAllocator,
        scope: ContextScope,
        node_id: Option<u64>,
        node_name: Option<String>,
        content: String,
        max_frame_budget: usize,
        truncated: &mut bool,
    ) -> bool {
        let estimated = estimate_tokens(&content);
        if estimated == 0 || max_frame_budget == 0 {
            return !allocator.is_exhausted();
        }

        let requested = estimated.min(max_frame_budget);
        let allocated = allocator.allocate(requested);
        if allocated == 0 {
            if !content.is_empty() {
                *truncated = true;
            }
            return false;
        }

        let (content, frame_truncated) = truncate_to_budget(&content, allocated);
        *truncated |= frame_truncated;
        frames.push(ContextFrame {
            scope,
            node_id,
            node_name,
            token_estimate: estimate_tokens(&content),
            content,
        });

        !allocator.is_exhausted()
    }

    fn session_frame_content(&self) -> String {
        let execution_id = self
            .session_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown");

        let mut content = format!(
            "- Execution: {}\n- Session dir: {}",
            execution_id,
            self.session_dir.display()
        );

        // Include workflow summary if available.
        if let Some(summary) = load_graph_summary(&self.session_dir) {
            if let Ok(summary_json) = serde_json::from_str::<serde_json::Value>(&summary) {
                if let Some(name) = summary_json.get("name").and_then(|v| v.as_str()) {
                    content.push_str(&format!("\n- Workflow: {}", name));
                }
                if let Some(node_count) = summary_json.get("node_count").and_then(|v| v.as_u64()) {
                    content.push_str(&format!("\n- Nodes: {}", node_count));
                }
                if let Some(edge_count) = summary_json.get("edge_count").and_then(|v| v.as_u64()) {
                    content.push_str(&format!("\n- Edges: {}", edge_count));
                }
            }
        }

        content
    }

    fn local_frame_content(&self, node_id: u64, profile: &str) -> Option<String> {
        let meta = self.node_metadata.get(&node_id)?;
        let op_type = meta.op_type.to_string();
        Some(format!(
            "- Node: {} (#{})\n- Operation: {}\n- Profile: {}",
            meta.name, node_id, op_type, profile
        ))
    }

    fn upstream_frame_content(
        &self,
        node_id: u64,
        meta: &NodeMetadata,
        rules: &ScopeRules,
    ) -> Option<String> {
        let mut sections = Vec::new();

        if rules.include_upstream_prompts
            && let Some(prompt) = load_node_prompt(&self.session_dir, node_id, &meta.name)
        {
            sections.push(("Prompt", prompt));
        }

        if let Some(output) = load_node_output(&self.session_dir, node_id, &meta.name) {
            sections.push(("Output", output));
        }

        // Also query memory for additional context
        if let (Some(memory), Some(exec_id)) = (&self.memory, &self.execution_id) {
            let op_type = meta.op_type.to_string();
            // Query STM for node-specific context
            // Use block_in_place to avoid nested runtime panic when called from async context
            let handle = tokio::runtime::Handle::try_current().ok()?;
            let scope_key = format!("node:{}", node_id);
            if let Ok(Some(stm_value)) = tokio::task::block_in_place(|| {
                handle.block_on(memory.read_scoped(
                    crate::memory::MemorySpace::Stm,
                    exec_id,
                    &scope_key,
                ))
            }) {
                // Convert value to string representation
                sections.push(("STM Context", format!("{:?}", stm_value)));
            }

            // Query episodic memory for node-related events
            if let Ok(entries) =
                tokio::task::block_in_place(|| handle.block_on(memory.query_episodes(exec_id)))
            {
                let node_events: Vec<_> = entries
                    .iter()
                    .filter(|e| {
                        // Filter events that might be related to this node
                        e.event_type.contains(&meta.name) || e.event_type.contains(&op_type)
                    })
                    .take(3) // Limit to most recent 3 events
                    .collect();

                if !node_events.is_empty() {
                    let episodic_text = node_events
                        .iter()
                        .map(|e| format!("- {}: {:?}", e.event_type, e.payload))
                        .collect::<Vec<_>>()
                        .join("\n");
                    sections.push(("Recent Events", episodic_text));
                }
            }
        }

        if sections.is_empty() {
            return None;
        }

        let mut content = format!("- Node: {}\n- Operation: {}\n", meta.name, meta.op_type);
        for (label, text) in sections {
            content.push_str("\n");
            content.push_str("### ");
            content.push_str(label);
            content.push_str("\n");
            content.push_str(&text);
            content.push('\n');
        }
        Some(content.trim_end().to_string())
    }

    fn upstream_chain(&self, node_id: u64, max_depth: usize) -> Vec<u64> {
        if max_depth == 0 {
            return Vec::new();
        }

        let mut queue = VecDeque::from([(node_id, 0usize)]);
        let mut seen = HashSet::from([node_id]);
        let mut ordered = Vec::new();

        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }

            for &(from, to) in self.graph_edges.iter() {
                if to != current || !seen.insert(from) {
                    continue;
                }
                ordered.push(from);
                queue.push_back((from, depth.saturating_add(1)));
            }
        }

        ordered
    }
}

impl fmt::Display for ContextAssembly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for frame in &self.frames {
            if !first {
                writeln!(f)?;
                writeln!(f)?;
            }
            first = false;

            match frame.scope {
                ContextScope::Session => writeln!(f, "## Session")?,
                ContextScope::Local => writeln!(f, "## Local")?,
                ContextScope::Upstream(node_id) => {
                    let node_name = frame.node_name.as_deref().unwrap_or("unknown");
                    writeln!(f, "## Upstream: {} (#{})", node_name, node_id)?;
                }
            }

            write!(f, "{}", frame.content)?;
        }

        Ok(())
    }
}

