//! Conformance for the Server-callable source-bundle compile port.
//!
//! Every test here calls the published entry point, `compile_source_bundle`, and
//! nothing else. No test names an interpreter, constructs a process, or reads a
//! harness: the interpreter boundary is the port's, and a caller that had to
//! know about it would be a port that had not been published.

mod common;

use apxm_core::grammar;
use apxm_program::{ExecutableArtifact, frontend_graph::IntentKind};
use apxm_source_port::{
    CompiledSource, Frontend, FrontendDrivers, FrontendRoots, Phase, Severity, SourceBundleRequest,
    SourceDiagnostic, SourceDiagnosticCode, compile_source_bundle, diagnostic_report,
};

use crate::common::{ENTRYPOINT, FRONTENDS, drivers, frontend_present, roots};

// ── The fixture programs ────────────────────────────────────────────────────
//
// Each rejection fixture differs from the accepted program of the same language
// by exactly the construct under test, so a rejection is attributable to that
// construct and not to unrelated damage in the fixture.

const PYTHON_PROGRAM: &str = r#"from apxm_program import Workflow, Model, Tool


class ReviewRequest:
    pass


class Review:
    pass


ReviewModel = Model[ReviewRequest, Review]("@MODEL@")
SearchWeb = Tool[ReviewRequest, Review]("search_web")


@Workflow(input=ReviewRequest, output=Review)
async def Reviewer(agent, request):
@BODY@
"#;

const TYPESCRIPT_PROGRAM: &str = r#"import { Workflow, Model, Tool } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type ReviewRequest = object;
type Review = object;

const ReviewModel = Model<ReviewRequest, Review>("@MODEL@");
const SearchWeb = Tool<ReviewRequest, Review>("search_web");

export const Reviewer = Workflow<ReviewRequest, Review>({
  name: "Reviewer",
  async run(agent, request) {
@BODY@
  },
});
"#;

#[test]
fn event_reference_source_and_payload_contract_are_shared_and_digest_bound() {
    let mut schemas = Vec::new();
    for frontend in FRONTENDS {
        assert!(
            frontend_present(frontend),
            "typed Event qualification requires both installed frontends"
        );
        let source = match frontend {
            Frontend::Typescript => {
                r#"import { Workflow, Event, type EventRef, Capability } from "@apxm/frontend";
type Payload = { reference: string; approved: boolean };
type Input = { event: EventRef<Payload> };
const Submitted = Event<Payload>("event.submitted");
const Read = Capability<Payload, Payload>("read");
export const Reviewer = Workflow<Input, Payload>({ name: "Reviewer", async run(agent, input) {
  const payload = await Submitted.wait(input.event);
  return await Read(payload);
}});
"#
            }
            Frontend::Python => {
                r#"from typing import TypedDict
from apxm_program import Workflow, Event, EventRef, Capability
class Payload(TypedDict):
    reference: str
    approved: bool
class Input(TypedDict):
    event: EventRef[Payload]
Submitted = Event[Payload]("event.submitted")
Read = Capability[Payload, Payload]("read")
@Workflow(input=Input, output=Payload)
async def Reviewer(agent, input):
    payload = await Submitted.wait(input["event"])
    return await Read(payload)
"#
            }
        };
        let compiled = compile(&SourceBundleRequest::new(frontend, ENTRYPOINT, source));
        assert_eq!(compiled.air.event_requirements.len(), 1);
        let requirement = &compiled.air.event_requirements[0];
        assert_eq!(requirement.type_id, "event.submitted");
        assert_eq!(
            requirement.schema_digest,
            requirement.payload_schema.canonical_digest().unwrap()
        );
        schemas.push((
            requirement.payload_schema.clone(),
            requirement.schema_digest.clone(),
        ));
        let wait = compiled
            .air
            .semantic_operations
            .iter()
            .find(|operation| operation.node_id == requirement.node_id)
            .unwrap();
        assert_eq!(wait.operands[0].type_ref, "EventRef");
        assert_ne!(wait.operands[0].value_id, "event.submitted");
        assert!(
            compiled
                .air
                .value_assemblies
                .iter()
                .any(|value| value.value_id == wait.operands[0].value_id)
        );
        assert!(
            compiled.frontend_graph.program_definitions[0]
                .input_schema
                .is_some()
        );
        let array_source = match frontend {
            Frontend::Typescript => source
                .replace(
                    "type Payload = { reference: string; approved: boolean };",
                    "type Payload = string[];",
                )
                .replace("Event<Payload>(", "Event<string[]>("),
            Frontend::Python => source.replace(
                "class Payload(TypedDict):\n    reference: str\n    approved: bool",
                "Payload = list[str]",
            ),
        };
        let array = compile(&SourceBundleRequest::new(
            frontend,
            ENTRYPOINT,
            array_source,
        ));
        assert_eq!(
            serde_json::to_value(&array.air.event_requirements[0].payload_schema).unwrap(),
            serde_json::json!({"type":"array","items":{"type":"string"}})
        );
        for invalid in match frontend {
            Frontend::Typescript => vec![
                source.replace("wait(input.event)", "wait()"),
                source.replace("wait(input.event)", "wait('event.submitted')"),
                source.replace("EventRef<Payload>", "EventRef<string>"),
                source.replace("Event<Payload>(", "Event<unknown>("),
            ],
            Frontend::Python => vec![
                source.replace("wait(input[\"event\"])", "wait()"),
                source.replace("wait(input[\"event\"])", "wait('event.submitted')"),
                source.replace("EventRef[Payload]", "EventRef[str]"),
                source.replace("Event[Payload](", "Event[object]("),
            ],
        } {
            reject(&SourceBundleRequest::new(frontend, ENTRYPOINT, invalid));
        }
    }
    assert_eq!(schemas[0], schemas[1]);
}

