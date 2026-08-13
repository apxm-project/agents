# APXM agents documentation

This repository is the product-neutral APXM abstract machine: source-first
Agent Programs, the closed AIR/AIS semantics, exact capability and inference
ports, and the runtime that executes admitted artifacts.

## Current reading order

1. [PXM theory](pxm/theory.md) — the current execution-model vocabulary.
2. [PXM lineage](pxm/readme.md) — historical theory retained for context.
3. [Agent Program composition and AIR](agents/agent-program-composition-and-air-contract.md).
4. [Portable core interface](agents/portable-core-interface-contract.md).
5. [Execution Admission](agents/execution-admission-contract.md).
6. [Authoring guides](guides/README.md).
7. [Architecture decisions](adr/README.md).

## Canonical pipeline

```text
Python or TypeScript source
  -> FrontendGraph v2
  -> Rust verification and lowering
  -> AIR v2
  -> registered AIS MLIR
  -> immutable executable artifact
  -> exact Invocation Admission and Port bindings
  -> generic execution kernel
  -> monotonic runtime evidence
```

The public semantic family is exactly:

1. `model.call`
2. `capability.invoke`
3. `program.new`
4. `program.invoke`
5. `await.event`

Compiler-emitted structural operations are a separate closed family. They are
not a raw authoring API and `ais.loop` is not a sixth semantic operation.

## Scope boundary

This repository does not own product control planes, Studio workflows,
Telegram/webhook integrations, deployment fleets, model zoos, prompt
evaluation studies, or hosted release infrastructure. Those systems may bind
to the contracts from outside through exact ports and admission.

The historical PXM pages remain because they explain the ideas that shaped the
current machine. They are theory and lineage only; no current implementation
may depend on their retired operations or runtime state models.

## Verification

```bash
dekk agents doctor
dekk agents ops list
dekk agents check
dekk agents test-frontend-examples
```
