/*
* @file  AISOps.structural.generated.td
* @brief GENERATED from Rust AIS structural operation definitions - DO NOT EDIT MANUALLY
*
* Source of truth: crates/machine/ais/src/operations/definitions.rs
*/

// Closed compiler-emitted structural operations.

def AIS_StructuralFunctionOp : AIS_Op<"function", []> {
  let summary = "Compiler-emitted function structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralRegionOp : AIS_Op<"region", []> {
  let summary = "Compiler-emitted region structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralBlockOp : AIS_Op<"block", []> {
  let summary = "Compiler-emitted block structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralValueOp : AIS_Op<"value", []> {
  let summary = "Compiler-emitted value structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralBranchOp : AIS_Op<"branch", []> {
  let summary = "Compiler-emitted branch structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralSwitchOp : AIS_Op<"switch", []> {
  let summary = "Compiler-emitted switch structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_LoopOp : AIS_Op<"loop", []> {
  let summary = "Compiler-emitted ais.loop structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralParallelJoinOp : AIS_Op<"parallel_join", []> {
  let summary = "Compiler-emitted parallel_join structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralTryOp : AIS_Op<"try", []> {
  let summary = "Compiler-emitted try structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralThrowOp : AIS_Op<"throw", []> {
  let summary = "Compiler-emitted throw structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralCatchOp : AIS_Op<"catch", []> {
  let summary = "Compiler-emitted catch structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralReturnOp : AIS_Op<"return", []> {
  let summary = "Compiler-emitted return structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_StructuralYieldOp : AIS_Op<"yield", []> {
  let summary = "Compiler-emitted yield structural operation";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}