#[test]
fn event_references_can_arrive_from_a_typed_capability_result() {
    for frontend in FRONTENDS {
        assert!(frontend_present(frontend));
        let source = match frontend {
            Frontend::Typescript => {
                r#"import { Workflow, Event, type EventRef, Capability } from "@apxm/frontend";
type Payload = { accepted: boolean };
type Input = {};
type Reservation = { event: EventRef<Payload> };
const Reserve = Capability<Input, Reservation>("read");
const Ready = Event<Payload>("event.ready");
export const Reviewer = Workflow<Input, Payload>({ name: "Reviewer", async run(agent, input) {
  const reservation = await Reserve(input);
  return await Ready.wait(reservation.event);
}});
"#
            }
            Frontend::Python => {
                r#"from typing import TypedDict
from apxm_program import Workflow, Event, EventRef, Capability
class Payload(TypedDict):
    accepted: bool
class Reservation(TypedDict):
    event: EventRef[Payload]
Reserve = Capability[dict, Reservation]("read")
Ready = Event[Payload]("event.ready")
@Workflow(input=dict, output=Payload)
async def Reviewer(agent, input):
    reservation = await Reserve(input)
    return await Ready.wait(reservation["event"])
"#
            }
        };
        let compiled = compile(&SourceBundleRequest::new(frontend, ENTRYPOINT, source));
        assert_eq!(compiled.air.event_requirements.len(), 1);
        assert_eq!(compiled.air.event_requirements[0].type_id, "event.ready");
        assert_eq!(
            compiled.frontend_graph.call_intents[0].intent_kind,
            IntentKind::CapabilityInvocation
        );
    }
}

/// Build a request for a selector from a model reference and a body.
fn request(frontend: Frontend, model_reference: &str, body: &str) -> SourceBundleRequest {
    let template = match frontend {
        Frontend::Python => PYTHON_PROGRAM,
        Frontend::Typescript => TYPESCRIPT_PROGRAM,
    };
    SourceBundleRequest::new(
        frontend,
        ENTRYPOINT,
        template
            .replace("@MODEL@", model_reference)
            .replace("@BODY@", body),
    )
}

/// The accepted body for a selector: one Tool invocation feeding one model
/// invocation, so the lowered AIR carries more than a single operation.
fn accepted_body(frontend: Frontend) -> &'static str {
    match frontend {
        Frontend::Python => {
            "    evidence = await SearchWeb(request)\n    return await ReviewModel(evidence)"
        }
        Frontend::Typescript => {
            "    const evidence = await SearchWeb(request);\n\
             \x20   return await ReviewModel(evidence);"
        }
    }
}

/// The accepted program for a selector.
fn accepted(frontend: Frontend) -> SourceBundleRequest {
    request(frontend, "review.model", accepted_body(frontend))
}

/// An authored display name outside the identifier grammar is an ordinary
/// source rejection, even when the rejected graph used it in a node ID.
#[test]
fn invalid_authored_name_reports_source_error_without_invalid_node_identity() {
    if !frontend_present(Frontend::Typescript) {
        return;
    }
    let valid = accepted(Frontend::Typescript);
    compile(&valid);
    let mut invalid = valid;
    invalid.source = invalid
        .source
        .replace("name: \"Reviewer\"", "name: \"Model boundary\"");
    let diagnostics = reject(&invalid);
    assert!(
        diagnostics.iter().any(|item| {
            item.wire_code() == "invalid_identifier"
                && item.phase == Phase::Lowering
                && item.location.is_some()
                && item.node_id.is_none()
        }),
        "the malformed authored name needs a located source diagnostic: {diagnostics:?}"
    );
    let report = diagnostic_report(&diagnostics);
    assert!(
        report
            .items
            .iter()
            .all(|item| { item.node_id.as_deref().is_none_or(grammar::is_identifier) }),
        "a rejected node ID escaped onto the compile protocol: {report:?}"
    );
}

#[test]
fn context_defaults_are_static_closed_values_in_both_frontends() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let source = match frontend {
            Frontend::Typescript => {
                r#"import { Workflow, Context } from "@apxm/frontend";
type State = { count: number };
type Input = {};
const StateContext = Context<State>({ count: 1 });
export const Reviewer = Workflow<Input, unknown, State>({
  name: "Reviewer", context: StateContext,
  async run() { return {}; }
});
"#
            }
            Frontend::Python => {
                r#"from apxm_program import Workflow, Context
@Context
class State:
    count: int = 1
@Workflow(input=dict, output=dict, context=State)
async def Reviewer(agent, incoming):
    return {}
"#
            }
        };
        let compiled = compile(&SourceBundleRequest::new(frontend, ENTRYPOINT, source));
        let definition = &compiled.frontend_graph.program_definitions[0];
        assert!(definition.has_default_context);
        assert_eq!(
            definition
                .default_context
                .as_ref()
                .unwrap()
                .literal_json()
                .unwrap(),
            serde_json::json!({"count": 1})
        );
        let computed = match frontend {
            Frontend::Typescript => source.replace("count: 1", "count: 1 + 1"),
            Frontend::Python => source.replace("count: int = 1", "count: int = 1 + 1"),
        };
        assert!(
            compile_source_bundle(
                &SourceBundleRequest::new(frontend, ENTRYPOINT, computed),
                &roots(),
                &drivers()
            )
            .is_err(),
            "computed {} Context default must reject",
            frontend.wire()
        );

        let no_default = match frontend {
            Frontend::Typescript => {
                source.replace("Context<State>({ count: 1 })", "Context<State>()")
            }
            Frontend::Python => source.replace("count: int = 1", "count: int"),
        };
        let compiled = compile(&SourceBundleRequest::new(frontend, ENTRYPOINT, no_default));
        assert!(!compiled.frontend_graph.program_definitions[0].has_default_context);
        assert!(
            compiled.frontend_graph.program_definitions[0]
                .default_context
                .is_none()
        );

        if frontend == Frontend::Python {
            let inherited = source.replace(
                "@Context\nclass State:",
                "class Base:\n    inherited: int = 2\n@Context\nclass State(Base):",
            );
            assert!(
                compile_source_bundle(
                    &SourceBundleRequest::new(frontend, ENTRYPOINT, inherited),
                    &roots(),
                    &drivers()
                )
                .is_err()
            );
        }
    }
}

