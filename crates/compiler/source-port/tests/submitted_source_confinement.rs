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
             from apxm_program import Workflow, Model\n\
             \n\
             {reach}\n\
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
             ReviewModel = Model[ReviewRequest, Review](\"review.model\")\n\
             \n\
             \n\
             @Workflow(input=ReviewRequest, output=Review)\n\
             async def Reviewer(agent, request):\n\
             \x20   return await ReviewModel(request)\n"
        ),
        Frontend::Typescript => format!(
            "{import}\n\
             import {{ Workflow, Model }} from \"@apxm/frontend\";\n\
             import {{ source }} from \"@apxm/frontend/node\";\n\
             \n\
             source(import.meta.url);\n\
             \n\
             {reach}\n\
             \n\
             type ReviewRequest = object;\n\
             type Review = object;\n\
             \n\
             const ReviewModel = Model<ReviewRequest, Review>(\"review.model\");\n\
             \n\
             export const Reviewer = Workflow<ReviewRequest, Review>({{\n\
             \x20 name: \"Reviewer\",\n\
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

/// The Python capture process installs OS resource ceilings in addition to
/// its audit wall. Keep this assertion in the hostile-source suite so a
/// future interpreter upgrade cannot silently remove the process and file
/// descriptor budgets while the import/event tests continue to pass.
#[cfg(unix)]
#[test]
fn submitted_python_runs_under_closed_resource_ceilings() {
    if !frontend_present(Frontend::Python) {
        return;
    }
    let source = reaching_program(
        Frontend::Python,
        "import resource",
        "assert resource.getrlimit(resource.RLIMIT_NOFILE)[0] == 64\n\
         assert resource.getrlimit(resource.RLIMIT_NPROC)[0] == 0",
    );
    let request = SourceBundleRequest::new(Frontend::Python, ENTRYPOINT, source);
    compile_source_bundle(&request, &roots(), &drivers()).unwrap_or_else(|diagnostics| {
        panic!("Python source must observe the installed resource ceilings; got {diagnostics:?}")
    });
}

/// Ambient Node globals must not provide a network escape hatch when no module
/// is named. The harness removes both fetch and process before evaluation.
#[test]
fn submitted_typescript_cannot_reach_network_without_a_module() {
    if !frontend_present(Frontend::Typescript) {
        return;
    }
    let source = reaching_program(
        Frontend::Typescript,
        "",
        "if (typeof fetch === \"function\" || (globalThis as any).process !== undefined) {\n\
           throw new Error(\"network/process globals are ambient\");\n\
         }",
    );
    let request = SourceBundleRequest::new(Frontend::Typescript, ENTRYPOINT, source);
    let compiled = compile_source_bundle(&request, &roots(), &drivers()).unwrap_or_else(|diagnostics| {
        panic!(
            "network/process globals must be unavailable to submitted TypeScript; got {diagnostics:?}"
        )
    });
    assert_eq!(compiled.frontend_graph.call_intents.len(), 1);
}

/// Worker threads must not become a second, less-confined execution surface.
/// The harness uses synchronous loader hooks, so submitted code attempting to
/// create a worker with widened permissions is rejected before it can run.
#[test]
fn submitted_typescript_cannot_start_a_privileged_worker() {
    if !frontend_present(Frontend::Typescript) {
        return;
    }
    let witness = std::env::temp_dir().join(format!(
        "apxm-source-port-worker-witness-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&witness);
    let source = reaching_program(
        Frontend::Typescript,
        "",
        &format!(
            "const workerThreads = (globalThis as {{ process: any }}).process.getBuiltinModule(\"worker_threads\");\n\
             new workerThreads.Worker(\"const fs = process.getBuiltinModule('fs'); fs.writeFileSync({:?}, 'escaped');\", {{ eval: true, execArgv: [\"--permission\", \"--allow-fs-write=/\"] }});",
            witness.display().to_string()
        ),
    );
    let diagnostics = codes(Frontend::Typescript, source);
    assert!(
        diagnostics.contains(&SourceDiagnosticCode::SourceRejected),
        "privileged worker creation must be rejected; got {diagnostics:?}"
    );
    assert!(
        !witness.exists(),
        "submitted worker wrote {}",
        witness.display()
    );
}

/// Capture and graph emission share the child Node process with submitted
/// source, so collection prototypes are part of the trusted boundary. A
/// source-level replacement of Array#map must fail closed before it can alter
/// the graph fold's traversal.
#[test]
fn submitted_typescript_cannot_poison_capture_intrinsics() {
    if !frontend_present(Frontend::Typescript) {
        return;
    }
    let source = reaching_program(
        Frontend::Typescript,
        "",
        "Array.prototype.map = (() => []) as typeof Array.prototype.map;",
    );
    let diagnostics = codes(Frontend::Typescript, source);
    assert!(
        diagnostics.contains(&SourceDiagnosticCode::SourceRejected),
        "prototype poisoning must reject before capture; got {diagnostics:?}"
    );
}

/// The evaluated frontend must not expose a mutable graph that can be
/// rewritten after capture. Otherwise submitted source could replace a
/// declaration or model target and have the forged graph lowered as canonical.
#[test]
fn submitted_source_cannot_mutate_the_captured_graph() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let mut source = reaching_program(frontend, "", "");
        source.push_str(match frontend {
            Frontend::Python => {
                "\ngraph = Reviewer.frontend_graph()\n\
                 graph[\"declarations\"][0][\"target_ref\"] = \"evil.model\"\n\
                 graph[\"model_requirements\"][0][\"model_target_ref\"] = \"evil.model\"\n"
            }
            Frontend::Typescript => {
                "\nconst graph = Reviewer.frontendGraph() as any;\n\
                 graph.declarations[0].target_ref = \"evil.model\";\n\
                 graph.model_requirements[0].model_target_ref = \"evil.model\";\n"
            }
        });
        let request = SourceBundleRequest::new(frontend, ENTRYPOINT, source);
        match compile_source_bundle(&request, &roots(), &drivers()) {
            Ok(compiled) => {
                assert!(
                    compiled
                        .frontend_graph
                        .declarations
                        .iter()
                        .all(|declaration| declaration.target_ref.as_deref() != Some("evil.model")),
                    "mutating a captured {} graph forged a declaration",
                    frontend.wire()
                );
                assert!(
                    compiled
                        .frontend_graph
                        .model_requirements
                        .iter()
                        .all(|requirement| requirement.model_target_ref != "evil.model"),
                    "mutating a captured {} graph forged a model requirement",
                    frontend.wire()
                );
            }
            Err(diagnostics) => assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == SourceDiagnosticCode::SourceRejected),
                "mutating a captured {} graph must reject or preserve the graph; got {diagnostics:?}",
                frontend.wire()
            ),
        }
    }
}

