---
status: accepted
date: 2026-07-16
decision: D-004D
owner: APXM agents
amends: ADR-0007, ADR-0011, ADR-0012
---

# Core semantics are closed and implementations enter through exact Port Bindings

## Context

ADR-0007 already requires focused Rust libraries and injected adapters;
ADR-0011 requires one end-to-end execution spine. They do not yet state the
complete type and binding law that prevents a future compiler/runtime change
from reintroducing provider branches, string registries, environment-selected
backends, generic plugin escape hatches, or first-party implementation bypasses.

The `agents` owner also needs a precise answer to an apparent tension: APXM
wants exhaustive enums and easy backend replacement. Enumerating backends in
core would make replacement harder, while making semantic states open would
make the execution model ambiguous.

## Decision

`agents` adopts workspace ADR-0009 and the
[portable core interface contract](../agents/portable-core-interface-contract.md).

### Closed semantics, open implementation catalogue

Finite Agent Program meaning is encoded as owner-generated closed enums or
tagged unions: AIR operation kinds, structural terminators, lifecycle and
effect states, Hook events, failure classes, feature states, evidence events,
and other exhaustive vocabularies. Unknown variants fail against the exact
contract digest. Semantic/security values have no `Other`, untyped extension,
or string discriminator. This prohibition applies to generic compatibility
catch-alls such as `Unknown(raw)`; owner-defined epistemic states such as
`OutcomeUnknown`, `UsageUnknown`, and `CancellationUnconfirmed` remain required
closed semantics.

Implementation identity is not an enum. Model-inference adapters, Capability
implementations, confinement providers, stores, transports, handlers, ACP
Client adapters, and telemetry sinks use non-convertible typed
content-addressed references to signed Implementation Descriptors. A Model
Deployment or External Agent Profile composes one such implementation with an
exact model or external peer and is not itself a Port implementation. Adding an
implementation therefore does not modify `apxm-core`, the compiler, AIR, or the
runtime kernel.

### Concrete semantic kernel

The following remain concrete `agents` ownership and are not plugin points:

- FrontendGraph and AIR validation;
- the five public AIR operations;
- structural branch/loop/task/try/yield/return lowering;
- canonical pass ordering and deterministic optimization semantics;
- artifact meaning and admission invariants;
- graph scheduling and NodeExecution/attempt identity;
- Program Instance/Invocation/yield/return state machines;
- static Hook callback placement and Agent Facade/context semantics;
- Capability Grant enforcement and effect-commit rules;
- cancellation observation, limits, terminal commits, and canonical evidence
  meaning.

An implementation cannot override or decorate these semantics. Changing any
of them requires an owner-contract change and accepted ADR, not another
adapter.

### Required replaceable boundaries

The runtime exposes focused Port Contracts for model inference, Capability
execution, exact Program artifact access, one atomic Execution Commit, durable
event wait, handler execution, confinement, Session Output preparation, and
non-authoritative observation. The Execution Commit Port atomically commits the
Program state revision, checkpoint/continuation, effect-journal facts,
canonical evidence, and references to already-written digest-addressed output;
these are not separately replaceable truths. Clock, identity/entropy,
cancellation, limits, executor, drain, and shutdown are explicit caller-owned
resources or focused ports where deterministic behavior requires an operation.

The exact owner contract may split a family into smaller interfaces. It may not
collapse families into one generic backend registry, `dyn Any`, JSON value,
string operation, or service locator.

The compiler has no general plugin pipeline. Artifacts record only authored
semantic requirements: exact model targets, Capability Definitions, handler
requirements, durable-event semantics, and required confinement features. A
Runtime Profile declares deployment-infrastructure slots for execution commit,
event transport, confinement, handler execution, Session Output, observation,
and other instance resources. Invocation Admission separately carries
identities, grants, approval references, budget reservations, credential
leases, deadlines, limits, resolved model bindings, cancellation, and lineage.
The compiler never selects deployment implementations or turns source limits
into authority. A target-lowering Port Contract is permitted only when multiple
real targets preserve identical AIR semantics. Deterministic semantic passes
stay concrete.

### Exact binding before execution

Every Composition Root explicitly selects one signed Runtime Profile. The
profile is immutable Compatibility Set data that predeclares one Implementation
Descriptor digest and one generated binding-payload contract for every
deployment slot; it contains no deployment resource references or invocation
authority. The Composition Root supplies exact configuration and stable opaque
resource references in a Deployment Composition Manifest and invokes the one
library-owned `verify_deployment_composition` path. A managed-service
Composition Root and a standalone embedding Composition Root use this same
verifier. Neither path resolves, searches, ranks, or tries candidates.

