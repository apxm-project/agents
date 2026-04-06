# Implementation Blocked

## Status

**Blocked**: no actionable implementation can be performed yet.

## Reason

The provided task payload is literally:

```text
INVALID TASK
```

No concrete feature, bug, or architectural change has been specified, and no repository structure or relevant source files were provided for inspection.

Because of that, any attempt to produce Rust code, exact repo-specific edits, or tests would be speculative and technically unsound.

## Blocking conditions

The following required inputs are currently missing:

- A real task description
- Repository tree / workspace structure
- Crate boundaries
- Public APIs
- Relevant module contents for:
  - `ContextStack`
  - `MemoCache`
  - `Scheduler`
  - `ModelRouter`
- Existing tests
- Runtime/router invariants
- Error model and config model

## Decision

**Do not implement anything yet.**

This is the only safe and correct action based on current information.

## Why implementation is blocked

### 1. No valid implementation target exists

Without a real task, there is no defensible way to determine:

- what behavior should change,
- which crate should change,
- which module owns the change,
- which tests should be added or updated,
- which invariants must be preserved.

### 2. Risk of architectural damage is high

Blind changes around routing/runtime behavior could create a **second source of truth** and accidentally bypass existing behavior such as:

- fallback logic,
- validation,
- timeout handling,
- metrics/telemetry,
- request normalization,
- provider capability matching,
- shared error mapping.

### 3. "Minimal patch" cannot be defined honestly

Even a conservative Rust patch requires visibility into:

- existing type signatures,
- current module layout,
- current config handling,
- execution flow,
- nearby tests.

Without that, any code would be fabricated.

## Required inputs before implementation

Provide the following:

### A. Actual task
A concrete objective, for example:

- Add fallback routing when a provider returns rate-limit
- Prefer local models for embeddings
- Integrate `MemoCache` into `Scheduler` dispatch
- Propagate `ContextStack` into model routing requests

### B. Repository tree
Run and paste:

```bash
tree -L 3 ~/projects/agents/apxm
```

### C. Relevant symbol usage
Run and paste:

```bash
rg -n "ContextStack|MemoCache|Scheduler|ModelRouter" ~/projects/agents/apxm
```

### D. Key files
Provide contents of these if they exist:

```text
~/projects/agents/apxm/Cargo.toml
~/projects/agents/apxm/crates/*/Cargo.toml
~/projects/agents/apxm/crates/apxm-runtime/src/model_router/mod.rs
~/projects/agents/apxm/crates/apxm-runtime/src/model_router/*.rs
```

And any files defining or using:

- `ContextStack`
- `MemoCache`
- `Scheduler`
- `ModelRouter`

## Constraints for the eventual implementation

Once the real task and code are available, the implementation must:

- reuse existing routing/runtime abstractions,
- avoid introducing parallel configuration or routing paths,
- prefer a single-point modification,
- preserve fallback, timeout, capability, logging, normalization, and error invariants,
- keep config changes minimal,
- add focused regression coverage near the changed behavior.

## What should not be built without explicit proof of need

Do not add any of the following unless the actual task and code clearly justify them:

- new routing DSL,
- plugin architecture,
- adaptive router,
- weighted policy engine,
- speculative fan-out,
- persistent routing memory,
- second cache layer,
- duplicate provider abstraction,
- replacement router stack.

## Conservative approach

- Implement nothing until the actual task and relevant source files are provided.
- Reuse existing router/runtime abstractions exclusively.
- Make the smallest possible change in one existing decision point.
- Add one focused regression test near the modified behavior.
- Refuse any design that creates a second source of truth.

## Bold approach

- Once repository tree and relevant files are available, perform a rapid architecture pass over the runtime/router code to identify the true extension seam.
- If the current router is fragmented, consolidate decision logic into one authoritative module before adding behavior.
- Add invariant tests covering fallback, timeout, metrics, and capability matching.
- Only pursue larger refactors if the actual code proves the requested change cannot be made cleanly otherwise.
