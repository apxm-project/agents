//! vLLM Hints Pass
//!
//! Walks the [`AirModule`] and, for every LLM operation (Ask / Think / Reason),
//! reads the upstream MLIR-emitted attributes (`ais.shared_prefix_group`,
//! `ais.shared_prefix_est_tokens`, `ais.warmup_candidate`, `ais.downstream_nodes`,
//! `priority`) and stamps a parallel set of `_vllm_*` attributes that the
//! runtime later converts into typed APXM graph hints for graph-aware
//! backends.
//!
//! All attribute keys come from `apxm_core::constants::graph::attrs::*` —
//! no string literals.
//!
//! # Pass position
//!
//! Logically runs *after* `ASSIGN_PRIORITY` (MLIR) and the AIR roundtrip
//! at O1+. Surface-level integration into [`build_pass_list`](super::pipeline::build_pass_list)
//! references the [`VLLM_HINTS_PASS_NAME`] sentinel below for ordering tests.
//! The actual transform is dispatched as a Rust-side pass on `AirModule`
//! alongside `profile.apply_to_module` and `annotate_token_estimates`.

use crate::air_builder::AirModule;
use apxm_core::{
    constants::{graph::attrs, llm::apxm as apxm_llm},
    types::{AISOperationType, Number, Value},
};

/// CLI / ordering-test sentinel for this pass. Keep in sync with
/// [`super::pipeline::build_pass_list`].
pub const VLLM_HINTS_PASS_NAME: &str = "vllm-hints";

/// Priority threshold above which a node is considered to be on the
/// critical path (matches `ASSIGN_PRIORITY`'s "High" tier ≥ 70).
const CRITICAL_PATH_PRIORITY_THRESHOLD: i64 = 70;

/// A node with this many or more downstream nodes plus a shared prefix is
/// pipeline-eligible.
const PIPELINE_DOWNSTREAM_THRESHOLD: u32 = 2;

/// Stamp `_vllm_*` hint attributes onto every LLM node in the module.
///
/// Returns the number of nodes annotated.
pub fn vllm_hints(module: &mut AirModule) -> usize {
    let mut annotated = 0;
    for node in &mut module.nodes {
        if !is_llm_op(node.op) {
            continue;
        }

        // --- Source attributes (with sane defaults if absent) -----------
        let priority: u8 = node
            .attributes
            .get(attrs::PRIORITY)
            .and_then(Value::as_i64)
            .unwrap_or(30)
            .clamp(0, u8::MAX as i64) as u8;

        let downstream_nodes =
            downstream_nodes_from_attr(node.attributes.get(attrs::AIS_DOWNSTREAM_NODES));

        let est_tokens: u32 = node
            .attributes
            .get(attrs::AIS_SHARED_PREFIX_EST_TOKENS)
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(u32::MAX as u64) as u32;

        let warmup: bool = node
            .attributes
            .get(attrs::AIS_WARMUP_CANDIDATE)
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let group: Option<String> = node
            .attributes
            .get(attrs::AIS_SHARED_PREFIX_GROUP)
            .and_then(Value::as_str)
            .map(str::to_owned);

        // --- Derived values ---------------------------------------------
        let priority_class = priority_class_from_priority(priority);
        let critical_path = priority_class == apxm_llm::PRIORITY_CRITICAL_PATH;
        let pin_mode = if group.is_some() {
            apxm_llm::PIN_MODE_PREFIX
        } else {
            apxm_llm::PIN_MODE_NONE
        };
        let pipeline =
            group.is_some() && downstream_nodes.len() as u32 >= PIPELINE_DOWNSTREAM_THRESHOLD;

        // --- Write back the eight `_vllm_*` keys ------------------------
        node.attributes.insert(
            attrs::VLLM_PRIORITY_CLASS.to_string(),
            Value::String(priority_class.to_string()),
        );
        node.attributes.insert(
            attrs::VLLM_DOWNSTREAM_NODES.to_string(),
            Value::Array(
                downstream_nodes
                    .iter()
                    .map(|id| Value::Number(Number::Integer(i64::from(*id))))
                    .collect(),
            ),
        );
        if let Some(g) = group {
            node.attributes
                .insert(attrs::VLLM_REUSE_GROUP.to_string(), Value::String(g));
        }
        node.attributes.insert(
            attrs::VLLM_CRITICAL_PATH.to_string(),
            Value::Bool(critical_path),
        );
        node.attributes.insert(
            attrs::VLLM_PIN_MODE.to_string(),
            Value::String(pin_mode.to_string()),
        );
        node.attributes.insert(
            attrs::VLLM_EST_TOKENS.to_string(),
            Value::Number(Number::Integer(est_tokens as i64)),
        );
        node.attributes
            .insert(attrs::VLLM_WARMUP.to_string(), Value::Bool(warmup));
        node.attributes
            .insert(attrs::VLLM_PIPELINE.to_string(), Value::Bool(pipeline));

        annotated += 1;
    }

    annotated
}

