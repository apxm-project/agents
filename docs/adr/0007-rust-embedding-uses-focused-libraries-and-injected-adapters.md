---
status: accepted
date: 2026-07-15
amended_by: ADR-0009, ADR-0010, ADR-0013
decision: D-004C
owner: APXM agents
supersedes: the current public `apxm` umbrella facade and ambient runtime composition
---

# Rust embedding uses focused libraries and injected adapters

## Context

D-004A established one APXM release family with distinct installable surfaces,
and D-004B established explicit in-process compiler bridges over the public Rust
compiler boundary. D-004C decides the Rust libraries behind those surfaces:
which responsibilities are public, what may depend on what, who owns process
resources, how adapters compose, and whether the current umbrella facade
survives the target-only migration.

At the pinned `agents@9e26a62adebb` baseline:

- `crates/facade` re-exports contracts, AIS, compiler, runtime, and every
  backend through one `apxm` crate, but repository search finds no Rust
  consumer proving that this all-plane facade is required;
- `apxm-compiler` directly depends on `apxm-backends`, joining compilation to
  provider, network, and storage implementation concerns;
- `apxm-runtime` directly depends on concrete backends, capability
  implementations, HTTP, process/filesystem helpers, and default `dashmap`
  plus SQLite features;
- the current `apxm-core` dependency graph includes environment, path,
  executable-discovery, async, and tracing-configuration concerns rather than
  only stable contracts; and
- runtime and supporting crates contain ambient environment reads and
  process-global mutable registries, meters, session ledgers, locks, and
  behavior switches that obstruct deterministic, isolated, multi-instance
  embedding.

These are implementation facts, not accepted target boundaries. The target
must support compiler-only and runtime-only consumers without installing a
server, provider backend, database, HTTP client, CLI, hidden process, or
unrelated application composition.

## Decision

### Public library roles

APXM publishes five focused Rust library roles. These are architectural roles;
private registry coordinates and family-version mechanics are fixed by APXM
coordinator ADR-0001 and the APXM master plan.

| Public role | Owns | Default dependency and network ceiling |
| --- | --- | --- |
| stable contracts/types | Versioned cross-plane values, identifiers, errors, schemas, and compatibility reporting | Validation/serialization primitives only; no environment, paths, executor, network, storage, providers, logging setup, or mutable global registries |
| AIS specification/authoring | Typed AIS operation specifications and Rust authoring/code-generation descriptions | Stable contracts and deterministic specification/code-generation support; no compiler execution, runtime, provider, storage, or network |
| artifact encoding/admission | Canonical artifact envelope, encoding/decoding, digest verification, limits, and version admission | Stable contracts plus deterministic codecs/cryptography; no compiler, scheduler, providers, persistence implementation, or network |
| compiler embedding | `FrontendGraph`/AIR admission, canonical AIR, MLIR lowering, deterministic optimization, artifact production, diagnostics, cancellation, limits, and version reporting | Contracts, AIS, artifact, deterministic compiler/toolchain components, and provider-neutral descriptor/evidence values only |
| runtime embedding | Artifact admission and the generic execution kernel: scheduling, static callback calls, explicit Program Context/state, regions/yields, grant enforcement, effects protocol, typed evidence/failures, cancellation, checkpoint/resume coordination, and version reporting | Contracts and artifact plus injected public interfaces; no concrete provider, network client, durable store, handler process, sandbox, secret source, or telemetry exporter by default |

The allowed public dependency direction is:

```text
contracts/types
   |       \
   v        v
AIS       artifact
   \       /  \
    v     v    v
    compiler   runtime

focused adapters ---------> runtime public interfaces
applications  ------------> selected public libraries + adapters
```

Compiler and runtime do not depend on one another. Concrete adapters never
become dependencies of contracts, AIS, artifact, compiler, or the minimal
runtime kernel.

### The umbrella facade is deleted

The target release deletes the current public `apxm` umbrella facade. It has no
evidenced consumer, conceals all-plane dependencies, and recreates the
monolithic installation model rejected by D-004A. There is no compatibility
alias or deprecated target path.

No convenience facade is part of canonical v1. Applications compose only the
focused roles and admitted adapters they require.

Application composition crates such as `driver`, CLI, Server composition, and
internal compiler/runtime implementation crates remain non-public unless a
separate clean external consumer journey proves a stable, focused API. An
internal crate is marked non-publishable intentionally rather than accidentally.

### Compiler dependency ceiling

The compiler may depend only on stable contracts, AIS, artifact encoding,
deterministic compiler/toolchain components, and provider-neutral
descriptor/evidence values needed to validate or lower a program. Under the
[current event/runtime ownership](0018-event-readiness-and-local-scheduling-are-agents-semantics.md),
it must not depend on:

- runtime or scheduling implementations;
- concrete model/provider backends or provider SDKs;
- HTTP, service discovery, network clients, TLS/proxy policy, or credentials;
- database, object-store, checkpoint, or state implementations;
- Server operational APIs or current-owner generated clients;
- Server-owned managed occurrence, delivery, target-application, activation,
  effect-work, schedule, Host-gateway, retry/DLQ, or recovery implementations;
- CLI, product UI, or application configuration; or
- a runtime adapter selected through a Cargo feature.

Compilation accepts complete values/bytes and explicit options. It does not
discover inputs from the current directory, `PATH`, default endpoints,
credentials, or a running service. Build-time toolchain discovery is isolated
from the public compile-call contract, declared in the platform matrix, and
recorded in build provenance.

### Runtime kernel and injected adapter interfaces

The runtime embedding library owns APXM execution semantics over injected
interfaces. Concrete model, Capability, atomic Execution Commit,
artifact/output blob access, event transport, handler, narrow Confinement, and
telemetry-observer implementations are focused adapter crates. Production
Implementation Descriptors are admitted at release level; exact instances are
bound only by a separately verified Deployment Composition Manifest. They are
not mutually exclusive runtime-core features.

Public runtime construction receives all resources explicitly through typed
constructors/builders. The default embedding surface contains no concrete:

- model provider or provider credential resolution;
- HTTP client, endpoint, TLS root/proxy choice, or network retry policy;
- durable store, SQLite/Postgres client, object store, or checkpoint directory;
- capability implementation, handler process, language worker, or Confinement
  implementation;
- secret source, environment loader, filesystem layout, or path discovery; or
- logging subscriber, metric exporter, trace exporter, or process-global meter.

The caller composes exactly one verified implementation for each required typed
slot, and no implementation for a slot absent from the selected composition.
Selection is predeclared signed data supplied to a runtime instance, never a
process-wide registry, candidate search, or environment-selected semantic mode.

ADR-0013 further fixes the interface constitution: finite APXM semantics use
closed generated types, implementation identities use typed descriptor refs,
the Composition Root selects inert Runtime Profile data, and managed Server or
standalone composition calls the same `verify_deployment_composition(...)`
contract to materialize immutable Exact Port Bindings.
There is no generic backend registry, implementation enum, string dispatch,
`Any`/JSON escape hatch, first-party bypass, or interface around a helper that
does not cross a real replaceable boundary.

### Caller and APXM ownership

| Embedding caller owns | APXM runtime retains |
| --- | --- |
| async runtime/executor lifecycle and thread ownership | validation of the already-admitted artifact/composition/invocation bundle |
| one atomic Execution Commit implementation plus preparatory artifact/output blob access | graph/runtime state-machine and atomic commit semantics |
| clock, id, randomness, and entropy sources used by deterministic behavior | scheduling and dependency semantics |
| model/provider and Capability implementations | generic node/region ordering, static Hook callback execution, explicit context flow, and model/Capability/program operation semantics |
| network clients, TLS/proxy configuration, endpoints, and credentials | complete Capability Grant validation/enforcement at execution points |
| handler/Confinement worker processes and their lifecycle | typed effect protocol and commit boundaries |
| telemetry sinks/exporters and redaction-capable observers | canonical semantic events and typed outcomes |
| resource ceilings, cancellation source, drain policy, and process shutdown | cancellation observation and APXM lifecycle transitions |

An APXM application may translate environment variables, files, command-line
arguments, or deployment configuration into these explicit resources at its
outer boundary. The reusable libraries themselves do not read ambient
environment, search paths, current directory, global registries, default
endpoints, or process-global mutable state to construct or alter a runtime
instance.

### Cargo features and runtime profiles

Cargo features are additive API or implementation availability only. Enabling
a feature cannot change:

- contract interpretation or version admission;
- generic node/region, static Hook callback, explicit context, model,
  Capability, or Program Invocation semantics;
- authorization, grant enforcement, redaction, or audit event meaning;
- persistence, delivery, cancellation, or recovery guarantees; or
- the typed result/failure semantics of an existing operation.

Mutually exclusive implementations are split into separate adapter crates
instead of feature-selected semantic modes. Every public crate tests
`--no-default-features`, every supported combination, and `--all-features`.
Default features must remain minimal and cannot silently choose a provider,
store, network path, executor, or telemetry exporter.

Named Runtime Profile templates are inert Compatibility Set data declaring the
minimal runtime kernel, exact production Implementation Descriptors,
configuration-contract versions, supported platforms, digests, and conformance
evidence. A Composition Root selects one and verifies a separate deployment
composition containing actual configuration and bindings. A profile is not an
admission actor, Cargo default, runtime string switch, or permission to alter
semantics.

