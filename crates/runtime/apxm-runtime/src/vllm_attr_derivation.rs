//! Runtime-side derivation of `_vllm_*` node attributes from MLIR-stamped
//! `ais.*` attributes that flow through the wire artifact.
//!
//! # Why this lives at runtime
//!
//! The compile-time `vllm_hints` pass in `apxm-compiler` runs on `AirModule`
//! *before* the AIR text is parsed and handed to the MLIR PassManager. The
//! MLIR side (`PromptCanonicalization`, `AssignPriority`) writes the
//! `ais.shared_prefix_group`, `ais.shared_prefix_est_tokens`,
//! `ais.warmup_candidate`, `ais.downstream_nodes`, and `priority` attributes
//! that `vllm_hints` reads — so when the Rust pass runs, those attributes
//! don't exist yet and the pass falls back to defaults.
//!
//! After MLIR runs, `ArtifactEmitter.cpp` strips the `ais.` dialect prefix
//! and copies the attributes into `WireNode.attributes`. The runtime
//! deserializes them into `Node.attributes` with bare names
//! (`shared_prefix_group`, etc.), exposed via
//! `apxm_core::constants::graph::attrs`.
//!
//! This module re-runs the same derivation logic as the compiler pass —
//! tally reuse-group sizes once, then for each LLM node compute
//! `pin_mode`, `is_critical_path`, `pipeline_candidate`, and write the
//! eight `_vllm_*` keys. The downstream runtime code
//! (`ApxmGraphHints::from_node_attrs`, `inject_vllm_hints`) is unchanged.

use crate::executor::pipeline::is_pure_llm_op;
use apxm_core::constants::{graph::attrs, llm::apxm as apxm_llm};
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::{Number, Value};

/// Priority threshold above which a node is considered to be on the
/// critical path (mirrors `ASSIGN_PRIORITY`'s "High" tier ≥ 70 and the
/// constant in the compile-time pass).
const CRITICAL_PATH_PRIORITY_THRESHOLD: i64 = 70;

/// A node with this many or more downstream nodes plus a shared prefix is
/// pipeline-eligible.
const PIPELINE_DOWNSTREAM_THRESHOLD: u32 = 2;

