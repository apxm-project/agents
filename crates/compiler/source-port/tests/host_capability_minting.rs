//! Conformance for the host-fulfilled Capability namespace at the source port.
//!
//! A `host:` reference is minted by the package manifest, not by the generated
//! catalogue (ADR-0025). The manifest is not visible inside the confined
//! interpreter, so the port carries the declared ids into capture and closes the
//! same set again over the lowered AIR — the second pass is what can say *where*
//! an undeclared reference is, because the frontend's markers run at module
//! evaluation with no source node to point at.

mod common;

use apxm_source_port::{Frontend, SourceBundleRequest, compile_source_bundle};

use crate::common::{ENTRYPOINT, FRONTENDS, drivers, frontend_present, roots};

/// One authored program that invokes the host capability `capability_ref`.
fn program_invoking(frontend: Frontend, capability_ref: &str) -> String {
    match frontend {
        Frontend::Python => format!(
            "from apxm_program import Agent, Capability\n\
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
             @Agent(input=ReviewRequest, output=Review)\n\
             async def Reviewer(agent, request):\n\
             \x20   return await Notes(request)\n"
        ),
        Frontend::Typescript => format!(
            "import {{ Agent, Capability }} from \"@apxm/frontend\";\n\
             import {{ source }} from \"@apxm/frontend/node\";\n\
             \n\
             source(import.meta.url);\n\
             \n\
             type ReviewRequest = object;\n\
             type Review = object;\n\
             \n\
             const Notes = Capability<ReviewRequest, Review>(\"{capability_ref}\");\n\
             \n\
             export const Reviewer = Agent<ReviewRequest, Review>({{\n\
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

/// An undeclared `host:` reference rejects, and the diagnostic names the line
/// and column of the reference in the submitted source.
#[test]
fn an_undeclared_host_capability_rejects_with_a_source_position() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let source = program_invoking(frontend, "host:notes.append");
        let expected = source
            .lines()
            .enumerate()
            .find_map(|(index, line)| {
                line.find("host:notes.append")
                    .map(|column| format!("{}:{}", index + 1, column + 1))
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
        let reported = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            reported.contains("host:notes.append"),
            "{}: the diagnostic names the reference; got {reported}",
            frontend.wire()
        );
        assert!(
            reported.contains(&expected),
            "{}: the diagnostic names {expected}; got {reported}",
            frontend.wire()
        );
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
