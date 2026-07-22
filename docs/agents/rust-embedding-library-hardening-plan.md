# APXM Rust Embedding Library Hardening Plan

- Status: canonical APXM v1 owner plan
- Approved: 2026-07-15
- Updated: 2026-07-16 for the canonical composition/AIR and portable-core contracts
- Decision: [ADR-0007](../adr/0007-rust-embedding-uses-focused-libraries-and-injected-adapters.md)
- Owner: APXM `agents`
- Baseline: `agents@9e26a62adebb`
- Migration: full replacement; no legacy facade, feature profile, or runtime composition

## Goal

Deliver five focused, stable Rust library roles for contracts/types, AIS,
artifact encoding/admission, compiler embedding, and runtime embedding. Make
the compiler independent of runtime/provider/network/storage concerns, make the
runtime a deterministic semantic kernel over explicitly injected interfaces,
ship concrete implementations as focused adapters, and delete the umbrella
facade and ambient composition in one Compatibility Set cutover.

Implementation starts after the APXM master plan's clean-baseline and relevant
v1 contract-freeze gates pass.

## Non-goals

- no package naming or registry decision inside this repository; coordinator
  ADR-0001 fixes the private APXM v1 family coordinates and shared version line;
- no Python, TypeScript, CLI, Server, or Studio runtime;
- no monolithic facade or compatibility alias;
- no concrete provider, durable store, network client, secret source, handler,
  confinement implementation, or telemetry exporter in the minimal runtime
  dependency graph;
- no second compiler, AIR printer, scheduler, Hook driver, Program Context
  engine, Conversational Loop, or grant-enforcement path;
- no features that change semantic, authority, persistence, audit, redaction,
  lifecycle, or failure behavior; and
- no old and target Rust compositions in one supported release.

## Target roles and dependency law

This plan uses architectural roles; coordinator ADR-0001 maps them to the
canonical private v1 coordinates.

| Role | Public journey | Allowed direct dependency classes | Forbidden defaults |
| --- | --- | --- | --- |
| contracts/types | Construct, validate, serialize, and inspect versioned APXM values | minimal validation/serialization/hash support | env/path discovery, async executor, network, storage, provider, logging setup, mutable registry |
| AIS specification/authoring | Build/inspect typed AIS descriptions and generation metadata | contracts plus deterministic specification/codegen support | compiler execution, runtime, provider, storage, network |
| artifact encoding/admission | Encode/decode, digest, limit-check, and admit artifacts | contracts plus deterministic codec/cryptographic support | compiler, scheduler, provider, persistence implementation, network |
| compiler embedding | Compile complete values/bytes into admitted artifacts with structured diagnostics | contracts, AIS, artifact, deterministic compiler/toolchain support | runtime, backend, HTTP, service, store, credentials, provider SDK |
| runtime embedding | Construct independent runtime instances from a Verified Deployment Composition and execute admitted artifacts under separate Invocation Admission | contracts, artifact, async/interface primitives proven necessary | concrete provider/network/store/secret/handler/confinement/exporter |

Focused adapter roles include model/provider, Capability, Program Artifact
Access, atomic Execution Commit, durable event, handler worker,
confinement, Session Output, and telemetry/export. Each production adapter has
one owner, one explicit public interface, one dependency/network ceiling, and
one Compatibility Set descriptor.

## Dependencies

- D-001C supplies the discovery-only Skill catalogue/association contract consumed by
  runtime admission.
- ADR-0008, ADR-0009, ADR-0010, and the canonical Agent Program composition
  and AIR contract supply Program Instance, operation, callback, explicit
  context, generic loop/yield, and evidence semantics.
- ADR-0013 and the portable core interface contract supply closed semantic
  types, focused Port Contracts, Implementation Descriptors, Exact Port
  Bindings, Runtime Profile data, Deployment Composition Manifests, Invocation
  Admission, the shared deployment verifier, atomic Execution Commit,
  confinement law and first-party equality.
- The APXM master contract spine supplies the Agent Definition, Source Bundle,
  handler, artifact, and semantic contract-version cutover.
- D-004B and the compiler bridge plan consume the public compiler role.
- Coordinator ADR-0001 and the APXM master plan supply family version rules,
  platform/MSRV matrices, private registries, generated-client ownership,
  candidate/promotion semantics, and support policy.
