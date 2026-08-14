//! Golden lowering for generic typed structural containment.
//!
//! A typed graph verifies, lowers to canonical AIR whose semantic operations are
//! selected from the call intents and carry typed operands, and yields a
//! validating digest-bound artifact whose requirements come from the declared
//! graph requirements.

use apxm_program::frontend_graph::PermissionDecision;
use apxm_program::{ExecutableArtifact, FrontendGraph, SourceBundle, frontend_graph_to_air};
use serde_json::{Value, json};

fn generic_graph_value() -> Value {
    json!({
        "schema_version": "apxm.frontend-graph",
        "source_language": "python",
        "program_definitions": [{
            "program_id": "Worker",
            "entrypoint": "run",
            "input_type_ref": "Input",
            "output_type_ref": "Output",
            "has_default_context": true
        }],
        "imported_program_refs": [],
        "declarations": [
            {
                "decl_id": "decl.model.target",
                "decl_kind": "model_binding",
                "input_type_ref": "ModelRequest",
                "output_type_ref": "ModelResponse",
                "target_ref": "model.target"
            },
            {
                "decl_id": "decl.cap.search",
                "decl_kind": "capability_binding",
                "input_type_ref": "SearchArguments",
                "output_type_ref": "SearchResult",
                "target_ref": "cap.search"
            }
        ],
        "functions": [{
            "function_id": "run",
            "parameters": [
                {"value_id": "value.input", "type_ref": "Input", "role": "input"}
            ],
            "result_type_ref": "Output",
            "body_region_id": "region.root",
            "is_entrypoint": true
        }],
        "values": [
            {"value_id": "value.input", "type_ref": "Input", "origin": "parameter", "origin_id": "run"},
            {"value_id": "value.context", "type_ref": "Context", "origin": "context_value", "expression": {"kind": "object", "fields": [{"name": "input", "value": {"kind": "ssa", "value_id": "value.input"}}]}},
            {"value_id": "value.model.out", "type_ref": "ModelResponse", "origin": "call_result", "origin_id": "node.model"},
            {"value_id": "value.cap.out", "type_ref": "SearchResult", "origin": "call_result", "origin_id": "node.capability"}
        ],
        "blocks": [],
        "regions": [
            {"region_id": "region.root", "region_role": "function_body", "execution_order": 0},
            {
                "region_id": "loop.main",
                "region_role": "loop_body",
                "parent_region_id": "region.root",
                "execution_order": 0
            }
        ],
        "data_edges": [
            {"from_value": "value.input", "to_consumer": "node.model", "consumer_slot": "request"},
            {"from_value": "value.model.out", "to_consumer": "node.capability", "consumer_slot": "arguments"}
        ],
        "call_intents": [
            {
                "node_id": "node.model",
                "intent_kind": "model_invocation",
                "parent_region_id": "loop.main",
                "execution_order": 0,
                "binding_ref": "decl.model.target",
                "operand_values": ["value.input"],
                "result_value": "value.model.out"
            },
            {
                "node_id": "node.capability",
                "intent_kind": "capability_invocation",
                "parent_region_id": "loop.main",
                "execution_order": 1,
                "binding_ref": "decl.cap.search",
                "operand_values": ["value.model.out"],
                "result_value": "value.cap.out"
            }
        ],
        "control_intents": [{
            "node_id": "node.loop",
            "control_kind": "loop",
            "parent_region_id": "region.root",
            "execution_order": 0,
            "body_region_ids": ["loop.main"]
        }],
        "context_flow": [{
            "from_node": "node.model",
            "to_node": "node.capability",
            "context_type_ref": "Context",
            "value_id": "value.context"
        }],
        "hook_bindings": [],
        "capability_requirements": [{"capability_ref": "cap.search"}],
        "model_requirements": [{"model_target_ref": "model.target"}],
        "source_map": {
            "schema_version": "apxm.source-map",
            "source_language": "python",
            "node_spans": [],
            "region_annotations": [
                {"region_id": "loop.main", "annotation": "structural_loop"}
            ]
        }
    })
}

