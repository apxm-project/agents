use crate::air_builder::{
    AirEdge, AirError, AirModule, AirModuleBuilder, PromptInputBinding, apply_prompt_input_bindings,
};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, DependencyType, Value};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

/// Compiler-owned DTO for frontend graph IR.
///
/// This accepts the Python/TypeScript `ApxmGraph.to_dict()` shape and converts
/// it to [`AirModule`]. It is not a printer; all AIR text still comes from
/// `AirModule::to_air()`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FrontendGraph {
    pub name: String,
    #[serde(default)]
    pub nodes: Vec<FrontendNode>,
    #[serde(default)]
    pub edges: Vec<FrontendEdge>,
    #[serde(default)]
    pub parameters: Vec<FrontendParameter>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FrontendNode {
    pub id: u64,
    pub name: String,
    pub op: AISOperationType,
    #[serde(default)]
    pub attributes: HashMap<String, Value>,
}

impl FrontendNode {
    /// Attach ordered prompt inputs to an ASK, THINK, or REASON frontend node.
    pub fn with_prompt_inputs(
        mut self,
        bindings: impl IntoIterator<Item = PromptInputBinding>,
    ) -> Self {
        apply_prompt_input_bindings(&mut self.attributes, bindings);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrontendEdge {
    pub from: u64,
    pub to: u64,
    #[serde(default = "default_dependency")]
    pub dependency: DependencyType,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FrontendParameter {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Error)]
pub enum FrontendGraphError {
    #[error("frontend graph node `{name}` (id={id}) is missing required attribute `{attr}`")]
    MissingAttribute {
        id: u64,
        name: String,
        attr: &'static str,
    },
    #[error("frontend graph node `{name}` (id={id}) has non-string attribute `{attr}`")]
    NonStringAttribute {
        id: u64,
        name: String,
        attr: &'static str,
    },
    #[error("frontend graph references unknown source node {0}")]
    UnknownSource(u64),
    #[error("frontend graph references unknown destination node {0}")]
    UnknownDestination(u64),
    #[error("frontend graph conversion produced invalid AIR: {0}")]
    InvalidAir(#[from] AirError),
}

const fn default_dependency() -> DependencyType {
    DependencyType::Data
}

impl FrontendGraph {
    /// Convert this frontend graph to the canonical Rust AIR module.
    ///
    /// The conversion preserves node IDs and attribute payloads. A frontend
    /// `AGENT` node with a `profile` expands to `SPAWN_AGENT` + `COMMUNICATE`,
    /// matching the old Python authoring semantics without invoking Python.
    pub fn to_air_module(&self) -> Result<AirModule, FrontendGraphError> {
        let mut builder = AirModuleBuilder::new(&self.name);
        for parameter in &self.parameters {
            builder.param(&parameter.name, &parameter.type_name);
        }
        for (key, value) in &self.metadata {
            builder.meta(key, value.clone());
        }
        let node_ids = self
            .nodes
            .iter()
            .map(|node| node.id)
            .collect::<HashSet<_>>();
        let mut expanded_agent_ids = HashMap::<u64, (u64, u64)>::new();
        let mut next_synthetic_id = next_id_after(&node_ids);

        for node in &self.nodes {
            if node.op == AISOperationType::Agent
                && node.attributes.contains_key(graph_attrs::PROFILE)
            {
                let spawn_id = node.id;
                let communicate_id = next_synthetic_id;
                next_synthetic_id += 1;
                expanded_agent_ids.insert(node.id, (spawn_id, communicate_id));

                let agent_name = agent_name(node)?;
                let mut spawn_attrs = HashMap::new();
                spawn_attrs.insert(
                    graph_attrs::AGENT_NAME.to_string(),
                    Value::String(agent_name.clone()),
                );
                copy_string_attr(node, &mut spawn_attrs, graph_attrs::PROFILE)?;
                copy_string_attr(node, &mut spawn_attrs, graph_attrs::CWD)?;
                copy_string_attr(node, &mut spawn_attrs, graph_attrs::MODEL)?;

                builder.node_with_id(
                    spawn_id,
                    format!("{}_spawn", node.name),
                    AISOperationType::SpawnAgent,
                    spawn_attrs,
                );

                let mut communicate_attrs = HashMap::new();
                communicate_attrs.insert(
                    graph_attrs::RECIPIENT.to_string(),
                    Value::String(agent_name),
                );
                if let Some(message) = template(node) {
                    communicate_attrs
                        .insert(graph_attrs::MESSAGE.to_string(), Value::String(message));
                }
                communicate_attrs.insert(
                    graph_attrs::LLM_OPERATION.to_string(),
                    Value::String(AISOperationType::Ask.to_string()),
                );
                copy_attr(node, &mut communicate_attrs, graph_attrs::INPUT_NAMES);
                copy_attr(node, &mut communicate_attrs, graph_attrs::RETRY_MAX);
                copy_attr(node, &mut communicate_attrs, graph_attrs::RETRY_BACKOFF_MS);
                copy_attr(node, &mut communicate_attrs, graph_attrs::CONTINUE_ON_ERROR);

                builder.node_with_id(
                    communicate_id,
                    node.name.clone(),
                    AISOperationType::Communicate,
                    communicate_attrs,
                );
                continue;
            }

            builder.node_with_id(node.id, node.name.clone(), node.op, node.attributes.clone());
        }

        let mut edges = Vec::with_capacity(self.edges.len() + expanded_agent_ids.len());
        for edge in &self.edges {
            if !node_ids.contains(&edge.from) {
                return Err(FrontendGraphError::UnknownSource(edge.from));
            }
            if !node_ids.contains(&edge.to) {
                return Err(FrontendGraphError::UnknownDestination(edge.to));
            }
            let from = expanded_agent_ids
                .get(&edge.from)
                .map_or(edge.from, |(_, communicate_id)| *communicate_id);
            let to = expanded_agent_ids
                .get(&edge.to)
                .map_or(edge.to, |(_, communicate_id)| *communicate_id);
            edges.push(AirEdge {
                from,
                to,
                dependency: edge.dependency.clone(),
            });
        }
        for (spawn_id, communicate_id) in expanded_agent_ids.values().copied() {
            edges.push(AirEdge {
                from: spawn_id,
                to: communicate_id,
                dependency: DependencyType::Data,
            });
        }
        for edge in edges {
            builder.edge(edge.from, edge.to, edge.dependency);
        }

        let module = builder.build();
        module.validate()?;
        Ok(module)
    }
}

impl TryFrom<FrontendGraph> for AirModule {
    type Error = FrontendGraphError;

    fn try_from(value: FrontendGraph) -> Result<Self, Self::Error> {
        value.to_air_module()
    }
}

impl TryFrom<&FrontendGraph> for AirModule {
    type Error = FrontendGraphError;

    fn try_from(value: &FrontendGraph) -> Result<Self, Self::Error> {
        value.to_air_module()
    }
}

fn next_id_after(node_ids: &HashSet<u64>) -> u64 {
    node_ids
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .saturating_add(1)
}

fn agent_name(node: &FrontendNode) -> Result<String, FrontendGraphError> {
    if let Some(value) = node.attributes.get(graph_attrs::AGENT_NAME) {
        return value.as_str().map(str::to_string).ok_or_else(|| {
            FrontendGraphError::NonStringAttribute {
                id: node.id,
                name: node.name.clone(),
                attr: graph_attrs::AGENT_NAME,
            }
        });
    }
    if !node.name.trim().is_empty() {
        return Ok(node.name.clone());
    }
    Err(FrontendGraphError::MissingAttribute {
        id: node.id,
        name: node.name.clone(),
        attr: graph_attrs::AGENT_NAME,
    })
}

fn template(node: &FrontendNode) -> Option<String> {
    node.attributes
        .get(graph_attrs::MESSAGE)
        .or_else(|| node.attributes.get(graph_attrs::TEMPLATE_STR))
        .or_else(|| node.attributes.get(graph_attrs::PROMPT))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn copy_string_attr(
    node: &FrontendNode,
    target: &mut HashMap<String, Value>,
    attr: &'static str,
) -> Result<(), FrontendGraphError> {
    let Some(value) = node.attributes.get(attr) else {
        return Ok(());
    };
    if value.as_str().is_none() {
        return Err(FrontendGraphError::NonStringAttribute {
            id: node.id,
            name: node.name.clone(),
            attr,
        });
    }
    target.insert(attr.to_string(), value.clone());
    Ok(())
}

fn copy_attr(node: &FrontendNode, target: &mut HashMap<String, Value>, attr: &'static str) {
    if let Some(value) = node.attributes.get(attr) {
        target.insert(attr.to_string(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_builder::PromptInputRole;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::types::Number;

    fn node(
        id: u64,
        name: &str,
        op: AISOperationType,
        attributes: HashMap<String, Value>,
    ) -> FrontendNode {
        FrontendNode {
            id,
            name: name.to_string(),
            op,
            attributes,
        }
    }

    fn string_attr(key: &str, value: &str) -> (String, Value) {
        (key.to_string(), Value::String(value.to_string()))
    }

    #[test]
    fn converts_frontend_graph_to_air_module() {
        let graph = FrontendGraph {
            name: "hello".to_string(),
            nodes: vec![
                node(
                    1,
                    "ask",
                    AISOperationType::Ask,
                    HashMap::from([
                        string_attr(graph_attrs::TEMPLATE_STR, "Say hi to {name}"),
                        (
                            graph_attrs::TOKEN_BUDGET.to_string(),
                            Value::Number(Number::Integer(128)),
                        ),
                    ]),
                ),
                node(2, "out", AISOperationType::Return, HashMap::new()),
            ],
            edges: vec![FrontendEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: vec![FrontendParameter {
                name: "name".to_string(),
                type_name: "str".to_string(),
            }],
            metadata: HashMap::new(),
        };

        let module = graph.to_air_module().expect("frontend graph converts");
        assert_eq!(module.nodes.len(), 2);
        assert_eq!(module.edges.len(), 1);
        let air = module.to_air().expect("canonical printer emits");
        assert!(air.contains("func.func @hello"));
        assert!(air.contains("ais.ask"));
        assert!(air.contains("token_budget"));
    }

    #[test]
    fn converts_inv_cap_with_canonical_params_json_string() {
        let graph = FrontendGraph {
            name: "tooling".to_string(),
            nodes: vec![node(
                1,
                "fetch",
                AISOperationType::InvCap,
                HashMap::from([
                    string_attr(graph_attrs::CAPABILITY, "http.get"),
                    string_attr(
                        graph_attrs::PARAMS_JSON,
                        r#"{"url":"https://example.test"}"#,
                    ),
                ]),
            )],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let module = graph.to_air_module().expect("canonical inv_cap converts");
        let air = module.to_air().expect("canonical printer emits");
        assert!(air.contains("ais.inv_cap"));
        assert!(air.contains("http.get"));
        assert!(air.contains("https://example.test"));
    }

    #[test]
    fn rejects_object_params_json_at_compiler_boundary() {
        let graph = FrontendGraph {
            name: "tooling".to_string(),
            nodes: vec![node(
                1,
                "fetch",
                AISOperationType::InvCap,
                HashMap::from([
                    string_attr(graph_attrs::CAPABILITY, "http.get"),
                    (
                        graph_attrs::PARAMS_JSON.to_string(),
                        Value::Object(HashMap::from([(
                            "url".to_string(),
                            Value::String("https://example.test".to_string()),
                        )])),
                    ),
                ]),
            )],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let err = graph
            .to_air_module()
            .expect_err("params_json object is not canonical AIS graph IR");
        assert!(err.to_string().contains("params_json"));
        assert!(err.to_string().contains("must be a string"));
    }

    #[test]
    fn expands_profiled_agent_to_spawn_and_communicate() {
        let graph = FrontendGraph {
            name: "agent_flow".to_string(),
            nodes: vec![node(
                1,
                "coder",
                AISOperationType::Agent,
                HashMap::from([
                    string_attr(graph_attrs::PROFILE, "codex"),
                    string_attr(graph_attrs::PROMPT, "Fix it"),
                    string_attr(graph_attrs::CWD, "/tmp/work"),
                ]),
            )],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let module = graph.to_air_module().expect("profiled agent expands");
        assert_eq!(module.nodes.len(), 2);
        assert!(
            module
                .nodes
                .iter()
                .any(|node| node.op == AISOperationType::SpawnAgent)
        );
        assert!(
            module
                .nodes
                .iter()
                .any(|node| node.op == AISOperationType::Communicate)
        );
        assert_eq!(module.edges.len(), 1);
        assert_eq!(module.edges[0].dependency, DependencyType::Data);
        let air = module.to_air().expect("expanded graph prints");
        assert!(air.contains("ais.spawn_agent"));
        assert!(air.contains("ais.communicate"));
    }

    #[test]
    fn emits_flow_call_with_agent_and_flow_positionals() {
        let graph = FrontendGraph {
            name: "host_loop".to_string(),
            nodes: vec![
                node(
                    1,
                    "run_turn",
                    AISOperationType::FlowCall,
                    HashMap::from([
                        string_attr(graph_attrs::AGENT_NAME, "conversation"),
                        string_attr(graph_attrs::FLOW_NAME, "turn"),
                    ]),
                ),
                node(2, "return_turn", AISOperationType::Return, HashMap::new()),
            ],
            edges: vec![FrontendEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let module = graph.to_air_module().expect("flow call graph converts");
        let air = module.to_air().expect("canonical printer emits");
        assert!(air.contains("ais.flow_call \"conversation\" \"turn\""));
        assert!(air.contains("ais.return %n1"));
    }

    #[test]
    fn rejects_unknown_edge_endpoint() {
        let graph = FrontendGraph {
            name: "bad".to_string(),
            nodes: vec![node(
                1,
                "ask",
                AISOperationType::Ask,
                HashMap::from([string_attr(graph_attrs::TEMPLATE_STR, "hi")]),
            )],
            edges: vec![FrontendEdge {
                from: 1,
                to: 99,
                dependency: DependencyType::Data,
            }],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let err = graph
            .to_air_module()
            .expect_err("unknown edge target rejected");
        assert!(matches!(err, FrontendGraphError::UnknownDestination(99)));
    }

    #[test]
    fn deserializes_op_from_canonical_ais_type() {
        let graph: FrontendGraph = serde_json::from_value(serde_json::json!({
            "name": "typed",
            "nodes": [
                {
                    "id": 1,
                    "name": "ask",
                    "op": "ASK",
                    "attributes": {"template_str": "hi"}
                }
            ],
            "edges": [],
            "parameters": [],
            "metadata": {}
        }))
        .expect("AISOperationType serde accepts canonical op");

        assert_eq!(graph.nodes[0].op, AISOperationType::Ask);

        let err = serde_json::from_value::<FrontendGraph>(serde_json::json!({
            "name": "typed",
            "nodes": [
                {
                    "id": 1,
                    "name": "unsupported_op",
                    "op": "llm",
                    "attributes": {}
                }
            ],
            "edges": [],
            "parameters": [],
            "metadata": {}
        }))
        .expect_err("canvas-only op names do not enter compiler graph IR");
        assert!(err.to_string().contains("unknown variant"));
    }

    #[test]
    fn frontend_node_prompt_bindings_match_direct_air_payloads() {
        let node = node(1, "ask", AISOperationType::Ask, HashMap::new()).with_prompt_inputs([
            PromptInputBinding::new("question", PromptInputRole::User),
            PromptInputBinding::new("policy", PromptInputRole::System),
            PromptInputBinding::new("dependency", PromptInputRole::DependencyOnly),
            PromptInputBinding::new("tool_result", PromptInputRole::ToolContext),
            PromptInputBinding::new("guard", PromptInputRole::Control),
        ]);

        assert_eq!(
            node.attributes.get(graph_attrs::INPUT_NAMES),
            Some(&Value::Array(vec![
                Value::String("question".to_string()),
                Value::String("policy".to_string()),
                Value::String("dependency".to_string()),
                Value::String("tool_result".to_string()),
                Value::String("guard".to_string()),
            ])),
        );
        assert_eq!(
            node.attributes.get(graph_attrs::INPUT_ROLES),
            Some(&Value::Array(vec![
                Value::String("user".to_string()),
                Value::String("system".to_string()),
                Value::String("dependency_only".to_string()),
                Value::String("tool_context".to_string()),
                Value::String("control".to_string()),
            ])),
        );
    }
}
