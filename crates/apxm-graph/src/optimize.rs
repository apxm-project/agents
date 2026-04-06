//! Graph-level optimization passes.
//!
//! These passes annotate [`ApxmGraph`] nodes with hint attributes that the
//! runtime or MLIR backend can exploit.  They run *before* MLIR lowering so
//! the hints are available in the generated IR.
//!
//! # Passes
//!
//! - [`ApxmGraph::prompt_caching`] — detect shared system prompts across
//!   ASK/THINK nodes and mark subsequent uses with `cached_system_prompt`.
//! - [`ApxmGraph::memoization_hints`] — detect duplicate pure operations
//!   with identical attributes and mark subsequent duplicates with `memoizable`.

use crate::{ApxmGraph, GraphError};
use apxm_core::constants::graph::attrs;
use apxm_core::types::{AISOperationType, Value};
use std::collections::HashMap;

impl ApxmGraph {
    /// Mark ASK/THINK nodes whose `system_prompt` attribute has already been
    /// sent by an earlier node in the graph.
    ///
    /// When multiple ASK or THINK nodes share the same `system_prompt` value,
    /// the first occurrence is left untouched and every subsequent occurrence
    /// gets a `cached_system_prompt: true` attribute.  The runtime can use
    /// this hint to avoid re-sending the system prompt text, saving tokens.
    pub fn prompt_caching(&mut self) -> Result<&mut Self, GraphError> {
        // Map: system_prompt text -> id of the first node that used it.
        let mut seen: HashMap<String, u64> = HashMap::new();

        for node in &mut self.nodes {
            // Only ASK and THINK carry system prompts.
            if !matches!(node.op, AISOperationType::Ask | AISOperationType::Think) {
                continue;
            }

            let prompt_value = match node.attributes.get(attrs::SYSTEM_PROMPT) {
                Some(Value::String(s)) if !s.is_empty() => s.clone(),
                _ => continue,
            };

            if seen.contains_key(&prompt_value) {
                // Subsequent use — mark as cached.
                node.attributes
                    .insert(attrs::CACHED_SYSTEM_PROMPT.to_string(), Value::Bool(true));
            } else {
                // First occurrence — record it.
                seen.insert(prompt_value, node.id);
            }
        }

        Ok(self)
    }

    /// Mark duplicate pure operations with identical attributes so the
    /// runtime can memoize (cache) their results.
    ///
    /// Pure operations are those with no side effects: `QMEM`, `CONST_STR`,
    /// and `VERIFY`.  When two such operations have the same `op` and
    /// identical attribute maps, the second (and subsequent) duplicates are
    /// annotated with `memoizable: true`.
    pub fn memoization_hints(&mut self) -> Result<&mut Self, GraphError> {
        /// Returns `true` if `op` is considered pure (deterministic, no
        /// side effects) for memoization purposes.
        fn is_pure(op: AISOperationType) -> bool {
            matches!(
                op,
                AISOperationType::QMem | AISOperationType::ConstStr | AISOperationType::Verify
            )
        }

        // Signature = (op, sorted attribute entries) -> first node id.
        // We serialise attributes to a canonical JSON string for comparison.
        let mut seen: HashMap<(AISOperationType, String), u64> = HashMap::new();

        // Collect signatures first (immutable borrow).
        let signatures: Vec<(usize, AISOperationType, String, bool)> = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(idx, node)| {
                if !is_pure(node.op) {
                    return None;
                }
                // Build a canonical key from sorted attribute entries.
                let mut entries: Vec<(&String, &Value)> = node.attributes.iter().collect();
                entries.sort_by_key(|(k, _)| k.as_str());
                let canonical = serde_json::to_string(&entries).unwrap_or_default();
                let key = (node.op, canonical.clone());
                let is_dup = seen.contains_key(&key);
                if !is_dup {
                    seen.insert(key, node.id);
                }
                Some((idx, node.op, canonical, is_dup))
            })
            .collect();