#[test]
fn nonboolean_typed_input_truthiness_rejects_before_execution() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let source = match frontend {
            Frontend::Typescript => {
                r#"import { Workflow } from "@apxm/frontend";
type Input = {value: string};
export const Reviewer = Workflow<Input, unknown>({name: "Reviewer", async run(agent, input) {
  if (input.value) { return {ok: true}; }
  return {ok: false};
}});
"#
            }
            Frontend::Python => {
                r#"from typing import TypedDict
from apxm_program import Workflow
class Input(TypedDict):
    value: str
@Workflow(input=Input, output=object)
async def Reviewer(agent, input):
    if input["value"]:
        return {"ok": True}
    return {"ok": False}
"#
            }
        };
        for loop_condition in [false, true] {
            let source = if loop_condition {
                source
                    .replace("if (input.value)", "while (input.value)")
                    .replace("if input[\"value\"]:", "while input[\"value\"]:")
            } else {
                source.to_owned()
            };
            let diagnostics = compile_source_bundle(
                &SourceBundleRequest::new(frontend, ENTRYPOINT, &source),
                &roots(),
                &drivers(),
            )
            .expect_err("known string truthiness must reject at compile time");
            assert!(
                diagnostics.iter().any(|diagnostic| diagnostic
                    .message
                    .contains("truthy predicate requires a boolean typed input")),
                "{diagnostics:?}"
            );
            let boolean = source
                .replace("value: string", "value: boolean")
                .replace("value: str", "value: bool");
            compile(&SourceBundleRequest::new(frontend, ENTRYPOINT, boolean));
            let equality = source
                .replace("(input.value)", "(input.value !== \"\")")
                .replace("input[\"value\"]:", "input[\"value\"] != \"\":");
            compile(&SourceBundleRequest::new(frontend, ENTRYPOINT, equality));
        }
    }
}

/// Both public declarations cross the same compiler boundary, but only Agent
/// carries a checked primary model claim. Model-free orchestration has no
/// model requirement to configure at execution.
#[test]
fn public_program_declarations_have_distinct_verified_authoring() {
    use apxm_program::frontend_graph::ProgramAuthoring;
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let workflow = compile(&accepted(frontend));
        assert_eq!(
            workflow.frontend_graph.program_definitions[0].authoring,
            Some(ProgramAuthoring::Workflow {})
        );

        let mut agent = accepted(frontend);
        agent.source = agent.source.replace("Workflow", "Agent");
        agent.source = match frontend {
            Frontend::Python => agent
                .source
                .replace("@Agent(input=", "@Agent(model=ReviewModel, input="),
            Frontend::Typescript => agent.source.replace(
                "name: \"Reviewer\",",
                "name: \"Reviewer\", model: ReviewModel,",
            ),
        };
        let compiled = compile(&agent);
        let authoring = Some(ProgramAuthoring::Agent {
            primary_model_ref: "review.model".into(),
        });
        assert_eq!(
            compiled.frontend_graph.program_definitions[0].authoring,
            authoring
        );
        let artifact =
            ExecutableArtifact::from_graph_and_air(&compiled.frontend_graph, &compiled.air)
                .unwrap();
        assert_eq!(artifact.entrypoints[0].authoring, authoring);

        let mut missing = agent.clone();
        missing.source = missing
            .source
            .replace("model=ReviewModel, ", "")
            .replace(" model: ReviewModel,", "");
        reject(&missing);
        let mut unused = agent;
        unused.source = unused
            .source
            .replace("return await ReviewModel(evidence)", "return evidence");
        reject(&unused);

        let body = match frontend {
            Frontend::Python => {
                "    mapped = {}\n    if True:\n        return {\"accepted\": True}\n    return None"
            }
            Frontend::Typescript => {
                "    const mapped = {}; if (true) { return {accepted:true}; } return null;"
            }
        };
        let pure = compile(&request(frontend, "unused.model", body));
        assert!(pure.frontend_graph.call_intents.is_empty());
        assert!(pure.frontend_graph.model_requirements.is_empty());
        assert!(pure.air.semantic_operations.is_empty());
        let false_body = body
            .replace("if True", "if False")
            .replace("if (true)", "if (false)");
        compile(&request(frontend, "unused.model", &false_body));
    }
}

/// The accepted body with its returned call replaced, keeping the preceding Tool
/// invocation intact so only the replaced construct differs.
fn body_with_return(frontend: Frontend, returned: &str) -> String {
    match frontend {
        Frontend::Python => format!(
            "    evidence = await SearchWeb(request)\n    return await {returned}(evidence)"
        ),
        Frontend::Typescript => format!(
            "    const evidence = await SearchWeb(request);\n\
             \x20   return await {returned}(evidence);"
        ),
    }
}

/// Compile an accepted request, requiring success.
fn compile(request: &SourceBundleRequest) -> CompiledSource {
    compile_source_bundle(request, &roots(), &drivers()).unwrap_or_else(|diagnostics| {
        panic!(
            "expected {} source to compile; rejected with {:?}",
            request.frontend.wire(),
            diagnostics
        )
    })
}

/// Compile a request that must be rejected, requiring diagnostics and no graph.
fn reject(request: &SourceBundleRequest) -> Vec<SourceDiagnostic> {
    match compile_source_bundle(request, &roots(), &drivers()) {
        Ok(compiled) => panic!(
            "expected {} source to be rejected; it produced a graph with {} call intents \
             and AIR with {} semantic operations",
            request.frontend.wire(),
            compiled.frontend_graph.call_intents.len(),
            compiled.air.semantic_operations.len()
        ),
        Err(diagnostics) => {
            assert!(
                !diagnostics.is_empty(),
                "a rejection carries at least one diagnostic; an empty list is an \
                 empty-but-valid rejection"
            );
            diagnostics
        }
    }
}

/// Assert one rejection class for one selector: the port rejects with the
/// expected closed code and returns nothing else.
fn assert_rejects(
    frontend: Frontend,
    class: &str,
    request: &SourceBundleRequest,
    expected: SourceDiagnosticCode,
) {
    assert_rejection(frontend, class, &reject(request), expected);
}

/// The same, plus the rejection has to *say* what it rejected.
///
/// `SourceRejected` is the umbrella code for every authoring rejection — an
/// unbound name carries it too — so a test named after one rejection proves
/// nothing about that rule until it holds the message to it.
fn assert_rejects_naming(
    frontend: Frontend,
    class: &str,
    request: &SourceBundleRequest,
    expected: SourceDiagnosticCode,
    naming: &str,
) {
    let diagnostics = reject(request);
    assert_rejection(frontend, class, &diagnostics, expected);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(naming)),
        "{} {class} rejects saying {naming:?}; got {diagnostics:?}",
        frontend.wire()
    );
}

