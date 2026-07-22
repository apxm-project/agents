/*
* @file  AISOps.semantic.generated.td
* @brief GENERATED from Rust AIS semantic operation definitions - DO NOT EDIT MANUALLY
*
* Source of truth: crates/machine/ais/src/operations/definitions.rs
*/

// Canonical five public semantic operations.


//===----------------------------------------------------------------------===//
// Canonical Semantic Operations (generated from definitions.rs)
//===----------------------------------------------------------------------===//

def AIS_ModelCallOp : AIS_Op<"model_call", []> {
  let summary = "Invoke an admitted model with explicit request and options";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_CapabilityInvokeOp : AIS_Op<"capability_invoke", []> {
  let summary = "Invoke one admitted Capability through the authority/effect chokepoint";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_ProgramNewOp : AIS_Op<"program_new", []> {
  let summary = "Create a stateful Program Instance from an exact Program reference";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_ProgramInvokeOp : AIS_Op<"program_invoke", []> {
  let summary = "Invoke a ProgramRef one-shot or a ProgramInstanceRef statefully";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


def AIS_AwaitEventOp : AIS_Op<"await_event", []> {
  let summary = "Suspend on a typed durable external event or approval signal";
  let results = (outs AIS_TokenType:$result);
  let assemblyFormat = "attr-dict `:` type($result)";
}