fn generic_graph() -> FrontendGraph {
    serde_json::from_value(generic_graph_value()).expect("generic graph")
}

#[test]
fn generic_graph_lowers_and_carries_source_map() {
    let graph = generic_graph();
    let air = frontend_graph_to_air(&graph).expect("lower graph");
    assert_eq!(air.source_map, graph.source_map);
    assert_eq!(air.context_flow.len(), graph.context_flow.len());

    // The typed model invocation intent selects model.call and the typed
    // capability invocation intent selects capability.invoke, each carrying
    // typed SSA operands whose slot names come from the AIS operand catalogue.
    let model = air
        .semantic_operations
        .iter()
        .find(|op| op.node_id == "node.model")
        .expect("model.call lowered");
    assert_eq!(model.op.wire(), "model.call");
    assert!(model.operands.iter().any(|o| o.slot == "request"));
    assert!(model.operands.iter().any(|operand| {
        operand.slot == "model_ref"
            && operand.type_ref == "ModelTargetRef"
            && operand.value_id == "model.target"
    }));
    assert_eq!(
        model.result.as_ref().map(|r| r.value_id.as_str()),
        Some("value.model.out")
    );

    let cap = air
        .semantic_operations
        .iter()
        .find(|op| op.node_id == "node.capability")
        .expect("capability.invoke lowered");
    assert_eq!(cap.op.wire(), "capability.invoke");
    assert!(cap.operands.iter().any(|o| o.slot == "arguments"));
}

#[test]
fn frontend_graph_rejects_an_air_model_ref_before_lowering() {
    let mut value = generic_graph_value();
    value["model_requirements"][0]["model_ref"] = json!("model.target");
    let error = serde_json::from_value::<FrontendGraph>(value)
        .expect_err("FrontendGraph has no AIR model_ref field");
    assert!(error.to_string().contains("model_ref"));
}

#[test]
fn generic_graph_produces_validating_artifact() {
    let graph = generic_graph();
    let artifact = ExecutableArtifact::from_frontend_graph(&graph).expect("artifact");
    assert!(artifact.validate().is_accepted());
    assert_eq!(artifact.entrypoints[0].program_id, "Worker");
    assert_eq!(artifact.artifact_semantic_requirements.len(), 2);
}

/// The same capability declared twice — once as a model-visible Tool carrying an
/// authored permission decision, once as a plain Capability carrying none.
/// Keying the collection by `capability_ref` would drop one of the two.
fn duplicate_declaration_graph_value(permission: Option<Value>) -> Value {
    let mut tool = json!({"capability_ref": "cap.search", "tool_schema_present": true});
    if let Some(permission) = permission {
        tool["requested_permission"] = permission;
    }
    let mut value = generic_graph_value();
    value["capability_requirements"] =
        json!([tool, {"capability_ref": "cap.search", "tool_schema_present": false}]);
    value
}

