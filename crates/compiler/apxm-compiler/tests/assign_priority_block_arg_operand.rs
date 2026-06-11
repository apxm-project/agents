//! Regression coverage for AssignPriority handling of block-argument operands.

use apxm_compiler::{Context, Pipeline};
use apxm_core::types::OptimizationLevel;

const BLOCK_ARG_OPERAND_AIR: &str = r#"module {
  func.func @forwards_param(%arg0: !ais.token {ais.param_name = "x", ais.param_type = "str"}) -> !ais.token attributes {ais.entry} {
    %r = ais.ask "{t}" [%arg0 : !ais.token] {input_names = ["t"]} : !ais.token
    func.return %r : !ais.token
  }
}
"#;

#[test]
fn assign_priority_survives_block_argument_operand_at_o1() {
    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O1);

    pipeline
        .compile(BLOCK_ARG_OPERAND_AIR)
        .expect("block-argument operands should compile at O1");
}
