use crate::constants::{
    graph::backend_kind,
    graph::{attrs, metadata as graph_meta},
    llm::apxm as apxm_llm,
};
use crate::types::graph_metrics::NodeGraphMetrics;
use crate::types::values::Value;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PinMode {
    Prefix,
    #[default]
    None,
}

impl PinMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prefix => apxm_llm::PIN_MODE_PREFIX,
            Self::None => apxm_llm::PIN_MODE_NONE,
        }
    }
}

impl fmt::Display for PinMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PinMode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            apxm_llm::PIN_MODE_PREFIX => Ok(Self::Prefix),
            apxm_llm::PIN_MODE_NONE => Ok(Self::None),
            _ => Err(format!("unknown pin mode: {value}")),
        }
    }
}

impl Serialize for PinMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PinMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PriorityClass {
    CriticalPath,
    Parallel,
}

impl PriorityClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CriticalPath => apxm_llm::PRIORITY_CRITICAL_PATH,
            Self::Parallel => apxm_llm::PRIORITY_PARALLEL,
        }
    }
}

impl fmt::Display for PriorityClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PriorityClass {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            apxm_llm::PRIORITY_CRITICAL_PATH => Ok(Self::CriticalPath),
            apxm_llm::PRIORITY_PARALLEL => Ok(Self::Parallel),
            _ => Err(format!("unknown priority class: {value}")),
        }
    }
}

impl Serialize for PriorityClass {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PriorityClass {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PinPolicy {
    pub mode: PinMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u32>,
}

impl PinPolicy {
    pub fn none() -> Self {
        Self {
            mode: PinMode::None,
            ttl_ms: None,
        }
    }

    pub fn prefix(ttl_ms: u32) -> Self {
        Self {
            mode: PinMode::Prefix,
            ttl_ms: Some(ttl_ms),
        }
    }

