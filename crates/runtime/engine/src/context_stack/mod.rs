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
    /// Typed provenance and budget evidence for the assembled context.
    pub plan: ContextPlan,
}

#[derive(Clone, Debug)]
pub struct ContextFrame {
    pub scope: ContextScope,
    pub node_id: Option<u64>,
    pub node_name: Option<String>,
    pub content: String,
    pub token_estimate: usize,
}

/// One typed context plan shared by all context consumers.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPlan {
    /// Stable tokenizer identity used for every segment estimate.
    pub tokenizer: String,
    /// Total caller-approved context capacity.
    pub token_budget: usize,
    /// Segments considered during deterministic packing, including omissions.
    #[serde(default)]
    pub segments: Vec<ContextPlanSegment>,
}

/// Content-free counters derived from a context plan for model-call telemetry.
///
/// This type intentionally excludes frames, provenance, scopes, permissions,
/// sensitivities, and segment contents. Those values remain in the execution
/// boundary and are never attached to provider requests or event payloads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextPlanMetrics {
    /// Maximum context tokens available to the plan.
    pub token_budget: usize,
    /// Tokens considered before plan admission.
    pub original_tokens: usize,
    /// Tokens admitted to the rendered context.
    pub admitted_tokens: usize,
    /// Segments retained without truncation.
    pub kept_segments: usize,
    /// Segments retained in truncated form.
    pub truncated_segments: usize,
    /// Segments omitted because the plan budget was exhausted.
    pub omitted_token_budget_segments: usize,
    /// Segments omitted because their source was empty.
    pub omitted_empty_segments: usize,
}

impl ContextPlan {
    /// Construct a plan using the runtime's canonical context tokenizer.
    pub fn new(token_budget: usize) -> Self {
        Self {
            tokenizer: "o200k_base".to_string(),
            token_budget,
            segments: Vec::new(),
        }
    }

    /// Return the token count actually admitted to the rendered context.
    pub fn admitted_tokens(&self) -> usize {
        self.segments
            .iter()
            .filter(|segment| {
                matches!(
                    segment.disposition,
                    ContextDisposition::Kept | ContextDisposition::Truncated
                )
            })
            .map(|segment| segment.token_estimate)
            .sum()
    }

    /// Derive only aggregate packing evidence suitable for observability.
    pub fn metrics(&self) -> ContextPlanMetrics {
        let mut metrics = ContextPlanMetrics {
            token_budget: self.token_budget,
            ..ContextPlanMetrics::default()
        };

        for segment in &self.segments {
            metrics.original_tokens = metrics
                .original_tokens
                .saturating_add(segment.original_token_estimate);
            match segment.disposition {
                ContextDisposition::Kept => {
                    metrics.admitted_tokens = metrics
                        .admitted_tokens
                        .saturating_add(segment.token_estimate);
                    metrics.kept_segments = metrics.kept_segments.saturating_add(1);
                }
                ContextDisposition::Truncated => {
                    metrics.admitted_tokens = metrics
                        .admitted_tokens
                        .saturating_add(segment.token_estimate);
                    metrics.truncated_segments = metrics.truncated_segments.saturating_add(1);
                }
                ContextDisposition::OmittedTokenBudget => {
                    metrics.omitted_token_budget_segments =
                        metrics.omitted_token_budget_segments.saturating_add(1);
                }
                ContextDisposition::OmittedEmpty => {
                    metrics.omitted_empty_segments =
                        metrics.omitted_empty_segments.saturating_add(1);
                }
            }
        }

        metrics
    }
}

/// One source segment considered by a [`ContextPlan`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPlanSegment {
    /// Context scope that owns this segment.
    pub scope: ContextScope,
    /// Stable source reference for audit and replay.
    pub provenance: String,
    /// Permission boundary under which the segment was selected.
    pub permission: ContextPermissionScope,
    /// Sensitivity classification used by future compressors and renderers.
    pub sensitivity: ContextSensitivity,
    /// Whether the segment must not be lossy-compressed or silently removed.
    pub protected: bool,
    /// Estimated tokens before packing.
    pub original_token_estimate: usize,
    /// Estimated tokens admitted after packing.
    pub token_estimate: usize,
    /// Kept, truncated, or omitted outcome with its reason.
    pub disposition: ContextDisposition,
}

/// Permission boundary carried with every context segment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextPermissionScope {
    /// Session-owned data visible to the current execution.
    Session,
    /// Node-owned data visible through a declared graph dependency.
    GraphDependency,
    /// Local execution metadata owned by the current node.
    Local,
}

/// Sensitivity classification used to protect context from lossy handling.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextSensitivity {
    /// Execution metadata contains no user or capability payload.
    Internal,
    /// User or workflow context remains within the execution boundary.
    Private,
    /// Capability, approval, or policy-bearing context requires protection.
    Sensitive,
}

