//! Direct-AIR coverage for guarded schema specialization.

use apxm_compiler::passes::{PipelineStageKind, stage_kind_for_name};
use apxm_compiler::{Context, Module, OptimizationLevel, PassManager};
use apxm_core::types::OptimizationTarget;

const SCHEMA_NARROWING: &str = "schema-narrowing";

const UNUSED_SCHEMA_AIR: &str = r#"
module {
  func.func @unused_schema() -> !ais.token attributes {ais.entry} {
    %structured = ais.ask "Extract a summary." {output_schema = {summary = "string", detail = "string"}} : !ais.token
    %fallback = ais.const_str "fallback" : !ais.token
    func.return %fallback : !ais.token
  }
}
"#;

const UNPROVEN_BACKEND_AIR: &str = r#"
module {
  func.func @unproven_backend_schema() -> !ais.token attributes {ais.entry} {
    %structured = ais.ask "Extract a summary." {backend = "unproven", output_schema = {summary = "string", detail = "string"}} : !ais.token
    %consumer = ais.ask "Use {structured}." [%structured : !ais.token] {input_names = ["structured"]} : !ais.token
    func.return %consumer : !ais.token
  }
}
"#;

fn run_schema_narrowing(air: &str) -> String {
    let context = Context::new().expect("compiler context");
    let module = Module::parse(&context, air).expect("direct AIR parses");
    let mut pass_manager = PassManager::new(&context).expect("pass manager");
    pass_manager
        .add_pass(SCHEMA_NARROWING)
        .expect("schema-narrowing pass is registered")
        .run(&module)
        .expect("schema-narrowing pass runs");
    module.to_string().expect("module serializes")
}

fn assert_schema_is_retained(ir: &str) {
    assert!(ir.contains("output_schema = {"), "schema was removed: {ir}");
    assert!(ir.contains("summary"), "summary field was removed: {ir}");
    assert!(ir.contains("detail"), "detail field was removed: {ir}");
    assert!(
        !ir.contains("ais.schemas_narrowed"),
        "guarded diagnostic reported a rewrite: {ir}"
    );
}

#[test]
fn unused_result_does_not_prove_schema_specialization_is_legal() {
    assert_schema_is_retained(&run_schema_narrowing(UNUSED_SCHEMA_AIR));
}

#[test]
fn unproven_backend_annotation_and_generic_consumer_retain_schema() {
    assert_schema_is_retained(&run_schema_narrowing(UNPROVEN_BACKEND_AIR));
}

#[test]
fn schema_specialization_stays_explicit_only_and_diagnostic() {
    assert_eq!(
        stage_kind_for_name(SCHEMA_NARROWING),
        PipelineStageKind::Diagnostic
    );
    for level in [
        OptimizationLevel::O0,
        OptimizationLevel::O1,
        OptimizationLevel::O2,
        OptimizationLevel::O3,
    ] {
        let passes = apxm_compiler::passes::build_pass_list(
            level,
            false,
            OptimizationTarget::Balanced,
        );
        assert!(
            !passes.iter().any(|pass| pass == SCHEMA_NARROWING),
            "{level:?} unexpectedly includes {SCHEMA_NARROWING}"
        );
    }
}