#[test]
fn an_authored_permission_survives_graph_air_and_artifact() {
    let requested = json!({"decision": "ask", "reason": "Reads whatever the model asks for."});
    let authored = duplicate_declaration_graph_value(Some(requested.clone()));
    let graph: FrontendGraph = serde_json::from_value(authored).expect("permissioned graph");

    // Boundary 1 — decode keeps both declarations of the shared ref and the
    // authored decision, with its reason, attached to the one that requested it.
    assert_eq!(graph.capability_requirements.len(), 2);
    assert_eq!(
        graph.capability_requirements[0].requested_permission,
        Some(PermissionDecision::ask(
            "Reads whatever the model asks for."
        ))
    );
    assert_eq!(graph.capability_requirements[1].requested_permission, None);
    assert!(graph.verify().is_accepted());

    // Boundary 2 — re-encoding emits the authored decision and omits the
    // absent one, so a graph that round-trips through JSON is unchanged.
    let encoded = serde_json::to_value(&graph).expect("re-encode graph");
    assert_eq!(
        encoded["capability_requirements"][0]["requested_permission"],
        requested
    );
    assert_eq!(
        encoded["capability_requirements"][1].get("requested_permission"),
        None
    );
    let decoded: FrontendGraph = serde_json::from_value(encoded).expect("decode re-encoded graph");
    assert_eq!(
        decoded.capability_requirements,
        graph.capability_requirements
    );

    // Boundary 3 — lowering to AIR neither consumes nor drops the requirement,
    // and AIR states it in the form a composition root handed bare AIR resolves
    // from. Two declarations of one reference collapse to the tightest request:
    // an author who wrote `ask` anywhere did not also authorize the reference
    // unqualified elsewhere.
    let air = frontend_graph_to_air(&graph).expect("lower permissioned graph");
    assert!(air.verify().is_accepted());
    assert_eq!(
        air.capability_permission_requests.get("cap.search"),
        Some(&PermissionDecision::ask(
            "Reads whatever the model asks for."
        ))
    );
    assert_eq!(
        serde_json::to_value(&air).expect("encode AIR")["capability_permission_requests"],
        json!({"cap.search": requested})
    );

    // Boundary 4 — the artifact's source bundle carries the permission verbatim
    // and both artifact digests are bound to it, so no consumer downstream of
    // the artifact can read a permission the author did not write.
    let bundle = SourceBundle::from_graph(&graph);
    assert_eq!(
        bundle.capability_requirements,
        graph.capability_requirements
    );
    let artifact = ExecutableArtifact::from_frontend_graph(&graph).expect("artifact");
    assert!(artifact.validate().is_accepted());
    assert_eq!(
        artifact.source_bundle_digest,
        bundle.digest().expect("bundle digest")
    );

    let unpermissioned: FrontendGraph =
        serde_json::from_value(duplicate_declaration_graph_value(None)).expect("control graph");
    let control = ExecutableArtifact::from_frontend_graph(&unpermissioned).expect("control");
    assert_ne!(
        artifact.source_bundle_digest, control.source_bundle_digest,
        "the authored permission must change the digest that binds the source bundle"
    );
    assert_ne!(artifact.artifact_digest, control.artifact_digest);

    // Boundary 5 — the reason is bound too, so an override that quietly
    // rewrote why a program asked would change the artifact digest.
    let reworded: FrontendGraph = serde_json::from_value(duplicate_declaration_graph_value(Some(
        json!({"decision": "ask", "reason": "Reads anything at all."}),
    )))
    .expect("reworded graph");
    let reworded = ExecutableArtifact::from_frontend_graph(&reworded).expect("reworded artifact");
    assert_ne!(
        artifact.source_bundle_digest, reworded.source_bundle_digest,
        "the reason a decision gives is bound to the source bundle"
    );
}

/// A program that authors no permission states no request, rather than a
/// request for nothing. AIR omits the field entirely, which is what keeps every
/// artifact and pinned AIR fixture compiled before requests were carried
/// byte-identical — and what lets a composition root read the absence as an
/// unqualified request instead of a narrowed one.
#[test]
fn an_unauthored_permission_leaves_no_request_in_air() {
    let graph: FrontendGraph =
        serde_json::from_value(duplicate_declaration_graph_value(None)).expect("control graph");
    let air = frontend_graph_to_air(&graph).expect("lower control graph");
    assert!(air.capability_permission_requests.is_empty());
    assert_eq!(
        serde_json::to_value(&air)
            .expect("encode AIR")
            .get("capability_permission_requests"),
        None
    );
}