- APXM Auth/Studio/platform ADRs constrain runtime data, telemetry,
  durability, and operational profiles without changing kernel semantics.
- Full replacement requires target-only promotion and whole-release rollback.

## Delivery sequence

### RLIB-0 — Freeze evidence and disposition every current surface

Deliverables:

- pin all current Rust manifests, dependency trees, public exports, feature
  graphs, build scripts, examples, first-party consumers, publish flags, unsafe
  blocks, environment/path reads, globals, process launches, network clients,
  stores, providers, handlers, confinement mechanisms, and telemetry setup;
- classify every crate/module/export as one target public role, focused
  adapter, application composition, internal implementation, one-time
  migration input, or deletion candidate;
- prove whether any external or first-party Rust consumer requires the current
  facade; absence of evidence retains the deletion decision;
- record compiler-only and runtime-minimal current dependency graphs and binary
  sizes as comparison evidence; and
- build a forbidden-edge/global-state inventory with owner and target
  disposition for every finding.

Gate RLIB-0: every current surface and dependency edge has exactly one target
disposition; no current crate name or `pub use` is assumed public merely because
it exists.

### RLIB-1 — Define stable contracts/types

Dependencies: master A0 owner descriptor, C0b contract-development bundle, and
`agents` ADR-0008 through ADR-0011.

Deliverables:

- define the versioned public identifiers, requests, results, diagnostics,
  errors, schemas, compatibility reports, and provider-neutral evidence shared
  across planes;
- generate finite semantic vocabularies as closed enums/tagged unions and use
  non-interchangeable typed refs/newtypes for implementation identities,
  digests, quantities and resources; prohibit generic `Other(String)`/
  `Unknown(raw)` escapes while preserving owner-defined epistemic states such as
  `OutcomeUnknown`, `UsageUnknown`, and `CancellationUnconfirmed`;
- separate runtime/compiler implementation values, environment/path helpers,
  build utilities, subscribers, executable discovery, and mutable registries
  from the public contract role;
- establish canonical serialization, unknown-field/version behavior, limits,
  validation, redaction metadata, and API-evolution rules;
- generate or drift-check consumer-language representations where the owning
  contract requires them; and
- add positive/negative vectors and public-API snapshots.

Gate RLIB-1: the contracts-only clean consumer builds with no executor,
network, store, provider, compiler, runtime, filesystem-layout, logging-setup,
or mutable-global dependency; all version and malformed-input vectors pass.

### RLIB-2 — Isolate AIS specification and authoring

Dependencies: RLIB-1.

Deliverables:

- expose the typed AIS operation/specification catalogue and deterministic
  Rust authoring/generation surface needed by compiler and tooling;
- remove runtime scheduler, provider, service, state, and application
  configuration dependencies;
- make generated metadata reproducible and digest-bound to its owning source;
- prove AIS can be consumed without loading or executing an artifact; and
- retain direct AIR as an explicitly versioned compiler input without creating
  another frontend semantic owner.

Gate RLIB-2: two clean builds generate digest-identical specification metadata;
the dependency scan contains no compiler execution, runtime, provider, store,
or network edge.

### RLIB-3 — Isolate artifact encoding and admission

Dependencies: RLIB-1 and the master C0b artifact contract digest.

Deliverables:

- own the artifact envelope, canonical encoding, digest/signature references,
  limits, contract pins, authored semantic requirements, handler/resource
  references, and admission report;
- reject deployment-infrastructure slots, Exact Port Bindings, grants,
  credential leases, budget reservations, endpoints, or placement from the
  artifact;
- separate pure encoding/admission from compiler construction, runtime
  scheduling, filesystem loading, object storage, and network retrieval;
- accept bytes/readers through explicit caller APIs and return typed failures;
- specify extension and unknown-version behavior without legacy downgrade; and
- add corruption, truncation, oversized-input, wrong-digest, incompatible
  contract, and unsupported-extension vectors.

Gate RLIB-3: a clean codec/admission consumer validates complete in-memory
artifacts without compiler/runtime/network/store dependencies; negative vectors
fail before execution.

### RLIB-4 — Harden compiler embedding

Dependencies: RLIB-1-RLIB-3, D-004B, and the compiler bridge B1 contract.

Deliverables:

- define a stable in-process request/result/diagnostic API for FrontendGraph and
  direct admitted AIR inputs;
