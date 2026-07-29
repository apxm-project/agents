//! Conformance for the Server-callable source-bundle compile port.
//!
//! Every test here calls the published entry point, `compile_source_bundle`, and
//! nothing else. No test names an interpreter, constructs a process, or reads a
//! harness: the interpreter boundary is the port's, and a caller that had to
//! know about it would be a port that had not been published.

mod common;

use apxm_program::frontend_graph::IntentKind;
use apxm_source_port::{
    CompiledSource, Frontend, FrontendDrivers, FrontendRoots, SourceBundleRequest,
    SourceDiagnostic, SourceDiagnosticCode, compile_source_bundle,
};
use sha2::{Digest, Sha256};

use crate::common::{ENTRYPOINT, FRONTENDS, drivers, frontend_present, roots};

// ── The fixture programs ────────────────────────────────────────────────────
//
// Each rejection fixture differs from the accepted program of the same language
// by exactly the construct under test, so a rejection is attributable to that
// construct and not to unrelated damage in the fixture.

const PYTHON_PROGRAM: &str = r#"from apxm_program import Agent, Model, Tool

ReviewModel = Model[object, object]("@MODEL@")
SearchWeb = Tool[object, object]("search.web.capability.v1")


@Agent(input="ReviewRequest", output="Review")
async def Reviewer(agent, request):
@BODY@
"#;

const TYPESCRIPT_PROGRAM: &str = r#"import { Agent, Model, Tool } from "@apxm/frontend";
import { staticSource } from "apxm:source";

const source = staticSource();
const ReviewModel = Model<object, object>("@MODEL@");
const SearchWeb = Tool<object, object>("search.web.capability.v1");

export const Reviewer = Agent<object, object>({
  name: "Reviewer",
  source,
  use: { ReviewModel, SearchWeb },
  async run(agent, request) {
@BODY@
  },
});
"#;

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
    request(frontend, "review.model.v1", accepted_body(frontend))
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
    let diagnostics = reject(request);
    assert!(
        diagnostics.iter().any(|d| d.code == expected),
        "{} {class} rejects with {expected}; got {diagnostics:?}",
        frontend.wire()
    );
    assert_reason_was_decoded(frontend, &diagnostics);
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
        "apxm.air.v1",
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
            lowered.contains("apxm.air.v1"),
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
            &request(frontend, "review.model.v1", body),
            SourceDiagnosticCode::SourceRejected,
        );
    }
}

/// A raw AIS spelling: source naming an AIS operation directly instead of
/// authoring a typed binding. Source never selects an AIS operation.
#[test]
fn a_raw_ais_spelling_rejects_with_no_graph() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let body = match frontend {
            Frontend::Python => "    return await ais.model_call(request)",
            Frontend::Typescript => "    return await ais.model_call(request);",
        };
        assert_rejects(
            frontend,
            "a raw AIS spelling",
            &request(frontend, "review.model.v1", body),
            SourceDiagnosticCode::SourceRejected,
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
            Frontend::Python => "    return await ais.loop(request)",
            Frontend::Typescript => "    return await ais.loop(request);",
        };
        assert_rejects(
            frontend,
            "a raw structural AIS spelling",
            &request(frontend, "review.model.v1", body),
            SourceDiagnosticCode::SourceRejected,
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
                "review.model.v1",
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
}

// ── External source ─────────────────────────────────────────────────────────

/// A first-party Agent Program authored in another repository compiles through
/// the ordinary submitted-source boundary, with no accommodation for it here.
///
/// Every other fixture in this file was written to exercise a property. This one
/// was not written here at all: it is an unmodified copy of source another
/// repository committed, so it is the only fixture that can show the boundary
/// accepts real external source rather than source shaped to pass. See
/// `fixtures/external-program/PROVENANCE.md` for its origin and digest.
///
/// The assertions are deliberately generic. `agents` ADR-0017 forbids branching
/// on an external program's name or product semantics, so this checks the shape
/// of the compilation — one program, capability and model requirements resolved,
/// AIR lowered — and not what the program is for.
#[test]
fn an_externally_authored_program_compiles_as_ordinary_submitted_source() {
    if !frontend_present(Frontend::Typescript) {
        return;
    }

    let source = std::fs::read_to_string(
        common::repository_root()
            .join("crates/compiler/source-port/tests/fixtures/external-program/program.ts"),
    )
    .expect("the external program fixture is checked in");

    let compiled = compile_source_bundle(
        &SourceBundleRequest::new(Frontend::Typescript, "Gao", source),
        &roots(),
        &drivers(),
    )
    .expect("externally authored source compiles through the ordinary boundary");

    let graph = &compiled.frontend_graph;
    assert_eq!(
        graph.program_definitions.len(),
        1,
        "one submitted source bundle records one program definition"
    );
    assert!(
        !graph.capability_requirements.is_empty(),
        "the external program's Tool references resolve into capability requirements"
    );
    assert!(
        !graph.model_requirements.is_empty(),
        "the external program's model reference resolves into a model requirement"
    );
    // Lowering is Rust's alone, so reaching AIR at all is the property: the
    // frontend recorded intent and this repository lowered it.
    assert!(
        !compiled.air.semantic_operations.is_empty(),
        "the captured graph lowers to canonical AIR"
    );
}

/// The external fixture is submitted exactly as its author committed it.
///
/// A fixture quietly edited to keep a test green would make the test above
/// prove the opposite of what it claims — that the boundary accepts source this
/// repository adjusted for it. Pinning the digest makes such an edit fail here,
/// where the reason is stated, rather than silently downgrade the evidence.
#[test]
fn the_external_fixture_is_the_bytes_its_author_committed() {
    const ORIGIN_DIGEST: &str = "56bc2101daae748510c47f4d2f7b86fc33f9c3157c37cd134c5a1041b407d7a9";

    let bytes = std::fs::read(
        common::repository_root()
            .join("crates/compiler/source-port/tests/fixtures/external-program/program.ts"),
    )
    .expect("the external program fixture is checked in");

    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        ORIGIN_DIGEST,
        "the external fixture no longer matches the origin bytes recorded in \
         PROVENANCE.md; re-copy it from the origin revision and update both, \
         rather than editing it in place"
    );
}
