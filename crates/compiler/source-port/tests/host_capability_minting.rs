//! Conformance for the host-fulfilled Capability namespace at the source port.
//!
//! A `host:` reference is minted by the package manifest, not by the generated
//! catalogue (ADR-0025). The manifest is not visible inside the confined
//! interpreter, so the port carries the declared ids into capture and closes the
//! same set again over the lowered AIR — the second pass is what can say *where*
//! an undeclared reference is, because the frontend's markers run at module
//! evaluation with no source node to point at.

mod common;

use apxm_source_port::{
    Frontend, Phase, Severity, SourceBundleRequest, SourceDiagnosticCode, compile_source_bundle,
    diagnostic_report,
};

use crate::common::{ENTRYPOINT, FRONTENDS, drivers, frontend_present, roots};

/// One authored program that invokes the host capability `capability_ref`.
fn program_invoking(frontend: Frontend, capability_ref: &str) -> String {
    match frontend {
        Frontend::Python => format!(
            "from apxm_program import Workflow, Capability\n\
             \n\
             \n\
             class ReviewRequest:\n\
             \x20   pass\n\
             \n\
             \n\
             class Review:\n\
             \x20   pass\n\
             \n\
             \n\
             Notes = Capability[ReviewRequest, Review](\"{capability_ref}\")\n\
             \n\
             \n\
             @Workflow(input=ReviewRequest, output=Review)\n\
             async def Reviewer(agent, request):\n\
             \x20   return await Notes(request)\n"
        ),
        Frontend::Typescript => format!(
            "import {{ Workflow, Capability }} from \"@apxm/frontend\";\n\
             import {{ source }} from \"@apxm/frontend/node\";\n\
             \n\
             source(import.meta.url);\n\
             \n\
             type ReviewRequest = object;\n\
             type Review = object;\n\
             \n\
             const Notes = Capability<ReviewRequest, Review>(\"{capability_ref}\");\n\
             \n\
             export const Reviewer = Workflow<ReviewRequest, Review>({{\n\
             \x20 name: \"Reviewer\",\n\
             \x20 async run(agent, request) {{\n\
             \x20   return await Notes(request);\n\
             \x20 }},\n\
             }});\n"
        ),
    }
}

/// A declared host capability compiles, and the AIR carries the prefixed
/// reference verbatim: the runtime decides what to do with it by the namespace
/// alone, so the prefix must survive lowering.
#[test]
fn a_declared_host_capability_compiles_and_keeps_its_namespace() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let request = SourceBundleRequest::new(
            frontend,
            ENTRYPOINT,
            program_invoking(frontend, "host:notes.search"),
        )
        .with_host_capabilities(["notes.search"]);
        let compiled =
            compile_source_bundle(&request, &roots(), &drivers()).unwrap_or_else(|diagnostics| {
                panic!(
                    "a declared host capability compiles in {}; rejected with {diagnostics:?}",
                    frontend.wire()
                )
            });
        assert!(
            compiled
                .air
                .invoked_capability_refs()
                .contains(&"host:notes.search"),
            "{}: the lowered AIR must carry the host reference verbatim",
            frontend.wire()
        );
    }
}

/// An undeclared `host:` reference rejects, and the diagnostic locates the
/// reference in the submitted source in source-map coordinates: 1-based line,
/// 0-based column, spanning the reference.
#[test]
fn an_undeclared_host_capability_rejects_with_a_source_position() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let source = program_invoking(frontend, "host:notes.append");
        let (line, column) = source
            .lines()
            .enumerate()
            .find_map(|(index, line)| {
                line.find("host:notes.append").map(|column| {
                    (
                        u32::try_from(index + 1).unwrap(),
                        u32::try_from(column).unwrap(),
                    )
                })
            })
            .expect("the fixture writes the reference exactly once");

        let request = SourceBundleRequest::new(frontend, ENTRYPOINT, source)
            .with_host_capabilities(["notes.search"]);
        let diagnostics = compile_source_bundle(&request, &roots(), &drivers())
            .err()
            .unwrap_or_else(|| {
                panic!(
                    "{}: an undeclared host capability is not compilable",
                    frontend.wire()
                )
            });
        assert_eq!(diagnostics.len(), 1, "{}: {diagnostics:?}", frontend.wire());
        let diagnostic = &diagnostics[0];
        assert_eq!(diagnostic.code, SourceDiagnosticCode::GraphRejected);
        assert_eq!(diagnostic.wire_code(), "graph_rejected");
        assert_eq!(diagnostic.severity, Severity::Error);
        assert!(
            diagnostic.message.contains("host:notes.append"),
            "{}: the diagnostic names the reference; got {diagnostic:?}",
            frontend.wire()
        );
        let location = diagnostic
            .location
            .as_ref()
            .unwrap_or_else(|| panic!("{}: the diagnostic is located", frontend.wire()));
        assert_eq!(location.source_file, frontend.submitted_source_file());
        assert_eq!(
            (
                location.span.start_line,
                location.span.start_column,
                location.span.end_line,
                location.span.end_column
            ),
            (line, column, line, column + 17),
            "{}: the span covers exactly the reference",
            frontend.wire()
        );
    }
}

