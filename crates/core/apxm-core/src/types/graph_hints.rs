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
/// Unsupported capability bits are reported explicitly in metrics and claim
/// evidence.
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
    /// Returns the subset of `fields_sent` the backend reports it does
    /// NOT support. The runtime emits this in `runtime.dispatch_ir_v1`
    /// telemetry so claim evidence shows which APXM hints were silently
    /// dropped because the backend cannot honor them.
    pub fn unsupported_dispatch_fields<'a>(
        &self,
        fields_sent: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        fields_sent
            .into_iter()
            .filter(|field| !self.field_supported(field))
            .map(ToOwned::to_owned)
            .collect()
    }

    /// Returns the subset of `fields_sent` the backend's *static
    /// capability table* reports as supported — the complement of
    /// `unsupported_dispatch_fields` over the same input. Together the
    /// two lists partition the request's hint surface.
    ///
    /// Distinct from `fields_honored`, which is per-request runtime
    /// evidence from the backend (the `x-apxm-fields-honored` response
    /// header — see `crate::constants::llm::apxm::APXM_FIELDS_HONORED_HEADER`).
    /// Capability-supported says "the backend declares it CAN honor this
    /// field"; honored says "the backend reported it DID honor this
    /// field on this specific request." The runtime-evidence contract reserves the
    /// `fields_honored` name for the runtime-evidence signal; do not
    /// conflate the two.
    pub fn dispatch_fields_capability_supported<'a>(
        &self,
        fields_sent: impl IntoIterator<Item = &'a str>,
    ) -> Vec<String> {
        fields_sent
            .into_iter()
            .filter(|field| self.field_supported(field))
            .map(ToOwned::to_owned)
            .collect()
    }

    fn field_supported(&self, field: &str) -> bool {
        use crate::constants::llm::apxm::dispatch_fields as df;
        match field {
            df::GRAPH_REGISTRATION => self.supports_graph_registration,
            df::REQUEST_HINTS => self.supports_request_hints,
            df::PRIORITY => self.supports_priority,
            df::PREFIX_COHORTS => self.supports_prefix_cohorts,
            df::PIN_RELEASE => self.supports_pin_release,
            df::STRUCTURED_OUTPUTS => self.supports_structured_outputs,
            df::BACKEND_QUEUE_STATE => self.supports_backend_queue_state,
            df::BACKEND_CACHE_STATE => self.supports_backend_cache_state,
            df::CANCEL_GROUPS => self.supports_cancel_groups,
            df::DISPATCH_IR_V1_INTERNAL => self.supports_dispatch_ir_v1_internal,
            df::ADMIN_RESET_PREFIX_CACHE => self.supports_admin_reset_prefix_cache,
            // Unknown field name is reported as unsupported so a caller
            // shipping an unrecognized hint cannot infer the backend
            // applied it. The runtime's fields-sent collector should
            // only emit names declared above; this branch is the
            // defensive seam.
            _ => false,
        }
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

        let reuse_group = attrs_map
            .get(attrs::REUSE_GROUP)
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

