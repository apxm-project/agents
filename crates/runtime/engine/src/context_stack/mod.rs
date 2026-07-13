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
    ContextTokenizer, estimate_tokens, estimate_tokens_with, load_graph_summary, load_node_output,
    load_node_prompt, load_node_status, truncate_to_budget, truncate_to_budget_with,
};
pub use policy::{ContextPlanningError, ContextPlanningPolicy, ScopeRules};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextStackConfig {
    pub session_dir: PathBuf,
    /// Complete policy evidence for context tokenizer, capacity, and profiles.
    pub planning: ContextPlanningPolicy,
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
    planning: Option<ContextPlanningPolicy>,
    node_metadata: Arc<HashMap<u64, NodeMetadata>>,
    graph_edges: Arc<Vec<(u64, u64)>>,
    memory: Option<Arc<crate::memory::MemorySystem>>,
    execution_id: Option<String>,
}

#[derive(Clone, Debug)]
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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPlan {
    /// Tokenizer used for every segment estimate and truncation decision.
    pub tokenizer: ContextTokenizer,
    /// Total context capacity approved by the configured planning policy.
    pub token_budget: usize,
    /// Requested scope profile, when this plan assembles graph context.
    pub profile: Option<String>,
    /// Segments considered during deterministic packing, including omissions.
    ///
    /// Segment provenance and scope are execution-only evidence. They are not
    /// part of a serializable context plan because serialized plans can leave
    /// the runtime boundary.
    #[serde(skip)]
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
    /// Segments retained for graph semantics but excluded from provider input.
    pub excluded_provider_segments: usize,
}

impl ContextPlan {
    /// Construct a plan using the configured tokenizer and capacity.
    pub fn new(tokenizer: ContextTokenizer, token_budget: usize, profile: Option<String>) -> Self {
        Self {
            tokenizer,
            token_budget,
            profile,
            segments: Vec::new(),
        }
    }

    /// Construct a plan from configured policy evidence.
    pub fn from_policy(
        policy: &ContextPlanningPolicy,
        profile: Option<&str>,
    ) -> Result<Self, ContextPlanningError> {
        if let Some(profile) = profile {
            policy.rules_for(profile)?;
        }
        Ok(Self::new(
            policy.tokenizer,
            policy.token_budget,
            profile.map(str::to_owned),
        ))
    }

    /// Stable tokenizer identity emitted with plan evidence.
    pub const fn tokenizer_identity(&self) -> &'static str {
        self.tokenizer.identity()
    }

    /// Count tokens using this plan's configured tokenizer.
    pub fn estimate(&self, text: &str) -> usize {
        estimate_tokens_with(self.tokenizer, text)
    }

    /// Return the unallocated input capacity remaining in the plan.
    pub fn remaining_tokens(&self) -> usize {
        self.token_budget.saturating_sub(self.admitted_tokens())
    }

    /// Admit one typed segment through the plan's shared packing policy.
    pub fn admit(
        &mut self,
        spec: ContextSegmentSpec,
        content: &str,
    ) -> Result<ContextAdmission, ContextPlanningError> {
        let original_token_estimate = self.estimate(content);
        if original_token_estimate == 0 {
            self.segments.push(spec.record(
                original_token_estimate,
                0,
                ContextDisposition::OmittedEmpty,
            ));
            return Ok(ContextAdmission {
                content: String::new(),
                disposition: ContextDisposition::OmittedEmpty,
            });
        }

        let available = self.remaining_tokens().min(spec.max_tokens);
        if available == 0 {
            self.segments.push(spec.record(
                original_token_estimate,
                0,
                ContextDisposition::OmittedTokenBudget,
            ));
            return Ok(ContextAdmission {
                content: String::new(),
                disposition: ContextDisposition::OmittedTokenBudget,
            });
        }
        if original_token_estimate > available && spec.protected {
            return Err(ContextPlanningError::ProtectedSegmentExceedsBudget {
                provenance: spec.provenance,
                required_tokens: original_token_estimate,
                available_tokens: available,
            });
        }

        let (content, truncated) = truncate_to_budget_with(self.tokenizer, content, available);
        let token_estimate = self.estimate(&content);
        let disposition = if truncated {
            ContextDisposition::Truncated
        } else {
            ContextDisposition::Kept
        };
        self.segments.push(spec.record(
            original_token_estimate,
            token_estimate,
            disposition.clone(),
        ));
        Ok(ContextAdmission {
            content,
            disposition,
        })
    }

    /// Record a typed segment that intentionally does not cross the provider boundary.
    pub fn exclude(&mut self, spec: ContextSegmentSpec, content: &str) {
        let original_token_estimate = self.estimate(content);
        self.segments.push(spec.record(
            original_token_estimate,
            0,
            ContextDisposition::ExcludedProviderBoundary,
        ));
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
                ContextDisposition::ExcludedProviderBoundary => {
                    metrics.excluded_provider_segments =
                        metrics.excluded_provider_segments.saturating_add(1);
                }
            }
        }

        metrics
    }
}

