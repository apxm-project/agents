//! Conformance for what submitted source can reach while it is being captured.
//!
//! Capturing typed intent from Python source requires the Python authoring
//! frontend to run, and the same holds for TypeScript. Both frontends resolve an
//! authored callback through their language's module system, so the submitted
//! text is evaluated. That is the mechanism the frontends require. What this
//! file asserts is the boundary around it: the submitted text is bound from
//! memory rather than written to disk, it cannot reach the filesystem, the
//! process table, the network, or an arbitrary module, and it cannot make the
//! port return a graph it did not capture.

mod common;

use apxm_source_port::{Frontend, SourceDiagnosticCode, compile_source_bundle};

use crate::common::{ENTRYPOINT, FRONTENDS, drivers, frontend_present, roots};

use apxm_source_port::SourceBundleRequest;

/// Compile submitted text that must be rejected, and return the codes.
///
/// Every fixture here is an otherwise-valid program that reaches for one thing
/// beyond the authoring surface, so the rejection is attributable to the reach
/// and not to a program that never defined its entrypoint.
fn codes(frontend: Frontend, source: String) -> Vec<SourceDiagnosticCode> {
    let request = SourceBundleRequest::new(frontend, ENTRYPOINT, source);
    match compile_source_bundle(&request, &roots(), &drivers()) {
        Ok(compiled) => panic!(
            "expected the {} boundary to reject this source; it captured {} call intents",
            frontend.wire(),
            compiled.frontend_graph.call_intents.len()
        ),
        Err(diagnostics) => diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect(),
    }
}

/// One otherwise-valid authored program, with `@REACH@` replaced by whatever the
/// fixture reaches for and `@IMPORT@` by whatever it must import to do so. The
/// program itself always defines the entrypoint, so a rejection can only come
/// from the reach.
fn reaching_program(frontend: Frontend, import: &str, reach: &str) -> String {
    match frontend {
        Frontend::Python => format!(
            "{import}\n\
             from apxm_program import Agent, Model\n\
             \n\
             {reach}\n\
             \n\
             ReviewModel = Model[object, object](\"review.model.v1\")\n\
             \n\
             \n\
             @Agent(input=\"ReviewRequest\", output=\"Review\")\n\
             async def Reviewer(agent, request):\n\
             \x20   return await ReviewModel(request)\n"
        ),
        Frontend::Typescript => format!(
            "{import}\n\
             import {{ Agent, Model }} from \"@apxm/frontend\";\n\
             import {{ staticSource }} from \"apxm:source\";\n\
             \n\
             {reach}\n\
             \n\
             const ReviewModel = Model<object, object>(\"review.model.v1\");\n\
             \n\
             export const Reviewer = Agent<object, object>({{\n\
             \x20 name: \"Reviewer\",\n\
             \x20 source: staticSource(),\n\
             \x20 use: {{ ReviewModel }},\n\
             \x20 async run(agent, request) {{\n\
             \x20   return await ReviewModel(request);\n\
             \x20 }},\n\
             }});\n"
        ),
    }
}

/// The same program with nothing reached for. It compiles, so every rejection
/// below is caused by the reach that fixture adds and by nothing else.
#[test]
fn the_confinement_fixture_program_compiles_when_it_reaches_for_nothing() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let request =
            SourceBundleRequest::new(frontend, ENTRYPOINT, reaching_program(frontend, "", ""));
        compile_source_bundle(&request, &roots(), &drivers()).unwrap_or_else(|diagnostics| {
            panic!(
                "the {} confinement fixture program compiles on its own; \
                 rejected with {diagnostics:?}",
                frontend.wire()
            )
        });
    }
}