/// The empty-object contract is compiler-owned. Submitted source may inspect
/// its captured graph, but it cannot add the proof marker before the trusted
/// source-port service turns that graph into an artifact.
#[test]
fn submitted_source_cannot_forge_input_contract() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let mut source = reaching_program(frontend, "", "");
        source.push_str(match frontend {
            Frontend::Python => {
                "\ngraph = Reviewer.frontend_graph()\n\
                 graph[\"program_definitions\"][0][\"input_contract\"] = \"accepts_empty_object\"\n"
            }
            Frontend::Typescript => {
                "\nconst graph = Reviewer.frontendGraph() as any;\n\
                 graph.program_definitions[0].input_contract = \"accepts_empty_object\";\n"
            }
        });
        let request = SourceBundleRequest::new(frontend, ENTRYPOINT, source);
        match compile_source_bundle(&request, &roots(), &drivers()) {
            Ok(compiled) => assert!(
                compiled.frontend_graph.program_definitions[0]
                    .input_contract
                    .is_none(),
                "submitted {} source forged the compiler-owned input contract",
                frontend.wire()
            ),
            Err(diagnostics) => assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == SourceDiagnosticCode::SourceRejected),
                "forging the {} input contract must reject or preserve the graph; got {diagnostics:?}",
                frontend.wire()
            ),
        }
    }
}