## Authority, secrets, data, and network boundaries

- A library or adapter can perform only the operations permitted by the
  interfaces and authority explicitly supplied to that instance.
- Program Context, Skill content, configuration, and adapter presence never
  grant authority. APXM still requires and enforces complete admitted
  Capability Grants.
- Compiler, minimal runtime, Composition Root, and adapters own no customer or
  provider secret. Auth is sole custodian. The Composition Root may request and
  inject one authorized short-lived credential lease for the exact bound
  adapter without retaining or logging it; stable bindings contain at most an
  opaque credential-reference identity.
- No implicit network access is permitted. Network-capable adapters are
  separately visible in the dependency graph and Compatibility Set.
- Runtime semantic evidence remains provider-neutral. Concrete adapters may
  append attributable implementation evidence but cannot reinterpret the
  canonical event or outcome.

## Failure and recovery behavior

- Missing required resources or adapters fail construction or admission with a
  typed error before execution; no library discovers or selects a fallback.
- A contract, artifact, adapter, profile, or platform outside the active
  Compatibility Set fails closed; no version downgrade is attempted.
- An adapter failure produces the typed APXM failure/cancellation/effect state
  required at that lifecycle point. It cannot change generic node/region,
  callback, explicit-context, or Program Invocation semantics or bypass
  terminal-commit rules.
- Invalid feature/profile combinations fail build, descriptor assembly, or
  startup. They do not survive as partially functional runtimes.
- Dropping a runtime handle does not silently own or terminate the caller's
  executor. The caller controls drain, timeout, and shutdown; APXM exposes
  explicit cancellation and completion handles.
- Multi-instance tests must prove that configuration, state, meters, session
  ledgers, adapter registries, and cancellation do not leak between instances.

## Full-replacement migration

The five target roles and focused adapters are built beside the pinned current
implementation as a new target-only dependency graph. The migration then:

1. moves stable public values out of the current mixed `apxm-core` surface;
2. separates artifact admission from compiler/runtime composition;
3. removes the compiler-to-backend dependency;
4. extracts runtime public interfaces and moves concrete implementations into
   focused adapter crates;
5. replaces ambient configuration and mutable process globals with explicit
   instance resources;
6. moves every first-party application to the new public libraries/adapters;
7. promotes the exact crates, adapters, profiles, applications, and evidence in
   one Compatibility Set; and
8. deletes the facade, old defaults/features, cross-plane dependencies, global
   registries, fallback construction, and obsolete documentation.

There is no compatibility facade, alias crate, old feature profile, dual
runtime composition, or mixed target release. Before the point of no return,
rollback restores one complete previous signed release. After irreversible
promotion, correction is forward-only through a new complete Compatibility
Set.

## Alternatives considered

### Keep the current umbrella facade

Rejected. It has no proven consumer and hides the exact compiler/runtime/
backend dependency boundaries consumers need to reason about.

### Keep concrete implementations as mutually exclusive runtime features

Rejected. Cargo feature unification makes mutually exclusive semantic modes
fragile, and feature-selected providers/stores obscure the installed authority,
network, state, and security surface.

### Let the runtime own Tokio and process configuration

Rejected for the embedding boundary. It prevents callers from controlling
threads, shutdown, multiple instances, tests, and integration with an existing
executor. APXM applications may still own a Tokio runtime at their outermost
composition boundary.

### Publish only compiler and runtime crates

Rejected. Contracts, AIS, and artifact encoding are stable consumer boundaries
with different dependency ceilings; hiding them inside compiler/runtime would
force duplication or inappropriate dependencies.

### Focused libraries with injected adapters

Accepted. The tradeoff is more explicit construction and more individually
described artifacts. The benefit is a genuinely embeddable compiler/runtime,
smaller dependency and attack surfaces, deterministic multi-instance behavior,
and observable provider/network/storage choices.

## Consequences

- Canonical crate coordinates and private registry names are fixed by the
  workspace master plan's C0b/M1 distribution lanes before package work begins.
- The current `apxm` facade is a mandatory deletion candidate, not an undecided
  convenience surface.
- `apxm-core`, `apxm-compiler`, and `apxm-runtime` are implementation evidence;
  their current contents and names do not define the public target roles.
- Applications must construct APXM explicitly and own resource lifecycle.
- Adapter and platform matrices become release evidence rather than hidden
  runtime defaults.
- Canonical v1 Skill discovery and semantic contracts are consumed through the
  stable, acyclic Rust boundary.
- Coordinator ADR-0001 owns family versioning, private coordinates,
  platform/support line, and promotion mechanics.

## Plan

See the [Rust embedding library hardening plan](../agents/rust-embedding-library-hardening-plan.md).