/// Submitted source that writes a file is rejected, and the file is not created.
///
/// This is the property that makes evaluation admissible at all: the frontend
/// needs to evaluate a definition, and evaluation is confined to that.
#[test]
fn submitted_source_cannot_write_to_the_filesystem() {
    let witness = std::env::temp_dir().join(format!(
        "apxm-source-port-filesystem-witness-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&witness);
    let path = witness.display().to_string();

    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let source = match frontend {
            Frontend::Python => reaching_program(
                frontend,
                "",
                &format!("open({path:?}, \"w\").write(\"escaped\")"),
            ),
            Frontend::Typescript => reaching_program(
                frontend,
                "import { writeFileSync } from \"node:fs\";",
                &format!("writeFileSync({path:?}, \"escaped\");"),
            ),
        };

        let codes = codes(frontend, source);
        assert!(
            codes.contains(&SourceDiagnosticCode::SourceRejected),
            "{} rejects source that writes to the filesystem; got {codes:?}",
            frontend.wire()
        );
        assert!(
            !witness.exists(),
            "{} submitted source reached the filesystem and created {}",
            frontend.wire(),
            witness.display()
        );
    }
}

/// The filesystem stays out of reach even where no module is named.
///
/// The module wall stops a reach that names a module, so a test that names one
/// proves only the module wall. Each language also exposes the filesystem
/// through the running process itself, with no module named anywhere, and that
/// reach arrives past every wall built out of module names.
#[test]
fn submitted_source_cannot_write_to_the_filesystem_without_naming_a_module() {
    let witness = std::env::temp_dir().join(format!(
        "apxm-source-port-unnamed-module-witness-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&witness);
    let path = witness.display().to_string();

    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let source = match frontend {
            // A builtin, reachable without importing anything.
            Frontend::Python => reaching_program(
                frontend,
                "",
                &format!("open({path:?}, \"w\").write(\"escaped\")"),
            ),
            // The process object's internal binding, reachable without
            // importing anything, and holding a real write.
            Frontend::Typescript => reaching_program(
                frontend,
                "",
                &format!(
                    "const _fs = (globalThis as {{ [key: string]: any }}).process\n\
                     \x20 .binding(\"fs\");\n\
                     _fs.writeFileUtf8({path:?}, \"escaped\", 577, 0o644);"
                ),
            ),
        };

        let codes = codes(frontend, source);
        assert!(
            codes.contains(&SourceDiagnosticCode::SourceRejected),
            "{} rejects a filesystem reach that names no module; got {codes:?}",
            frontend.wire()
        );
        assert!(
            !witness.exists(),
            "{} submitted source reached the filesystem without naming a module and created {}",
            frontend.wire(),
            witness.display()
        );
    }
}

/// Submitted source that starts a process is rejected. Source capture reads
/// typed authoring declarations and control flow; it launches nothing.
#[test]
fn submitted_source_cannot_start_a_process() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let source = match frontend {
            Frontend::Python => {
                reaching_program(frontend, "import subprocess", "subprocess.run([\"true\"])")
            }
            Frontend::Typescript => reaching_program(
                frontend,
                "import { execSync } from \"node:child_process\";",
                "execSync(\"true\");",
            ),
        };

        let codes = codes(frontend, source);
        assert!(
            codes.contains(&SourceDiagnosticCode::SourceRejected),
            "{} rejects source that starts a process; got {codes:?}",
            frontend.wire()
        );
    }
}

/// Submitted source reaches the authoring frontend and nothing else. An
/// arbitrary module is not part of the authoring surface, so importing one is a
/// rejection rather than an ambient capability.
#[test]
fn submitted_source_reaches_no_module_beyond_the_authoring_frontend() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let source = match frontend {
            Frontend::Python => {
                reaching_program(frontend, "import http.client", "_reached = http.client")
            }
            Frontend::Typescript => reaching_program(
                frontend,
                "import net from \"node:net\";",
                "const _reached = net;",
            ),
        };

        let codes = codes(frontend, source);
        assert!(
            codes.contains(&SourceDiagnosticCode::SourceRejected),
            "{} rejects source that imports beyond the authoring frontend; got {codes:?}",
            frontend.wire()
        );
    }
}

/// The module wall holds at evaluation, not only where the source is read.
///
/// A statically named module is rejected while the submitted text is still being
/// read, so that rejection says nothing about what the text can reach once it
/// runs. A specifier assembled at run time is named nowhere in the text and
/// reaches the module system only during evaluation, which is the moment the
/// wall exists for.
#[test]
fn submitted_source_reaches_no_module_it_names_only_while_running() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        // The module name is assembled from parts, so nothing that reads the
        // text sees a module being named.
        let source = match frontend {
            Frontend::Python => reaching_program(
                frontend,
                "",
                "_reached = __import__(\"ht\" + \"tp.client\")",
            ),
            Frontend::Typescript => reaching_program(
                frontend,
                "",
                "const _reached = await import(\"node\" + \":net\");",
            ),
        };

        let codes = codes(frontend, source);
        assert!(
            codes.contains(&SourceDiagnosticCode::SourceRejected),
            "{} rejects source that reaches a module it names only while running; got {codes:?}",
            frontend.wire()
        );
    }
}

/// Submitted source shares the capture process's standard output, so it can
/// write raw bytes there. It cannot forge a result by doing so: the port decodes
/// the entire standard output as exactly one FrontendGraph document, so an
/// injected byte makes the whole document unparseable and the port reports the
/// output invalid instead of returning a graph.
#[test]
fn submitted_source_cannot_forge_a_graph_through_standard_output() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        // Each injection is written by an otherwise valid program, so the
        // capture itself succeeds and only the injected bytes stand between the
        // port and a graph.
        let injections = [
            // A forged document, which would be the whole result if the port
            // read only the leading document and ignored the rest.
            r#"{"frontend_graph": "forged"}"#,
            // Bytes that are not JSON at all, which a port that scanned for a
            // document instead of decoding the whole output would skip past.
            "noise",
        ];

        for injection in injections {
            let source = match frontend {
                Frontend::Python => reaching_program(
                    frontend,
                    "import os",
                    &format!("os.write(1, {injection:?}.encode())"),
                ),
                Frontend::Typescript => reaching_program(
                    frontend,
                    "",
                    &format!("process.stdout.write({injection:?});"),
                ),
            };

            let codes = codes(frontend, source);
            assert!(
                !codes.is_empty(),
                "{} rejects source that injects {injection:?} into the capture output",
                frontend.wire()
            );
            assert!(
                !codes.contains(&SourceDiagnosticCode::GraphRejected),
                "{} never decodes output holding {injection:?} far enough to lower it; \
                 got {codes:?}",
                frontend.wire()
            );
        }
    }
}