        // Now apply mutable writes for duplicates.
        for (idx, _op, _canonical, is_dup) in signatures {
            if is_dup {
                self.nodes[idx]
                    .attributes
                    .insert(attrs::MEMOIZABLE.to_string(), Value::Bool(true));
            }
        }

        Ok(self)
    }

    /// Analyzes the graph DAG and computes parallelism metrics.
    ///
    /// This is a read-only pass that emits analysis results as metadata:
    /// - `analysis.max_parallelism`: maximum concurrent node count
    /// - `analysis.critical_path_length`: length of longest dependency chain
    /// - `analysis.total_nodes`: total node count (for reference)
    pub fn parallelism_analysis(&mut self) -> Result<&mut Self, GraphError> {
        let analysis = compute_parallelism_metrics(self);

        // Emit metrics as metadata
        self.metadata.insert(
            "analysis.max_parallelism".to_string(),
            Value::Number((analysis.max_parallelism as i64).into()),
        );
        self.metadata.insert(
            "analysis.critical_path_length".to_string(),
            Value::Number((analysis.critical_path_length as i64).into()),
        );
        self.metadata.insert(
            "analysis.total_nodes".to_string(),
            Value::Number((self.nodes.len() as i64).into()),
        );

        Ok(self)
    }

    /// Folds CONST_STR nodes into downstream THINK/ASK templates.
    ///
    /// When a CONST_STR node is the sole input to a THINK/ASK node and the
    /// template contains a `{0}` placeholder, inline the constant value
    /// directly into the template and remove the CONST_STR node.
    ///
    /// This reduces graph size and simplifies execution.
    pub fn constant_folding(&mut self) -> Result<&mut Self, GraphError> {
        let mut to_remove = Vec::new();
        let mut template_updates: Vec<(usize, String)> = Vec::new();

        // Build adjacency map: node_id -> list of outgoing edges
        let mut out_edges: HashMap<u64, Vec<u64>> = HashMap::new();
        for edge in &self.edges {
            out_edges.entry(edge.from).or_default().push(edge.to);
        }

        // Scan for foldable patterns
        for (_idx, node) in self.nodes.iter().enumerate() {
            if node.op != AISOperationType::ConstStr {
                continue;
            }

            // Get the constant value
            let const_value = match node.attributes.get(attrs::VALUE) {
                Some(Value::String(s)) => s.clone(),
                _ => continue,
            };

            // Check if this CONST_STR feeds exactly one downstream node
            let downstream = match out_edges.get(&node.id) {
                Some(targets) if targets.len() == 1 => targets[0],
                _ => continue,
            };

            // Find the downstream node
            let target_idx = match self.nodes.iter().position(|n| n.id == downstream) {
                Some(i) => i,
                None => continue,
            };
            let target = &self.nodes[target_idx];

            // Only fold into THINK/ASK/REASON nodes (all support template_str)
            if !matches!(
                target.op,
                AISOperationType::Think | AISOperationType::Ask | AISOperationType::Reason
            ) {
                continue;
            }

            // Check if template contains {0} placeholder
            let template = match target.attributes.get(attrs::TEMPLATE_STR) {
                Some(Value::String(s)) if s.contains("{0}") => s.clone(),
                _ => continue,
            };

            // Inline the constant
            let new_template = template.replace("{0}", &const_value);
            template_updates.push((target_idx, new_template));
            to_remove.push(node.id);
        }

        // Apply template updates
        for (idx, new_template) in template_updates {
            self.nodes[idx]
                .attributes
                .insert(attrs::TEMPLATE_STR.to_string(), Value::String(new_template));
        }

        // Remove folded CONST_STR nodes and their edges
        self.nodes.retain(|n| !to_remove.contains(&n.id));
        self.edges
            .retain(|e| !to_remove.contains(&e.from) && !to_remove.contains(&e.to));

        Ok(self)
    }
}

#[derive(Debug)]
struct ParallelismMetrics {
    max_parallelism: usize,
    critical_path_length: usize,
}