/// A request may only narrow an effect the module actually performs. One naming
/// a Capability no `capability.invoke` names would sit in the code layer of
/// every resolution forever, narrowing nothing and reading like authority.
#[test]
fn an_air_permission_request_must_name_an_invoked_capability() {
    let graph: FrontendGraph =
        serde_json::from_value(duplicate_declaration_graph_value(None)).expect("control graph");
    let mut air = frontend_graph_to_air(&graph).expect("lower control graph");
    assert!(air.verify().is_accepted());
    air.capability_permission_requests.insert(
        "cap.never.invoked".to_string(),
        PermissionDecision::deny("narrows an effect that does not exist"),
    );
    assert!(!air.verify().is_accepted());
}

/// A decision that structurally carries a reason and says nothing would put an
/// empty explanation into the digest-bound source bundle.
#[test]
fn an_authored_permission_reason_must_say_something() {
    let graph: FrontendGraph = serde_json::from_value(duplicate_declaration_graph_value(Some(
        json!({"decision": "deny", "reason": "   "}),
    )))
    .expect("decodable graph");
    assert!(!graph.verify().is_accepted());
}

/// One Hook, captured whole: the binding places the body around its target and
/// the body's own operations are ordinary AIR inside the Hook's region. Before
/// bodies were captured a Hook lowered to a childless region and the artifact
/// re-attached the binding from the graph, so the artifact's Hooks and its AIR
/// were two copies nothing compared.
#[test]
fn a_captured_hook_body_is_air_inside_the_region_the_binding_names() {
    let mut value = generic_graph_value();
    value["regions"]
        .as_array_mut()
        .expect("regions array")
        .push(json!({
            "region_id": "hook.before.model.body",
            "region_role": "hook_body",
            "parent_region_id": "loop.main",
            "execution_order": 2
        }));
    value["values"]
        .as_array_mut()
        .expect("values array")
        .push(json!({
            "value_id": "value.hook.budget",
            "type_ref": "SearchResult",
            "origin": "call_result",
            "origin_id": "node.hook.count"
        }));
    value["values"]
        .as_array_mut()
        .expect("values array")
        .push(json!({
            "value_id": "value.hook.arguments",
            "type_ref": "SearchArguments",
            "origin": "literal",
            "expression": {"kind": "string", "value": "count"}
        }));
    value["data_edges"]
        .as_array_mut()
        .expect("data edge array")
        .push(json!({
            "from_value": "value.hook.arguments",
            "to_consumer": "node.hook.count",
            "consumer_slot": "arguments"
        }));
    value["call_intents"]
        .as_array_mut()
        .expect("call intent array")
        .push(json!({
            "node_id": "node.hook.count",
            "intent_kind": "capability_invocation",
            "parent_region_id": "hook.before.model.body",
            "execution_order": 0,
            "binding_ref": "decl.cap.search",
            "operand_values": ["value.hook.arguments"],
            "result_value": "value.hook.budget"
        }));
    value["hook_bindings"] = json!([{
        "hook_id": "hook.before.model",
        "scope": "model",
        "phase": "before",
        "target_selector": "node.model",
        "declaration_order": 0,
        "handler_ref": "hooks.before_model",
        "handler_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "input_type_ref": "ModelContext",
        "output_type_ref": "Unit",
        "return_mode": "observe",
        "body_region_id": "hook.before.model.body"
    }]);
    let graph: FrontendGraph = serde_json::from_value(value).expect("hook graph");
    let air = frontend_graph_to_air(&graph).expect("lower hook graph");
    let body = air
        .structural_ir
        .iter()
        .find(|node| node.region_id == "hook.before.model.body")
        .expect("captured Hook body region");
    let model = air
        .semantic_operations
        .iter()
        .find(|operation| operation.node_id == "node.model")
        .expect("model operation");
    let captured = air
        .semantic_operations
        .iter()
        .find(|operation| operation.node_id == "node.hook.count")
        .expect("captured Hook operation");

    assert_eq!(body.parent_region_id.as_deref(), Some("loop.main"));
    assert!(body.execution_order < model.execution_order);
    assert_eq!(captured.parent_region_id, "hook.before.model.body");
    assert_eq!(
        captured.op,
        apxm_program::air::SemanticOpKind::CapabilityInvoke
    );
    assert_eq!(
        body.hook.as_ref().map(|hook| hook.hook_id.as_str()),
        Some("hook.before.model")
    );
    assert_eq!(body.hook.as_ref(), graph.hook_bindings.first());
    assert!(air.verify().is_accepted());

    let artifact = ExecutableArtifact::from_frontend_graph(&graph).expect("hook artifact");
    assert!(artifact.validate().is_accepted());

    // The declared binding and the binding the AIR carries are one fact: change
    // either alone and the artifact stops validating.
    let mut drifted = artifact.clone();
    drifted.hook_bindings[0].return_mode =
        apxm_program::frontend_graph::HookReturnMode::ReplaceResult;
    assert!(!drifted.validate().is_accepted());
}