/// Every undeclared reference is its own diagnostic with its own location, in
/// source order — a repeated reference included — and none is folded into
/// another's text. The report stops before capture: nothing later ran.
#[test]
fn every_undeclared_host_reference_is_reported_at_its_own_location() {
    for frontend in FRONTENDS {
        let source = match frontend {
            Frontend::Python => "from apxm_program import Capability\n\
                 Append = Capability[dict, dict](\"host:notes.append\")\n\
                 Delete = Capability[dict, dict](\"host:notes.delete\")\n\
                 Again = Capability[dict, dict]('host:notes.append')\n"
                .to_owned(),
            Frontend::Typescript => "import { Capability } from \"@apxm/frontend\";\n\
                 const Append = Capability<object, object>(\"host:notes.append\");\n\
                 const Delete = Capability<object, object>(\"host:notes.delete\");\n\
                 const Again = Capability<object, object>('host:notes.append');\n"
                .to_owned(),
        };
        let request = SourceBundleRequest::new(frontend, ENTRYPOINT, source.clone())
            .with_host_capabilities(["notes.search"]);
        let diagnostics = compile_source_bundle(&request, &roots(), &drivers())
            .expect_err("undeclared host references never compile");

        let located = diagnostics
            .iter()
            .map(|diagnostic| {
                let location = diagnostic.location.as_ref().expect("every item is located");
                (location.span.start_line, location.span.start_column)
            })
            .collect::<Vec<_>>();
        let expected = source
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                line.find("host:").map(|column| {
                    (
                        u32::try_from(index + 1).unwrap(),
                        u32::try_from(column).unwrap(),
                    )
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(located, expected, "{}: {diagnostics:?}", frontend.wire());
        assert_eq!(diagnostics.len(), 3);
        for (diagnostic, name) in diagnostics.iter().zip([
            "host:notes.append",
            "host:notes.delete",
            "host:notes.append",
        ]) {
            assert!(diagnostic.message.contains(name), "{diagnostic:?}");
            assert!(
                !diagnostic.message.contains('\n'),
                "one reference per item, never concatenated: {diagnostic:?}"
            );
        }

        let report = diagnostic_report(&diagnostics);
        assert_eq!(report.total_count, 3);
        assert!(!report.truncated);
        assert_eq!(report.first_error_code(), Some("graph_rejected"));
        assert_eq!(report.stopped_at, Some(Phase::TypeCheck));
    }
}

/// A caller that declares nothing mints nothing, so a package cannot reach the
/// host namespace by omitting the declaration the manifest is supposed to carry.
#[test]
fn declaring_no_host_capability_mints_none() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let request = SourceBundleRequest::new(
            frontend,
            ENTRYPOINT,
            program_invoking(frontend, "host:notes.search"),
        );
        assert!(
            compile_source_bundle(&request, &roots(), &drivers()).is_err(),
            "{}: an undeclared host namespace admits nothing",
            frontend.wire()
        );
    }
}

/// A report carries at most 64 diagnostics in emission order and counts the
/// rest, so a consumer knows the list is partial.
#[test]
fn a_report_of_many_undeclared_references_is_bounded_and_counted() {
    let references = (0..70)
        .map(|index| format!("const Reference{index} = \"host:notes.n{index}\";\n"))
        .collect::<String>();
    let request = SourceBundleRequest::new(Frontend::Typescript, ENTRYPOINT, references)
        .with_host_capabilities(["notes.search"]);
    let diagnostics = compile_source_bundle(&request, &roots(), &drivers())
        .expect_err("undeclared host references never compile");
    assert_eq!(diagnostics.len(), 70, "the port keeps every diagnostic");

    let report = diagnostic_report(&diagnostics);
    assert_eq!(report.items.len(), apxm_source_port::MAX_REPORT_ITEMS);
    assert!(report.truncated);
    assert_eq!(report.total_count, 70);
    assert_eq!(report.stopped_at, Some(Phase::TypeCheck));
    for (index, item) in report.items.iter().enumerate() {
        let location = item.location.as_ref().expect("located");
        assert_eq!(location.span.start_line, u32::try_from(index + 1).unwrap());
        assert!(item.message.contains(&format!("host:notes.n{index}'")));
    }
}
