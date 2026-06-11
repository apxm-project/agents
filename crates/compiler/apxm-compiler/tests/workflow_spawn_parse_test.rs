use apxm_compiler::{Context, Pipeline};

#[test]
fn pipeline_compiles_workflow_spawn_air() {
    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::new(&context);
    let air = r#"
module {
  func.func @workflow_spawn_parse() -> !ais.token attributes {ais.entry} {
    %child = ais.workflow_spawn "workflow_path" "workflows/review.apxmw"
      {await_result = true, session_root = ".apxm/child-sessions"} : !ais.token
    func.return %child : !ais.token
  }
}
"#;

    let module = pipeline.compile(air).expect("compile workflow_spawn AIR");
    let rendered = module.to_string().expect("render compiled module");
    assert!(rendered.contains("ais.workflow_spawn"));
}