#[test]
fn submitted_source_cannot_replace_compiler_owned_entrypoint_metadata() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let mut source = reaching_program(frontend, "", "");
        source.push_str(match frontend {
            Frontend::Python => "\ngraph = Reviewer.frontend_graph()\ngraph[\"program_definitions\"][0].update({\"input_schema\": {\"type\": \"string\"}, \"default_context\": {\"kind\": \"string\", \"value\": \"forged\"}, \"authoring\": {\"kind\": \"agent\", \"primary_model_ref\": \"forged.model\"}})\n",
            Frontend::Typescript => "\nconst graph = Reviewer.frontendGraph() as any;\nObject.assign(graph.program_definitions[0], {input_schema: {type: \"string\"}, default_context: {kind: \"string\", value: \"forged\"}, authoring: {kind: \"agent\", primary_model_ref: \"forged.model\"}});\n",
        });
        match compile_source_bundle(
            &SourceBundleRequest::new(frontend, ENTRYPOINT, source),
            &roots(),
            &drivers(),
        ) {
            Ok(compiled) => {
                let definition = &compiled.frontend_graph.program_definitions[0];
                assert!(definition.input_schema.is_none());
                assert!(definition.default_context.is_none());
                assert_eq!(
                    definition.authoring,
                    Some(apxm_program::frontend_graph::ProgramAuthoring::Workflow {})
                );
            }
            Err(diagnostics) => assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == SourceDiagnosticCode::SourceRejected)
            ),
        }
    }
}

/// A marker record is part of the capture input before the Workflow is built. Its
/// public TypeScript fields are readonly only to the type checker, so the
/// runtime record must also reject an `any`-cast mutation.
#[test]
fn submitted_typescript_cannot_mutate_marker_before_capture() {
    if !frontend_present(Frontend::Typescript) {
        return;
    }
    let source = reaching_program(Frontend::Typescript, "", "").replace(
        "const ReviewModel = Model<ReviewRequest, Review>(\"review.model\");",
        "const ReviewModel = Model<ReviewRequest, Review>(\"review.model\");\n\
         (ReviewModel as any).targetRef = \"evil.model\";",
    );
    let diagnostics = codes(Frontend::Typescript, source);
    assert!(
        diagnostics.contains(&SourceDiagnosticCode::SourceRejected),
        "a TypeScript marker mutation must reject before capture; got {diagnostics:?}"
    );
}

/// Python's ``_capture`` module is visible to submitted source, but its source
/// reader must not consult a module-global bridge that source can replace. The
/// reader closes over the native source sealed by the harness, so a forged
/// callback body cannot change the captured intent.
#[test]
fn submitted_python_cannot_redirect_the_capture_source_bridge() {
    if !frontend_present(Frontend::Python) {
        return;
    }

    let original = reaching_program(Frontend::Python, "", "");
    let forged_callback = original.replace("return await ReviewModel(request)", "return request");
    // The callback's code object carries its source line. Keep the forged
    // bundle's callback at the same line after the six-line attack prelude.
    let forged_bundle = format!("{}\n{}", "\n".repeat(6), forged_callback);
    let attack = format!(
        "import apxm_program._capture as capture\n\
         class _FakeBridge:\n\
         \x20   @staticmethod\n\
         \x20   def authored_source():\n\
         \x20       return {forged_bundle:?}\n\
         capture._native_bridge = _FakeBridge\n"
    );
    let source = format!("{attack}{original}");
    let request = SourceBundleRequest::new(Frontend::Python, ENTRYPOINT, source);
    match compile_source_bundle(&request, &roots(), &drivers()) {
        Ok(compiled) => assert!(
            compiled
                .frontend_graph
                .model_requirements
                .iter()
                .any(|requirement| requirement.model_target_ref == "review.model"),
            "redirecting the Python capture bridge changed the captured graph"
        ),
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == SourceDiagnosticCode::SourceRejected),
            "redirecting the Python capture bridge must reject or preserve the graph; got {diagnostics:?}"
        ),
    }
}

/// Python marker records are immutable at the representation level, not only
/// through a dataclass setter. ``object.__setattr__`` must not be able to alter
/// a capability/model target before the Workflow decorator captures bindings.
#[test]
fn submitted_python_cannot_mutate_marker_before_capture() {
    if !frontend_present(Frontend::Python) {
        return;
    }
    let source = reaching_program(Frontend::Python, "", "").replace(
        "ReviewModel = Model[ReviewRequest, Review](\"review.model\")",
        "ReviewModel = Model[ReviewRequest, Review](\"review.model\")\n\
         object.__setattr__(ReviewModel, \"target_ref\", \"evil.model\")",
    );
    let diagnostics = codes(Frontend::Python, source);
    assert!(
        diagnostics.contains(&SourceDiagnosticCode::SourceRejected),
        "a Python marker mutation must reject before capture; got {diagnostics:?}"
    );
}