/// Typed metadata required to admit one context segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextSegmentSpec {
    pub scope: ContextScope,
    pub role: ContextSegmentRole,
    pub provenance: String,
    pub permission: ContextPermissionScope,
    pub sensitivity: ContextSensitivity,
    pub protected: bool,
    pub max_tokens: usize,
}

impl ContextSegmentSpec {
    fn record(
        self,
        original_token_estimate: usize,
        token_estimate: usize,
        disposition: ContextDisposition,
    ) -> ContextPlanSegment {
        ContextPlanSegment {
            scope: self.scope,
            role: self.role,
            provenance: self.provenance,
            permission: self.permission,
            sensitivity: self.sensitivity,
            protected: self.protected,
            original_token_estimate,
            token_estimate,
            disposition,
        }
    }
}

/// Content admitted by a context plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextAdmission {
    pub content: String,
    pub disposition: ContextDisposition,
}

/// One source segment considered by a [`ContextPlan`].
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPlanSegment {
    /// Context scope that owns this segment.
    pub scope: ContextScope,
    /// Provider-facing semantic role carried by the segment.
    pub role: ContextSegmentRole,
    /// Stable source reference for audit and replay.
    #[serde(skip_serializing, default)]
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

/// Semantic role preserved while context is packed and rendered.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextSegmentRole {
    Context,
    System,
    User,
    Tool,
    Control,
    Dependency,
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
    /// The segment remains graph-visible but is excluded from provider input.
    ExcludedProviderBoundary,
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
            planning: None,
            node_metadata,
            graph_edges,
            memory: None,
            execution_id: None,
        }
    }

    /// Attach the complete context-planning policy for this stack.
    pub fn with_planning_policy(mut self, planning: ContextPlanningPolicy) -> Self {
        self.planning = Some(planning);
        self
    }

    /// Return the configured planning evidence required for model-call token
    /// accounting and bounded context reinjection.
    pub fn planning_policy(&self) -> Result<&ContextPlanningPolicy, ContextPlanningError> {
        self.planning
            .as_ref()
            .ok_or(ContextPlanningError::PolicyNotConfigured)
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
        .with_planning_policy(config.planning.clone())
    }

    pub fn assemble(
        &self,
        node_id: u64,
        profile: &str,
    ) -> Result<ContextAssembly, ContextPlanningError> {
        let planning = self.planning_policy()?;
        let rules = planning.rules_for(profile)?;
        let mut frames = Vec::new();
        let mut truncated = false;
        let mut plan = ContextPlan::from_policy(planning, Some(profile))?;

        let session_content = self.session_frame_content();
        self.push_frame(
            &mut frames,
            ContextScope::Session,
            None,
            None,
            session_content,
            rules.session_frame_budget,
            &mut truncated,
            &mut plan,
        )?;

        if let Some(local_content) = self.local_frame_content(node_id, profile) {
            let local_budget = plan.remaining_tokens();
            self.push_frame(
                &mut frames,
                ContextScope::Local,
                Some(node_id),
                self.node_metadata
                    .get(&node_id)
                    .map(|meta| meta.name.clone()),
                local_content,
                local_budget,
                &mut truncated,
                &mut plan,
            )?;
        }

        let upstream_chain = self.upstream_chain(node_id, rules.upstream_depth);
        for upstream_id in upstream_chain {
            let Some(meta) = self.node_metadata.get(&upstream_id) else {
                continue;
            };
            let Some(content) = self.upstream_frame_content(upstream_id, meta, rules) else {
                continue;
            };

            if !self.push_frame(
                &mut frames,
                ContextScope::Upstream(upstream_id),
                Some(upstream_id),
                Some(meta.name.clone()),
                content,
                rules.upstream_frame_budget,
                &mut truncated,
                &mut plan,
            )? {
                break;
            }
        }

        Ok(ContextAssembly {
            total_estimated_tokens: frames.iter().map(|frame| frame.token_estimate).sum(),
            frames,
            truncated,
            plan,
        })
    }

    fn push_frame(
        &self,
        frames: &mut Vec<ContextFrame>,
        scope: ContextScope,
        node_id: Option<u64>,
        node_name: Option<String>,
        content: String,
        max_frame_budget: usize,
        truncated: &mut bool,
        plan: &mut ContextPlan,
    ) -> Result<bool, ContextPlanningError> {
        let spec = self.segment_spec(&scope, node_id, node_name.as_deref(), max_frame_budget);
        let admission = plan.admit(spec, &content)?;
        *truncated |= admission.disposition == ContextDisposition::Truncated;
        if admission.content.is_empty() {
            return Ok(plan.remaining_tokens() > 0);
        }
        frames.push(ContextFrame {
            scope,
            node_id,
            node_name,
            token_estimate: plan.estimate(&admission.content),
            content: admission.content,
        });

        Ok(plan.remaining_tokens() > 0)
    }

    fn segment_spec(
        &self,
        scope: &ContextScope,
        node_id: Option<u64>,
        node_name: Option<&str>,
        max_tokens: usize,
    ) -> ContextSegmentSpec {
        let (permission, sensitivity, provenance) = match scope {
            ContextScope::Session => (
                ContextPermissionScope::Session,
                ContextSensitivity::Internal,
                "session".to_string(),
            ),
            ContextScope::Local => (
                ContextPermissionScope::Local,
                ContextSensitivity::Private,
                format!(
                    "node:{}:{}",
                    node_id.unwrap_or_default(),
                    node_name.unwrap_or("unknown")
                ),
            ),
            ContextScope::Upstream(upstream_id) => (
                ContextPermissionScope::GraphDependency,
                ContextSensitivity::Private,
                format!(
                    "upstream:{}:{}",
                    upstream_id,
                    node_name.unwrap_or("unknown")
                ),
            ),
        };
        ContextSegmentSpec {
            scope: scope.clone(),
            role: ContextSegmentRole::Context,
            provenance,
            permission,
            sensitivity,
            protected: matches!(scope, ContextScope::Local),
            max_tokens,
        }
    }

    fn session_frame_content(&self) -> String {
        let mut content = "- Workflow context".to_string();

        // Workflow summaries add provider-relevant graph facts without exposing storage paths.
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
    use std::collections::BTreeMap;

    fn planning(token_budget: usize) -> ContextPlanningPolicy {
        ContextPlanningPolicy {
            tokenizer: ContextTokenizer::O200kBase,
            token_budget,
            profiles: BTreeMap::from([(
                "answering".to_string(),
                ScopeRules {
                    upstream_depth: 1,
                    upstream_frame_budget: 2_000,
                    session_frame_budget: 200,
                    include_upstream_prompts: false,
                },
            )]),
        }
    }

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
        )
        .with_planning_policy(planning(0));

        let assembly = stack.assemble(7, "answering").expect("configured policy");
        assert_eq!(assembly.plan.tokenizer_identity(), "o200k_base");
        assert_eq!(assembly.plan.token_budget, 0);
        assert_eq!(assembly.plan.profile.as_deref(), Some("answering"));
        assert_eq!(assembly.plan.admitted_tokens(), 0);
        assert!(assembly.plan.segments.iter().any(|segment| {
            segment.scope == ContextScope::Session
                && !segment.protected
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
            tokenizer: ContextTokenizer::O200kBase,
            token_budget: 20,
            profile: Some("answering".to_string()),
            segments: vec![
                ContextPlanSegment {
                    scope: ContextScope::Session,
                    role: ContextSegmentRole::Context,
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
                    role: ContextSegmentRole::System,
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
                    role: ContextSegmentRole::Dependency,
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
                    role: ContextSegmentRole::Dependency,
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
                excluded_provider_segments: 0,
            }
        );
    }

    #[test]
    fn one_plan_packs_and_records_provider_roles_without_collapsing_them() {
        let policy = planning(64);
        let mut plan = ContextPlan::from_policy(&policy, None).expect("configured policy");
        for (role, provenance, content) in [
            (
                ContextSegmentRole::System,
                "request:system",
                "system policy",
            ),
            (ContextSegmentRole::User, "request:user", "user prompt"),
            (ContextSegmentRole::Tool, "request:tool", "tool result"),
        ] {
            plan.admit(
                ContextSegmentSpec {
                    scope: ContextScope::Local,
                    role,
                    provenance: provenance.to_string(),
                    permission: ContextPermissionScope::Local,
                    sensitivity: ContextSensitivity::Private,
                    protected: true,
                    max_tokens: plan.remaining_tokens(),
                },
                content,
            )
            .expect("protected request role fits");
        }
        plan.exclude(
            ContextSegmentSpec {
                scope: ContextScope::Upstream(3),
                role: ContextSegmentRole::Dependency,
                provenance: "request:dependency".to_string(),
                permission: ContextPermissionScope::GraphDependency,
                sensitivity: ContextSensitivity::Private,
                protected: true,
                max_tokens: 0,
            },
            "dependency-only value",
        );

        assert_eq!(
            plan.segments
                .iter()
                .map(|segment| segment.role.clone())
                .collect::<Vec<_>>(),
            vec![
                ContextSegmentRole::System,
                ContextSegmentRole::User,
                ContextSegmentRole::Tool,
                ContextSegmentRole::Dependency,
            ]
        );
        assert_eq!(plan.metrics().excluded_provider_segments, 1);
    }

    #[test]
    fn assembly_rejects_profiles_absent_from_configured_policy() {
        let stack = ContextStack::new(
            PathBuf::from("/tmp/apxm-context-plan"),
            Arc::new(HashMap::new()),
            Arc::new(Vec::new()),
        )
        .with_planning_policy(planning(256));

        assert_eq!(
            stack
                .assemble(7, "unconfigured")
                .expect_err("unconfigured profile must fail"),
            ContextPlanningError::ProfileNotConfigured {
                profile: "unconfigured".to_string(),
            }
        );
    }

    #[test]
    fn assembly_fails_closed_without_planning_policy() {
        let stack = ContextStack::new(
            PathBuf::from("/tmp/apxm-context-plan"),
            Arc::new(HashMap::new()),
            Arc::new(Vec::new()),
        );

        assert_eq!(
            stack
                .assemble(7, "answering")
                .expect_err("planning policy must be configured"),
            ContextPlanningError::PolicyNotConfigured
        );
    }

    #[test]
    fn session_frame_content_excludes_the_session_storage_path() {
        let session_dir = PathBuf::from("/private/output-root/run-42");
        let stack = ContextStack::new(
            session_dir.clone(),
            Arc::new(HashMap::new()),
            Arc::new(Vec::new()),
        );

        assert!(
            !stack
                .session_frame_content()
                .contains(session_dir.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn serialized_plan_excludes_segment_provenance_and_paths() {
        let stack = ContextStack::new(
            PathBuf::from("/private/output-root/run-42"),
            Arc::new(HashMap::new()),
            Arc::new(Vec::new()),
        )
        .with_planning_policy(planning(256));
        let plan = stack
            .assemble(7, "answering")
            .expect("configured context planning")
            .plan;

        let serialized = serde_json::to_string(&plan).expect("serialize redacted plan");
        assert!(!serialized.contains("segments"));
        assert!(!serialized.contains("provenance"));
        assert!(!serialized.contains("/private/output-root/run-42"));

        let segment = plan.segments.first().expect("session segment");
        let serialized_segment =
            serde_json::to_string(segment).expect("serialize redacted segment");
        assert!(!serialized_segment.contains("provenance"));
        assert!(!serialized_segment.contains("/private/output-root/run-42"));
    }
}