    pub fn prefix_default() -> Self {
        Self {
            mode: PinMode::Prefix,
            ttl_ms: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CompilerHints {
    /// Estimated static shared-prefix tokens, produced by compiler analysis.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_prefix_est_tokens: Option<u32>,
    /// Declarative compiler hint: this node is eligible for runtime prefix prefill.
    /// The runtime still decides whether to dispatch a warmup request based on
    /// backend capabilities and runtime policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_candidate: Option<bool>,
    /// Declarative compiler hint for backend request pipelining.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline_candidate: Option<bool>,
}

impl CompilerHints {
    pub fn is_empty(&self) -> bool {
        self.shared_prefix_est_tokens.is_none()
            && self.warmup_candidate.is_none()
            && self.pipeline_candidate.is_none()
    }
}

/// Backend support surface for graph-aware APXM execution.
///
/// These booleans describe what APXM can rely on for one configured backend.
/// The runtime may still lower internal `DispatchIrV1` into legacy backend
/// controls, but unsupported capability bits make that fallback explicit in
/// metrics and claim evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BackendGraphCapabilities {
    pub supports_graph_registration: bool,
    pub supports_request_hints: bool,
    pub supports_priority: bool,
    pub supports_prefix_cohorts: bool,
    pub supports_pin_release: bool,
    pub supports_structured_outputs: bool,
    pub supports_backend_queue_state: bool,
    pub supports_backend_cache_state: bool,
    pub supports_cancel_groups: bool,
    pub supports_dispatch_ir_v1_internal: bool,
    /// Whether the backend exposes `POST /v1/apxm/admin/reset_prefix_cache`,
    /// the operator endpoint benchmark harnesses use for cell isolation.
    pub supports_admin_reset_prefix_cache: bool,
}

impl BackendGraphCapabilities {
    pub fn unsupported_dispatch_fields<'a>(
        &self,
        fields_sent: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        fields_sent
            .into_iter()
            .filter(|field| match *field {
                "graph_registration" => !self.supports_graph_registration,
                "request_hints" => !self.supports_request_hints,
                "priority" => !self.supports_priority,
                "prefix_cohorts" => !self.supports_prefix_cohorts,
                "pin_release" => !self.supports_pin_release,
                "structured_outputs" => !self.supports_structured_outputs,
                "backend_queue_state" => !self.supports_backend_queue_state,
                "backend_cache_state" => !self.supports_backend_cache_state,
                "cancel_groups" => !self.supports_cancel_groups,
                "dispatch_ir_v1_internal" => !self.supports_dispatch_ir_v1_internal,
                _ => false,
            })
            .map(ToOwned::to_owned)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApxmGraphHints {
    pub schema_version: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_class: Option<PriorityClass>,
    pub downstream_nodes: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reuse_group: Option<String>,
    #[serde(default, skip_serializing_if = "NodeGraphMetrics::is_empty")]
    pub graph_metrics: NodeGraphMetrics,
    pub pin_policy: PinPolicy,
    pub compiler_hints: CompilerHints,
}

impl Default for ApxmGraphHints {
    fn default() -> Self {
        Self {
            schema_version: 1,
            graph_id: None,
            execution_id: None,
            node_id: None,
            node_name: None,
            priority_class: None,
            downstream_nodes: Vec::new(),
            reuse_group: None,
            graph_metrics: NodeGraphMetrics::default(),
            pin_policy: PinPolicy::none(),
            compiler_hints: CompilerHints::default(),
        }
    }
}

impl ApxmGraphHints {
    pub fn critical_path(
        graph_id: impl Into<String>,
        execution_id: impl Into<String>,
        node_id: u32,
        node_name: impl Into<String>,
        downstream_nodes: Vec<u32>,
        pin_ttl_ms: u32,
    ) -> Self {
        Self {
            schema_version: 1,
            graph_id: Some(graph_id.into()),
            execution_id: Some(execution_id.into()),
            node_id: Some(node_id),
            node_name: Some(node_name.into()),
            priority_class: Some(PriorityClass::CriticalPath),
            downstream_nodes,
            reuse_group: None,
            graph_metrics: NodeGraphMetrics::default(),
            pin_policy: PinPolicy::prefix(pin_ttl_ms),
            compiler_hints: CompilerHints::default(),
        }
    }

    pub fn parallel(
        graph_id: impl Into<String>,
        execution_id: impl Into<String>,
        node_id: u32,
        node_name: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: 1,
            graph_id: Some(graph_id.into()),
            execution_id: Some(execution_id.into()),
            node_id: Some(node_id),
            node_name: Some(node_name.into()),
            priority_class: Some(PriorityClass::Parallel),
            downstream_nodes: Vec::new(),
            reuse_group: None,
            graph_metrics: NodeGraphMetrics::default(),
            pin_policy: PinPolicy::none(),
            compiler_hints: CompilerHints::default(),
        }
    }

    pub fn from_node_attrs(
        graph_id: String,
        node_label: String,
        attrs_map: &HashMap<String, Value>,
    ) -> Self {
        Self::try_from_node_attrs(graph_id, node_label, attrs_map)
            .expect("invalid compiler-derived graph hint attributes")
    }

    pub fn try_from_node_attrs(
        graph_id: String,
        node_label: String,
        attrs_map: &HashMap<String, Value>,
    ) -> Result<Self, String> {
        let numeric_node_id = node_label.parse::<u32>().ok();

        let priority_class = attrs_map.get(attrs::PRIORITY).and_then(|value| {
            let priority = value.as_i64()?;
            if priority >= graph_meta::CRITICAL_PATH_PRIORITY_THRESHOLD {
                Some(PriorityClass::CriticalPath)
            } else {
                Some(PriorityClass::Parallel)
            }
        });

        let downstream_nodes = attrs_map
            .get(attrs::DOWNSTREAM_NODES)
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_u64().map(|raw| raw as u32))
                    .collect()
            })
            .unwrap_or_default();

        // The canonical attribute name is `shared_prefix_group` (what MLIR's
        // PromptCanonicalization pass emits, propagated into apxm-core via
        // build.rs from apxm-ais::attrs::REUSE_GROUP). The Python frontend
        // historically lets users pass a `reuse_group=...` kwarg, which
        // ends up as a node attribute under the literal key "reuse_group".
        // Look up both so explicit user-set hints land regardless of which
        // name was used.
        let reuse_group = attrs_map
            .get(attrs::REUSE_GROUP)
            .or_else(|| attrs_map.get(attrs::REUSE_GROUP_LEGACY))
            .and_then(|value| value.as_string())
            .map(ToOwned::to_owned);

        let pin_policy = if reuse_group.is_some() {
            PinPolicy::prefix_default()
        } else {
            PinPolicy::none()
        };

        let shared_prefix_est_tokens = attrs_map
            .get(attrs::SHARED_PREFIX_EST_TOKENS)
            .and_then(|value| value.as_u64())
            .map(|value| value as u32);

