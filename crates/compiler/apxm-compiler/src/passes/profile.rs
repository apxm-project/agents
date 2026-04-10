//! Profile-guided optimization infrastructure.
//!
//! Collects runtime execution profiles and applies them to graphs before
//! MLIR lowering. The profile annotates nodes with observed latency,
//! error-rate, and token-usage data so that downstream passes (scheduling,
//! fusion) can make better decisions.
//!
//! # File format
//!
//! Profiles are serialized as JSON via [`ExecutionProfile::save_to_file`] and
//! loaded with [`ExecutionProfile::load_from_file`].

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use apxm_core::types::Value;
use crate::air_builder::AirModule;

/// Error-rate threshold above which a retry attribute is injected.
const ERROR_RATE_RETRY_THRESHOLD: f64 = 0.05;

/// Default retry count injected for high-error-rate nodes.
const DEFAULT_RETRY_COUNT: i64 = 2;

// Attribute keys used by the profile pass.
const ATTR_PROFILE_LATENCY_MS: &str = "__profile_latency_ms";
const ATTR_PROFILE_P99_LATENCY_MS: &str = "__profile_p99_latency_ms";
const ATTR_PROFILE_ERROR_RATE: &str = "__profile_error_rate";
const ATTR_PROFILE_AVG_TOKENS: &str = "__profile_avg_tokens";
const ATTR_RETRY_COUNT: &str = "retry_count";
const ATTR_PROFILE_TOKEN_WARNING: &str = "__profile_token_warning";

/// Runtime execution profile collected from previous runs.
///
/// Keyed by node *name* (not ID) so profiles remain valid across
/// re-compilations that change node numbering.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionProfile {
    /// Per-node execution statistics: node name -> stats.
    pub node_stats: HashMap<String, NodeProfile>,
    /// Total number of executions that contributed to this profile.
    pub execution_count: u64,
}

/// Per-node runtime statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeProfile {
    /// Average latency in milliseconds.
    pub avg_latency_ms: u64,
    /// 99th-percentile latency in milliseconds.
    pub p99_latency_ms: u64,
    /// Total number of times this node was invoked.
    pub call_count: u64,
    /// Average token count consumed per invocation.
    pub avg_tokens: u64,
    /// Fraction of invocations that resulted in an error (0.0 .. 1.0).
    pub error_rate: f64,
}

impl ExecutionProfile {
    /// Load a profile from a JSON file on disk.
    pub fn load_from_file(path: &Path) -> Result<Self, ProfileError> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| ProfileError::Io(path.display().to_string(), e))?;
        let profile: Self = serde_json::from_str(&data)
            .map_err(|e| ProfileError::Parse(path.display().to_string(), e.to_string()))?;
        Ok(profile)
    }

    /// Persist the profile as JSON.
    pub fn save_to_file(&self, path: &Path) -> Result<(), ProfileError> {
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| ProfileError::Parse("serialization".to_string(), e.to_string()))?;
        std::fs::write(path, data).map_err(|e| ProfileError::Io(path.display().to_string(), e))?;
        Ok(())
    }

    /// Merge another profile into this one.
    ///
    /// Statistics are combined as weighted averages using the respective
    /// execution counts. Call counts and execution counts are summed.
    pub fn merge(&mut self, other: &ExecutionProfile) {
        let total = self.execution_count + other.execution_count;
        if total == 0 {
            return;
        }

        for (name, other_stats) in &other.node_stats {
            let entry = self.node_stats.entry(name.clone());
            match entry {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    let existing = e.get_mut();
                    let combined_calls = existing.call_count + other_stats.call_count;
                    if combined_calls > 0 {
                        existing.avg_latency_ms = weighted_avg(
                            existing.avg_latency_ms,
                            existing.call_count,
                            other_stats.avg_latency_ms,
                            other_stats.call_count,
                        );
                        existing.avg_tokens = weighted_avg(
                            existing.avg_tokens,
                            existing.call_count,
                            other_stats.avg_tokens,
                            other_stats.call_count,
                        );
                        existing.error_rate = weighted_avg_f64(
                            existing.error_rate,
                            existing.call_count,
                            other_stats.error_rate,
                            other_stats.call_count,
                        );
                        existing.p99_latency_ms =
                            existing.p99_latency_ms.max(other_stats.p99_latency_ms);
                    }
                    existing.call_count = combined_calls;
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(other_stats.clone());
                }
            }
        }
        self.execution_count = total;
    }

    /// Apply profile data to a module.
    ///
    /// For each node in the module whose name appears in the profile:
    ///   - Writes `__profile_latency_ms` and `__profile_p99_latency_ms` attributes.
    ///   - Writes `__profile_error_rate` and `__profile_avg_tokens` attributes.
    ///   - If `error_rate > 0.05`, injects a `retry_count` attribute (if not
    ///     already set).
    ///   - If `avg_tokens > token_budget`, writes a `__profile_token_warning`
    ///     attribute with a human-readable message.
    ///
    /// Returns the number of nodes that were annotated.
    pub fn apply_to_module(&self, module: &mut AirModule, token_budget: Option<u64>) -> usize {
        let mut annotated = 0;
        for node in &mut module.nodes {
            if let Some(stats) = self.node_stats.get(&node.name) {
                // Latency annotations
                node.attributes.insert(
                    ATTR_PROFILE_LATENCY_MS.to_string(),
                    Value::Number(apxm_core::types::Number::Integer(
                        stats.avg_latency_ms as i64,
                    )),
                );
                node.attributes.insert(
                    ATTR_PROFILE_P99_LATENCY_MS.to_string(),
                    Value::Number(apxm_core::types::Number::Integer(
                        stats.p99_latency_ms as i64,
                    )),
                );

                // Error-rate annotation
                node.attributes.insert(
                    ATTR_PROFILE_ERROR_RATE.to_string(),
                    Value::Number(apxm_core::types::Number::Float(stats.error_rate)),
                );

                // Token usage annotation
                node.attributes.insert(
                    ATTR_PROFILE_AVG_TOKENS.to_string(),
                    Value::Number(apxm_core::types::Number::Integer(stats.avg_tokens as i64)),
                );

                // Inject retry_count for high-error-rate nodes
                if stats.error_rate > ERROR_RATE_RETRY_THRESHOLD
                    && !node.attributes.contains_key(ATTR_RETRY_COUNT)
                {
                    node.attributes.insert(
                        ATTR_RETRY_COUNT.to_string(),
                        Value::Number(apxm_core::types::Number::Integer(DEFAULT_RETRY_COUNT)),
                    );
                }

                // Token budget warning
                if let Some(budget) = token_budget {
                    if stats.avg_tokens > budget {
                        node.attributes.insert(
                            ATTR_PROFILE_TOKEN_WARNING.to_string(),
                            Value::String(format!(
                                "avg_tokens ({}) exceeds budget ({})",
                                stats.avg_tokens, budget
                            )),
                        );
                    }
                }

                annotated += 1;
            }
        }
        annotated
    }
}

