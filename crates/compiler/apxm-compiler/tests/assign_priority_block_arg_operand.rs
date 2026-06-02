//! Regression: the AssignPriority pass must not crash when an AIS op takes a
//! function parameter (an MLIR block argument) directly as an operand.
//!
//! Such an operand has no defining op — `Value::getDefiningOp()` returns null.
//! `AssignPriority::analyzeDag` feeds that null pointer to `isArtifactOperation`,
//! whose `isa<func::ReturnOp>(op)` arm dereferences it. Without a null guard this
//! segfaults the whole compiler — and any server compiling untrusted AIR at O1+,
//! which is a remote DoS. The fix adds `op &&` to `isArtifactOperation`.
//!
//! AssignPriority only runs at O1+, so this is exercised at O1. O0 never tripped
//! the bug (it omits the pass) and so is not a sufficient guard.

use apxm_compiler::{Context, Pipeline};
use apxm_core::types::OptimizationLevel;

/// An entry function whose AIS op consumes the function parameter `%arg0`
/// directly as a context operand. `%arg0` is a block argument, so the op has a
/// null-defining-op operand — the exact shape that crashed AssignPriority.
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

    // The contract under test is solely "does not crash the process": reaching
    // this assertion at all means AssignPriority ran without segfaulting. The
    // compile may legitimately succeed or return a structured error; either is
    // acceptable. (Before the fix, this line was never reached — SIGSEGV.)
    let result = pipeline.compile(BLOCK_ARG_OPERAND_AIR);
    let _ = result.is_ok();
}