        let warmup_candidate = attrs_map
            .get(attrs::WARMUP_CANDIDATE)
            .and_then(|value| value.as_bool());

        Ok(Self {
            schema_version: 1,
            graph_id: Some(graph_id),
            execution_id: None,
            node_id: numeric_node_id,
            node_name: Some(node_label),
            priority_class,
            downstream_nodes,
            reuse_group,
            graph_metrics: NodeGraphMetrics::try_from_attrs(attrs_map)?,
            pin_policy,
            compiler_hints: CompilerHints {
                shared_prefix_est_tokens,
                warmup_candidate,
                pipeline_candidate: None,
            },
        })
    }

    pub fn has_graph_context(&self) -> bool {
        self.graph_id.is_some() || self.node_id.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSpec {
    pub node_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_prompt_tokens: Option<u32>,
    pub downstream_nodes: Vec<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_class: Option<PriorityClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reuse_group: Option<String>,
    #[serde(default, skip_serializing_if = "NodeGraphMetrics::is_empty")]
    pub graph_metrics: NodeGraphMetrics,
    pub is_critical_path: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMetadata {
    pub graph_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_path_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_parallelism: Option<u32>,
    pub nodes: Vec<NodeSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_pin_ttl_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphBackendKind {
    GraphAware,
    Generic,
}

impl GraphBackendKind {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::GraphAware => backend_kind::GRAPH_AWARE,
            Self::Generic => backend_kind::GENERIC,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphStatusSnapshot {
    pub backend_kind: GraphBackendKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend_name: Option<String>,
    pub graph_id: String,
    pub registered: bool,
    pub pinned_handles: u64,
    pub pinned_blocks: u64,
    /// Peak `pinned_handles` recorded by `LLMRegistry::start_pin_polling`.
    #[serde(default)]
    pub pinned_handles_peak: u64,
    /// Peak `pinned_blocks` recorded by `LLMRegistry::start_pin_polling`.
    #[serde(default)]
    pub pinned_blocks_peak: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub critical_path_length: Option<u64>,
}

impl GraphStatusSnapshot {
    pub fn new(backend_kind: GraphBackendKind, graph_id: impl Into<String>) -> Self {
        Self {
            backend_kind,
            backend_name: None,
            graph_id: graph_id.into(),
            registered: false,
            pinned_handles: 0,
            pinned_blocks: 0,
            pinned_handles_peak: 0,
            pinned_blocks_peak: 0,
            node_count: None,
            critical_path_length: None,
        }
    }

    pub fn graph_aware(graph_id: impl Into<String>) -> Self {
        Self::new(GraphBackendKind::GraphAware, graph_id)
    }

    pub fn with_registered(mut self, registered: bool) -> Self {
        self.registered = registered;
        self
    }

    pub fn with_backend_name(mut self, backend_name: impl Into<String>) -> Self {
        self.backend_name = Some(backend_name.into());
        self
    }

    pub fn with_pin_counts(mut self, handles: u64, blocks: u64) -> Self {
        self.pinned_handles = handles;
        self.pinned_blocks = blocks;
        self
    }

    /// Fold pin-peak counters into the snapshot, clamping each peak to be
    /// at least the corresponding live count.
    pub fn with_pin_peaks(mut self, handles_peak: u64, blocks_peak: u64) -> Self {
        self.pinned_handles_peak = handles_peak.max(self.pinned_handles);
        self.pinned_blocks_peak = blocks_peak.max(self.pinned_blocks);
        self
    }

    pub fn with_shape(
        mut self,
        node_count: Option<u64>,
        critical_path_length: Option<u64>,
    ) -> Self {
        self.node_count = node_count;
        self.critical_path_length = critical_path_length;
        self
    }

    pub fn to_metrics_json(&self) -> serde_json::Value {
        use crate::types::metrics::GraphStatusKey as K;

        let mut map = serde_json::Map::new();
        map.insert(
            K::Object.as_str().into(),
            apxm_llm::OBJECT_GRAPH_STATUS.into(),
        );
        map.insert(
            K::BackendKind.as_str().into(),
            self.backend_kind.as_str().into(),
        );
        if let Some(backend_name) = &self.backend_name {
            map.insert(K::BackendName.as_str().into(), backend_name.clone().into());
        }
        map.insert(K::GraphId.as_str().into(), self.graph_id.clone().into());
        map.insert(K::Registered.as_str().into(), self.registered.into());
        map.insert(K::PinnedHandles.as_str().into(), self.pinned_handles.into());
        map.insert(K::PinnedBlocks.as_str().into(), self.pinned_blocks.into());
        if self.pinned_handles_peak > 0 {
            map.insert(
                K::PinnedHandlesPeak.as_str().into(),
                self.pinned_handles_peak.into(),
            );
        }
        if self.pinned_blocks_peak > 0 {
            map.insert(
                K::PinnedBlocksPeak.as_str().into(),
                self.pinned_blocks_peak.into(),
            );
        }
        if let Some(node_count) = self.node_count {
            map.insert(K::NodeCount.as_str().into(), node_count.into());
        }
        if let Some(critical_path_length) = self.critical_path_length {
            map.insert(
                K::CriticalPathLength.as_str().into(),
                critical_path_length.into(),
            );
        }
        serde_json::Value::Object(map)
    }
}

impl GraphMetadata {
    pub fn new(graph_id: impl Into<String>, execution_id: impl Into<String>) -> Self {
        Self {
            graph_id: graph_id.into(),
            execution_id: Some(execution_id.into()),
            critical_path_length: None,
            node_count: None,
            max_parallelism: None,
            nodes: Vec::new(),
            default_pin_ttl_ms: None,
        }
    }

    pub fn with_pin_ttl(mut self, ttl_ms: u32) -> Self {
        self.default_pin_ttl_ms = Some(ttl_ms);
        self
    }

    pub fn with_critical_path_length(mut self, len: u32) -> Self {
        self.critical_path_length = Some(len);
        self
    }

    pub fn with_nodes(mut self, nodes: Vec<NodeSpec>) -> Self {
        self.node_count = Some(nodes.len() as u32);
        self.nodes = nodes;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::graph_metrics::LatencyClass;

    #[test]
    fn priority_class_serializes_canonical_strings() {
        assert_eq!(
            serde_json::to_string(&PriorityClass::CriticalPath).unwrap(),
            "\"critical_path\""
        );
        assert_eq!(
            serde_json::to_string(&PriorityClass::Parallel).unwrap(),
            "\"parallel\""
        );
    }

    #[test]
    fn priority_class_rejects_unsupported_strings() {
        assert!(serde_json::from_str::<PriorityClass>("\"normal\"").is_err());
        assert!(serde_json::from_str::<PriorityClass>("\"speculative\"").is_err());
    }

    #[test]
    fn pin_mode_rejects_unsupported_strings() {
        assert!(serde_json::from_str::<PinMode>("\"pin_strong\"").is_err());
        assert!(serde_json::from_str::<PinMode>("\"pin_weak\"").is_err());
    }

    #[test]
    fn critical_path_hints_serialize_expected_shape() {
        let hints = ApxmGraphHints::critical_path(
            "graph-abc",
            "exec-123",
            5,
            "reason-node",
            vec![6, 7],
            30_000,
        );

        let json = serde_json::to_value(&hints).unwrap();
        assert_eq!(json[apxm_llm::GRAPH_ID], "graph-abc");
        assert_eq!(json[apxm_llm::NODE_ID], 5);
        assert_eq!(
            json[apxm_llm::PRIORITY_CLASS],
            apxm_llm::PRIORITY_CRITICAL_PATH
        );
        assert_eq!(
            json[apxm_llm::PIN_POLICY][apxm_llm::PIN_POLICY_MODE],
            apxm_llm::PIN_MODE_PREFIX
        );
        assert_eq!(
            json[apxm_llm::PIN_POLICY][apxm_llm::PIN_POLICY_TTL_MS],
            30_000
        );
        assert_eq!(json[apxm_llm::DOWNSTREAM_NODES], serde_json::json!([6, 7]));
        assert_eq!(json[apxm_llm::SCHEMA_VERSION], 1);
    }

    #[test]
    fn from_node_attrs_uses_backend_agnostic_graph_attrs() {
        let mut attrs_map = NodeGraphMetrics::default()
            .with_fanout_count(2)
            .with_remaining_path_len(4)
            .with_latency_class(LatencyClass::Long)
            .with_stage_index(1)
            .to_attrs();
        attrs_map.extend([
            (
                attrs::PRIORITY.to_owned(),
                Value::Number(crate::types::Number::Integer(
                    graph_meta::CRITICAL_PATH_PRIORITY_THRESHOLD,
                )),
            ),
            (
                attrs::DOWNSTREAM_NODES.to_owned(),
                Value::Array(vec![
                    Value::Number(crate::types::Number::Integer(2)),
                    Value::Number(crate::types::Number::Integer(3)),
                ]),
            ),
            (
                attrs::REUSE_GROUP.to_owned(),
                Value::String("shared-prefix-a".to_owned()),
            ),
            (
                attrs::SHARED_PREFIX_EST_TOKENS.to_owned(),
                Value::Number(crate::types::Number::Integer(1024)),
            ),
            (attrs::WARMUP_CANDIDATE.to_owned(), Value::Bool(true)),
        ]);

        let hints =
            ApxmGraphHints::from_node_attrs("graph-abc".to_owned(), "7".to_owned(), &attrs_map);

        assert_eq!(hints.priority_class, Some(PriorityClass::CriticalPath));
        assert_eq!(hints.downstream_nodes, vec![2, 3]);
        assert_eq!(hints.reuse_group.as_deref(), Some("shared-prefix-a"));
        assert_eq!(hints.pin_policy.mode, PinMode::Prefix);
        assert_eq!(hints.compiler_hints.shared_prefix_est_tokens, Some(1024));
        assert_eq!(hints.compiler_hints.warmup_candidate, Some(true));
        assert_eq!(hints.graph_metrics.fanout_count, Some(2));
        assert_eq!(hints.graph_metrics.remaining_path_len, Some(4));
        assert_eq!(hints.graph_metrics.latency_class, Some(LatencyClass::Long));
        assert_eq!(hints.graph_metrics.stage_index, Some(1));
    }

    /// Regression for the 2026-05 silent-pin-skip bug: the Python frontend
    /// passes `g.ask(reuse_group=...)` kwargs through verbatim, so the literal
    /// key `"reuse_group"` shows up in node attributes. The runtime's canonical
    /// constant is `attrs::REUSE_GROUP = "shared_prefix_group"` (matching
    /// MLIR's `PromptCanonicalization` pass output). For 5+ paired benchmark
    /// runs the runtime looked up only the canonical name, missed the legacy
    /// kwarg name, and `pin_policy.mode` collapsed to `None` — pin never
    /// engaged. The fix accepts both names. This test pins that contract so
    /// future refactors of the Python frontend or runtime can't regress it.
    #[test]
    fn from_node_attrs_accepts_python_kwarg_legacy_reuse_group_name() {
        let mut attrs_map: HashMap<String, Value> = HashMap::new();
        attrs_map.insert(
            attrs::REUSE_GROUP_LEGACY.to_owned(),
            Value::String("pin_demo_cohort".to_owned()),
        );

        let hints = ApxmGraphHints::from_node_attrs(
            "graph-pin-fix".to_owned(),
            "0".to_owned(),
            &attrs_map,
        );

        assert_eq!(
            hints.reuse_group.as_deref(),
            Some("pin_demo_cohort"),
            "legacy kwarg name `reuse_group` must be accepted alongside canonical `shared_prefix_group`"
        );
        assert_eq!(
            hints.pin_policy.mode,
            PinMode::Prefix,
            "pin_policy must engage when reuse_group is set under EITHER name"
        );
    }

    #[test]
    fn graph_metadata_builder_sets_node_count() {
        let metadata = GraphMetadata::new("graph-xyz", "exec-456")
            .with_pin_ttl(60_000)
            .with_critical_path_length(4)
            .with_nodes(vec![NodeSpec {
                node_id: 0,
                node_name: Some("plan".to_string()),
                estimated_prompt_tokens: Some(500),
                downstream_nodes: vec![1, 2],
                priority_class: Some(PriorityClass::CriticalPath),
                reuse_group: None,
                graph_metrics: NodeGraphMetrics::default(),
                is_critical_path: true,
            }]);

        assert_eq!(metadata.graph_id, "graph-xyz");
        assert_eq!(metadata.default_pin_ttl_ms, Some(60_000));
        assert_eq!(metadata.critical_path_length, Some(4));
        assert_eq!(metadata.node_count, Some(1));
        assert_eq!(
            metadata.nodes[0].priority_class,
            Some(PriorityClass::CriticalPath)
        );
    }
}