/// Profile-related errors.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("I/O error on profile file '{0}': {1}")]
    Io(String, #[source] std::io::Error),
    #[error("failed to parse profile '{0}': {1}")]
    Parse(String, String),
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn weighted_avg(a: u64, w_a: u64, b: u64, w_b: u64) -> u64 {
    let total = w_a + w_b;
    if total == 0 {
        return 0;
    }
    (a as u128 * w_a as u128 + b as u128 * w_b as u128)
        .checked_div(total as u128)
        .unwrap_or(0) as u64
}

fn weighted_avg_f64(a: f64, w_a: u64, b: f64, w_b: u64) -> f64 {
    let total = w_a + w_b;
    if total == 0 {
        return 0.0;
    }
    (a * w_a as f64 + b * w_b as f64) / total as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_builder::{AirEdge, AirNode};
    use apxm_core::types::AISOperationType;
    use std::collections::HashMap;

    fn make_profile() -> ExecutionProfile {
        let mut profile = ExecutionProfile::default();
        profile.execution_count = 100;
        profile.node_stats.insert(
            "ask_node".to_string(),
            NodeProfile {
                avg_latency_ms: 250,
                p99_latency_ms: 800,
                call_count: 100,
                avg_tokens: 1500,
                error_rate: 0.02,
            },
        );
        profile.node_stats.insert(
            "flaky_node".to_string(),
            NodeProfile {
                avg_latency_ms: 500,
                p99_latency_ms: 2000,
                call_count: 50,
                avg_tokens: 3000,
                error_rate: 0.10,
            },
        );
        profile
    }

    fn make_module() -> AirModule {
        AirModule {
            name: "test".to_string(),
            nodes: vec![
                AirNode {
                    id: 1,
                    name: "ask_node".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([(
                        "template_str".to_string(),
                        Value::String("{0}".to_string()),
                    )]),
                },
                AirNode {
                    id: 2,
                    name: "flaky_node".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([(
                        "template_str".to_string(),
                        Value::String("{0}".to_string()),
                    )]),
                },
                AirNode {
                    id: 3,
                    name: "unknown_node".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        "value".to_string(),
                        Value::String("hi".to_string()),
                    )]),
                },
            ],
            edges: vec![
                AirEdge {
                    from: 1,
                    to: 2,
                    dependency: apxm_core::types::DependencyType::Data,
                },
                AirEdge {
                    from: 3,
                    to: 1,
                    dependency: apxm_core::types::DependencyType::Data,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn apply_profile_annotates_matching_nodes() {
        let profile = make_profile();
        let mut graph = make_module();

        let annotated = profile.apply_to_module(&mut graph, None);
        assert_eq!(annotated, 2, "should annotate ask_node and flaky_node");

        // ask_node should have latency but no retry (error_rate < threshold)
        let ask = &graph.nodes[0];
        assert_eq!(
            ask.attributes.get(ATTR_PROFILE_LATENCY_MS),
            Some(&Value::Number(apxm_core::types::Number::Integer(250)))
        );
        assert!(
            !ask.attributes.contains_key(ATTR_RETRY_COUNT),
            "ask_node error_rate 0.02 < threshold, no retry"
        );

        // flaky_node should have retry injected
        let flaky = &graph.nodes[1];
        assert_eq!(
            flaky.attributes.get(ATTR_RETRY_COUNT),
            Some(&Value::Number(apxm_core::types::Number::Integer(
                DEFAULT_RETRY_COUNT
            )))
        );

        // unknown_node should be untouched
        let unknown = &graph.nodes[2];
        assert!(
            !unknown.attributes.contains_key(ATTR_PROFILE_LATENCY_MS),
            "no profile data for unknown_node"
        );
    }

    #[test]
    fn apply_profile_token_budget_warning() {
        let profile = make_profile();
        let mut graph = make_module();

        let _annotated = profile.apply_to_module(&mut graph, Some(2000));

        // ask_node (1500 tokens) should be under budget
        assert!(
            !graph.nodes[0]
                .attributes
                .contains_key(ATTR_PROFILE_TOKEN_WARNING),
            "1500 < 2000, no warning"
        );

        // flaky_node (3000 tokens) should get a warning
        let warning = graph.nodes[1]
            .attributes
            .get(ATTR_PROFILE_TOKEN_WARNING)
            .expect("should have warning");
        match warning {
            Value::String(s) => assert!(s.contains("3000") && s.contains("2000")),
            _ => panic!("expected String warning"),
        }
    }

    #[test]
    fn apply_profile_does_not_overwrite_existing_retry() {
        let profile = make_profile();
        let mut graph = make_module();

        // Pre-set a retry_count on flaky_node
        graph.nodes[1].attributes.insert(
            ATTR_RETRY_COUNT.to_string(),
            Value::Number(apxm_core::types::Number::Integer(5)),
        );

        let _annotated = profile.apply_to_module(&mut graph, None);

        // The existing retry_count should be preserved
        assert_eq!(
            graph.nodes[1].attributes.get(ATTR_RETRY_COUNT),
            Some(&Value::Number(apxm_core::types::Number::Integer(5)))
        );
    }

    #[test]
    fn merge_profiles() {
        let mut p1 = ExecutionProfile::default();
        p1.execution_count = 50;
        p1.node_stats.insert(
            "node_a".to_string(),
            NodeProfile {
                avg_latency_ms: 100,
                p99_latency_ms: 300,
                call_count: 50,
                avg_tokens: 1000,
                error_rate: 0.04,
            },
        );

        let mut p2 = ExecutionProfile::default();
        p2.execution_count = 50;
        p2.node_stats.insert(
            "node_a".to_string(),
            NodeProfile {
                avg_latency_ms: 200,
                p99_latency_ms: 400,
                call_count: 50,
                avg_tokens: 2000,
                error_rate: 0.08,
            },
        );
        p2.node_stats.insert(
            "node_b".to_string(),
            NodeProfile {
                avg_latency_ms: 300,
                p99_latency_ms: 500,
                call_count: 30,
                avg_tokens: 500,
                error_rate: 0.01,
            },
        );

        p1.merge(&p2);

        assert_eq!(p1.execution_count, 100);

        // node_a should be a weighted average
        let node_a = p1.node_stats.get("node_a").expect("node_a");
        assert_eq!(node_a.call_count, 100);
        // (100*50 + 200*50) / 100 = 150
        assert_eq!(node_a.avg_latency_ms, 150);
        // p99 is max(300, 400) = 400
        assert_eq!(node_a.p99_latency_ms, 400);
        // (1000*50 + 2000*50) / 100 = 1500
        assert_eq!(node_a.avg_tokens, 1500);
        // (0.04*50 + 0.08*50) / 100 = 0.06
        assert!((node_a.error_rate - 0.06).abs() < 1e-9);

        // node_b should be copied from p2
        let node_b = p1.node_stats.get("node_b").expect("node_b");
        assert_eq!(node_b.avg_latency_ms, 300);
    }

    #[test]
    fn roundtrip_json() {
        let profile = make_profile();
        let json = serde_json::to_string_pretty(&profile).expect("serialize");
        let restored: ExecutionProfile = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.execution_count, profile.execution_count);
        assert_eq!(restored.node_stats.len(), profile.node_stats.len());
    }

    #[test]
    fn save_and_load() {
        let profile = make_profile();
        let dir = tempfile::tempdir().expect("create tempdir");
        let path = dir.path().join("test_profile.json");

        profile.save_to_file(&path).expect("save");
        let loaded = ExecutionProfile::load_from_file(&path).expect("load");

        assert_eq!(loaded.execution_count, profile.execution_count);
        assert_eq!(loaded.node_stats.len(), profile.node_stats.len());

        let ask = loaded.node_stats.get("ask_node").expect("ask_node");
        assert_eq!(ask.avg_latency_ms, 250);
    }

    #[test]
    fn load_nonexistent_file_returns_error() {
        let result = ExecutionProfile::load_from_file(Path::new("/tmp/does_not_exist.json"));
        assert!(result.is_err());
    }

    #[test]
    fn empty_profile_applies_nothing() {
        let profile = ExecutionProfile::default();
        let mut graph = make_module();
        let annotated = profile.apply_to_module(&mut graph, None);
        assert_eq!(annotated, 0);
    }
}