/// Compute parallelism metrics for a graph using level-based scheduling.
fn compute_parallelism_metrics(graph: &ApxmGraph) -> ParallelismMetrics {
    if graph.nodes.is_empty() {
        return ParallelismMetrics {
            max_parallelism: 0,
            critical_path_length: 0,
        };
    }

    // Build adjacency lists
    let mut incoming: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut outgoing: HashMap<u64, Vec<u64>> = HashMap::new();

    for edge in &graph.edges {
        incoming.entry(edge.to).or_default().push(edge.from);
        outgoing.entry(edge.from).or_default().push(edge.to);
    }

    // Compute longest path to each node (critical path)
    let mut longest_path: HashMap<u64, usize> = HashMap::new();
    let mut queue: Vec<u64> = graph
        .nodes
        .iter()
        .filter(|n| !incoming.contains_key(&n.id))
        .map(|n| n.id)
        .collect();

    for id in &queue {
        longest_path.insert(*id, 1);
    }

    let mut visited = std::collections::HashSet::new();
    while let Some(node_id) = queue.pop() {
        if !visited.insert(node_id) {
            continue;
        }

        let current_depth = *longest_path.get(&node_id).unwrap_or(&1);

        if let Some(children) = outgoing.get(&node_id) {
            for &child in children {
                let new_depth = current_depth + 1;
                let entry = longest_path.entry(child).or_insert(0);
                *entry = (*entry).max(new_depth);
                queue.push(child);
            }
        }
    }

    let critical_path_length = longest_path.values().max().copied().unwrap_or(0);

    // Compute max parallelism via level sets
    let mut levels: HashMap<usize, Vec<u64>> = HashMap::new();
    for (id, depth) in &longest_path {
        levels.entry(*depth).or_default().push(*id);
    }

    let max_parallelism = levels.values().map(|v| v.len()).max().unwrap_or(0);

    ParallelismMetrics {
        max_parallelism,
        critical_path_length,
    }
}

#[cfg(test)]
mod prompt_caching_memo_tests {
    use super::*;
    use crate::{GraphEdge, GraphNode};
    use apxm_core::types::DependencyType;

    // -----------------------------------------------------------------------
    // prompt_caching tests
    // -----------------------------------------------------------------------