fn assert_rejection(
    frontend: Frontend,
    class: &str,
    diagnostics: &[SourceDiagnostic],
    expected: SourceDiagnosticCode,
) {
    assert!(
        diagnostics.iter().any(|d| d.code == expected),
        "{} {class} rejects with {expected}; got {diagnostics:?}",
        frontend.wire()
    );
    assert_reason_was_decoded(frontend, diagnostics);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.trim().is_empty()),
        "{} {class} explains why it rejected; got {diagnostics:?}",
        frontend.wire()
    );
}

// ── The accepted path ───────────────────────────────────────────────────────

#[test]
fn accepted_source_compiles_to_a_graph_air_and_a_source_map() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let compiled = compile(&accepted(frontend));

        assert_eq!(
            compiled.frontend_graph.source_language,
            frontend.source_language(),
            "{} capture records the language it authored",
            frontend.wire()
        );

        // The authored body is one Tool invocation feeding one model
        // invocation; both survive capture as typed intents.
        let intents: Vec<IntentKind> = compiled
            .frontend_graph
            .call_intents
            .iter()
            .map(|intent| intent.intent_kind)
            .collect();
        assert_eq!(
            intents,
            vec![IntentKind::ToolInvocation, IntentKind::ModelInvocation],
            "{} capture records the authored calls in order",
            frontend.wire()
        );

        // AIR exists, and its source map is the one the port returns.
        assert!(
            !compiled.air.semantic_operations.is_empty(),
            "{} lowering produces semantic operations",
            frontend.wire()
        );
        assert!(
            compiled.air.verify().is_accepted(),
            "{} lowering produces AIR that verifies",
            frontend.wire()
        );
        assert_eq!(
            compiled.source_map,
            compiled.air.source_map,
            "{} port returns the source map its AIR carries",
            frontend.wire()
        );
        assert_eq!(
            compiled.source_map.source_language,
            frontend.source_language(),
            "{} source map records the authored language",
            frontend.wire()
        );
        assert!(
            !compiled.source_map.node_spans.is_empty(),
            "{} source map spans the authored nodes",
            frontend.wire()
        );
        assert!(
            !compiled.source_map.region_spans.is_empty(),
            "{} source map carries structural region lineage",
            frontend.wire()
        );
        assert!(
            !compiled.source_map.edge_spans.is_empty(),
            "{} source map carries typed edge lineage",
            frontend.wire()
        );
        assert!(
            compiled.execution_lineage_ref.starts_with("sha256:"),
            "{} compile result carries an opaque execution lineage reference",
            frontend.wire()
        );
    }
}

/// Both authoring frontends are equivalent, so the same authored program
/// compiles through this one port to the same canonical semantics whichever
/// language recorded it: the same AIS operations in the same order, filling the
/// same operand slots, over the same structural shape. Value identifiers are
/// language-local naming and are not part of that equivalence.
#[test]
fn both_frontends_lower_the_same_program_to_equivalent_air() {
    if !FRONTENDS.iter().copied().all(frontend_present) {
        return;
    }

    /// The canonical semantics of one lowered module, without language-local
    /// value naming or source spans.
    fn shape(compiled: &CompiledSource) -> (Vec<(String, Vec<String>)>, Vec<String>) {
        let semantic = compiled
            .air
            .semantic_operations
            .iter()
            .map(|op| {
                let mut slots: Vec<String> = op
                    .operands
                    .iter()
                    .map(|operand| operand.slot.clone())
                    .collect();
                slots.sort();
                (op.op.wire().to_string(), slots)
            })
            .collect();
        let structural = compiled
            .air
            .structural_ir
            .iter()
            .map(|node| node.kind.wire().to_string())
            .collect();
        (semantic, structural)
    }

    let python = compile(&accepted(Frontend::Python));
    let typescript = compile(&accepted(Frontend::Typescript));

    assert_eq!(
        shape(&python),
        shape(&typescript),
        "equivalent authoring frontends compile through this port to equivalent AIR"
    );
}

// ── The frontend never emits AIR ────────────────────────────────────────────

/// A frontend records typed source intent. Only the Rust lowering selects AIS
/// operations and produces AIR, so no AIS operation name, no `ais.` structural
/// kind, and no AIR schema version appears anywhere in what a frontend hands
/// over — while the AIR the port lowers is full of them.
#[test]
fn the_frontend_emits_no_air_and_only_rust_lowering_produces_it() {
    // Every AIS spelling AIR uses: the five semantic operations, the structural
    // kind that carries a dialect prefix, the AIR schema version, and MLIR
    // module text.
    const AIR_SPELLINGS: [&str; 9] = [
        "model.call",
        "capability.invoke",
        "program.new",
        "program.invoke",
        "await.event",
        "ais.",
        "apxm.air",
        "module {",
        "func.func @",
    ];

    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let compiled = compile(&accepted(frontend));

        let captured = serde_json::to_string(&compiled.frontend_graph)
            .expect("a captured FrontendGraph serializes");
        for spelling in AIR_SPELLINGS {
            assert!(
                !captured.contains(spelling),
                "the {} authoring frontend recorded the AIR spelling '{spelling}'; \
                 a frontend records typed source intent and never emits AIR",
                frontend.wire()
            );
        }

        // The same program, after Rust lowering, does carry AIS operations. This
        // is what makes the assertion above a statement about the frontend and
        // not about a graph that happened to be empty.
        let lowered = serde_json::to_string(&compiled.air).expect("lowered AIR serializes");
        assert!(
            lowered.contains("apxm.air"),
            "{} lowering produces an AIR document",
            frontend.wire()
        );
        assert!(
            lowered.contains("model.call"),
            "{} lowering selects the model.call AIS operation the source never named",
            frontend.wire()
        );
        assert!(
            lowered.contains("capability.invoke"),
            "{} lowering selects the capability.invoke AIS operation the source never named",
            frontend.wire()
        );
    }
}