- emit typed Artifact Semantic Requirements for exact model targets,
  Capability Definitions, handlers, durable events, and confinement features
  without resolving a concrete implementation, endpoint, process or deployment;
- accept complete values/bytes, explicit compiler options, cancellation,
  resource limits, source maps, and deterministic toolchain identity;
- remove direct dependencies on runtime, concrete backends, provider SDKs,
  network, credentials, service clients, storage, and application composition;
- isolate build-time MLIR/toolchain discovery from public invocation and record
  every toolchain input in provenance;
- make CLI, native Python/Node bridges, and compile service consume the exact
  same public compiler API; and
- define panic containment and stable diagnostic mapping at FFI boundaries.

Gate RLIB-4: two independent in-process compiler instances produce
digest-identical artifacts and diagnostics; forbidden-edge scans pass; no call
reads cwd, `PATH`, endpoints, credentials, or fallback configuration; clean
Rust/PyO3/Node-API fixtures use public APIs only.

### RLIB-5 — Define runtime interfaces and instance-owned state

Dependencies: RLIB-1, RLIB-3, ADR-0008, ADR-0009, ADR-0010, ADR-0011,
and the accepted Agent Program composition/AIR contract.

Deliverables:

- define public injected interfaces for clock/id/randomness, model/provider,
  Capability invocation, Program Artifact Access, one atomic Execution
  Commit, durable events, handler/confinement, Session Output preparation,
  telemetry observation, limits, cancellation, and shutdown;
- implement the focused Port families and immutable typed binding bundle
  defined by the portable core contract; prohibit one generic backend map,
  `dyn Any`, JSON/string dispatch, service locators and late discovery;
- define Runtime Profile as immutable Compatibility Set data with one exact
  Implementation Descriptor and generated binding-payload contract per
  Deployment Port Slot; it contains no deployment resource ref or invocation
  authority;
- expose one library-owned `verify_deployment_composition` path used identically
  by managed Server and standalone embedding Composition Roots; it accepts only
  the explicitly selected Runtime Profile and Deployment Composition Manifest
  and performs no catalogue access, resolution, ranking, or fallback;
- build one runtime instance from the resulting Verified Deployment Composition
  through an explicit typed builder with no default provider, store, network,
  secret, process, confinement implementation, exporter, or executor;
- accept Invocation Admission separately for every invocation and keep grants,
  approval references, credential leases, budget reservations, resolved model
  bindings, deadlines, cancellation, and lineage out of Exact Port Bindings;
- make the Execution Commit Port atomically commit Program state/continuation,
  effect-journal facts, canonical evidence, and references to prepared Session
  Output so crashes cannot create competing truths;
- keep model/Capability adapter attempts, reconciliation, NodeExecution
  outcomes, and source-visible values as distinct closed types;
- move scheduler registries, session ledgers, meters, locks, configuration, and
  behavior state from process globals into instance-owned values;
- keep canonical APXM semantics—admission, generic scheduling, static Hook
  callback execution, explicit Program Context, region/yield/invocation
  ordering, grant enforcement, effect protocol, and typed evidence/failures—
  inside the kernel without introducing a conversation-specific runtime type;
- specify ownership/borrowing, `Send`/`Sync`, backpressure, cancellation,
  drain, shutdown, panic, and adapter-timeout behavior; and
- make the caller's async executor lifecycle explicit without nested hidden
  runtime creation.

Gate RLIB-5: two differently configured instances execute concurrently in one
process without state/configuration/telemetry leakage; deterministic fakes
produce identical evidence; managed and standalone composition fixtures produce
the same verified binding set from the same profile/manifest; atomic commit
crash vectors expose no partial state/effect/evidence truth; missing resources
fail construction/admission; the minimal dependency scan contains no forbidden
concrete implementation.

### RLIB-6 — Extract focused adapters

Dependencies: RLIB-5 and the relevant later data/authority decisions.

Deliverables:

- create separately owned adapter crates for every admitted provider,
  Capability implementation family, Program Artifact Access, atomic Execution
  Commit store, durable event transport, handler worker, confinement provider,
  Session Output store, and telemetry exporter;
- define each adapter's secrets, data, network, filesystem, process, retry,
  timeout, redaction, shutdown, and failure ceiling;
- require explicit construction and instance registration—never environment or
  process-global discovery inside the kernel;
