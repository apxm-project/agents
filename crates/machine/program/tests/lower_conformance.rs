//! Golden lowering: FrontendGraph fixtures lower to checked-in AIR vectors.

use std::path::PathBuf;

use apxm_program::{frontend_graph_to_air, ExecutableArtifact, FrontendGraph};
use serde_json::Value;

fn agents_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
}

fn load_json(relative: &str) -> Value {
    let path = agents_root().join(relative);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn load_graph_fixture(name: &str) -> FrontendGraph {
    let doc = load_json(&format!("crates/compiler/frontend/native/parity/{name}.json"));
    serde_json::from_value(doc).expect("decode frontend graph fixture")
}

fn air_json(graph: &FrontendGraph) -> Value {
    let air = frontend_graph_to_air(graph).expect("lower fixture");
    serde_json::to_value(air).expect("serialize air")
}

#[test]
fn specialist_graph_lowers_to_golden_air() {
    let graph = load_graph_fixture("frontend-graph.example");
    let golden = load_json("crates/compiler/frontend/native/parity/air.expected.json");
    assert_eq!(air_json(&graph), golden);
}

#[test]
fn gao_graph_lowers_to_golden_air() {
    let graph = gao_graph();
    let golden = load_json("crates/compiler/frontend/native/parity/air.gao.expected.json");
    assert_eq!(air_json(&graph), golden);
}

#[test]
fn external_agent_graph_lowers_to_golden_air() {
    let graph = external_agent_graph();
    let golden = load_json("crates/compiler/frontend/native/parity/air.external-agent.expected.json");
    assert_eq!(air_json(&graph), golden);
}

#[test]
fn frontend_graph_produces_validating_artifact() {
    let graph = load_graph_fixture("frontend-graph.example");
    let artifact = ExecutableArtifact::from_frontend_graph(&graph).expect("artifact");
    assert!(artifact.validate().is_accepted());
    assert_eq!(artifact.entrypoints[0].program_id, "Specialist");
    assert_eq!(artifact.artifact_semantic_requirements.len(), 2);
}

#[test]
fn rejects_orphan_context_flow_edge() {
    let mut value: Value = serde_json::to_value(load_graph_fixture("frontend-graph.example"))
        .expect("graph value");
    value["context_flow"] = serde_json::json!([{
        "from_node": "node.missing",
        "to_node": "node.cap.1",
        "context_type_ref": "SpecialistContext"
    }]);
    let graph: FrontendGraph = serde_json::from_value(value).expect("graph");
    let err = frontend_graph_to_air(&graph).expect_err("orphan context edge");
    assert!(!err.is_accepted());
}

fn gao_graph() -> FrontendGraph {
    serde_json::from_value(serde_json::json!({
        "schema_version": "apxm.frontend-graph.v1",
        "source_language": "python",
        "program_definitions": [{
            "program_id": "Gao",
            "entrypoint": "run",
            "input_type_ref": "GaoInput",
            "output_type_ref": "GaoOutput",
            "context_type_ref": "GaoContext",
            "has_default_context": true
        }],
        "imported_program_refs": [{
            "program_ref": "Specialist",
            "artifact_digest": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            "entrypoint": "run",
            "target_agent_identity_requirement": "specialist-identity"
        }],
        "semantic_operations": [
            { "node_id": "node.turn.model", "op": "model.call", "operands": { "model_target_ref": "model.default" } },
            { "node_id": "node.turn.tool", "op": "capability.invoke", "operands": { "capability_ref": "cap.search" } },
            { "node_id": "node.specialist.new", "op": "program.new" },
            { "node_id": "node.specialist.invoke", "op": "program.invoke" },
            { "node_id": "node.turn.await", "op": "await.event", "operands": { "event_ref": "event.turn.input" } }
        ],
        "structural_regions": [
            { "region_id": "region.loop.turn", "kind": "loop" },
            { "region_id": "region.return", "kind": "return" }
        ],
        "context_flow": [{
            "from_node": "node.turn.model",
            "to_node": "node.turn.tool",
            "context_type_ref": "GaoContext"
        }],
        "hook_bindings": [
            {
                "hook_id": "hook.before.turn",
                "scope": "loop",
                "phase": "before",
                "target_selector": "region.loop.turn",
                "declaration_order": 0,
                "handler_ref": "hooks.before_turn",
                "handler_digest": "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
                "input_type_ref": "GaoContext",
                "output_type_ref": "GaoContext",
                "return_mode": "observe"
            },
            {
                "hook_id": "hook.after.model",
                "scope": "model",
                "phase": "after",
                "target_selector": "node.turn.model",
                "declaration_order": 1,
                "handler_ref": "hooks.after_model",
                "handler_digest": "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
                "input_type_ref": "ModelResult",
                "output_type_ref": "ModelResult",
                "return_mode": "replace_result"
            }
        ],
        "capability_requirements": [{ "capability_ref": "cap.search" }],
        "model_requirements": [{ "model_target_ref": "model.default" }],
        "source_map": {
            "schema_version": "apxm.source-map.v1",
            "source_language": "python",
            "node_spans": [
                {"node_id":"node.turn.model","source_file":"Gao.py","span":{"start_line":1,"start_column":0,"end_line":1,"end_column":1},"semantic_annotation":"model.call"},
                {"node_id":"node.turn.tool","source_file":"Gao.py","span":{"start_line":2,"start_column":0,"end_line":2,"end_column":1},"semantic_annotation":"capability.invoke"},
                {"node_id":"node.specialist.new","source_file":"Gao.py","span":{"start_line":3,"start_column":0,"end_line":3,"end_column":1},"semantic_annotation":"program.new"},
                {"node_id":"node.specialist.invoke","source_file":"Gao.py","span":{"start_line":4,"start_column":0,"end_line":4,"end_column":1},"semantic_annotation":"program.invoke"},
                {"node_id":"node.turn.await","source_file":"Gao.py","span":{"start_line":5,"start_column":0,"end_line":5,"end_column":1},"semantic_annotation":"await.event"}
            ],
            "region_annotations": [
                { "region_id": "region.loop.turn", "annotation": "conversational_loop" }
            ]
        }
    }))
    .expect("gao graph")
}

fn external_agent_graph() -> FrontendGraph {
    serde_json::from_value(serde_json::json!({
        "schema_version": "apxm.frontend-graph.v1",
        "source_language": "python",
        "program_definitions": [{
            "program_id": "Delegator",
            "entrypoint": "run",
            "input_type_ref": "DelegatorInput",
            "output_type_ref": "DelegatorOutput",
            "context_type_ref": "DelegatorContext",
            "has_default_context": true
        }],
        "imported_program_refs": [],
        "semantic_operations": [{
            "node_id": "node.acp.1",
            "op": "capability.invoke",
            "operands": {
                "capability_ref": "external-agent:acp:claude-code",
                "external_agent_session": "session.acp.1"
            }
        }],
        "structural_regions": [{ "region_id": "region.return.1", "kind": "return" }],
        "context_flow": [],
        "hook_bindings": [],
        "capability_requirements": [{ "capability_ref": "external-agent:acp:claude-code" }],
        "model_requirements": [],
        "source_map": {
            "schema_version": "apxm.source-map.v1",
            "source_language": "python",
            "node_spans": [],
            "region_annotations": []
        }
    }))
    .expect("external agent graph")
}