/// A captured handle's graph method must not be replaceable through its
/// prototype/class after static capture. The port asks the handle for its
/// graph only after module evaluation, so dispatch itself is part of the
/// boundary.
#[test]
fn submitted_source_cannot_replace_graph_capture_method() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let mut source = reaching_program(frontend, "", "");
        source.push_str(match frontend {
            Frontend::Python => {
                "\n_original_graph = Reviewer.frontend_graph\n\
                 def _forged_graph(_self):\n\
                      graph = _original_graph()\n\
                      graph[\"declarations\"][0][\"target_ref\"] = \"evil.model\"\n\
                      graph[\"model_requirements\"][0][\"model_target_ref\"] = \"evil.model\"\n\
                      return graph\n\
                 type(Reviewer).frontend_graph = _forged_graph\n"
            }
            Frontend::Typescript => {
                "\nconst _originalGraph = Reviewer.frontendGraph.bind(Reviewer);\n\
                 (Object.getPrototypeOf(Reviewer) as any).frontendGraph = () => {\n\
                    const graph = _originalGraph() as any;\n\
                    graph.declarations[0].target_ref = \"evil.model\";\n\
                    graph.model_requirements[0].model_target_ref = \"evil.model\";\n\
                    return graph;\n\
                 };\n"
            }
        });
        let request = SourceBundleRequest::new(frontend, ENTRYPOINT, source);
        match compile_source_bundle(&request, &roots(), &drivers()) {
            Ok(compiled) => assert!(
                compiled
                    .frontend_graph
                    .model_requirements
                    .iter()
                    .all(|requirement| requirement.model_target_ref != "evil.model"),
                "prototype replacement forged a {} graph",
                frontend.wire()
            ),
            Err(diagnostics) => assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == SourceDiagnosticCode::SourceRejected),
                "prototype replacement must reject or preserve the {} graph; got {diagnostics:?}",
                frontend.wire()
            ),
        }
    }
}

/// The capture implementation must not depend on mutable global helpers or
/// source-visible storage slots. These are ordinary reflection attempts, not
/// native-process escapes, and must still preserve the captured graph.
#[test]
fn submitted_source_cannot_replace_graph_capture_intrinsics() {
    for frontend in FRONTENDS {
        if !frontend_present(frontend) {
            continue;
        }
        let mut source = reaching_program(frontend, "", "");
        source.push_str(match frontend {
            Frontend::Python => {
                "\n_original_graph = Reviewer.frontend_graph()\n\
                 _forged = dict(_original_graph)\n\
                 _forged[\"declarations\"] = [dict(_original_graph[\"declarations\"][0], target_ref=\"evil.model\")]\n\
                 object.__setattr__(Reviewer, \"_graph_json\", __import__(\"json\").dumps(_forged))\n"
            }
            Frontend::Typescript => {
                "\nconst _originalParse = JSON.parse;\n\
                 JSON.parse = ((text: string) => {\n\
                    const graph = _originalParse(text) as any;\n\
                    if (graph?.declarations?.[0]) graph.declarations[0].target_ref = \"evil.model\";\n\
                    if (graph?.model_requirements?.[0]) graph.model_requirements[0].model_target_ref = \"evil.model\";\n\
                    return graph;\n\
                 }) as typeof JSON.parse;\n"
            }
        });
        let request = SourceBundleRequest::new(frontend, ENTRYPOINT, source);
        match compile_source_bundle(&request, &roots(), &drivers()) {
            Ok(compiled) => assert!(
                compiled
                    .frontend_graph
                    .model_requirements
                    .iter()
                    .all(|requirement| requirement.model_target_ref != "evil.model"),
                "intrinsic replacement forged a {} graph",
                frontend.wire()
            ),
            Err(diagnostics) => assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == SourceDiagnosticCode::SourceRejected),
                "intrinsic replacement must reject or preserve the {} graph; got {diagnostics:?}",
                frontend.wire()
            ),
        }
    }
}

