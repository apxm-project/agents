# apxm-compiler

- Current status: canonical AIR verification slice plus pre-canonical MLIR/pass
  infrastructure
- Target status: design governed by the Agent Program contract and frontend
  D0/P1/P2 gates
- Target plan:
  [Source-first Agent frontend master plan](../../../docs/agents/simple-agent-authoring-frontend-plan.md)
- Normative execution boundary:
  [ADR-0011](../../../docs/adr/0011-agent-program-execution-is-one-end-to-end-spine.md)

`apxm-compiler` is the Rust owner of registered AIS MLIR lowering and compiler
optimization. Python, TypeScript, Studio, CLI, and Server must not implement a
second AIR printer, AIS selector, pass pipeline, or artifact builder.

## Target compiler path

```text
typed Python / TypeScript / Studio-generated source
  -> native AST + immutable bound/typed source tree
  -> FrontendGraph
  -> closed validation
  -> typed CFG, SSA, regions, block arguments, and context flow
  -> canonical AIR
  -> registered ais.* MLIR operations with complete operands/results
  -> semantics-preserving analyses and transformations
  -> verified immutable artifact + diagnostics + optimization provenance
```

The compiler may derive backend-neutral scheduling, prefix-reuse, cost, or
resource analysis. It cannot select a provider, endpoint, credential,
deployment, grant, Runtime Profile, or fallback. If admission later binds a
Model target to APXM-vLLM, the admitted adapter may translate supported
backend-neutral metadata into vLLM graph/prefix/priority hints without changing
Agent semantics.

## Current baseline reality

At the pinned frontend-plan baseline, the complete target path above is not yet
wired:

- [`apxm-program::lower`](../../machine/program/src/lower.rs) verifies the
  current shallow FrontendGraph and copies semantic, structural, operand, and
  context records almost field-for-field into AIR.
- [`canonical.rs`](src/canonical.rs) proves deterministic AIR text emission and
  MLIR parsing, but emits semantic operations as unregistered `apxm.*` records
  with `() -> ()` and flat structural token records.
- The compiler MLIR context currently
  [permits unregistered dialects](mlir/include/ais/CAPI/Internal.h), so that
  parse/verify test does not prove registered-AIS conformance.
- The generated registered semantic operations in
  [`AISOps.semantic.generated.td`](mlir/include/ais/Dialect/AIS/IR/AISOps.semantic.generated.td)
  currently declare a result token and no complete semantic operands.
- Native Python and Node bridges call `apxm-program` to return AIR JSON or an
  artifact containing AIR; they do not yet drive this crate's registered AIS
  emitter and optimized artifact path.

The remaining MLIR passes, legacy graph builders, `ExecutionDag` codegen, and
prototype `.apxmobj` machinery are implementation evidence to reconcile or
replace. Their presence is not proof that the target FrontendGraph → registered
AIS → artifact spine is connected.

## Target responsibilities

The completed compiler owns:

1. closed FrontendGraph decoding, type/effect verification, and canonicalization;
2. deterministic CFG/SSA construction, including loop-carried Context,
   structured joins, try/catch, yield/resume, return, and static Hook wrappers;
3. selection of the five semantic AIR operations and construction of the
   closed structural AIS family;
4. registered AIS emission with every contract operand, result, attribute,
   region, block argument, and source mapping preserved;
5. optimization legality, analysis invalidation, deterministic pass order, and
   inspectable provenance;
6. artifact validation, digest binding, requirements, source maps, and typed
   diagnostics; and
7. stable correlation from authored source through static nodes to runtime
   NodeExecution/effect evidence.

## Optimization law

An optimization is legal only when it preserves:

- typed inputs, outputs, Context, and control/data dependencies;
- Model, Capability, Event, Hook, and composition effects and their order;
- exact requirements, authority checks, budgets, cancellation, and durability;
- retry/idempotency and outcome-unknown semantics;
- source/static-node correlation and authoritative evidence meaning; and
- the observable result under every admitted runtime profile.

Parallel execution is derived only from proven independence or an authored
structured task scope. The compiler never invents a model/Tool loop, Agent
handoff, provider route, retry policy, or detached task.

## Primary modules

| Path | Responsibility |
| --- | --- |
| `src/canonical.rs` | Current AIR → deterministic MLIR verification slice; target registered AIS emitter |
| `src/api/` | MLIR context, module, and pipeline wrappers |
| `src/passes/` | Pass planning, registration, diagnostics, and metrics |
| `src/codegen/` | Prototype optimized-MLIR artifact machinery to reconcile with canonical artifact ownership |
| `mlir/include/ais/Dialect/AIS/IR/` | Registered AIS dialect types, attributes, and generated operations |
| `mlir/lib/Dialect/AIS/` | AIS operation/pass implementations |
| `mlir/lib/CAPI/` | Focused Rust/C++ MLIR boundary |

AIS operation definitions remain Rust-owned and generate TableGen. Do not edit
generated operation files to conceal source/generator drift.

## Verification

Use the repository Dekk surface:

```bash
dekk agents test-program
dekk agents test-compiler
dekk agents check-frontend-codegen
```

After an AIS definition or TableGen signature change:

```bash
dekk agents build-dialect
dekk agents codegen
```

Target completion additionally requires registered-only MLIR negative tests,
FrontendGraph/AIR/AIS/artifact goldens, Python/TypeScript parity, optimization
differential tests, runtime/evidence conformance, exact inference-adapter
conformance, and absence scans for the retired frontend/compiler paths.