/// Packing result and omission or compression reason for a context segment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextDisposition {
    /// The original segment fit without alteration.
    Kept,
    /// The deterministic tokenizer truncator retained a bounded extract.
    Truncated,
    /// The segment was omitted because no budget remained.
    OmittedTokenBudget,
    /// The source had no content to admit.
    OmittedEmpty,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
        let mut plan = ContextPlan::new(token_budget);

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
            &mut plan,
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
                &mut plan,
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
                &mut plan,
            ) {
                break;
            }
        }

        ContextAssembly {
            total_estimated_tokens: frames.iter().map(|frame| frame.token_estimate).sum(),
            frames,
            truncated,
            plan,
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
        plan: &mut ContextPlan,
    ) -> bool {
        let estimated = estimate_tokens(&content);
        if estimated == 0 || max_frame_budget == 0 {
            plan.segments.push(self.plan_segment(
                scope,
                node_id,
                node_name.as_deref(),
                estimated,
                0,
                if estimated == 0 {
                    ContextDisposition::OmittedEmpty
                } else {
                    ContextDisposition::OmittedTokenBudget
                },
            ));
            return !allocator.is_exhausted();
        }

        let requested = estimated.min(max_frame_budget);
        let allocated = allocator.allocate(requested);
        if allocated == 0 {
            if !content.is_empty() {
                *truncated = true;
            }
            plan.segments.push(self.plan_segment(
                scope,
                node_id,
                node_name.as_deref(),
                estimated,
                0,
                ContextDisposition::OmittedTokenBudget,
            ));
            return false;
        }

        let (content, frame_truncated) = truncate_to_budget(&content, allocated);
        *truncated |= frame_truncated;
        let disposition = if frame_truncated {
            ContextDisposition::Truncated
        } else {
            ContextDisposition::Kept
        };
        plan.segments.push(self.plan_segment(
            scope.clone(),
            node_id,
            node_name.as_deref(),
            estimated,
            estimate_tokens(&content),
            disposition,
        ));
        frames.push(ContextFrame {
            scope,
            node_id,
            node_name,
            token_estimate: estimate_tokens(&content),
            content,
        });

        !allocator.is_exhausted()
    }

    fn plan_segment(
        &self,
        scope: ContextScope,
        node_id: Option<u64>,
        node_name: Option<&str>,
        original_token_estimate: usize,
        token_estimate: usize,
        disposition: ContextDisposition,
    ) -> ContextPlanSegment {
        let (permission, sensitivity, protected, provenance) = match &scope {
            ContextScope::Session => (
                ContextPermissionScope::Session,
                ContextSensitivity::Internal,
                true,
                format!("session:{}", self.session_dir.display()),
            ),
            ContextScope::Local => (
                ContextPermissionScope::Local,
                ContextSensitivity::Private,
                true,
                format!(
                    "node:{}:{}",
                    node_id.unwrap_or_default(),
                    node_name.unwrap_or("unknown")
                ),
            ),
            ContextScope::Upstream(upstream_id) => (
                ContextPermissionScope::GraphDependency,
                ContextSensitivity::Private,
                false,
                format!(
                    "upstream:{}:{}",
                    upstream_id,
                    node_name.unwrap_or("unknown")
                ),
            ),
        };
        ContextPlanSegment {
            scope,
            provenance,
            permission,
            sensitivity,
            protected,
            original_token_estimate,
            token_estimate,
            disposition,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembly_records_protected_provenance_and_budget_omissions() {
        let stack = ContextStack::new(
            PathBuf::from("/tmp/apxm-context-plan"),
            Arc::new(HashMap::from([(
                7,
                NodeMetadata {
                    name: "answer".to_string(),
                    op_type: AISOperationType::Ask,
                },
            )])),
            Arc::new(Vec::new()),
        );

        let assembly = stack.assemble(7, "default", 0);
        assert_eq!(assembly.plan.tokenizer, "o200k_base");
        assert_eq!(assembly.plan.token_budget, 0);
        assert_eq!(assembly.plan.admitted_tokens(), 0);
        assert!(assembly.plan.segments.iter().any(|segment| {
            segment.scope == ContextScope::Session
                && segment.protected
                && segment.permission == ContextPermissionScope::Session
                && segment.disposition == ContextDisposition::OmittedTokenBudget
        }));
        assert!(assembly.plan.segments.iter().any(|segment| {
            segment.scope == ContextScope::Local
                && segment.protected
                && segment.provenance == "node:7:answer"
                && segment.disposition == ContextDisposition::OmittedTokenBudget
        }));
    }

    #[test]
    fn plan_metrics_expose_only_packing_aggregates() {
        let plan = ContextPlan {
            tokenizer: "o200k_base".to_string(),
            token_budget: 20,
            segments: vec![
                ContextPlanSegment {
                    scope: ContextScope::Session,
                    provenance: "session:private".to_string(),
                    permission: ContextPermissionScope::Session,
                    sensitivity: ContextSensitivity::Private,
                    protected: true,
                    original_token_estimate: 10,
                    token_estimate: 10,
                    disposition: ContextDisposition::Kept,
                },
                ContextPlanSegment {
                    scope: ContextScope::Local,
                    provenance: "node:7:secret".to_string(),
                    permission: ContextPermissionScope::Local,
                    sensitivity: ContextSensitivity::Sensitive,
                    protected: true,
                    original_token_estimate: 8,
                    token_estimate: 4,
                    disposition: ContextDisposition::Truncated,
                },
                ContextPlanSegment {
                    scope: ContextScope::Upstream(3),
                    provenance: "node:3:output".to_string(),
                    permission: ContextPermissionScope::GraphDependency,
                    sensitivity: ContextSensitivity::Private,
                    protected: false,
                    original_token_estimate: 6,
                    token_estimate: 0,
                    disposition: ContextDisposition::OmittedTokenBudget,
                },
                ContextPlanSegment {
                    scope: ContextScope::Upstream(4),
                    provenance: "node:4:empty".to_string(),
                    permission: ContextPermissionScope::GraphDependency,
                    sensitivity: ContextSensitivity::Internal,
                    protected: false,
                    original_token_estimate: 0,
                    token_estimate: 0,
                    disposition: ContextDisposition::OmittedEmpty,
                },
            ],
        };

        assert_eq!(
            plan.metrics(),
            ContextPlanMetrics {
                token_budget: 20,
                original_tokens: 24,
                admitted_tokens: 14,
                kept_segments: 1,
                truncated_segments: 1,
                omitted_token_budget_segments: 1,
                omitted_empty_segments: 1,
            }
        );
    }
}