    #[test]
    fn prompt_caching_marks_duplicate_system_prompts() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "ask1".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String("You are a helpful assistant.".into()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "ask2".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String("You are a helpful assistant.".into()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 3,
                    name: "think1".to_string(),
                    op: AISOperationType::Think,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String("You are a helpful assistant.".into()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
            ],
            edges: vec![
                GraphEdge {
                    from: 1,
                    to: 2,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 2,
                    to: 3,
                    dependency: DependencyType::Data,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.prompt_caching().unwrap();

        // First node should NOT have cached_system_prompt.
        assert!(
            !graph.nodes[0]
                .attributes
                .contains_key(attrs::CACHED_SYSTEM_PROMPT)
        );

        // Second and third nodes share the same system_prompt -> should be cached.
        assert_eq!(
            graph.nodes[1].attributes.get(attrs::CACHED_SYSTEM_PROMPT),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            graph.nodes[2].attributes.get(attrs::CACHED_SYSTEM_PROMPT),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn prompt_caching_different_prompts_not_cached() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "ask1".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String("Prompt A".into()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "ask2".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String("Prompt B".into()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.prompt_caching().unwrap();

        // Different prompts -> neither should be cached.
        assert!(
            !graph.nodes[0]
                .attributes
                .contains_key(attrs::CACHED_SYSTEM_PROMPT)
        );
        assert!(
            !graph.nodes[1]
                .attributes
                .contains_key(attrs::CACHED_SYSTEM_PROMPT)
        );
    }

    #[test]
    fn prompt_caching_ignores_non_ask_think_ops() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "reason1".to_string(),
                    op: AISOperationType::Reason,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String("Shared prompt".into()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "reason2".to_string(),
                    op: AISOperationType::Reason,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String("Shared prompt".into()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.prompt_caching().unwrap();

        // REASON ops should not be processed.
        assert!(
            !graph.nodes[0]
                .attributes
                .contains_key(attrs::CACHED_SYSTEM_PROMPT)
        );
        assert!(
            !graph.nodes[1]
                .attributes
                .contains_key(attrs::CACHED_SYSTEM_PROMPT)
        );
    }

    #[test]
    fn prompt_caching_skips_empty_system_prompt() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "ask1".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String(String::new()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "ask2".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String(String::new()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.prompt_caching().unwrap();

        // Empty system prompts should be skipped.
        assert!(
            !graph.nodes[0]
                .attributes
                .contains_key(attrs::CACHED_SYSTEM_PROMPT)
        );
        assert!(
            !graph.nodes[1]
                .attributes
                .contains_key(attrs::CACHED_SYSTEM_PROMPT)
        );
    }

    #[test]
    fn prompt_caching_no_system_prompt_attribute() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![GraphNode {
                id: 1,
                name: "ask1".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([(
                    attrs::TEMPLATE_STR.to_string(),
                    Value::String("{0}".into()),
                )]),
            }],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.prompt_caching().unwrap();

        assert!(
            !graph.nodes[0]
                .attributes
                .contains_key(attrs::CACHED_SYSTEM_PROMPT)
        );
    }

    // -----------------------------------------------------------------------
    // memoization_hints tests
    // -----------------------------------------------------------------------

    #[test]
    fn memoization_marks_duplicate_const_str() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "const1".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("hello".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "const2".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("hello".into()),
                    )]),
                },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.memoization_hints().unwrap();

        // First occurrence should NOT be memoizable.
        assert!(!graph.nodes[0].attributes.contains_key(attrs::MEMOIZABLE));

        // Second occurrence should be memoizable.
        assert_eq!(
            graph.nodes[1].attributes.get(attrs::MEMOIZABLE),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn memoization_different_attributes_not_marked() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "const1".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("hello".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "const2".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("world".into()),
                    )]),
                },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.memoization_hints().unwrap();

        assert!(!graph.nodes[0].attributes.contains_key(attrs::MEMOIZABLE));
        assert!(!graph.nodes[1].attributes.contains_key(attrs::MEMOIZABLE));
    }

    #[test]
    fn memoization_marks_duplicate_qmem() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "qmem1".to_string(),
                    op: AISOperationType::QMem,
                    attributes: HashMap::from([(
                        attrs::QUERY.to_string(),
                        Value::String("recall last interaction".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "qmem2".to_string(),
                    op: AISOperationType::QMem,
                    attributes: HashMap::from([(
                        attrs::QUERY.to_string(),
                        Value::String("recall last interaction".into()),
                    )]),
                },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.memoization_hints().unwrap();

        assert!(!graph.nodes[0].attributes.contains_key(attrs::MEMOIZABLE));
        assert_eq!(
            graph.nodes[1].attributes.get(attrs::MEMOIZABLE),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn memoization_marks_duplicate_verify() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "verify1".to_string(),
                    op: AISOperationType::Verify,
                    attributes: HashMap::from([(
                        attrs::CONDITION.to_string(),
                        Value::String("x > 0".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "verify2".to_string(),
                    op: AISOperationType::Verify,
                    attributes: HashMap::from([(
                        attrs::CONDITION.to_string(),
                        Value::String("x > 0".into()),
                    )]),
                },
                GraphNode {
                    id: 3,
                    name: "verify3".to_string(),
                    op: AISOperationType::Verify,
                    attributes: HashMap::from([(
                        attrs::CONDITION.to_string(),
                        Value::String("y > 0".into()),
                    )]),
                },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.memoization_hints().unwrap();

        // First verify: not memoizable.
        assert!(!graph.nodes[0].attributes.contains_key(attrs::MEMOIZABLE));
        // Second verify (same condition): memoizable.
        assert_eq!(
            graph.nodes[1].attributes.get(attrs::MEMOIZABLE),
            Some(&Value::Bool(true))
        );
        // Third verify (different condition): not memoizable.
        assert!(!graph.nodes[2].attributes.contains_key(attrs::MEMOIZABLE));
    }

    #[test]
    fn memoization_ignores_impure_ops() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "ask1".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([(
                        attrs::TEMPLATE_STR.to_string(),
                        Value::String("same".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "ask2".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([(
                        attrs::TEMPLATE_STR.to_string(),
                        Value::String("same".into()),
                    )]),
                },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.memoization_hints().unwrap();

        // ASK is not pure -> should not be marked.
        assert!(!graph.nodes[0].attributes.contains_key(attrs::MEMOIZABLE));
        assert!(!graph.nodes[1].attributes.contains_key(attrs::MEMOIZABLE));
    }

    #[test]
    fn memoization_multiple_duplicates() {
        let attrs_map = HashMap::from([(attrs::VALUE.to_string(), Value::String("x".into()))]);
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "c1".into(),
                    op: AISOperationType::ConstStr,
                    attributes: attrs_map.clone(),
                },
                GraphNode {
                    id: 2,
                    name: "c2".into(),
                    op: AISOperationType::ConstStr,
                    attributes: attrs_map.clone(),
                },
                GraphNode {
                    id: 3,
                    name: "c3".into(),
                    op: AISOperationType::ConstStr,
                    attributes: attrs_map.clone(),
                },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.memoization_hints().unwrap();

        assert!(!graph.nodes[0].attributes.contains_key(attrs::MEMOIZABLE));
        assert_eq!(
            graph.nodes[1].attributes.get(attrs::MEMOIZABLE),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            graph.nodes[2].attributes.get(attrs::MEMOIZABLE),
            Some(&Value::Bool(true))
        );
    }
}

#[cfg(test)]
mod analysis_folding_tests {
    use super::*;
    use crate::{GraphEdge, GraphNode};
    use apxm_core::types::{DependencyType, Number};

    // -----------------------------------------------------------------------
    // parallelism_analysis tests
    // -----------------------------------------------------------------------

    #[test]
    fn parallelism_analysis_linear_chain() {
        let mut graph = ApxmGraph {
            name: "linear".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "n1".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 2,
                    name: "n2".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 3,
                    name: "n3".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
            ],
            edges: vec![
                GraphEdge {
                    from: 1,
                    to: 2,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 2,
                    to: 3,
                    dependency: DependencyType::Data,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.parallelism_analysis().unwrap();

        assert_eq!(
            graph.metadata.get("analysis.max_parallelism"),
            Some(&Value::Number(Number::Integer(1)))
        );
        assert_eq!(
            graph.metadata.get("analysis.critical_path_length"),
            Some(&Value::Number(Number::Integer(3)))
        );
    }

    #[test]
    fn parallelism_analysis_fan_out() {
        let mut graph = ApxmGraph {
            name: "fanout".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "root".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 2,
                    name: "branch1".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 3,
                    name: "branch2".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 4,
                    name: "branch3".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
            ],
            edges: vec![
                GraphEdge {
                    from: 1,
                    to: 2,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 1,
                    to: 3,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 1,
                    to: 4,
                    dependency: DependencyType::Data,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.parallelism_analysis().unwrap();

        // 3 branches can run in parallel
        assert_eq!(
            graph.metadata.get("analysis.max_parallelism"),
            Some(&Value::Number(Number::Integer(3)))
        );
        // Critical path: root -> any branch = 2
        assert_eq!(
            graph.metadata.get("analysis.critical_path_length"),
            Some(&Value::Number(Number::Integer(2)))
        );
    }

    #[test]
    fn parallelism_analysis_empty_graph() {
        let mut graph = ApxmGraph {
            name: "empty".to_string(),
            nodes: vec![],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.parallelism_analysis().unwrap();

        assert_eq!(
            graph.metadata.get("analysis.max_parallelism"),
            Some(&Value::Number(Number::Integer(0)))
        );
        assert_eq!(
            graph.metadata.get("analysis.critical_path_length"),
            Some(&Value::Number(Number::Integer(0)))
        );
    }

    #[test]
    fn parallelism_analysis_complex_dag() {
        // Diamond pattern: root -> (A, B) -> merge
        let mut graph = ApxmGraph {
            name: "diamond".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "root".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 2,
                    name: "a".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 3,
                    name: "b".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 4,
                    name: "merge".into(),
                    op: AISOperationType::Merge,
                    attributes: HashMap::new(),
                },
            ],
            edges: vec![
                GraphEdge {
                    from: 1,
                    to: 2,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 1,
                    to: 3,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 2,
                    to: 4,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 3,
                    to: 4,
                    dependency: DependencyType::Data,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.parallelism_analysis().unwrap();

        // Max parallelism = 2 (A and B can run together)
        assert_eq!(
            graph.metadata.get("analysis.max_parallelism"),
            Some(&Value::Number(Number::Integer(2)))
        );
        // Critical path: root -> A/B -> merge = 3
        assert_eq!(
            graph.metadata.get("analysis.critical_path_length"),
            Some(&Value::Number(Number::Integer(3)))
        );
    }

    #[test]
    fn parallelism_analysis_multi_branch_accuracy() {
        // Test complex DAG with multiple parallel branches of different lengths
        // to verify critical path calculation is accurate
        //
        // Structure:
        //   root (1) -> branch_a (2) -> merge (7)
        //            -> branch_b1 (3) -> branch_b2 (4) -> branch_b3 (5) -> merge (7)
        //            -> branch_c (6) -> merge (7)
        //
        // Expected:
        //   Max parallelism = 3 (nodes 2, 3, 6 run in parallel)
        //   Critical path = 5 (root -> b1 -> b2 -> b3 -> merge)
        let mut graph = ApxmGraph {
            name: "multi_branch".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "root".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 2,
                    name: "branch_a".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 3,
                    name: "branch_b1".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 4,
                    name: "branch_b2".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 5,
                    name: "branch_b3".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 6,
                    name: "branch_c".into(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 7,
                    name: "merge".into(),
                    op: AISOperationType::Merge,
                    attributes: HashMap::new(),
                },
            ],
            edges: vec![
                GraphEdge {
                    from: 1,
                    to: 2,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 1,
                    to: 3,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 1,
                    to: 6,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 3,
                    to: 4,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 4,
                    to: 5,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 2,
                    to: 7,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 5,
                    to: 7,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 6,
                    to: 7,
                    dependency: DependencyType::Data,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.parallelism_analysis().unwrap();

        // Max parallelism = 3 (2, 3, 6 can run in parallel)
        assert_eq!(
            graph.metadata.get("analysis.max_parallelism"),
            Some(&Value::Number(Number::Integer(3)))
        );
        // Critical path: root -> b1 -> b2 -> b3 -> merge = 5
        assert_eq!(
            graph.metadata.get("analysis.critical_path_length"),
            Some(&Value::Number(Number::Integer(5)))
        );
    }

    // -----------------------------------------------------------------------
    // constant_folding tests
    // -----------------------------------------------------------------------

    #[test]
    fn constant_folding_folds_single_input() {
        let mut graph = ApxmGraph {
            name: "fold_test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "const1".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("hello world".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "think1".to_string(),
                    op: AISOperationType::Think,
                    attributes: HashMap::from([(
                        attrs::TEMPLATE_STR.to_string(),
                        Value::String("Process: {0}".into()),
                    )]),
                },
            ],
            edges: vec![GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.constant_folding().unwrap();

        // CONST_STR node should be removed
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].op, AISOperationType::Think);

        // Template should be inlined
        assert_eq!(
            graph.nodes[0].attributes.get(attrs::TEMPLATE_STR),
            Some(&Value::String("Process: hello world".into()))
        );

        // Edge should be removed
        assert_eq!(graph.edges.len(), 0);
    }

    #[test]
    fn constant_folding_ignores_multiple_outputs() {
        let mut graph = ApxmGraph {
            name: "multi_out".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "const1".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("shared".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "think1".to_string(),
                    op: AISOperationType::Think,
                    attributes: HashMap::from([(
                        attrs::TEMPLATE_STR.to_string(),
                        Value::String("A: {0}".into()),
                    )]),
                },
                GraphNode {
                    id: 3,
                    name: "think2".to_string(),
                    op: AISOperationType::Think,
                    attributes: HashMap::from([(
                        attrs::TEMPLATE_STR.to_string(),
                        Value::String("B: {0}".into()),
                    )]),
                },
            ],
            edges: vec![
                GraphEdge {
                    from: 1,
                    to: 2,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 1,
                    to: 3,
                    dependency: DependencyType::Data,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.constant_folding().unwrap();

        // Should NOT fold (CONST_STR has multiple consumers)
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);
    }

    #[test]
    fn constant_folding_requires_placeholder() {
        let mut graph = ApxmGraph {
            name: "no_placeholder".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "const1".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("value".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "think1".to_string(),
                    op: AISOperationType::Think,
                    attributes: HashMap::from([(
                        attrs::TEMPLATE_STR.to_string(),
                        Value::String("No placeholder here".into()),
                    )]),
                },
            ],
            edges: vec![GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.constant_folding().unwrap();

        // Should NOT fold (no {0} in template)
        assert_eq!(graph.nodes.len(), 2);
    }

    #[test]
    fn constant_folding_works_with_reason() {
        let mut graph = ApxmGraph {
            name: "fold_reason".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "const1".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("analyze this".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "reason1".to_string(),
                    op: AISOperationType::Reason,
                    attributes: HashMap::from([(
                        attrs::TEMPLATE_STR.to_string(),
                        Value::String("Reason about: {0}".into()),
                    )]),
                },
            ],
            edges: vec![GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.constant_folding().unwrap();

        // CONST_STR should be folded into REASON
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].op, AISOperationType::Reason);
        assert_eq!(
            graph.nodes[0].attributes.get(attrs::TEMPLATE_STR),
            Some(&Value::String("Reason about: analyze this".into()))
        );
        assert_eq!(graph.edges.len(), 0);
    }

    #[test]
    fn constant_folding_only_targets_think_ask() {
        let mut graph = ApxmGraph {
            name: "wrong_target".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "const1".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("value".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "merge1".to_string(),
                    op: AISOperationType::Merge,
                    attributes: HashMap::new(),
                },
            ],
            edges: vec![GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.constant_folding().unwrap();

        // Should NOT fold (target is not THINK/ASK)
        assert_eq!(graph.nodes.len(), 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphEdge, GraphNode};
    use apxm_core::types::DependencyType;

    // -----------------------------------------------------------------------
    // Combined passes
    // -----------------------------------------------------------------------

    #[test]
    fn both_passes_compose() {
        let mut graph = ApxmGraph {
            name: "combo".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "const1".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("hello".into()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "ask1".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String("Be concise".into()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 3,
                    name: "const2".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        attrs::VALUE.to_string(),
                        Value::String("hello".into()),
                    )]),
                },
                GraphNode {
                    id: 4,
                    name: "ask2".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (
                            attrs::SYSTEM_PROMPT.to_string(),
                            Value::String("Be concise".into()),
                        ),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
            ],
            edges: vec![
                GraphEdge {
                    from: 1,
                    to: 2,
                    dependency: DependencyType::Data,
                },
                GraphEdge {
                    from: 3,
                    to: 4,
                    dependency: DependencyType::Data,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.prompt_caching().unwrap().memoization_hints().unwrap();

        // prompt_caching: ask2 should be cached.
        assert!(
            !graph.nodes[1]
                .attributes
                .contains_key(attrs::CACHED_SYSTEM_PROMPT)
        );
        assert_eq!(
            graph.nodes[3].attributes.get(attrs::CACHED_SYSTEM_PROMPT),
            Some(&Value::Bool(true))
        );

        // memoization_hints: const2 should be memoizable.
        assert!(!graph.nodes[0].attributes.contains_key(attrs::MEMOIZABLE));
        assert_eq!(
            graph.nodes[2].attributes.get(attrs::MEMOIZABLE),
            Some(&Value::Bool(true))
        );
    }
}