#[test]
fn typescript_input_contract_reaches_the_digest_bound_service_artifact() {
    if !frontend_present(Frontend::Typescript) {
        return;
    }
    for (input_type, expected) in [
        ("unknown", true),
        ("{}", true),
        ("{ label?: string }", true),
        ("{ label: string }", false),
        ("string", false),
        ("string[]", false),
        ("never", false),
        ("{ label?: string } | string", false),
    ] {
        let source = format!(
            r#"import {{ Workflow }} from "@apxm/frontend";
import {{ source }} from "@apxm/frontend/node";
source(import.meta.url);
type Input = {input_type};
type Output = Input;
export const InputContractAgent = Workflow<Input, Output>({{
  name: "InputContractAgent",
  async run(agent, input) {{ return input; }},
}});
"#
        );
        let compiled = compile(&SourceBundleRequest::new(
            Frontend::Typescript,
            "InputContractAgent",
            source,
        ));
        let artifact =
            ExecutableArtifact::from_graph_and_air(&compiled.frontend_graph, &compiled.air)
                .expect("service artifact");
        assert_eq!(
            artifact.entrypoints[0].accepts_empty_json_object(),
            expected,
            "input type {input_type:?} has the expected empty-object contract"
        );
    }
}

#[test]
fn both_frontends_bind_typed_json_input_to_the_service_artifact() {
    let expected = serde_json::json!({
        "type": "object", "additionalProperties": false, "required": ["reference", "count", "accepted", "labels"],
        "properties": {
            "reference": {"type": "string"}, "count": {"type": "number"},
            "accepted": {"type": "boolean"}, "labels": {"type": "array", "items": {"type": "string"}},
            "note": {"type": "string"}
        }
    });
    for (frontend, source) in [
        (
            Frontend::Typescript,
            r#"import { Workflow } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";
source(import.meta.url);
type Input = { reference: string; count: number; accepted: boolean; labels: string[]; note?: string };
export const JsonInput = Workflow<Input, Input>({ name: "JsonInput", async run(agent, input) { return input; } });
"#,
        ),
        (
            Frontend::Python,
            r#"from typing import TypedDict, NotRequired
from apxm_program import Workflow
class Input(TypedDict):
    reference: str
    count: float
    accepted: bool
    labels: list[str]
    note: NotRequired[str]
@Workflow(input=Input, output=Input)
async def JsonInput(agent, input):
    return input
"#,
        ),
    ] {
        if !frontend_present(frontend) {
            continue;
        }
        let compiled = compile(&SourceBundleRequest::new(frontend, "JsonInput", source));
        let artifact =
            ExecutableArtifact::from_graph_and_air(&compiled.frontend_graph, &compiled.air)
                .unwrap();
        assert_eq!(
            serde_json::to_value(&artifact.entrypoints[0].input_schema).unwrap(),
            expected
        );
        assert_eq!(
            artifact.canonical_digest().unwrap(),
            artifact.artifact_digest
        );
    }
}

#[test]
fn pure_locals_cannot_escape_their_branch_or_mutate_after_binding() {
    for (frontend, source) in [
        (
            Frontend::Typescript,
            r#"import { Workflow } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";
source(import.meta.url);
export const Transform = Workflow<unknown, unknown>({name: "Transform", async run(agent, input) {
BODY
}});
"#,
        ),
        (
            Frontend::Python,
            "from apxm_program import Workflow\n@Workflow(input=object, output=object)\nasync def Transform(agent, input):\nBODY\n",
        ),
    ] {
        if !frontend_present(frontend) {
            continue;
        }
        let bodies = match frontend {
            Frontend::Typescript => [
                "const mapped = {}; mapped.value = 1; return mapped;",
                "if (input) {const mapped = {};} return mapped;",
                "const mapped = agent.context; return mapped;",
            ],
            Frontend::Python => [
                "    mapped = {}\n    mapped[\"value\"] = 1\n    return mapped",
                "    if input:\n        mapped = {}\n    return mapped",
                "    mapped = agent.context\n    return mapped",
            ],
        };
        for body in bodies {
            assert!(
                compile_source_bundle(
                    &SourceBundleRequest::new(frontend, "Transform", source.replace("BODY", body)),
                    &roots(),
                    &drivers()
                )
                .is_err()
            );
        }
    }
}

#[test]
fn typescript_public_permissions_compile_through_source_port_into_artifact_air() {
    if !frontend_present(Frontend::Typescript) {
        return;
    }
    let source = r#"import { Workflow, Model, Tool } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";
import { Allow, Ask, Deny } from "@apxm/frontend/permissions";
source(import.meta.url);
type Input = {};
type Output = unknown;
const Read = Tool<Input, Output>("read", { permission: Allow });
const Write = Tool<Input, Output>("write", { permission: Ask("writes host state") });
const Search = Tool<Input, Output>("search_web", { permission: Deny("no network") });
const Result = Model<Input, Output>("permission.model");
export const PermissionAgent = Workflow<Input, Output>({
  name: "PermissionAgent",
  async run(agent, input) {
    await Read(input);
    await Write(input);
    await Search(input);
    return await Result(input);
  },
});
"#;
    let compiled = compile(&SourceBundleRequest::new(
        Frontend::Typescript,
        "PermissionAgent",
        source,
    ));
    assert_eq!(
        compiled.frontend_graph.capability_requirements.len(),
        3,
        "source capture records each permissioned Tool declaration"
    );
    let requests = &compiled.air.capability_permission_requests;
    assert_eq!(
        requests.get("read").map(|permission| permission.as_str()),
        Some("allow")
    );
    assert_eq!(
        requests
            .get("write")
            .map(|permission| (permission.as_str(), permission.reason())),
        Some(("ask", Some("writes host state")))
    );
    assert_eq!(
        requests
            .get("search_web")
            .map(|permission| (permission.as_str(), permission.reason())),
        Some(("deny", Some("no network")))
    );
    let artifact = ExecutableArtifact::from_graph_and_air(&compiled.frontend_graph, &compiled.air)
        .expect("permissioned source lowers to an executable artifact");
    assert!(
        artifact.entrypoints[0].accepts_empty_json_object(),
        "the same service artifact carries the compiler-owned empty-object proof"
    );
    assert_eq!(
        artifact.air.capability_permission_requests, *requests,
        "artifact AIR preserves the source permission requests"
    );
}

// ── The rejection classes ───────────────────────────────────────────────────

/// Invalid syntax: the accepted body with one unclosed call.
#[test]
fn invalid_syntax_rejects_with_no_graph() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let body = match frontend {
            Frontend::Python => "    evidence = await SearchWeb(request\n    return evidence",
            Frontend::Typescript => {
                "    const evidence = await SearchWeb(request;\n    return evidence;"
            }
        };
        assert_rejects(
            frontend,
            "invalid syntax",
            &request(frontend, "review.model", body),
            SourceDiagnosticCode::SourceRejected,
        );
    }
}