- prohibit an adapter from minting/expanding grants or reinterpreting APXM
  lifecycle semantics;
- require Auth-issued short-lived credential leases at the invocation/effect
  boundary; no adapter or Composition Root becomes secret custodian;
- keep Model Deployments, External Agent Profiles, Integration revisions, and
  Host connections as higher-level compositions referencing admitted
  Implementation Descriptors, never as Implementation Descriptors themselves;
- emit package, source, platform, dependency, SBOM, provenance, signature,
  conformance, and configuration-contract evidence per production adapter; and
- require every first-party implementation to use the same descriptor,
  Composition Root, Exact Port Binding and conformance path with no internal
  constructor or direct kernel registration.

Deterministic test fakes implement the same Port Contract and vectors but are
non-admissible in production. They require no release signature, SBOM,
Implementation Descriptor, Runtime Profile slot, or Compatibility Set entry.

Gate RLIB-6: each adapter passes public-interface conformance and negative
authority/network/secret tests in isolation; removing one adapter does not
alter the kernel or another adapter's dependency graph.

### RLIB-7 — Define additive features and named runtime profiles

Dependencies: RLIB-1-RLIB-6 and the coordinator v1 release matrix.

Deliverables:

- inventory and minimize features for each public crate;
- prove every feature is additive availability and cannot change existing
  semantics, authority, audit, redaction, lifecycle, persistence, or failure;
- replace mutually exclusive backend/store modes with separate adapter crates;
- test no-default, every supported combination, and all-features builds;
- define named runtime profiles as immutable Compatibility Set compositions of
  kernel, exact Implementation Descriptors, generated binding-payload contracts,
  platforms, and conformance evidence, with no deployment resource ref,
  customer secret ref, credential lease, grant, budget, or invocation id;
- define Deployment Composition Manifests as installation-specific exact
  configuration/resource/binding facts validated against one selected profile;
  and
- prohibit implicit default profiles and runtime string mode selection.

Gate RLIB-7: feature-unification tests cannot create contradictory behavior;
every named profile names exact artifacts/digests and passes the same semantic
vectors; every deployment manifest verifies only against its explicitly
selected profile; undeclared combinations fail descriptor assembly/startup
without search.

### RLIB-8 — Harden the public consumer and supply-chain surface

Dependencies: RLIB-1-RLIB-7 and the coordinator v1 release matrix.

Deliverables:

- add clean external compiler-only, artifact-only, runtime-minimal, and
  profile-specific consumer repositories/fixtures using public APIs only;
- add managed and standalone embedding fixtures that select the same Runtime
  Profile, supply the same Deployment Composition Manifest, call the same
  `verify_deployment_composition` API, and receive the same binding proof;
- set explicit MSRV, Rust edition, platform/architecture/libc matrix, public
  dependency policy, unsafe-code policy, documentation, examples, and support
  expectations;
- run API/SemVer diff, minimal-versions where supportable, package-content,
  duplicate-dependency, license, vulnerability, source-size, binary-size,
  documentation, and `cargo package` dry-run gates;
- build exact FFI compiler bridges against the public library without internal
  path dependencies;
- generate owner release descriptors with contract, feature/profile, source,
  platform, SBOM, provenance, signature, and conformance digests; and
- verify that application crates remain explicit consumers and internal crates
  are intentionally non-publishable.

Gate RLIB-8: every clean journey succeeds from candidate artifacts only; no
internal module/path dependency or facade import exists; all release evidence
is immutable and complete.

### RLIB-9 — Cut over and eradicate the old composition

Dependencies: RLIB-0-RLIB-8 and the complete promoted APXM Compatibility Set.

Deliverables:

- move every first-party compiler, bridge, CLI, Server, OS, and runtime
  application consumer to the target roles and explicit adapters;
- promote the five roles, selected production adapters, named Runtime Profiles,
  applications, and release evidence atomically; deployment-specific manifests
  remain installation admission records rather than Compatibility Set content;
- delete `crates/facade`, umbrella re-exports, compiler-to-backend edges,
  concrete runtime defaults/features, core env/path/application helpers,
  process-global runtime registries/meters/state, implicit executor ownership,
  and obsolete install/examples/docs;
- reject old artifacts, profiles, contract combinations, and mixed dependency
  graphs at build/admission/startup; and