Successful verification returns a typed immutable Verified Deployment
Composition and `PortBindingSet`. Each binding contains one exact descriptor,
the generated per-Port-Contract binding payload, and binding-admission evidence
only. Capability Grants, credential leases, budget reservations, invocation
identities, resolved model bindings, and execution evidence arrive separately
through Invocation Admission.

Runtime cannot list, rank, discover, or try implementations. A missing,
unsupported, incompatible, revoked, or unavailable binding is a typed failure.
There is no fallback, first-available behavior, provider/confinement
substitution, or degraded unconfined execution.

### Composition Root and dispatch mechanics

An outer application or embedding host is the Composition Root. It owns the
executor, threads, environment/configuration parsing, clients, processes,
implementation construction, readiness, drain, and shutdown. It may obtain an
authorized short-lived credential lease for one exact bound adapter, inject it,
and then discard it; APXM Auth retains secret custody. The Composition Root
passes the Verified Deployment Composition and caller-owned resources into the
compiler/runtime library.

The semantic contract does not require one Rust dispatch mechanism. A profile
may use generics/static dispatch, trait objects, process-start injection, or a
generated remote client when its exact ABI/wire contract and artifact are
admitted. V1 does not allow ambient dynamic-library/package discovery or
runtime downloading.

### Confinement rule

The runtime kernel never branches on Docker, OCI, namespaces, Firecracker,
Wasm, or another confinement implementation and never calls those facilities
directly. It requests one exact closed Confinement Port covering prepare,
attach, execute, cancel, terminate, attestation, cleanup, roots, mounts,
executables, egress, resources, and lifecycle. Deployment scheduling and remote
worker placement are separate boundaries when present; they are not treated as
confinement implementations. If the required confinement cannot be satisfied
exactly, admission fails.

### First-party equality

APXM's vLLM, ACP, confinement, state/effect store, object storage, event
transport, Capability, handler, and telemetry implementations are ordinary
admitted implementations. They carry descriptors, enter at the Composition
Root, and pass the same suite as another implementation. The kernel has no
internal constructor, direct module call, feature default, or privileged
registration path for them.

Model Deployments, External Agent Profiles, Integration revisions, and Host
connections are higher-level compositions over admitted implementations; they
do not masquerade as Implementation Descriptors. Deterministic test fakes
implement the same Port Contract and pass the owner vectors, but they are
explicitly non-admissible in production and require no release descriptor,
signature, SBOM, or Compatibility Set entry.

## Failure and evidence

Every port returns closed typed outcomes. Model and effect ports keep adapter
attempt outcomes, reconciliation results, runtime NodeExecution outcomes, and
program-visible values distinct. `reconciled` is a transition to proven
committed, proven not committed, or still unknown; it is not a source-visible
result. An APXM Auth denial occurs before adapter dispatch, while a provider
refusal is typed provider output. External-send boundaries preserve a stable
effect id and return the owner-defined outcome-unknown state when completion
cannot be proven. An implementation may append attributable evidence but
cannot redefine canonical lifecycle, authority, usage, redaction, or success.

The Execution Commit Port is the one authoritative commit boundary for Program
state, effect facts, canonical evidence, and output references. Best-effort
telemetry observation is separate; its outage cannot change program truth.

## Full-replacement migration

The target migration removes:

- provider/vendor/platform branches from core/compiler/runtime;
- string-keyed backend registries and generic backend traits;
- `Other(String)`/`Unknown(raw)` semantic escape variants and copied
  operation/state strings, without deleting owner-defined uncertainty states;
- compiler dependencies on runtime/backends/network/stores;
- runtime dependencies on provider SDKs, process/container APIs, SQL/object/
  broker clients, filesystem defaults, and telemetry exporters;
- environment/PATH/cwd/service discovery and default implementation selection;
- first-party bypass constructors, mutually exclusive semantic Cargo features,
  global registries, and fallback chains; and
- old adapter/binding readers, aliases, translators, and dual construction.

No compatibility interface or mixed port generation remains in canonical v1.

## Alternatives considered

### Enumerate implementations in core

Rejected. It is exhaustive but hard-codes the implementation catalogue and
requires a compiler/runtime release for every new backend.

### Open semantic values and one generic plugin API

Rejected. It permits implementations to invent behavior and defeats exhaustive
validation, authority, and evidence.

### Trait every dependency

Rejected. Interfaces are reserved for proven replaceable boundaries; internal
semantic code remains direct and concrete.

### Closed semantic types with narrow exact-bound ports

Accepted. It preserves one PXM while allowing infrastructure to be replaced.

## Consequences

ADR-0007's runtime interfaces, the Rust embedding plan, compiler/runtime lane
descriptors, adapter crates, Runtime Profiles, and Compatibility Set now obey
this exact binding contract. Port conformance, forbidden-edge/literal scans,
first-party equality, replacement tests, and multi-instance isolation are v1
release gates rather than optional hardening.