/// A parse failure stops the compile before capture, and the report says so:
/// `stopped_at` names the static-check phase, so no consumer reads capture or
/// lowering as verified. The syntax error is located in the submitted source.
#[test]
fn an_early_parse_failure_stops_the_report_at_type_check() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let body = match frontend {
            Frontend::Python => "    evidence = await SearchWeb(request\n    return evidence",
            Frontend::Typescript => {
                "    const evidence = await SearchWeb(request;\n    return evidence;"
            }
        };
        let submitted = request(frontend, "review.model", body);
        let body_line = submitted
            .source
            .lines()
            .position(|line| line.contains("SearchWeb(request"))
            .map(|index| u32::try_from(index + 1).unwrap())
            .expect("the body is in the source");
        let diagnostics = reject(&submitted);
        let report = diagnostic_report(&diagnostics);
        assert_eq!(
            report.stopped_at,
            Some(Phase::TypeCheck),
            "{}: {diagnostics:?}",
            frontend.wire()
        );
        let located = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .find_map(|diagnostic| diagnostic.location.as_ref())
            .unwrap_or_else(|| {
                panic!(
                    "{}: the syntax error is located: {diagnostics:?}",
                    frontend.wire()
                )
            });
        assert_eq!(located.source_file, frontend.submitted_source_file());
        assert!(
            (body_line..=body_line + 1).contains(&located.span.start_line),
            "{}: located at the unclosed call, got {located:?}",
            frontend.wire()
        );
        assert!(
            report
                .items
                .iter()
                .all(|item| item.phase == Phase::TypeCheck)
        );
    }
}

/// A compiler warning does not reject: it travels with the compiled source as
/// a located warning, and the report of a successful compile has no error and
/// names no stopping phase.
#[test]
fn a_compile_warning_travels_with_the_compiled_source() {
    if !frontend_present(Frontend::Python) {
        return;
    }
    let mut submitted = accepted(Frontend::Python);
    submitted.source = submitted.source.replacen(
        "\n\n\nclass ReviewRequest:",
        "\n\nIDENTITY = 1 is 1\n\nclass ReviewRequest:",
        1,
    );
    let warning_line = submitted
        .source
        .lines()
        .position(|line| line.starts_with("IDENTITY"))
        .map(|index| u32::try_from(index + 1).unwrap())
        .expect("the warning fixture is in the source");
    let compiled = compile(&submitted);

    assert_eq!(compiled.diagnostics.len(), 1, "{:?}", compiled.diagnostics);
    let warning = &compiled.diagnostics[0];
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(warning.wire_code(), "source_warning");
    assert_eq!(warning.phase, Phase::TypeCheck);
    let location = warning.location.as_ref().expect("the warning is located");
    assert_eq!(location.source_file, "submitted_source.py");
    assert_eq!(location.span.start_line, warning_line);

    let report = diagnostic_report(&compiled.diagnostics);
    assert!(!report.has_errors());
    assert_eq!(report.stopped_at, None);
    assert_eq!(report.total_count, 1);
}

/// Every semantic operation the TypeScript frontend captured keeps its own
/// source span through lowering: a host capability call, and one Tool invoked
/// from both arms of a branch, which is two operations with two spans.
#[test]
fn every_lowered_operation_keeps_its_own_frontend_span() {
    if !frontend_present(Frontend::Typescript) {
        return;
    }
    let source = BRANCHED_TYPESCRIPT_PROGRAM;
    let submitted = SourceBundleRequest::new(Frontend::Typescript, ENTRYPOINT, source)
        .with_host_capabilities(["notes.search"]);
    let compiled = compile(&submitted);
    assert_branched_spans(&compiled.air, &compiled.source_map, source);
}

/// A TypeScript program with a host capability call and the same Tool invoked
/// in both arms of a branch.
pub const BRANCHED_TYPESCRIPT_PROGRAM: &str = r#"import { Workflow, Capability, Tool } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type ReviewRequest = { urgent: boolean };
type Review = object;

const Notes = Capability<ReviewRequest, Review>("host:notes.search");
const SearchWeb = Tool<ReviewRequest, Review>("search_web");

export const Reviewer = Workflow<ReviewRequest, Review>({
  name: "Reviewer",
  async run(agent, request) {
    const noted = await Notes(request);
    if (request.urgent) {
      return await SearchWeb(request);
    } else {
      return await SearchWeb(request);
    }
  },
});
"#;

/// Every semantic operation has exactly one node span, and the three
/// capability invocations each point at their own call in the source.
pub fn assert_branched_spans(
    air: &apxm_program::air::AirModule,
    source_map: &apxm_program::source_map::SourceMap,
    source: &str,
) {
    use std::collections::BTreeMap;

    let spans: BTreeMap<&str, &apxm_program::source_map::NodeSpan> = source_map
        .node_spans
        .iter()
        .map(|span| (span.node_id.as_str(), span))
        .collect();
    assert!(!air.semantic_operations.is_empty());
    for operation in &air.semantic_operations {
        assert!(
            spans.contains_key(operation.node_id.as_str()),
            "semantic operation {} has no node span; spans: {:?}",
            operation.node_id,
            source_map.node_spans
        );
    }
    let lines = source.lines().collect::<Vec<_>>();
    let text_at = |span: &apxm_program::source_map::Span| {
        assert_eq!(
            span.start_line, span.end_line,
            "a call span sits on one line"
        );
        let line = lines[usize::try_from(span.start_line).unwrap() - 1];
        line.chars()
            .skip(usize::try_from(span.start_column).unwrap())
            .take(usize::try_from(span.end_column - span.start_column).unwrap())
            .collect::<String>()
    };
    let invoked = air
        .semantic_operations
        .iter()
        .filter(|operation| {
            serde_json::to_value(operation.op).expect("op serializes") == "capability.invoke"
        })
        .map(|operation| {
            let span = spans[operation.node_id.as_str()];
            (operation.node_id.clone(), span.span, text_at(&span.span))
        })
        .collect::<Vec<_>>();
    assert_eq!(invoked.len(), 3, "{invoked:?}");
    let host = invoked
        .iter()
        .filter(|(_, _, text)| text.contains("Notes(request)"))
        .count();
    assert_eq!(host, 1, "{invoked:?}");
    let searches = invoked
        .iter()
        .filter(|(_, _, text)| text.contains("SearchWeb(request)"))
        .collect::<Vec<_>>();
    assert_eq!(searches.len(), 2, "{invoked:?}");
    assert_ne!(searches[0].0, searches[1].0, "two operations, two node ids");
    assert_ne!(
        searches[0].1.start_line, searches[1].1.start_line,
        "each branch's call keeps its own line"
    );
}

