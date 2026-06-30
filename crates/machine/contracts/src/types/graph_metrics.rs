use crate::constants::graph::attrs;
use crate::types::values::{Number, Value};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

mod latency_class_wire {
    pub(super) const SHORT: &str = "short";
    pub(super) const MEDIUM: &str = "medium";
    pub(super) const LONG: &str = "long";
    pub(super) const UNKNOWN: &str = "unknown";
}

/// Backend-agnostic latency class inferred by compiler analysis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LatencyClass {
    Short,
    Medium,
    Long,
    #[default]
    Unknown,
}

impl LatencyClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Short => latency_class_wire::SHORT,
            Self::Medium => latency_class_wire::MEDIUM,
            Self::Long => latency_class_wire::LONG,
            Self::Unknown => latency_class_wire::UNKNOWN,
        }
    }

    pub fn to_attr_value(self) -> Value {
        Value::String(self.as_str().to_owned())
    }
}

impl fmt::Display for LatencyClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LatencyClass {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            latency_class_wire::SHORT => Ok(Self::Short),
            latency_class_wire::MEDIUM => Ok(Self::Medium),
            latency_class_wire::LONG => Ok(Self::Long),
            latency_class_wire::UNKNOWN => Ok(Self::Unknown),
            _ => Err(format!("unknown latency class: {value}")),
        }
    }
}

impl Serialize for LatencyClass {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LatencyClass {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(D::Error::custom)
    }
}

/// Compiler-derived graph facts for one node.
///
/// These are APXM-level metrics, not vLLM-specific request fields. Backends may
/// lower them into provider-specific controls when their capability contract
/// supports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct NodeGraphMetrics {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fanout_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_path_len: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_class: Option<LatencyClass>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_dynamic_tokens: Option<u32>,
}

impl NodeGraphMetrics {
    pub fn from_attrs(attrs_map: &HashMap<String, Value>) -> Self {
        Self::try_from_attrs(attrs_map).expect("invalid compiler-derived graph metrics attributes")
    }

    pub fn try_from_attrs(attrs_map: &HashMap<String, Value>) -> Result<Self, String> {
        Ok(Self {
            fanout_count: u32_attr(attrs_map, attrs::FANOUT_COUNT),
            remaining_path_len: u32_attr(attrs_map, attrs::REMAINING_PATH_LEN),
            latency_class: attrs_map
                .get(attrs::LATENCY_CLASS)
                .and_then(Value::as_str)
                .map(LatencyClass::from_str)
                .transpose()?,
            batch_group: attrs_map
                .get(attrs::BATCH_GROUP)
                .and_then(Value::as_str)
                .map(str::to_owned),
            stage_index: u32_attr(attrs_map, attrs::STAGE_INDEX),
            estimated_dynamic_tokens: u32_attr(attrs_map, attrs::ESTIMATED_DYNAMIC_TOKENS),
        })
    }

    pub fn to_attrs(&self) -> HashMap<String, Value> {
        let mut attrs_map = HashMap::new();
        if let Some(value) = self.fanout_count {
            attrs_map.insert(
                attrs::FANOUT_COUNT.to_owned(),
                Value::Number(Number::Integer(i64::from(value))),
            );
        }
        if let Some(value) = self.remaining_path_len {
            attrs_map.insert(
                attrs::REMAINING_PATH_LEN.to_owned(),
                Value::Number(Number::Integer(i64::from(value))),
            );
        }
        if let Some(value) = self.latency_class {
            attrs_map.insert(attrs::LATENCY_CLASS.to_owned(), value.to_attr_value());
        }
        if let Some(value) = &self.batch_group {
            attrs_map.insert(attrs::BATCH_GROUP.to_owned(), Value::String(value.clone()));
        }
        if let Some(value) = self.stage_index {
            attrs_map.insert(
                attrs::STAGE_INDEX.to_owned(),
                Value::Number(Number::Integer(i64::from(value))),
            );
        }
        if let Some(value) = self.estimated_dynamic_tokens {
            attrs_map.insert(
                attrs::ESTIMATED_DYNAMIC_TOKENS.to_owned(),
                Value::Number(Number::Integer(i64::from(value))),
            );
        }
        attrs_map
    }

    pub fn with_fanout_count(mut self, value: u32) -> Self {
        self.fanout_count = Some(value);
        self
    }

    pub fn with_remaining_path_len(mut self, value: u32) -> Self {
        self.remaining_path_len = Some(value);
        self
    }

    pub fn with_latency_class(mut self, value: LatencyClass) -> Self {
        self.latency_class = Some(value);
        self
    }

    pub fn with_batch_group(mut self, value: impl Into<String>) -> Self {
        self.batch_group = Some(value.into());
        self
    }

    pub fn with_stage_index(mut self, value: u32) -> Self {
        self.stage_index = Some(value);
        self
    }

    pub fn with_estimated_dynamic_tokens(mut self, value: u32) -> Self {
        self.estimated_dynamic_tokens = Some(value);
        self
    }

    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

fn u32_attr(attrs_map: &HashMap<String, Value>, key: &str) -> Option<u32> {
    attrs_map
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}
