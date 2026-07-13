//! Direct-AIR coverage for fail-closed pure dead-node elimination.

use apxm_compiler::passes::build_pass_list;
use apxm_compiler::{Context, Module, OptimizationLevel, PassManager};
use apxm_core::types::OptimizationTarget;

const PURE_DEAD_NODE_ELIMINATION: &str = "pure-dead-node-elimination";
const SYMBOL_DCE: &str = "symbol-dce";

const DEAD_PURE_NODES_AIR: &str = r#"
module {
  func.func @dead_pure_nodes() -> !ais.token attributes {ais.entry} {
    %live = ais.const_str "live" : !ais.token
    %left = ais.const_str "unused left" : !ais.token
    %right = ais.const_str "unused right" : !ais.token
    %merged = ais.merge %left, %right : !ais.token, !ais.token -> !ais.token
    %waited = ais.wait_all %merged : !ais.token -> !ais.token
    func.return %live : !ais.token
  }
}
"#;

const RETAINED_NODES_AIR: &str = r#"
module {
  func.func @retained_nodes() -> !ais.token attributes {ais.entry} {
    %live = ais.const_str "live" : !ais.token
    %identity = ais.identity %live : !ais.token -> !ais.token
    %fence = ais.fence : !ais.token
    %left = ais.const_str "left" : !ais.token
    %right = ais.const_str "right" : !ais.token
    %nop = ais.nop(%left, %right : !ais.token, !ais.token) : !ais.token
    func.return %nop : !ais.token
  }
}
"#;

fn run_pure_dead_node_elimination(air: &str) -> String {
    let context = Context::new().expect("compiler context");
    let module = Module::parse(&context, air).expect("direct AIR parses");
    let mut pass_manager = PassManager::new(&context).expect("pass manager");
    pass_manager
        .add_pass(PURE_DEAD_NODE_ELIMINATION)
        .expect("pure dead-node pass is registered")
        .run(&module)
        .expect("pure dead-node pass runs");
    module.to_string().expect("module serializes")
}

#[test]
fn removes_only_unused_allow_list_nodes_until_the_inert_chain_is_gone() {
    let ir = run_pure_dead_node_elimination(DEAD_PURE_NODES_AIR);

    assert!(ir.contains("ais.const_str \"live\""));
    assert!(!ir.contains("unused left"));
    assert!(!ir.contains("unused right"));
    assert!(!ir.contains("ais.merge"));
    assert!(!ir.contains("ais.wait_all"));
}

#[test]
fn retains_identity_fence_and_multi_input_nop() {
    let ir = run_pure_dead_node_elimination(RETAINED_NODES_AIR);

    assert!(ir.contains("ais.identity"));
    assert!(ir.contains("ais.fence"));
    let nop_line = ir
        .lines()
        .find(|line| line.contains("ais.nop"))
        .expect("multi-input nop is preserved");
    assert!(
        nop_line.contains("!ais.token, !ais.token"),
        "nop no longer retains two token inputs: {nop_line}"
    );
}

#[test]
fn optimized_default_pipelines_run_pure_dead_node_elimination_before_symbol_dce() {
    for level in [
        OptimizationLevel::O1,
        OptimizationLevel::O2,
        OptimizationLevel::O3,
    ] {
        let passes = build_pass_list(level, false, OptimizationTarget::Balanced);
        let pure_dce_index = passes
            .iter()
            .position(|pass| pass == PURE_DEAD_NODE_ELIMINATION)
            .expect("optimized pipeline includes pure dead-node elimination");
        let symbol_dce_index = passes
            .iter()
            .position(|pass| pass == SYMBOL_DCE)
            .expect("optimized pipeline includes symbol DCE");

        assert!(pure_dce_index < symbol_dce_index, "{level:?}");
    }
}