- archive migration-only inventories/fixtures outside supported runtime and
  publication paths.

Gate RLIB-9: repository, dependency, package, binary, image, and documentation
scans contain only target paths; clean consumers use no facade or internal
crate; multi-instance and cross-surface conformance pass against the exact
promoted manifest.

## Parallel work and synchronization

- RLIB-1 contracts, RLIB-2 AIS, and RLIB-3 artifact design may proceed in
  parallel after their shared contract ids close, using the same vectors.
- RLIB-4 compiler and RLIB-5 runtime can proceed in parallel once their public
  inputs stabilize; neither may import the other.
- Adapter owners can implement against RLIB-5 interface vectors in parallel,
  but no adapter changes kernel semantics.
- Compiler bridge B2 consumes RLIB-4; bridge packaging consumes the
  coordinator v1 release matrix.
- Agent Program composition/AIR, discovery-only Skills, and contract-cutover plans
  supply RLIB-5 admission/lifecycle vectors.
- All lanes synchronize only through versioned contracts, generated code,
  public interfaces, artifacts, owner descriptors, and conformance evidence.

## Security and failure invariants

- Context, configuration, installed adapters, environment variables, and
  Cargo features never grant authority.
- Runtime Profiles and Exact Port Bindings contain no invocation authority;
  Invocation Admission carries current identities, grants, approvals, leases,
  budgets, model effects, deadlines, cancellation, and lineage separately.
- The minimal compiler/runtime performs no implicit network, credential, path,
  executable, store, or service discovery.
- Finite semantic states are closed generated types; implementation catalogues
  are typed descriptor refs, never provider enums or generic strings.
- A network-, secret-, filesystem-, process-, or store-capable adapter is
  visible as a separate dependency and Compatibility Set artifact.
- No library creates process-global mutable registries or subscribers that
  make instances affect one another.
- All external input is limit-checked and version-admitted before expensive
  compilation or execution.
- Unknown contracts, artifacts, adapters, profiles, features, and platforms
  fail closed with typed diagnostics and no downgrade/fallback.
- Adapter failure cannot skip authorization, static callback execution,
  explicit context flow, atomic Execution Commit, cancellation, redaction, or
  audit requirements.
- Generic compatibility catch-alls are forbidden, but owner-defined epistemic
  variants remain required and cannot be collapsed into an implementation
  error or best-effort log.
- Build/release jobs contain no customer/provider secret and retain signed
  SBOM/provenance/scan evidence for every production crate and adapter.

## Migration, rollback, and recovery

The current implementation is read-only migration evidence. Target crates and
adapters are built and tested in isolation, first-party consumers switch in the
same candidate, and one complete Compatibility Set is promoted. No facade
alias, deprecated feature, dual composition, runtime fallback, or mixed crate
generation is supported.

Before the declared point of no return, rollback restores one complete prior
signed APXM release and matching application/configuration/state snapshot. It
never combines a prior runtime with target adapters or target contracts. After
irreversible publication or state effects, stop intake and repair forward by
building and promoting a new complete Compatibility Set.

## Completion definition

This plan completes only when:

1. all five public roles pass clean-consumer, API, dependency, feature, package,
   platform, and supply-chain gates;
2. compiler-only has no runtime/provider/network/storage edge;
3. runtime-minimal has no concrete provider/network/store/secret/handler/
   confinement/exporter implementation or hidden executor;
4. every production implementation is an explicit admitted adapter, while
   deterministic test fakes are provably non-admissible;
5. multi-instance deterministic tests prove no process-global state leakage;
6. named Runtime Profiles are exact Compatibility Set data rather than feature
   defaults, Deployment Composition Manifests hold installation bindings, and
   Invocation Admission holds current authority/lease/budget/model facts;
7. all first-party applications and compiler bridges consume public roles;
8. the facade and all old ambient/default/cross-plane paths are deleted;
9. one promoted Compatibility Set ties every crate, production adapter,
   Runtime Profile, contract, source, platform, binary, image, SBOM,
   provenance, signature, and conformance result
   to immutable digests without containing deployment or invocation facts; and
10. every required Port Contract, Implementation Descriptor, Deployment
    Composition Manifest and Exact Port Binding passes closed-type, replacement,
    first-party-equality, confinement and no-hardcoding/no-discovery gates from
    the portable core contract.