/// A Hook body no binding claims would be structure the artifact executes and
/// never described.
#[test]
fn an_unclaimed_captured_hook_body_is_refused() {
    let mut value = generic_graph_value();
    value["regions"]
        .as_array_mut()
        .expect("regions array")
        .push(json!({
            "region_id": "hook.orphan.body",
            "region_role": "hook_body",
            "parent_region_id": "loop.main",
            "execution_order": 1
        }));
    let graph: FrontendGraph = serde_json::from_value(value).expect("orphan hook body graph");
    assert!(!graph.verify().is_accepted());
}

/// An observing Hook that names an assigned Context value is claiming to mutate
/// nothing while pointing at the mutation.
#[test]
fn an_observing_hook_cannot_name_an_assigned_context_value() {
    let mut value = generic_graph_value();
    value["regions"]
        .as_array_mut()
        .expect("regions array")
        .push(json!({
            "region_id": "hook.before.model.body",
            "region_role": "hook_body",
            "parent_region_id": "loop.main",
            "execution_order": 1
        }));
    value["hook_bindings"] = json!([{
        "hook_id": "hook.before.model",
        "scope": "model",
        "phase": "before",
        "target_selector": "node.model",
        "declaration_order": 0,
        "handler_ref": "hooks.before_model",
        "handler_digest": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "input_type_ref": "ModelContext",
        "output_type_ref": "Unit",
        "return_mode": "observe",
        "body_region_id": "hook.before.model.body",
        "assigned_context_value_id": "value.context"
    }]);
    let graph: FrontendGraph = serde_json::from_value(value).expect("observing hook graph");
    assert!(!graph.verify().is_accepted());
}

#[test]
fn yield_resume_value_is_owned_once_by_the_exact_yield_node() {
    let mut value = generic_graph_value();
    value["values"]
        .as_array_mut()
        .expect("values array")
        .push(json!({
            "value_id": "value.resume",
            "type_ref": "Input",
            "origin": "resume_input",
            "origin_id": "node.yield"
        }));
    value["blocks"] = json!([{
        "block_id": "block.loop",
        "region_id": "loop.main",
        "block_arguments": ["value.resume"],
        "execution_order": 0
    }]);
    value["control_intents"]
        .as_array_mut()
        .expect("control intent array")
        .push(json!({
            "node_id": "node.yield",
            "control_kind": "yield",
            "parent_region_id": "loop.main",
            "execution_order": 2,
            "result_value": "value.resume"
        }));

    let graph: FrontendGraph = serde_json::from_value(value).expect("yield graph");
    let air = frontend_graph_to_air(&graph).expect("yield graph lowers without duplicate SSA");
    let owners: Vec<&str> = air
        .structural_ir
        .iter()
        .filter(|node| {
            node.block_arguments
                .iter()
                .any(|argument| argument.value_id == "value.resume")
        })
        .map(|node| node.region_id.as_str())
        .collect();
    assert_eq!(owners, ["node.yield"]);
    assert!(air.verify().is_accepted());
}
