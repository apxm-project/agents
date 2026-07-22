# APXM Agent Program Full-Replacement Portfolio

- Status: canonical APXM v1 portfolio
- Approved: 2026-07-16
- Decision: [ADR-0003](../adr/0003-agent-program-contract-migrations-remove-old-semantics.md)
- Baseline: `agents@9e26a62adebb`

## Goal

Coordinate the APXM-owned Agent Program work as parallel target-only plans,
then cut over one Python/TypeScript/compiler/artifact/runtime Compatibility Set
without legacy operations, composition, Hook, Context, named conversational,
or Gao core/package/product semantics.

## Owning plans

| Plan | Owner | Parallel work | Synchronization gate |
| --- | --- | --- | --- |
| [Agent Program composition and AIR full replacement](agent-program-composition-and-air-full-replacement-plan.md) | APXM contracts, frontends, AIS, compiler, artifact, runtime, examples, cross-plane consumers | `program.new`/`program.invoke`, two closed AIS families, Agent Facade, explicit context, structured loops, generic iteration evidence, Program Instances, deletion | P0-P9 |
| [Rust embedding library hardening](rust-embedding-library-hardening-plan.md) | APXM contracts, AIS, artifact, compiler, runtime, adapter owners | Public role vectors, acyclic dependencies, injected runtime interfaces, adapters, profiles, clean consumers | RLIB-1-RLIB-9 |
| [Compiler bridge delivery](compiler-bridge-delivery-plan.md) | APXM graph contracts, compiler, Python/Node bridges, compile service/client | FrontendGraph v1 vectors, pure frontends, native bridges, remote boundary, build tools | B1-B9; B1 consumes P0-P1 and B2 consumes RLIB-1-RLIB-4 |

Coordinator ADR-0001 owns the canonical v1 release family, shared version line,
private coordinates, Compatibility Set, platform matrix, and promotion. The
Rust embedding plan owns repository-local public library/adapter boundaries;
the compiler bridge plan owns graph and compiler delivery. Lanes start only
after their APXM master-plan baseline and input-contract gates pass.

## Parallel lanes

After P0 versioned schema vectors exist:

1. Rust contract and artifact types are implemented by their owning crates.
2. Python and TypeScript frontend APIs and generated types are implemented in
   parallel against the same vectors.
3. Compiler/AIR structured state flow and Hook bindings are implemented against
   the Rust-owned contract.
4. Runtime Program Instance, invocation, generic durability, and evidence
   behavior is implemented against the same contract.
5. Conversational and Gao repository examples are prepared against packed
   generic frontend APIs while generic loop-iteration projection fixtures are
   built.

Lanes exchange schemas, generated code, FrontendGraph DTOs, artifacts, and
conformance vectors. They do not call or retain the old semantic engine.

## Integration sequence

```mermaid
flowchart LR
    D["APXM v1 baseline"] --> V["Shared vectors"]
    V --> C["P0 + P1 + B1 contracts"]
    C --> F["Python + TypeScript frontends"]
    C --> P["Compiler + AIR"]
    C --> R["Runtime + artifacts"]
    F --> P6["P6 Generic loops + examples"]
    P --> P6
    R --> P6
    P6 --> P8["P8 evidence + Studio projection"]
    P8 --> P9["P9 target-only release"]
```

P9 is not a compatibility phase. It updates every included first-party
consumer, rejects old semantic versions, deletes old dispatch and direct-loop
paths, and emits one candidate Compatibility Set.

## Target-only release gate

The APXM candidate must prove:

- equivalent Python and TypeScript programs produce equivalent semantic graph,
  AIR, artifact, and lifecycle evidence;
- `program.new`, `program.invoke`, and `instance.invoke` obey one state and
  structured-concurrency contract;
- AIR contains exactly five effect/composition operations and a separate
  closed structural family including `ais.loop`;
- one Agent Facade callback and explicit Program Context implementation execute;
- conversational and Gao examples use only packed generic frontend APIs;
- named package exports/product surfaces, direct Gao core branches, unversioned
  Hook types, broad decision union, duplicate dispatch, alias fields/events,
  and old artifact acceptance are absent;
- committed loop iterations emit replay-stable `LoopIterationCompleted`, while
  failed or rolled-back bodies emit none;
- local, CLI, Server, and OS-hosted execution use the same contract versions;
- old/unknown versions fail before runtime execution;
- release and repository scans find no legacy semantic feature switch.

## Rollback

Before target promotion, discard a failed candidate and retain its evidence.
After promotion, rollback restores one complete prior APXM/product release and
matching snapshot before the point of no return. No runtime contains both
semantic generations. After irreversible target work, stop intake and repair
forward.

## Completion definition

This portfolio completes when P9 passes for the same source and artifact
digests, every included first-party consumer uses the target contracts, old
semantic paths are deleted, unknown/old artifacts are rejected, and the APXM
Compatibility Set contains one coherent frontend/compiler/artifact/runtime
generation.