fn downstream_nodes_from_attr(value: Option<&Value>) -> Vec<u32> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_u64().map(|u| u as u32))
            .collect(),
        _ => Vec::new(),
    }
}

fn priority_class_from_priority(priority: u8) -> &'static str {
    if (priority as i64) >= CRITICAL_PATH_PRIORITY_THRESHOLD {
        apxm_llm::PRIORITY_CRITICAL_PATH
    } else {
        apxm_llm::PRIORITY_PARALLEL
    }
}

#[inline]
fn is_llm_op(op: AISOperationType) -> bool {
    matches!(
        op,
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_builder::{AirEdge, AirNode};
    use apxm_core::types::DependencyType;
    use std::collections::HashMap;

    fn ask_with_hints(
        id: u64,
        name: &str,
        priority: i64,
        group: &str,
        est_tokens: i64,
        downstream: &[u32],
        warmup: bool,
    ) -> AirNode {
        let mut a = HashMap::new();
        a.insert(
            attrs::PRIORITY.to_string(),
            Value::Number(Number::Integer(priority)),
        );
        a.insert(
            attrs::AIS_SHARED_PREFIX_GROUP.to_string(),
            Value::String(group.to_string()),
        );
        a.insert(
            attrs::AIS_SHARED_PREFIX_EST_TOKENS.to_string(),
            Value::Number(Number::Integer(est_tokens)),
        );
        a.insert(
            attrs::AIS_DOWNSTREAM_NODES.to_string(),
            Value::Array(
                downstream
                    .iter()
                    .map(|id| Value::Number(Number::Integer(i64::from(*id))))
                    .collect(),
            ),
        );
        a.insert(attrs::AIS_WARMUP_CANDIDATE.to_string(), Value::Bool(warmup));
        AirNode {
            id,
            name: name.to_string(),
            op: AISOperationType::Ask,
            attributes: a,
        }
    }

    #[test]
    fn vllm_hints_writes_all_eight_keys_for_shared_group() {
        let mut module = AirModule {
            name: "t".to_string(),
            nodes: vec![
                ask_with_hints(1, "ask_a", 90, "grp_x", 4096, &[2, 3], true),
                ask_with_hints(2, "ask_b", 30, "grp_x", 4096, &[4], false),
            ],
            edges: vec![AirEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let n = vllm_hints(&mut module);
        assert_eq!(n, 2);

        // ---- ask_a: priority 90, on critical path, pipeline (group + downstream>=2) ----
        let a = &module.nodes[0].attributes;
        assert_eq!(
            a.get(attrs::VLLM_PRIORITY_CLASS),
            Some(&Value::String(apxm_llm::PRIORITY_CRITICAL_PATH.to_string()))
        );
        assert_eq!(
            a.get(attrs::VLLM_DOWNSTREAM_NODES),
            Some(&Value::Array(vec![
                Value::Number(Number::Integer(2)),
                Value::Number(Number::Integer(3)),
            ]))
        );
        assert_eq!(
            a.get(attrs::VLLM_REUSE_GROUP),
            Some(&Value::String("grp_x".to_string()))
        );
        assert_eq!(a.get(attrs::VLLM_CRITICAL_PATH), Some(&Value::Bool(true)));
        assert_eq!(
            a.get(attrs::VLLM_PIN_MODE),
            Some(&Value::String(apxm_llm::PIN_MODE_PREFIX.to_string()))
        );
        assert_eq!(
            a.get(attrs::VLLM_EST_TOKENS),
            Some(&Value::Number(Number::Integer(4096)))
        );
        assert_eq!(a.get(attrs::VLLM_WARMUP), Some(&Value::Bool(true)));
        // group present + downstream fanout 2 → pipeline
        assert_eq!(a.get(attrs::VLLM_PIPELINE), Some(&Value::Bool(true)));

        // ---- ask_b: priority 30 (parallel), downstream 1 (not pipeline) ----
        let b = &module.nodes[1].attributes;
        assert_eq!(
            b.get(attrs::VLLM_PRIORITY_CLASS),
            Some(&Value::String(apxm_llm::PRIORITY_PARALLEL.to_string()))
        );
        assert_eq!(b.get(attrs::VLLM_CRITICAL_PATH), Some(&Value::Bool(false)));
        assert_eq!(
            b.get(attrs::VLLM_PIN_MODE),
            Some(&Value::String(apxm_llm::PIN_MODE_PREFIX.to_string()))
        );
        // group present but downstream (1) < 2 → not pipeline
        assert_eq!(b.get(attrs::VLLM_PIPELINE), Some(&Value::Bool(false)));
        assert_eq!(b.get(attrs::VLLM_WARMUP), Some(&Value::Bool(false)));
    }

    #[test]
    fn vllm_hints_uses_prefix_pin_for_grouped_node() {
        let mut module = AirModule {
            name: "t".to_string(),
            nodes: vec![ask_with_hints(1, "solo", 90, "lonely", 100, &[5], false)],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };
        vllm_hints(&mut module);
        assert_eq!(
            module.nodes[0].attributes.get(attrs::VLLM_PIN_MODE),
            Some(&Value::String(apxm_llm::PIN_MODE_PREFIX.to_string()))
        );
    }

    #[test]
    fn vllm_hints_skips_non_llm_nodes() {
        let mut module = AirModule {
            name: "t".to_string(),
            nodes: vec![AirNode {
                id: 1,
                name: "literal".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    attrs::PRIORITY.to_string(),
                    Value::Number(Number::Integer(90)),
                )]),
            }],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };
        let n = vllm_hints(&mut module);
        assert_eq!(n, 0);
        assert!(
            !module.nodes[0]
                .attributes
                .contains_key(attrs::VLLM_PRIORITY_CLASS),
            "non-LLM nodes should not get _vllm_* attrs"
        );
    }

    #[test]
    fn vllm_hints_handles_missing_source_attrs_with_defaults() {
        let mut module = AirModule {
            name: "t".to_string(),
            nodes: vec![AirNode {
                id: 1,
                name: "bare_ask".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::new(),
            }],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };
        let n = vllm_hints(&mut module);
        assert_eq!(n, 1);
        let a = &module.nodes[0].attributes;
        assert_eq!(
            a.get(attrs::VLLM_PRIORITY_CLASS),
            Some(&Value::String(apxm_llm::PRIORITY_PARALLEL.to_string())),
            "default priority class is parallel"
        );
        assert_eq!(a.get(attrs::VLLM_CRITICAL_PATH), Some(&Value::Bool(false)));
        // No group → reuse_group attr absent.
        assert!(!a.contains_key(attrs::VLLM_REUSE_GROUP));
        assert_eq!(
            a.get(attrs::VLLM_PIN_MODE),
            Some(&Value::String(apxm_llm::PIN_MODE_NONE.to_string()))
        );
        assert_eq!(a.get(attrs::VLLM_PIPELINE), Some(&Value::Bool(false)));
        assert_eq!(a.get(attrs::VLLM_WARMUP), Some(&Value::Bool(false)));
    }
}