/// The TypeScript harness serializes after submitted module evaluation. A
/// source replacement of the mutable global JSON serializer must not be able
/// to swap the captured graph for one it forged from a mutable clone.
#[test]
fn submitted_typescript_cannot_replace_output_serializer() {
    let frontend = Frontend::Typescript;
    if !frontend_present(frontend) {
        return;
    }
    let mut source = reaching_program(frontend, "", "");
    source.push_str(
        "\nconst _forgedGraph = Reviewer.frontendGraph() as any;\n\
         _forgedGraph.declarations[0].target_ref = \"evil.model\";\n\
         _forgedGraph.model_requirements[0].model_target_ref = \"evil.model\";\n\
         const _trustedStringify = JSON.stringify;\n\
         JSON.stringify = (() => _trustedStringify({ frontend_graph: _forgedGraph })) as typeof JSON.stringify;\n",
    );
    let request = SourceBundleRequest::new(frontend, ENTRYPOINT, source);
    match compile_source_bundle(&request, &roots(), &drivers()) {
        Ok(compiled) => {
            assert!(
                compiled
                    .frontend_graph
                    .declarations
                    .iter()
                    .all(|declaration| declaration.target_ref.as_deref() != Some("evil.model")),
                "replacing the TypeScript output serializer forged a declaration"
            );
            assert!(
                compiled
                    .frontend_graph
                    .model_requirements
                    .iter()
                    .all(|requirement| requirement.model_target_ref != "evil.model"),
                "replacing the TypeScript output serializer forged a model requirement"
            );
        }
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == SourceDiagnosticCode::SourceRejected),
            "replacing the TypeScript output serializer must reject or preserve the graph; got {diagnostics:?}"
        ),
    }
}

/// Python's function defaults and linecache are both process-visible mutable
/// state. They must not be the source-port's graph or source authority: the
/// native bridge seals both values before submitted code is evaluated.
#[test]
fn submitted_python_cannot_forge_through_defaults_or_linecache() {
    let frontend = Frontend::Python;
    if !frontend_present(frontend) {
        return;
    }
    let source = reaching_program(
        frontend,
        "import linecache\nfrom apxm_program._agent import AgentDefinition",
        "linecache.cache[\"submitted_source.py\"] = (0, None, [\"evil = True\\n\"], \"submitted_source.py\")\n\
         _forged = {\"declarations\": [{\"target_ref\": \"evil.model\"}]}\n\
         AgentDefinition.frontend_graph.__defaults__ = ({Reviewer: __import__(\"json\").dumps(_forged)}, __import__(\"json\").loads)",
    );
    let request = SourceBundleRequest::new(frontend, ENTRYPOINT, source);
    match compile_source_bundle(&request, &roots(), &drivers()) {
        Ok(compiled) => assert!(
            compiled
                .frontend_graph
                .declarations
                .iter()
                .all(|declaration| declaration.target_ref.as_deref() != Some("evil.model")),
            "Python defaults or linecache forged the graph"
        ),
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == SourceDiagnosticCode::SourceRejected),
            "Python defaults or linecache must reject or preserve the graph; got {diagnostics:?}"
        ),
    }
}

/// Python module globals remain writable even when the native graph bridge is
/// sealed. Redirecting the capture helper must therefore fail closed rather
/// than selecting a second source authority.
#[test]
fn submitted_python_cannot_redirect_capture_module() {
    let frontend = Frontend::Python;
    if !frontend_present(frontend) {
        return;
    }
    let source = reaching_program(
        frontend,
        "import apxm_program._capture as capture",
        "capture._source_for_function = lambda _func: \"async def Reviewer(agent, request):\\n    return request\\n\"",
    );
    let diagnostics = codes(frontend, source);
    assert!(
        diagnostics.contains(&SourceDiagnosticCode::SourceRejected),
        "redirecting the Python capture module must reject; got {diagnostics:?}"
    );
}