/// A raw AIS spelling: source naming an AIS operation directly instead of
/// authoring a typed binding. Source never selects an AIS operation.
#[test]
fn a_raw_ais_spelling_rejects_with_no_graph() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        // `ais` is bound in both fixtures. Left unbound it is an ordinary
        // unknown name, which rejects under the same umbrella code and would
        // prove nothing about this rule.
        let body = match frontend {
            Frontend::Python => "    ais = {}\n    return await ais.model_call(request)",
            Frontend::Typescript => {
                "    const ais: any = {};\n\
                 \x20   return await ais.model_call(request);"
            }
        };
        assert_rejects_naming(
            frontend,
            "a raw AIS spelling",
            &request(frontend, "review.model", body),
            SourceDiagnosticCode::SourceRejected,
            "unsupported call '.model_call(...)'",
        );
    }
}

/// A raw structural AIS spelling. `ais.loop` is constructed by Rust from a
/// language-neutral control intent; source that names it is not authoring.
#[test]
fn a_raw_structural_ais_spelling_rejects_with_no_graph() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let body = match frontend {
            Frontend::Python => "    ais = {}\n    return await ais.loop(request)",
            Frontend::Typescript => {
                "    const ais: any = {};\n\
                 \x20   return await ais.loop(request);"
            }
        };
        assert_rejects_naming(
            frontend,
            "a raw structural AIS spelling",
            &request(frontend, "review.model", body),
            SourceDiagnosticCode::SourceRejected,
            "unsupported call '.loop(...)'",
        );
    }
}

/// Model reference drift: a display name where an exact typed reference belongs.
#[test]
fn a_drifted_model_reference_rejects_with_no_graph() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        assert_rejects(
            frontend,
            "a drifted model reference",
            &request(frontend, "default", accepted_body(frontend)),
            SourceDiagnosticCode::SourceRejected,
        );
    }
}

/// An unresolved Tool reference: an awaited call to a name that is bound to no
/// declared Tool, Model, or Capability.
#[test]
fn an_unresolved_tool_reference_rejects_with_no_graph() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        assert_rejects(
            frontend,
            "an unresolved Tool reference",
            &request(
                frontend,
                "review.model",
                &body_with_return(frontend, "UnboundTool"),
            ),
            SourceDiagnosticCode::SourceRejected,
        );
    }
}

/// An entrypoint the submitted source does not define.
#[test]
fn an_absent_entrypoint_rejects_with_no_graph() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let mut request = accepted(frontend);
        request.entrypoint = "NotDefinedHere".to_string();
        assert_rejects(
            frontend,
            "an absent entrypoint",
            &request,
            SourceDiagnosticCode::EntrypointNotAnAgentProgram,
        );
    }
}

// ── Unavailability ──────────────────────────────────────────────────────────

/// An absent frontend package is a typed diagnostic, not a panic and not a
/// silently degraded compile. This test needs no build product, so it runs on
/// every checkout.
#[test]
fn an_absent_frontend_package_rejects_with_a_typed_diagnostic() {
    let absent = common::repository_root().join("crates/compiler/source-port/no-frontend-here");
    assert!(
        !absent.exists(),
        "the unavailability fixture names a path that does not exist"
    );
    let roots = FrontendRoots::new(&absent, &absent);
    let drivers = drivers();

    for frontend in FRONTENDS {
        let diagnostics = compile_source_bundle(&accepted(frontend), &roots, &drivers)
            .expect_err("an absent frontend package cannot produce a graph");
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec![SourceDiagnosticCode::FrontendUnavailable],
            "an absent {} frontend package is reported as unavailability",
            frontend.wire()
        );
        assert_reason_was_decoded(frontend, &diagnostics);
    }
}

/// A declared driver is an exact composition binding. The port neither searches
/// `PATH` nor substitutes another interpreter when that binding is absent.
#[test]
fn an_absent_declared_driver_never_falls_back_to_path() {
    let absent = common::repository_root().join("crates/compiler/source-port/no-driver-here");
    assert!(
        !absent.exists(),
        "the absent-driver fixture names a path that does not exist"
    );
    let drivers = FrontendDrivers::new(&absent, &absent);

    for frontend in FRONTENDS {
        let diagnostics = compile_source_bundle(&accepted(frontend), &roots(), &drivers)
            .expect_err("an absent declared driver cannot produce a graph");
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec![SourceDiagnosticCode::FrontendUnavailable],
            "an absent declared {} driver is reported as unavailability",
            frontend.wire()
        );
        assert!(
            diagnostics[0].message.contains("declared"),
            "the diagnostic identifies the missing composition binding: {diagnostics:?}"
        );
    }
}

/// Composition cannot use a relative driver because that would defer its
/// meaning to the current directory or `PATH` instead of the exact binding.
#[test]
fn a_relative_declared_driver_is_rejected_before_capture() {
    let drivers = FrontendDrivers::new("python", "node");

    for frontend in FRONTENDS {
        let diagnostics = compile_source_bundle(&accepted(frontend), &roots(), &drivers)
            .expect_err("a relative declared driver cannot produce a graph");
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec![SourceDiagnosticCode::FrontendUnavailable],
            "a relative {} driver is rejected as unavailable",
            frontend.wire()
        );
        assert!(
            diagnostics[0].message.contains("absolute path"),
            "the diagnostic identifies relative composition data: {diagnostics:?}"
        );
    }
}

/// A driver bound to the wrong frontend does not cause the port to select a
/// substitute: the declared program fails closed before it can capture source.
#[test]
fn swapped_declared_drivers_fail_closed_without_substitution() {
    let installed = drivers();
    let swapped = FrontendDrivers::new(
        installed.driver(Frontend::Typescript),
        installed.driver(Frontend::Python),
    );

    for frontend in FRONTENDS {
        let diagnostics = compile_source_bundle(&accepted(frontend), &roots(), &swapped)
            .expect_err("a swapped declared driver cannot produce a graph");
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec![SourceDiagnosticCode::FrontendUnavailable],
            "a swapped {} driver fails closed instead of selecting a substitute",
            frontend.wire()
        );
    }
}

