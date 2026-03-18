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
                node.attributes.insert(
                    attrs::CACHED_SYSTEM_PROMPT.to_string(),
                    Value::Bool(true),
                );
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
                AISOperationType::QMem
                    | AISOperationType::ConstStr
                    | AISOperationType::Verify
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
                self.nodes[idx].attributes.insert(
                    attrs::MEMOIZABLE.to_string(),
                    Value::Bool(true),
                );
            }
        }

        Ok(self)
    }
}

#[cfg(test)]
mod tests {
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
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String("You are a helpful assistant.".into())),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "ask2".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String("You are a helpful assistant.".into())),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 3,
                    name: "think1".to_string(),
                    op: AISOperationType::Think,
                    attributes: HashMap::from([
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String("You are a helpful assistant.".into())),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
            ],
            edges: vec![
                GraphEdge { from: 1, to: 2, dependency: DependencyType::Data },
                GraphEdge { from: 2, to: 3, dependency: DependencyType::Data },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.prompt_caching().unwrap();

        // First node should NOT have cached_system_prompt.
        assert!(!graph.nodes[0].attributes.contains_key(attrs::CACHED_SYSTEM_PROMPT));

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
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String("Prompt A".into())),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "ask2".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String("Prompt B".into())),
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
        assert!(!graph.nodes[0].attributes.contains_key(attrs::CACHED_SYSTEM_PROMPT));
        assert!(!graph.nodes[1].attributes.contains_key(attrs::CACHED_SYSTEM_PROMPT));
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
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String("Shared prompt".into())),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "reason2".to_string(),
                    op: AISOperationType::Reason,
                    attributes: HashMap::from([
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String("Shared prompt".into())),
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
        assert!(!graph.nodes[0].attributes.contains_key(attrs::CACHED_SYSTEM_PROMPT));
        assert!(!graph.nodes[1].attributes.contains_key(attrs::CACHED_SYSTEM_PROMPT));
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
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String(String::new())),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "ask2".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String(String::new())),
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
        assert!(!graph.nodes[0].attributes.contains_key(attrs::CACHED_SYSTEM_PROMPT));
        assert!(!graph.nodes[1].attributes.contains_key(attrs::CACHED_SYSTEM_PROMPT));
    }

    #[test]
    fn prompt_caching_no_system_prompt_attribute() {
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "ask1".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.prompt_caching().unwrap();

        assert!(!graph.nodes[0].attributes.contains_key(attrs::CACHED_SYSTEM_PROMPT));
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
                    attributes: HashMap::from([
                        (attrs::VALUE.to_string(), Value::String("hello".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "const2".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([
                        (attrs::VALUE.to_string(), Value::String("hello".into())),
                    ]),
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
                    attributes: HashMap::from([
                        (attrs::VALUE.to_string(), Value::String("hello".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "const2".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([
                        (attrs::VALUE.to_string(), Value::String("world".into())),
                    ]),
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
                    attributes: HashMap::from([
                        (attrs::QUERY.to_string(), Value::String("recall last interaction".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "qmem2".to_string(),
                    op: AISOperationType::QMem,
                    attributes: HashMap::from([
                        (attrs::QUERY.to_string(), Value::String("recall last interaction".into())),
                    ]),
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
                    attributes: HashMap::from([
                        (attrs::CONDITION.to_string(), Value::String("x > 0".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "verify2".to_string(),
                    op: AISOperationType::Verify,
                    attributes: HashMap::from([
                        (attrs::CONDITION.to_string(), Value::String("x > 0".into())),
                    ]),
                },
                GraphNode {
                    id: 3,
                    name: "verify3".to_string(),
                    op: AISOperationType::Verify,
                    attributes: HashMap::from([
                        (attrs::CONDITION.to_string(), Value::String("y > 0".into())),
                    ]),
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
                    attributes: HashMap::from([
                        (attrs::TEMPLATE_STR.to_string(), Value::String("same".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "ask2".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (attrs::TEMPLATE_STR.to_string(), Value::String("same".into())),
                    ]),
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
        let attrs_map = HashMap::from([
            (attrs::VALUE.to_string(), Value::String("x".into())),
        ]);
        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![
                GraphNode { id: 1, name: "c1".into(), op: AISOperationType::ConstStr, attributes: attrs_map.clone() },
                GraphNode { id: 2, name: "c2".into(), op: AISOperationType::ConstStr, attributes: attrs_map.clone() },
                GraphNode { id: 3, name: "c3".into(), op: AISOperationType::ConstStr, attributes: attrs_map.clone() },
            ],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.memoization_hints().unwrap();

        assert!(!graph.nodes[0].attributes.contains_key(attrs::MEMOIZABLE));
        assert_eq!(graph.nodes[1].attributes.get(attrs::MEMOIZABLE), Some(&Value::Bool(true)));
        assert_eq!(graph.nodes[2].attributes.get(attrs::MEMOIZABLE), Some(&Value::Bool(true)));
    }

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
                    attributes: HashMap::from([
                        (attrs::VALUE.to_string(), Value::String("hello".into())),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "ask1".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String("Be concise".into())),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
                GraphNode {
                    id: 3,
                    name: "const2".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([
                        (attrs::VALUE.to_string(), Value::String("hello".into())),
                    ]),
                },
                GraphNode {
                    id: 4,
                    name: "ask2".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([
                        (attrs::SYSTEM_PROMPT.to_string(), Value::String("Be concise".into())),
                        (attrs::TEMPLATE_STR.to_string(), Value::String("{0}".into())),
                    ]),
                },
            ],
            edges: vec![
                GraphEdge { from: 1, to: 2, dependency: DependencyType::Data },
                GraphEdge { from: 3, to: 4, dependency: DependencyType::Data },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        graph.prompt_caching().unwrap().memoization_hints().unwrap();

        // prompt_caching: ask2 should be cached.
        assert!(!graph.nodes[1].attributes.contains_key(attrs::CACHED_SYSTEM_PROMPT));
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