/// A source-visible replacement of the native module attribute must not affect
/// the C function object the harness captured before evaluation.
#[test]
fn submitted_python_cannot_redirect_native_graph_reader() {
    let frontend = Frontend::Python;
    if !frontend_present(frontend) {
        return;
    }
    let source = reaching_program(
        frontend,
        "import apxm_program._native as native",
        "native.frontend_graph = lambda _owner: {\"model_requirements\": [{\"model_target_ref\": \"evil.model\"}]}",
    );
    let request = SourceBundleRequest::new(frontend, ENTRYPOINT, source);
    match compile_source_bundle(&request, &roots(), &drivers()) {
        Ok(compiled) => assert!(
            compiled
                .frontend_graph
                .model_requirements
                .iter()
                .all(|requirement| requirement.model_target_ref != "evil.model"),
            "redirecting the native graph module attribute forged a graph"
        ),
        Err(diagnostics) => assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == SourceDiagnosticCode::SourceRejected),
            "redirecting the native graph module attribute must reject or preserve; got {diagnostics:?}"
        ),
    }
}

/// A source can replace the Python marker factory's call implementation before
/// declarations are built. The original source contract still owns the model
/// target, so this redirection must not reach the accepted graph.
#[test]
fn submitted_python_cannot_redirect_marker_factory() {
    let frontend = Frontend::Python;
    if !frontend_present(frontend) {
        return;
    }
    let source = reaching_program(
        frontend,
        "import apxm_program._markers as markers",
        "type.__setattr__(markers._ModelFactory, \"__call__\", lambda _self, _ref: markers.ModelBinding(\"review.model\", \"evil.input\", \"evil.output\"))",
    );
    let diagnostics = codes(frontend, source);
    assert!(
        diagnostics.contains(&SourceDiagnosticCode::SourceRejected),
        "redirecting the Python marker factory must reject; got {diagnostics:?}"
    );
}

/// A source module can temporarily replace the Python Workflow emitter and restore
/// the module attribute before the post-evaluation integrity snapshot runs.
/// The source contract is independent of that mutable dispatch path: a forged
/// capability target must not survive the source-port boundary.
#[test]
fn submitted_python_cannot_temporarily_forge_a_capability_target() {
    if !frontend_present(Frontend::Python) {
        return;
    }
    let source = r#"
import __main__
import apxm_program._agent as implementation
from apxm_program import Workflow, Capability

class ReviewRequest:
    pass

class Review:
    pass

READ_ID = "read"
ReviewCapability = Capability[ReviewRequest, Review](READ_ID)

original_capture = implementation.capture_program
original_emit = implementation.emit_frontend_graph

def forged_capture(*args, **kwargs):
    return original_capture(*args, **kwargs)

def forged_emit(program):
    graph = original_emit(program)
    for declaration in graph["declarations"]:
        if declaration.get("decl_kind") == "capability_binding":
            declaration["target_ref"] = "evil.capability"
    for requirement in graph["capability_requirements"]:
        requirement["capability_ref"] = "evil.capability"
    return graph

implementation.capture_program = forged_capture
implementation.emit_frontend_graph = forged_emit
__main__._validate_graph_provenance = lambda *_args: None
__main__._json_dump = lambda *_args, **_kwargs: None
__main__.sys.stdout = None
__main__.sys.stderr = None

@Workflow(input=ReviewRequest, output=Review)
async def Reviewer(agent, request):
    return await ReviewCapability(request)

implementation.capture_program = original_capture
implementation.emit_frontend_graph = original_emit
"#;
    let diagnostics = codes(Frontend::Python, source.to_string());
    assert!(
        diagnostics.contains(&SourceDiagnosticCode::SourceRejected),
        "temporary emitter replacement must reject a forged capability target; got {diagnostics:?}"
    );
}

/// Frame objects expose the harness call stack, including post-evaluation
/// locals that hold trusted validators and serializers. The Python audit wall
/// rejects this introspection instead of relying on interpreter-version-
/// dependent ``frame.f_locals`` semantics.
#[test]
fn submitted_python_cannot_reach_harness_frames() {
    if !frontend_present(Frontend::Python) {
        return;
    }
    let source = reaching_program(Frontend::Python, "import sys", "_frame = sys._getframe()");
    let diagnostics = codes(Frontend::Python, source);
    assert!(
        diagnostics.contains(&SourceDiagnosticCode::SourceRejected),
        "Python frame introspection must reject; got {diagnostics:?}"
    );
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
