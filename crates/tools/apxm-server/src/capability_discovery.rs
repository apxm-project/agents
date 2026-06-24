//! Runtime capability discovery for conversational agents.
//!
//! This is read-only authoring-time discovery. It returns capability templates
//! and never grants authority or returns delegated runtime capability ids.

use std::collections::HashMap;
use std::sync::Arc;

use apxm_core::error::RuntimeError;
use apxm_core::types::values::Value;
use apxm_runtime::capability::CapabilitySystem;
use apxm_runtime::capability::executor::{CapabilityExecutor, CapabilityResult};
use apxm_runtime::capability::metadata::CapabilityMetadata;
use async_trait::async_trait;
use tracing::{info, warn};

const NAME: &str = apxm_core::constants::capabilities::CAPABILITY_DISCOVERY;
const DEFAULT_K: usize = 8;
const MAX_K: usize = 32;

pub(crate) struct CapabilityDiscoveryCapability {
    metadata: CapabilityMetadata,
    capability_system: Arc<CapabilitySystem>,
}

impl CapabilityDiscoveryCapability {
    pub(crate) fn new(capability_system: Arc<CapabilitySystem>) -> Self {
        let metadata = CapabilityMetadata::new(
            NAME,
            "Discover runtime capability templates by intent, operation, or group. \
             Returns authoring-time templates only; it does not grant authority, \
             mint capability_id values, or authorize execution.",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "request": {
                        "type": "string",
                        "description": "Natural-language description of the capability you need."
                    },
                    "operations": {
                        "type": "array",
                        "items": { "type": "string", "enum": ["read", "write"] },
                        "description": "Optional operation filter."
                    },
                    "groups": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional capability-template group filter."
                    },
                    "requires_auth": {
                        "type": "boolean",
                        "description": "Optional auth requirement filter."
                    },
                    "k": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_K,
                        "description": "Maximum number of templates to return (default 8)."
                    }
                }
            }),
        )
        .with_returns("JSON array of CapabilityTemplateV1-like summaries")
        .with_groups(vec![
            apxm_core::constants::capabilities::groups::DISCOVERY.to_string(),
        ])
        .with_read_only();
        Self {
            metadata,
            capability_system,
        }
    }
}

#[async_trait]
impl CapabilityExecutor for CapabilityDiscoveryCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let request = args
            .get("request")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let operations = string_array_arg(&args, "operations");
        let groups = string_array_arg(&args, "groups");
        let requires_auth = args.get("requires_auth").and_then(Value::as_bool);
        let k = args
            .get("k")
            .and_then(Value::as_i64)
            .map_or(DEFAULT_K, |value| value.clamp(1, MAX_K as i64) as usize);

        let mut templates: Vec<_> = self
            .capability_system
            .list_capabilities()
            .into_iter()
            .filter(|metadata| metadata.name != NAME)
            .filter(|metadata| operation_matches(metadata, &operations))
            .filter(|metadata| group_matches(metadata, &groups))
            .filter(|metadata| requires_auth.is_none_or(|wanted| metadata.requires_auth == wanted))
            .map(|metadata| {
                let score = match_score(&metadata, &request);
                (score, metadata)
            })
            .filter(|(score, _)| request.is_empty() || *score > 0)
            .collect();

        templates.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| left.name.cmp(&right.name))
        });

        let out: Vec<_> = templates
            .into_iter()
            .take(k)
            .map(|(_, metadata)| {
                let operation = if metadata.read_only { "read" } else { "write" };
                serde_json::json!({
                    "schema_version": apxm_core::types::CAPABILITY_TEMPLATE_SCHEMA_V1,
                    "template_key": metadata.name,
                    "tool_binding": metadata.name,
                    "description": metadata.description,
                    "operations": [operation],
                    "requires_auth": metadata.requires_auth,
                    "groups": metadata.groups,
                    "parameters_schema": metadata.parameters_schema,
                    "authority": "template_only"
                })
            })
            .collect();

        serde_json::to_string(&out)
            .map(Value::String)
            .map_err(|error| RuntimeError::Capability {
                capability: NAME.to_string(),
                message: format!("failed to serialize capability discovery results: {error}"),
            })
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}

fn string_array_arg(args: &HashMap<String, Value>, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| item.to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

fn operation_matches(metadata: &CapabilityMetadata, operations: &[String]) -> bool {
    if operations.is_empty() {
        return true;
    }
    let operation = if metadata.read_only { "read" } else { "write" };
    operations.iter().any(|wanted| wanted == operation)
}

fn group_matches(metadata: &CapabilityMetadata, groups: &[String]) -> bool {
    if groups.is_empty() {
        return true;
    }
    metadata.groups.iter().any(|group| {
        let group = group.to_ascii_lowercase();
        groups.iter().any(|wanted| wanted == &group)
    })
}

fn match_score(metadata: &CapabilityMetadata, request: &str) -> usize {
    if request.is_empty() {
        return 1;
    }
    request
        .split_whitespace()
        .map(|term| {
            let name_match = metadata.name.to_ascii_lowercase().contains(term) as usize * 4;
            let description_match =
                metadata.description.to_ascii_lowercase().contains(term) as usize * 2;
            let group_match = metadata
                .groups
                .iter()
                .any(|group| group.to_ascii_lowercase().contains(term))
                as usize;
            name_match + description_match + group_match
        })
        .sum()
}

pub(crate) fn register(runtime: &apxm_runtime::Runtime) {
    let cap: Arc<dyn CapabilityExecutor> = Arc::new(CapabilityDiscoveryCapability::new(
        runtime.capability_system_arc(),
    ));
    match runtime.capability_system().register(cap) {
        Ok(()) => info!(
            capability = NAME,
            "registered capability-discovery capability"
        ),
        Err(error) => {
            warn!(capability = NAME, %error, "failed to register capability_discovery");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::StaticCapability;
    use apxm_runtime::{Runtime, RuntimeConfig};

    #[tokio::test]
    async fn capability_discovery_returns_template_only_results() {
        let runtime = Runtime::new(RuntimeConfig::in_memory())
            .await
            .expect("runtime");
        runtime
            .capability_system()
            .register(Arc::new(StaticCapability {
                metadata: CapabilityMetadata::new(
                    "slack.list_channels",
                    "List Slack channels",
                    serde_json::json!({ "type": "object" }),
                )
                .with_groups(vec!["slack".to_string()])
                .with_auth()
                .with_read_only(),
                static_response: Value::String("[]".to_string()),
            }))
            .expect("register fixture capability");

        let discovery = CapabilityDiscoveryCapability::new(runtime.capability_system_arc());
        let out = discovery
            .execute(HashMap::from([
                (
                    "request".to_string(),
                    Value::String("slack channels".to_string()),
                ),
                (
                    "operations".to_string(),
                    Value::Array(vec![Value::String("read".to_string())]),
                ),
            ]))
            .await
            .expect("discover");
        let raw = out.as_str().expect("json string");
        let parsed: serde_json::Value = serde_json::from_str(raw).expect("json");
        assert_eq!(parsed[0]["template_key"], "slack.list_channels");
        assert_eq!(parsed[0]["operations"][0], "read");
        assert_eq!(parsed[0]["requires_auth"], true);
        assert_eq!(parsed[0]["authority"], "template_only");
    }
}
