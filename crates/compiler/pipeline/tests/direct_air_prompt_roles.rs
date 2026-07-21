//! Direct-AIR artifact validation for typed prompt input roles.

use apxm_compiler::{AirEdge, AirModule, AirNode, Context, OptimizationLevel, Pipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, DependencyType, Value};
use std::collections::HashMap;

const UNKNOWN_ROLE_AIR: &str = r#"
module {
  func.func @unknown_prompt_role() -> !ais.token attributes {ais.entry} {
    %source = ais.ask "Source material." : !ais.token
    %answer = ais.ask "Answer {source}." [%source : !ais.token] {input_names = ["source"], input_roles = ["assistant"]} : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

const MISMATCHED_ROLE_AIR: &str = r#"
module {
  func.func @mismatched_prompt_roles() -> !ais.token attributes {ais.entry} {
    %source = ais.ask "Source material." : !ais.token
    %answer = ais.ask "Answer {source}." [%source : !ais.token] {input_names = ["source"], input_roles = ["user", "system"]} : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

const MISSING_ROLE_AIR: &str = r#"
module {
  func.func @missing_prompt_role() -> !ais.token attributes {ais.entry} {
    %source = ais.ask "Source material." : !ais.token
    %answer = ais.ask "Answer {source}." [%source : !ais.token] {input_names = ["source"]} : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

const NON_ARRAY_ROLE_AIR: &str = r#"
module {
  func.func @non_array_prompt_role() -> !ais.token attributes {ais.entry} {
    %source = ais.ask "Source material." : !ais.token
    %answer = ais.ask "Answer {source}." [%source : !ais.token] {input_names = ["source"], input_roles = "user"} : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

const PROTECTED_PLACEHOLDER_AIR: &str = r#"
module {
  func.func @protected_placeholder() -> !ais.token attributes {ais.entry} {
    %system = ais.ask "System instructions." : !ais.token
    %answer = ais.ask "Answer {system}." [%system : !ais.token] {input_names = ["system"], input_roles = ["system"]} : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

const PROTECTED_ROLE_PRUNING_AIR: &str = r#"
module {
  func.func @protected_role_pruning() -> !ais.token attributes {ais.entry} {
    %subject = ais.ask "Subject." : !ais.token
    %unused_user = ais.ask "Unused user input." : !ais.token
    %system = ais.ask "System instructions." : !ais.token
    %dependency = ais.ask "Dependency state." : !ais.token
    %tool = ais.ask "Tool context." : !ais.token
    %control = ais.ask "Control state." : !ais.token
    %answer = ais.ask "Answer {subject}." [%subject, %unused_user, %system, %dependency, %tool, %control : !ais.token, !ais.token, !ais.token, !ais.token, !ais.token, !ais.token] {input_names = ["subject", "unused_user", "system", "dependency", "tool", "control"], input_roles = ["user", "user", "system", "dependency_only", "tool_context", "control"]} : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

const SPECIALIZED_ROLE_AIR: &str = r#"
module {
  func.func @specialized_prompt_role() -> !ais.token attributes {ais.entry} {
    %message = ais.const_str "release status" : !ais.token
    %answer = ais.ask "Summarize {message}." [%message : !ais.token] {input_names = ["message"], input_roles = ["user"]} : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

const PROTECTED_SPECIALIZATION_AIR: &str = r#"
module {
  func.func @protected_specialization_role() -> !ais.token attributes {ais.entry} {
    %system = ais.const_str "Follow the deployment policy." : !ais.token
    %message = ais.const_str "release status" : !ais.token
    %answer = ais.ask "Summarize {message}." [%system, %message : !ais.token, !ais.token] {input_names = ["system", "message"], input_roles = ["system", "user"]} : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

const SHARED_PROMPT_ROLE_FIXTURE_AIR: &str = r#"
module {
  func.func @prompt_role_flow() -> !ais.token attributes {ais.entry} {
    %question = ais.ask "Question." : !ais.token
    %policy = ais.ask "Policy." : !ais.token
    %dependency = ais.ask "Dependency." : !ais.token
    %tool_result = ais.ask "Tool result." : !ais.token
    %guard = ais.ask "Guard." : !ais.token
    %answer = ais.ask "Answer {question}" [%question, %policy, %dependency, %tool_result, %guard : !ais.token, !ais.token, !ais.token, !ais.token, !ais.token] {input_names = ["question", "policy", "dependency", "tool_result", "guard"], input_roles = ["user", "system", "dependency_only", "tool_context", "control"]} : !ais.token
    func.return %answer : !ais.token
  }
}
"#;

#[test]
fn shared_prompt_role_fixture_compiles_through_direct_air() {
    let context = Context::new().expect("compiler context");
    let module = Pipeline::with_opt_level(&context, OptimizationLevel::O0)
        .compile(SHARED_PROMPT_ROLE_FIXTURE_AIR)
        .expect("shared frontend role fixture compiles through direct AIR");
    let ir = module.to_string().expect("direct AIR module serializes");
    let answer = ir
        .lines()
        .find(|line| line.contains("Answer {question}"))
        .expect("shared fixture answer operation remains");

    assert!(answer.contains(
        "input_names = [\"question\", \"policy\", \"dependency\", \"tool_result\", \"guard\"]"
    ));
    assert!(answer.contains(
        "input_roles = [\"user\", \"system\", \"dependency_only\", \"tool_context\", \"control\"]"
    ));
    module
        .generate_artifact()
        .expect("shared fixture prompt-role artifact validates");
}

#[test]
fn direct_air_rejects_unknown_prompt_roles_before_artifact_publication() {
    let context = Context::new().expect("compiler context");
    let module = Pipeline::with_opt_level(&context, OptimizationLevel::O0)
        .compile(UNKNOWN_ROLE_AIR)
        .expect("direct AIR compiles before artifact validation");

    let error = module
        .generate_artifact()
        .expect_err("unknown prompt role must reject the artifact");
    let message = error.to_string();
    assert!(message.contains(graph_attrs::INPUT_ROLES));
    assert!(message.contains("assistant"));
}

#[test]
fn direct_air_rejects_prompt_role_arrays_with_mismatched_arity() {
    let context = Context::new().expect("compiler context");
    let module = Pipeline::with_opt_level(&context, OptimizationLevel::O0)
        .compile(MISMATCHED_ROLE_AIR)
        .expect("direct AIR compiles before artifact validation");

    let error = module
        .generate_artifact()
        .expect_err("mismatched prompt role arrays must reject the artifact");
    let message = error.to_string();
    assert!(message.contains(graph_attrs::INPUT_ROLES));
    assert!(message.contains(graph_attrs::INPUT_NAMES));
}

#[test]
fn direct_air_rejects_context_bearing_llm_nodes_without_roles() {
    let context = Context::new().expect("compiler context");
    let module = Pipeline::with_opt_level(&context, OptimizationLevel::O0)
        .compile(MISSING_ROLE_AIR)
        .expect("direct AIR compiles before artifact validation");

    let error = module
        .generate_artifact()
        .expect_err("context-bearing LLM nodes require explicit roles");
    let message = error.to_string();
    assert!(message.contains(graph_attrs::INPUT_ROLES));
    assert!(message.contains("explicit positional"));
}

#[test]
fn direct_air_rejects_non_array_prompt_roles() {
    let context = Context::new().expect("compiler context");
    match Pipeline::with_opt_level(&context, OptimizationLevel::O0).compile(NON_ARRAY_ROLE_AIR) {
        Err(_) => {}
        Ok(_) => panic!("MLIR must reject scalar prompt-role metadata"),
    }
}

#[test]
fn direct_air_rejects_protected_role_placeholders() {
    let context = Context::new().expect("compiler context");
    let module = Pipeline::with_opt_level(&context, OptimizationLevel::O0)
        .compile(PROTECTED_PLACEHOLDER_AIR)
        .expect("direct AIR compiles before artifact validation");

    let error = module
        .generate_artifact()
        .expect_err("protected context cannot enter the user template channel");
    let message = error.to_string();
    assert!(message.contains("only user-role inputs"));
    assert!(message.contains("system"));
}

#[test]
fn o3_prunes_only_unused_user_role_inputs_and_retains_protected_roles() {
    let context = Context::new().expect("compiler context");
    let module = Pipeline::with_opt_level(&context, OptimizationLevel::O3)
        .compile(PROTECTED_ROLE_PRUNING_AIR)
        .expect("O3 pipeline compiles protected prompt roles");
    let ir = module.to_string().expect("optimized module serializes");

    let answer = ir
        .lines()
        .find(|line| line.contains("Answer {subject}."))
        .expect("answer operation remains");
    assert!(answer.contains(
        "input_names = [\"subject\", \"system\", \"dependency\", \"tool\", \"control\"]"
    ));
    assert!(answer.contains(
        "input_roles = [\"user\", \"system\", \"dependency_only\", \"tool_context\", \"control\"]"
    ));
    assert!(!answer.contains("unused_user"));
}

#[test]
fn o1_template_specialization_keeps_prompt_role_arity_aligned() {
    let context = Context::new().expect("compiler context");
    let module = Pipeline::with_opt_level(&context, OptimizationLevel::O1)
        .compile(SPECIALIZED_ROLE_AIR)
        .expect("O1 pipeline specializes the constant prompt input");
    let ir = module.to_string().expect("optimized module serializes");
    let answer = ir
        .lines()
        .find(|line| line.contains("Summarize release status."))
        .expect("specialized answer operation remains");

    assert!(answer.contains("input_names = []"));
    assert!(answer.contains("input_roles = []"));
    module
        .generate_artifact()
        .expect("specialized prompt-role contract remains executable");
}

#[test]
fn o1_template_specialization_preserves_protected_prompt_channels() {
    let context = Context::new().expect("compiler context");
    let module = Pipeline::with_opt_level(&context, OptimizationLevel::O1)
        .compile(PROTECTED_SPECIALIZATION_AIR)
        .expect("O1 pipeline preserves protected prompt roles");
    let ir = module.to_string().expect("optimized module serializes");
    let answer = ir
        .lines()
        .find(|line| line.contains("Summarize release status."))
        .expect("protected answer operation remains");

    assert!(answer.contains("input_names = [\"system\"]"));
    assert!(answer.contains("input_roles = [\"system\"]"));
    module
        .generate_artifact()
        .expect("protected prompt-role contract remains executable");
}

#[test]
fn programmatic_air_specialization_keeps_materialized_prompt_roles_aligned() {
    let graph = AirModule {
        name: "programmatic_prompt_roles".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "message".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    graph_attrs::VALUE.to_string(),
                    Value::String("release status".to_string()),
                )]),
            },
            AirNode {
                id: 2,
                name: "answer".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([
                    (
                        graph_attrs::TEMPLATE_STR.to_string(),
                        Value::String("Summarize {message}.".to_string()),
                    ),
                    (
                        graph_attrs::INPUT_NAMES.to_string(),
                        Value::Array(vec![Value::String("message".to_string())]),
                    ),
                    (
                        graph_attrs::INPUT_ROLES.to_string(),
                        Value::Array(vec![Value::String("user".to_string())]),
                    ),
                ]),
            },
        ],
        edges: vec![AirEdge {
            from: 1,
            to: 2,
            dependency: DependencyType::Data,
        }],
        parameters: Vec::new(),
        metadata: HashMap::new(),
    };

    let context = Context::new().expect("compiler context");
    let module = Pipeline::with_opt_level(&context, OptimizationLevel::O1)
        .compile_graph(&graph)
        .expect("programmatic AIR graph compiles");
    let ir = module.to_string().expect("optimized module serializes");
    let answer = ir
        .lines()
        .find(|line| line.contains("ais.ask"))
        .expect("specialized answer operation remains");

    assert!(answer.contains("input_names = []"));
    assert!(answer.contains("input_roles = []"));
    module
        .generate_artifact()
        .expect("programmatic prompt-role contract remains executable");
}
