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
4. **Future-typed**: async operations return `Future<T>` handles that integrate with the dataflow token system.

## Instruction Categories

The AIS operations are organized across multiple categories. Run `apxm ops list` for the complete, current operation set.

| Category | Representative Operations | Purpose |
|----------|-----------|---------|
| **Reasoning** | ASK, THINK, REASON, PLAN, REFLECT, VERIFY | Language model interactions at different latency tiers |
| **Memory** | QMEM, UMEM | Three-tier memory access (STM/LTM/Episodic) |
| **Tools** | INV | External tool invocation with typed parameter marshalling |
| **ControlFlow** | BRANCH_ON_VALUE, SWITCH, FLOW_CALL | Conditional routing and sub-flow invocation |
| **Synchronization** | MERGE, WAIT_ALL, FENCE | Synchronization barriers and token collection |
| **Communication** | COMM | Inter-agent messaging |
| **Coordination** | DELEGATE, NEGOTIATE | Multi-agent task distribution |
| **ErrorHandling** | TRY_CATCH | Exception handling with recovery subgraphs |
| **Identity** | NOP, IDENTITY | Pass-through operations for graph structuring |

For per-operation details and examples, see the [apxm-ais README](../../crates/core/apxm-ais/README.md).

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
| `Future<T>` | Handle to an async result |
| `Verdict` | Typed verification result |
| `Critique` | Structured self-assessment |
| `ToolResult` | Result of tool invocation |
| `Message` | Inter-agent message |
| `Ack` | Message acknowledgement |

### Future and Handle Types

Async operations return `Future<T>` handles that participate in the dataflow token system:

```
%result = ais.ask(%prompt, %ctx) : Future<String>
%tool_out = ais.inv("search", %params) : Future<ToolResult>

// WAIT_ALL consumes futures, produces resolved values
%values = ais.wait_all(%result, %tool_out) : (String, ToolResult)
```

Futures are first-class tokens: they flow along DAG edges, trigger downstream operations when resolved, and carry type information for compile-time verification.

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
| INV | C (capability lookup), B (params) | B (tool result) |
| QMEM | B (STM/LTM/Episodic) | -- (read-only) |
| UMEM | -- | B (memory write) |
| COMM | -- | Outbound message |

This explicit mapping of reads and writes enables the compiler to perform side-effect analysis, determine operation independence, and verify that state transitions are well-formed.

## MLIR Dialect

AIS is implemented as an MLIR dialect with custom operations, types, and verifiers. See [apxm-compiler](../../crates/compiler/apxm-compiler/README.md) for the full pipeline.

```mlir
%0 = "ais.ask"(%prompt, %ctx) {
  latency_budget = 1000 : i64,
  model = "default"
} : (!ais.string, !ais.context) -> !ais.future<!ais.string>
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
- [apxm-ais README](../../crates/core/apxm-ais/README.md) -- per-operation reference
- [Compiler Pipeline](../../crates/compiler/apxm-compiler/README.md) -- how AIS maps to MLIR
- [Optimization Passes](../compiler/passes.md) -- compiler passes that transform AIS graphs

---

## References

1. C. Lattner et al., "MLIR: Scaling Compiler Infrastructure for Domain Specific Computation," in *Proc. CGO '21*, IEEE, 2021. DOI: [10.1109/CGO51591.2021.9370308](https://doi.org/10.1109/CGO51591.2021.9370308)

2. R. Cytron et al., "Efficiently Computing Static Single Assignment Form and the Control Dependence Graph," *ACM TOPLAS*, vol. 13, no. 4, pp. 451-490, 1991. DOI: [10.1145/115372.115320](https://doi.org/10.1145/115372.115320)

3. G. R. Gao, R. Patel, and T. St. John, "The Codelet Program Execution Model," presented at *WiA, ISCA '13*, Tel-Aviv, Israel, 2013.
