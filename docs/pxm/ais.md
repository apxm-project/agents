---
title: "Agent Instruction Set (AIS)"
description: "The typed operation taxonomy that forms A-PXM's intermediate representation -- design principles, instruction categories, latency model, and type system."
---

# Agent Instruction Set (AIS)

The AIS is a typed intermediate representation -- the ISA contract for agentic AI (see [foundations.md](foundations.md) for why this contract matters). Every operation takes typed inputs, produces typed outputs, and transitions the [AAM](aam.md) state deterministically. The AIS is what makes agent workflows visible: each operation is a typed node with declared dependencies, not an opaque function call.

## Design Principles

1. **Typed end-to-end**: every operand and result carries a type. Type mismatches are caught at compile time, not at runtime after an expensive LLM call.
2. **Latency-aware**: LLM operations are stratified by latency budget (ASK ~1s, THINK ~3s, REASON ~10s), enabling the scheduler to make informed decisions about overlap and prioritization.
3. **Effect-explicit**: side effects (memory writes, tool calls, messages) are first-class operations, not hidden behind opaque function calls.
4. **Token-carried**: operations return `!ais.token` values that integrate with
   the dataflow dependency system.
5. **Topology-agnostic**: multi-agent operations carry concrete targets. They do
   not encode organization policy such as reporting lines, directory visibility,
   approval chains, or who may reach whom.

## Instruction Categories

The AIS operation catalog is generated from Rust-owned definitions and published
as `crates/machine/ais/generated/op-spec.v1.json`. Run `dekk agents ops list`
for the complete current operation set instead of copying a static list into
docs or frontend code. For per-operation details and examples, see the
[apxm-ais README](../../crates/machine/ais/README.md).

`COMMUNICATE`, `DELEGATE`, `HANDOFF`, and `SPAWN_AGENT` are executable
coordination primitives. They are not an agent hierarchy model. Hosts such as
`apxm-os` may use topology policy to decide whether these operations are allowed,
but APXM runtime receives only the admitted concrete target. See
[Agent Topology Boundary](../agent-topology-boundary.md).

## Latency-Typed LLM Operations

A distinguishing feature of AIS is that LLM operations carry explicit latency budgets:

| Operation | Budget | Use Case | Example |
|-----------|--------|----------|---------|
| `ASK` | ~1s | Classification, extraction, simple Q&A | "Is this email spam?" |
| `THINK` | ~3s | Multi-step reasoning, summarization | "Summarize and extract key claims" |
| `REASON` | ~10s | Complex analysis, planning, synthesis | "Analyze 5 reports, identify contradictions" |

The [scheduler](scheduling.md) uses these budgets to:
- **Overlap** long-running REASON operations with independent ASK/THINK operations
- **Prioritize** operations on the critical path
- **Timeout** operations that exceed their budget, triggering TRY_CATCH recovery

This stratification makes the cost structure of a workflow visible at compile time.

## Type System

### Core Types

| Type | Description |
|------|-------------|
| `String` | Text values (prompts, responses) |
| `Value` | Typed union (String, Int, Float, Bool, JSON) |
| `Context` | Accumulated context for LLM operations |
| `SessionID` | Memory session identifier |
| `Goal` | Structured goal with priority |
| `PlanTree` | Goal decomposition tree |
| `Token` | Dataflow synchronization token |
| `Verdict` | Typed verification result |
| `Critique` | Structured self-assessment |
| `ToolResult` | Result of tool invocation |
| `Message` | Inter-agent message |
| `Ack` | Message acknowledgement |

### Token Values and Synchronization

Operations produce `!ais.token` values that participate in the dataflow system:

```mlir
module {
  func.func @capability_join() -> !ais.token attributes {ais.entry} {
    %lookup = ais.inv_cap "lookup_docs" ("{\"query\":\"release checklist\"}") : !ais.token
    %summary = ais.ask "Summarize {lookup}." [%lookup : !ais.token] {input_names = ["lookup"]} : !ais.token
    %joined = ais.wait_all %lookup, %summary : !ais.token, !ais.token -> !ais.token
    ais.return %joined : !ais.token
  }
}
```

Tokens flow along DAG edges, trigger downstream operations when resolved, and
carry type information for compile-time verification.

## State Transitions

Every AIS instruction is a deterministic state transition on the [AAM](aam.md):

```
d(AAM, Instr) -> AAM'
```

Different instructions affect different components of the AAM triple:

| Instruction | Reads | Writes |
|-------------|-------|--------|
| ASK/THINK/REASON | B (context), C (model selection) | B (response stored) |
| PLAN | G (current goals), B (context) | G (decomposed sub-goals) |
| REFLECT | Episodic trace | G (revised goals), B (insights) |
| VERIFY | B (claim + evidence) | B (verdict) |
| INV_CAP | C (capability lookup), B (params) | B (tool result) |
| QMEM | B (STM/LTM/Episodic) | -- (read-only) |
| UMEM | -- | B (memory write) |
| COMMUNICATE | -- | Outbound message |

This explicit mapping of reads and writes enables the compiler to perform side-effect analysis, determine operation independence, and verify that state transitions are well-formed.

## MLIR Dialect

AIS is implemented as an MLIR dialect with custom operations, types, and verifiers. See [apxm-compiler](../../crates/compiler/pipeline/README.md) for the full pipeline.

```mlir
module {
  func.func @ask_with_backend() -> !ais.token attributes {ais.entry} {
    %answer = ais.ask "Summarize the incident." {backend = "registered-route", model = "served-model-id"} : !ais.token
    ais.return %answer : !ais.token
  }
}
```

Each operation carries:
- **Custom verifiers** that check operand types and structural constraints
- **Canonicalization patterns** for normalization
- **Folding rules** for compile-time evaluation of constant expressions
- **Side-effect declarations** for memory resources (Belief, Goal, Capability, Episodic)

---

## Further Reading

- [PXM Foundations](foundations.md) -- the five separations and the ISA contract
- [AAM: Agent Abstract Machine](aam.md) -- the state model AIS operates on
- [Compute in PXMs](compute.md) -- how AIS operations compare to six classical PXM compute models
- [apxm-ais README](../../crates/machine/ais/README.md) -- per-operation reference
- [Compiler Pipeline](../../crates/compiler/pipeline/README.md) -- how AIS maps to MLIR
- [Optimization Pipeline](../compiler/pipeline.md) -- compiler passes that transform AIS graphs

---

## References

1. C. Lattner et al., "MLIR: Scaling Compiler Infrastructure for Domain Specific Computation," in *Proc. CGO '21*, IEEE, 2021. DOI: [10.1109/CGO51591.2021.9370308](https://doi.org/10.1109/CGO51591.2021.9370308)

2. R. Cytron et al., "Efficiently Computing Static Single Assignment Form and the Control Dependence Graph," *ACM TOPLAS*, vol. 13, no. 4, pp. 451-490, 1991. DOI: [10.1145/115372.115320](https://doi.org/10.1145/115372.115320)

3. G. R. Gao, R. Patel, and T. St. John, "The Codelet Program Execution Model," presented at *WiA, ISCA '13*, Tel-Aviv, Israel, 2013.