/// Walk every LLM-eligible node in `dag` and stamp the eight `_vllm_*`
/// attributes derived from MLIR-stamped (bare-name) attributes. Existing
/// `_vllm_*` values are overwritten — the runtime view is authoritative
/// because the compile-time pass cannot see post-MLIR attributes.
///
/// Returns the number of LLM nodes annotated.
pub fn derive_vllm_attrs(dag: &mut ExecutionDag) -> usize {
    let mut annotated = 0;
    for node in &mut dag.nodes {
        if !is_pure_llm_op(&node.op_type) {
            continue;
        }

        // Only derive when MLIR (or another upstream stage) stamped at
        // least one bare-name source attribute. If the wire format
        // provided none, leave any pre-existing `_vllm_*` attrs alone —
        // they may have been set deliberately by the user, by the
        // compile-time pass on graphs that bypass MLIR canonicalization,
        // or by a hand-built Node in a unit test.
        let has_mlir_stamped_attrs = node.attributes.contains_key(attrs::PRIORITY)
            || node.attributes.contains_key(attrs::REUSE_GROUP)
            || node
                .attributes
                .contains_key(attrs::SHARED_PREFIX_EST_TOKENS)
            || node.attributes.contains_key(attrs::WARMUP_CANDIDATE)
            || node.attributes.contains_key(attrs::DOWNSTREAM_NODES);
        if !has_mlir_stamped_attrs {
            continue;
        }

        // --- Source attributes (bare names from the wire format) -------
        let priority: u8 = node
            .attributes
            .get(attrs::PRIORITY)
            .and_then(Value::as_i64)
            .unwrap_or(30)
            .clamp(0, u8::MAX as i64) as u8;

        let downstream_nodes =
            downstream_nodes_from_attr(node.attributes.get(attrs::DOWNSTREAM_NODES));

        let est_tokens: u32 = node
            .attributes
            .get(attrs::SHARED_PREFIX_EST_TOKENS)
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(u32::MAX as u64) as u32;

        let warmup: bool = node
            .attributes
            .get(attrs::WARMUP_CANDIDATE)
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let group: Option<String> = node
            .attributes
            .get(attrs::REUSE_GROUP)
            .and_then(Value::as_str)
            .map(str::to_owned);

        // --- Derivations -----------------------------------------------
        let critical_path = (priority as i64) >= CRITICAL_PATH_PRIORITY_THRESHOLD;
        let priority_class = priority_class_from_priority(priority);
        let pin_mode = if group.is_some() {
            apxm_llm::PIN_MODE_PREFIX
        } else {
            apxm_llm::PIN_MODE_NONE
        };
        let pipeline =
            group.is_some() && downstream_nodes.len() as u32 >= PIPELINE_DOWNSTREAM_THRESHOLD;

        // --- Write back the eight `_vllm_*` keys -----------------------
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
        } else {
            // Overwrite any stale value the compile-time pass may have left.
            node.attributes.remove(attrs::VLLM_REUSE_GROUP);
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

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::execution::Node;

    fn ask_node(id: u64) -> Node {
        Node::new(id, AISOperationType::Ask)
    }

    fn dag_with_nodes(nodes: Vec<Node>) -> ExecutionDag {
        let mut dag = ExecutionDag::new();
        for n in nodes {
            dag.add_node(n).expect("add node");
        }
        dag
    }

    #[test]
    fn derives_vllm_attrs_for_shared_group() {
        let mut a = ask_node(0);
        a.set_attribute(
            attrs::PRIORITY.to_string(),
            Value::Number(Number::Integer(90)),
        );
        a.set_attribute(
            attrs::REUSE_GROUP.to_string(),
            Value::String("grp_x".to_string()),
        );
        a.set_attribute(
            attrs::SHARED_PREFIX_EST_TOKENS.to_string(),
            Value::Number(Number::Integer(4096)),
        );
        a.set_attribute(
            attrs::DOWNSTREAM_NODES.to_string(),
            Value::Array(vec![
                Value::Number(Number::Integer(2)),
                Value::Number(Number::Integer(3)),
            ]),
        );
        a.set_attribute(attrs::WARMUP_CANDIDATE.to_string(), Value::Bool(true));

        let mut b = ask_node(1);
        b.set_attribute(
            attrs::PRIORITY.to_string(),
            Value::Number(Number::Integer(30)),
        );
        b.set_attribute(
            attrs::REUSE_GROUP.to_string(),
            Value::String("grp_x".to_string()),
        );
        b.set_attribute(
            attrs::DOWNSTREAM_NODES.to_string(),
            Value::Array(vec![Value::Number(Number::Integer(4))]),
        );

        let mut dag = dag_with_nodes(vec![a, b]);
        let n = derive_vllm_attrs(&mut dag);
        assert_eq!(n, 2);

        let a_attrs = &dag.nodes[0].attributes;
        assert_eq!(
            a_attrs.get(attrs::VLLM_PRIORITY_CLASS),
            Some(&Value::String(apxm_llm::PRIORITY_CRITICAL_PATH.to_string()))
        );
        assert_eq!(
            a_attrs.get(attrs::VLLM_CRITICAL_PATH),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            a_attrs.get(attrs::VLLM_REUSE_GROUP),
            Some(&Value::String("grp_x".to_string()))
        );
        assert_eq!(
            a_attrs.get(attrs::VLLM_PIN_MODE),
            Some(&Value::String(apxm_llm::PIN_MODE_PREFIX.to_string()))
        );
        assert_eq!(
            a_attrs.get(attrs::VLLM_DOWNSTREAM_NODES),
            Some(&Value::Array(vec![
                Value::Number(Number::Integer(2)),
                Value::Number(Number::Integer(3)),
            ]))
        );
        // group present + downstream fanout 2 → pipeline
        assert_eq!(a_attrs.get(attrs::VLLM_PIPELINE), Some(&Value::Bool(true)));
        assert_eq!(a_attrs.get(attrs::VLLM_WARMUP), Some(&Value::Bool(true)));

        let b_attrs = &dag.nodes[1].attributes;
        assert_eq!(
            b_attrs.get(attrs::VLLM_PRIORITY_CLASS),
            Some(&Value::String(apxm_llm::PRIORITY_PARALLEL.to_string()))
        );
        assert_eq!(
            b_attrs.get(attrs::VLLM_CRITICAL_PATH),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            b_attrs.get(attrs::VLLM_PIN_MODE),
            Some(&Value::String(apxm_llm::PIN_MODE_PREFIX.to_string()))
        );
        // group present but downstream (1) < 2 → not pipeline
        assert_eq!(b_attrs.get(attrs::VLLM_PIPELINE), Some(&Value::Bool(false)));
        // No warmup attr → defaults to false.
        assert_eq!(b_attrs.get(attrs::VLLM_WARMUP), Some(&Value::Bool(false)));
    }

    #[test]
    fn grouped_node_uses_prefix_pin_mode() {
        let mut a = ask_node(0);
        a.set_attribute(
            attrs::PRIORITY.to_string(),
            Value::Number(Number::Integer(90)),
        );
        a.set_attribute(
            attrs::REUSE_GROUP.to_string(),
            Value::String("lonely".to_string()),
        );
        a.set_attribute(
            attrs::DOWNSTREAM_NODES.to_string(),
            Value::Array(vec![Value::Number(Number::Integer(5))]),
        );

        let mut dag = dag_with_nodes(vec![a]);
        derive_vllm_attrs(&mut dag);
        assert_eq!(
            dag.nodes[0].attributes.get(attrs::VLLM_PIN_MODE),
            Some(&Value::String(apxm_llm::PIN_MODE_PREFIX.to_string()))
        );
    }

    #[test]
    fn skips_non_llm_nodes() {
        let merge = Node::new(0, AISOperationType::Merge);
        let mut dag = dag_with_nodes(vec![merge]);
        let n = derive_vllm_attrs(&mut dag);
        assert_eq!(n, 0);
        assert!(
            !dag.nodes[0]
                .attributes
                .contains_key(attrs::VLLM_PRIORITY_CLASS),
            "non-LLM nodes should not get _vllm_* attrs"
        );
    }

    #[test]
    fn missing_source_attrs_use_safe_defaults_when_at_least_one_is_present() {
        // PRIORITY alone is enough to trigger derivation; the rest get
        // sensible defaults.
        let mut bare = ask_node(0);
        bare.set_attribute(
            attrs::PRIORITY.to_string(),
            Value::Number(Number::Integer(30)),
        );
        let mut dag = dag_with_nodes(vec![bare]);
        let n = derive_vllm_attrs(&mut dag);
        assert_eq!(n, 1);
        let a = &dag.nodes[0].attributes;
        assert_eq!(
            a.get(attrs::VLLM_PRIORITY_CLASS),
            Some(&Value::String(apxm_llm::PRIORITY_PARALLEL.to_string()))
        );
        assert_eq!(a.get(attrs::VLLM_CRITICAL_PATH), Some(&Value::Bool(false)));
        assert!(!a.contains_key(attrs::VLLM_REUSE_GROUP));
        assert_eq!(
            a.get(attrs::VLLM_PIN_MODE),
            Some(&Value::String(apxm_llm::PIN_MODE_NONE.to_string()))
        );
        assert_eq!(a.get(attrs::VLLM_PIPELINE), Some(&Value::Bool(false)));
        assert_eq!(a.get(attrs::VLLM_WARMUP), Some(&Value::Bool(false)));
    }

    #[test]
    fn skips_node_with_no_mlir_stamped_attrs_preserving_user_vllm_hints() {
        // If a Node has only `_vllm_*` attrs (e.g. from a unit test or a
        // graph that bypasses MLIR canonicalization), the runtime
        // derivation must not clobber them.
        let mut a = ask_node(0);
        a.set_attribute(
            attrs::VLLM_PRIORITY_CLASS.to_string(),
            Value::String("critical_path".to_string()),
        );
        a.set_attribute(
            attrs::VLLM_REUSE_GROUP.to_string(),
            Value::String("user-group".to_string()),
        );

        let mut dag = dag_with_nodes(vec![a]);
        let n = derive_vllm_attrs(&mut dag);
        assert_eq!(n, 0);
        let attrs_map = &dag.nodes[0].attributes;
        assert_eq!(
            attrs_map.get(attrs::VLLM_PRIORITY_CLASS),
            Some(&Value::String("critical_path".to_string())),
            "user-set _vllm_priority_class must be preserved"
        );
        assert_eq!(
            attrs_map.get(attrs::VLLM_REUSE_GROUP),
            Some(&Value::String("user-group".to_string()))
        );
    }

    #[test]
    fn overwrites_stale_compile_time_attrs() {
        // The compile-time vllm_hints pass runs before MLIR populates
        // ais.* attrs, so it stamps default values. The runtime derivation
        // must overwrite those defaults with values derived from
        // MLIR-stamped (bare-name) attrs.
        let mut a = ask_node(0);
        // Stale compile-time defaults (parallel, no group, etc.)
        a.set_attribute(
            attrs::VLLM_PRIORITY_CLASS.to_string(),
            Value::String(apxm_llm::PRIORITY_PARALLEL.to_string()),
        );
        a.set_attribute(attrs::VLLM_CRITICAL_PATH.to_string(), Value::Bool(false));
        a.set_attribute(
            attrs::VLLM_REUSE_GROUP.to_string(),
            Value::String("stale".to_string()),
        );
        // The actual MLIR-stamped values (post-canonicalization).
        a.set_attribute(
            attrs::PRIORITY.to_string(),
            Value::Number(Number::Integer(95)),
        );
        // No REUSE_GROUP from MLIR → derivation must clear the stale entry.

        let mut dag = dag_with_nodes(vec![a]);
        derive_vllm_attrs(&mut dag);
        let attrs_map = &dag.nodes[0].attributes;
        assert_eq!(
            attrs_map.get(attrs::VLLM_PRIORITY_CLASS),
            Some(&Value::String(apxm_llm::PRIORITY_CRITICAL_PATH.to_string())),
            "runtime derivation must overwrite stale compile-time priority class"
        );
        assert_eq!(
            attrs_map.get(attrs::VLLM_CRITICAL_PATH),
            Some(&Value::Bool(true)),
            "priority 95 ≥ 70 → critical path"
        );
        assert!(
            !attrs_map.contains_key(attrs::VLLM_REUSE_GROUP),
            "stale reuse group must be removed when MLIR didn't stamp one"
        );
    }
}
