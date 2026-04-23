use crate::constants::{graph::attrs, llm::apxm as apxm_llm};
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
            apxm_llm::PIN_MODE_PREFIX | apxm_llm::PIN_MODE_PREFIX_LEGACY => Ok(Self::Prefix),
            apxm_llm::PIN_MODE_NONE | apxm_llm::PIN_MODE_NONE_LEGACY => Ok(Self::None),
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
    Speculative,
}

impl PriorityClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CriticalPath => apxm_llm::PRIORITY_CRITICAL_PATH,
            Self::Parallel => apxm_llm::PRIORITY_PARALLEL,
            Self::Speculative => apxm_llm::PRIORITY_SPECULATIVE_LEGACY,
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
            apxm_llm::PRIORITY_PARALLEL | apxm_llm::PRIORITY_NORMAL_LEGACY => Ok(Self::Parallel),
            apxm_llm::PRIORITY_SPECULATIVE_LEGACY => Ok(Self::Speculative),
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompilerHints {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_prefix_est_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_candidate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pipeline_candidate: Option<bool>,
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
            pin_policy: PinPolicy::none(),
            compiler_hints: CompilerHints::default(),
        }
    }

    pub fn from_node_attrs(
        graph_id: String,
        node_label: String,
        attrs_map: &HashMap<String, Value>,
    ) -> Self {
        let numeric_node_id = node_label.parse::<u32>().ok();

        let priority_class = attrs_map
            .get(attrs::VLLM_PRIORITY_CLASS)
            .and_then(|value| value.as_string())
            .and_then(|value| PriorityClass::from_str(value).ok())
            .or_else(|| {
                attrs_map
                    .get(attrs::VLLM_CRITICAL_PATH)
                    .and_then(|value| value.as_bool())
                    .and_then(|critical_path| critical_path.then_some(PriorityClass::CriticalPath))
            });

        let downstream_nodes = attrs_map
            .get(attrs::VLLM_DOWNSTREAM_NODES)
            .and_then(|value| value.as_array())
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_u64().map(|raw| raw as u32))
                    .collect()
            })
            .unwrap_or_default();

        let reuse_group = attrs_map
            .get(attrs::VLLM_REUSE_GROUP)
            .and_then(|value| value.as_string())
            .map(ToOwned::to_owned);

        let pin_policy = match attrs_map
            .get(attrs::VLLM_PIN_MODE)
            .and_then(|value| value.as_string())
            .and_then(|value| PinMode::from_str(value).ok())
        {
            Some(PinMode::Prefix) => PinPolicy::prefix_default(),
            Some(PinMode::None) => PinPolicy::none(),
            None => {
                if reuse_group.is_some() {
                    PinPolicy::prefix_default()
                } else {
                    PinPolicy::none()
                }
            }
        };

        let shared_prefix_est_tokens = attrs_map
            .get(attrs::VLLM_EST_TOKENS)
            .and_then(|value| value.as_u64())
            .map(|value| value as u32);

        let warmup_candidate = attrs_map
            .get(attrs::VLLM_WARMUP)
            .and_then(|value| value.as_bool());

        let pipeline_candidate = attrs_map
            .get(attrs::VLLM_PIPELINE)
            .and_then(|value| value.as_bool());

        Self {
            schema_version: 1,
            graph_id: Some(graph_id),
            execution_id: None,
            node_id: numeric_node_id,
            node_name: Some(node_label),
            priority_class,
            downstream_nodes,
            reuse_group,
            pin_policy,
            compiler_hints: CompilerHints {
                shared_prefix_est_tokens,
                warmup_candidate,
                pipeline_candidate,
            },
        }
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
    fn priority_class_accepts_legacy_strings() {
        assert_eq!(
            serde_json::from_str::<PriorityClass>("\"normal\"").unwrap(),
            PriorityClass::Parallel
        );
        assert_eq!(
            serde_json::from_str::<PriorityClass>("\"speculative\"").unwrap(),
            PriorityClass::Speculative
        );
    }

    #[test]
    fn pin_mode_accepts_legacy_strings() {
        assert_eq!(
            serde_json::from_str::<PinMode>("\"pin_strong\"").unwrap(),
            PinMode::Prefix
        );
        assert_eq!(
            serde_json::from_str::<PinMode>("\"pin_weak\"").unwrap(),
            PinMode::None
        );
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
        assert_eq!(json["graph_id"], "graph-abc");
        assert_eq!(json["node_id"], 5);
        assert_eq!(json["priority_class"], apxm_llm::PRIORITY_CRITICAL_PATH);
        assert_eq!(json["pin_policy"]["mode"], apxm_llm::PIN_MODE_PREFIX);
        assert_eq!(json["pin_policy"]["ttl_ms"], 30_000);
        assert_eq!(json["downstream_nodes"], serde_json::json!([6, 7]));
        assert_eq!(json["schema_version"], 1);
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
