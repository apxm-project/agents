/*
* @file  AISOps.structural.generated.td
* @brief GENERATED from Rust AIS structural operation definitions - DO NOT EDIT MANUALLY
*
* Source of truth: crates/machine/ais/src/operations/definitions.rs
*/

// Closed compiler-emitted structural operations.

def AIS_StructuralFunctionOp : AIS_Op<"function", [NoTerminator]> {
  let summary = "Compiler-emitted function structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralRegionOp : AIS_Op<"region", [NoTerminator]> {
  let summary = "Compiler-emitted region structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralBlockOp : AIS_Op<"block", [NoTerminator]> {
  let summary = "Compiler-emitted block structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralValueOp : AIS_Op<"value", [NoTerminator]> {
  let summary = "Compiler-emitted value structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralBranchOp : AIS_Op<"branch", [NoTerminator]> {
  let summary = "Compiler-emitted branch structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralSwitchOp : AIS_Op<"switch", [NoTerminator]> {
  let summary = "Compiler-emitted switch structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_LoopOp : AIS_Op<"loop", [NoTerminator]> {
  let summary = "Compiler-emitted ais.loop structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralParallelJoinOp : AIS_Op<"parallel_join", [NoTerminator]> {
  let summary = "Compiler-emitted parallel_join structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralTryOp : AIS_Op<"try", [NoTerminator]> {
  let summary = "Compiler-emitted try structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralThrowOp : AIS_Op<"throw", [NoTerminator]> {
  let summary = "Compiler-emitted throw structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralCatchOp : AIS_Op<"catch", [NoTerminator]> {
  let summary = "Compiler-emitted catch structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralReturnOp : AIS_Op<"return", [NoTerminator]> {
  let summary = "Compiler-emitted return structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}


def AIS_StructuralYieldOp : AIS_Op<"yield", [NoTerminator]> {
  let summary = "Compiler-emitted yield structural operation";
  let arguments = (ins Variadic<AIS_TokenType>:$operands);
  let regions = (region AnyRegion:$body);
  let results = (outs AIS_TokenType:$result);
}