/// A relative frontend root is rejected before it can be resolved through the
/// current directory.
#[test]
fn a_relative_frontend_root_is_rejected_before_capture() {
    let roots = FrontendRoots::new("python-frontend", "typescript-frontend");
    let drivers = drivers();

    for frontend in FRONTENDS {
        let diagnostics = compile_source_bundle(&accepted(frontend), &roots, &drivers)
            .expect_err("a relative frontend root cannot produce a graph");
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec![SourceDiagnosticCode::FrontendUnavailable],
            "a relative {} frontend root is rejected as unavailable",
            frontend.wire()
        );
        assert!(
            diagnostics[0].message.contains("absolute path"),
            "the diagnostic identifies relative composition data: {diagnostics:?}"
        );
    }
}

/// A reason the capture reported is decoded into the typed code, leaving the
/// message carrying only the explanation.
///
/// This separates a reported rejection from an unexplained one. The port also
/// reports unavailability when a capture dies without reporting anything at all
/// — killed by a resource limit, or crashed — and that case quotes what it
/// received verbatim. A raw reason token surviving into the message means the
/// port received a reason and failed to decode it, which is the same code for a
/// different fact.
fn assert_reason_was_decoded(frontend: Frontend, diagnostics: &[SourceDiagnostic]) {
    const REASON_TOKENS: [&str; 4] = [
        "harness_request_invalid",
        "frontend_unavailable",
        "source_rejected",
        "entrypoint_not_an_agent_program",
    ];
    for diagnostic in diagnostics {
        for token in REASON_TOKENS {
            assert!(
                !diagnostic.message.contains(token),
                "the {} capture reported the reason '{token}', which belongs in the \
                 diagnostic code; it survived into the message: {}",
                frontend.wire(),
                diagnostic.message
            );
        }
    }
}

/// A frontend root that exists but holds no frontend package is unavailability
/// too: the port reaches the interpreter, the interpreter finds no package, and
/// the harness reports the closed reason rather than capturing nothing.
#[test]
fn an_empty_frontend_root_rejects_with_a_typed_diagnostic() {
    let empty = std::env::temp_dir().join(format!(
        "apxm-source-port-empty-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&empty).expect("create the empty frontend root");
    let roots = FrontendRoots::new(&empty, &empty);
    let drivers = drivers();

    for frontend in FRONTENDS {
        let diagnostics = compile_source_bundle(&accepted(frontend), &roots, &drivers)
            .expect_err("an empty frontend root cannot produce a graph");
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == SourceDiagnosticCode::FrontendUnavailable),
            "an empty {} frontend root is reported as unavailability; got {diagnostics:?}",
            frontend.wire()
        );
        assert_reason_was_decoded(frontend, &diagnostics);
    }

    std::fs::remove_dir_all(&empty).expect("remove the empty frontend root");
}

/// A frontend records the source language it authored. A graph attributed to a
/// different language is not the compilation of the submitted source, however
/// well-formed it is, so the port rejects it rather than handing it back.
///
/// The frontend roots are declared by the caller, so what stands at a declared
/// root is not something the port can assume. This test points the Python root
/// at a stand-in that records TypeScript.
#[test]
fn a_graph_attributed_to_the_wrong_language_rejects() {
    let fixture = common::repository_root()
        .join("crates/compiler/source-port/tests/fixtures/mislabeling-frontend");
    assert!(
        fixture.join("apxm_program/__init__.py").is_file(),
        "the mislabeling stand-in frontend is checked in"
    );
    let roots = FrontendRoots::new(&fixture, &fixture);
    let drivers = drivers();

    let request = SourceBundleRequest::new(
        Frontend::Python,
        "Reviewer",
        "from apxm_program import mislabeled_program\n\nReviewer = mislabeled_program()\n",
    );

    let diagnostics = compile_source_bundle(&request, &roots, &drivers)
        .expect_err("a graph attributed to another language is not this compilation");
    assert_eq!(
        diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>(),
        vec![SourceDiagnosticCode::FrontendOutputInvalid],
        "a mislabeled graph is reported as invalid frontend output; got {diagnostics:?}"
    );
}

// ── Request validation ──────────────────────────────────────────────────────

/// A request that is not a compilable unit is rejected before any interpreter
/// is reached, so an invalid request never becomes submitted source.
#[test]
fn an_invalid_request_rejects_before_any_capture() {
    let absent = common::repository_root().join("crates/compiler/source-port/no-frontend-here");
    let roots = FrontendRoots::new(&absent, &absent);
    let drivers = drivers();

    let cases = [
        (
            "an empty entrypoint",
            SourceBundleRequest::new(Frontend::Python, "  ", "print(1)"),
        ),
        (
            "empty source",
            SourceBundleRequest::new(Frontend::Python, "Reviewer", ""),
        ),
        (
            "source past the accepted size",
            SourceBundleRequest::new(
                Frontend::Python,
                "Reviewer",
                "#".repeat(apxm_source_port::MAX_SOURCE_BYTES + 1),
            ),
        ),
    ];

    for (class, request) in cases {
        let diagnostics = compile_source_bundle(&request, &roots, &drivers)
            .expect_err("an invalid request cannot produce a graph");
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            vec![SourceDiagnosticCode::RequestInvalid],
            // The frontend roots point nowhere, so an unavailability code here
            // would mean the port had already tried to capture.
            "{class} is rejected as an invalid request, before capture"
        );
    }

    let exact_boundary = SourceBundleRequest::new(
        Frontend::Python,
        "Reviewer",
        "#".repeat(apxm_source_port::MAX_SOURCE_BYTES),
    );
    let absent = common::repository_root().join("crates/compiler/source-port/no-frontend-here");
    let unavailable_roots = FrontendRoots::new(&absent, &absent);
    let diagnostics = compile_source_bundle(&exact_boundary, &unavailable_roots, &drivers)
        .expect_err("the exact source-size boundary reaches frontend availability validation");
    assert_eq!(
        diagnostics[0].code,
        SourceDiagnosticCode::FrontendUnavailable,
        "the maximum accepted byte count is inclusive; only the next byte is request-invalid"
    );
}
