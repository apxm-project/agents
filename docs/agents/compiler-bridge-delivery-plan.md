# APXM Compiler Bridge Delivery Plan

- Status: canonical APXM v1 owner plan
- Approved: 2026-07-15
- Decision: [ADR-0006](../adr/0006-authoring-frontends-use-explicit-compiler-bridges.md)
- Owner: APXM `agents`
- Migration: full replacement; no legacy support

## Goal

Ship one versioned `FrontendGraph` contract and one Rust compiler through
explicit local Python, local Node, and remote browser/server compilation
surfaces. Remove subprocess compilation, frontend runtime behavior, hidden
fallbacks, and competing AIR printers in the same Compatibility Set.

Implementation begins from the APXM master plan's clean-baseline and v1
contract-freeze gates. Canonical v1 has no pending or deferred architecture
branch.

## Target surfaces

| Surface | Responsibility | Dependency/network ceiling |
| --- | --- | --- |
| `FrontendGraph` contract | Versioned authoring-to-compiler DTO and conformance vectors | Contract types only; no runtime or network |
| Python authoring frontend | Language-native Agent Program authoring and graph recording | Pure Python plus generated contract types |
| TypeScript authoring frontend | Language-native Agent Program authoring and graph recording | Browser-safe TypeScript plus generated contract types |
| Python compiler bridge | In-process call into the Rust compiler | PyO3/maturin native binary; no network/runtime |
| Node compiler bridge | In-process call into the Rust compiler | Node-API native binary; no network/runtime/browser bundle |
| Remote compile client | Generated calls to the owning compile service | Explicit network transport; no compiler/runtime |
| Handler build tools | Reproducible callback/handler bundling | Source-language build dependencies; no runtime execution |
| Rust compiler library | Validate graph/AIR, lower, optimize, and produce artifact/diagnostics | Focused compiler embedding role fixed by [ADR-0007](../adr/0007-rust-embedding-uses-focused-libraries-and-injected-adapters.md) |

Final package coordinates are release-governance data, not semantic names in
this plan.

## Cross-cutting invariants

1. Every mode accepts the same canonical `apxm.frontend-graph.v1` value and
   produces the same compiler result or typed diagnostic for the same
   Compatibility Set.
2. Frontend source is the sole source of Agent Program behavior. A Source
   Bundle manifest identifies entrypoints, handlers, resources, and
   requirements but does not restate Hooks, loop semantics, or graph behavior.
3. Native and remote are explicit imports/configurations, never fallbacks.
4. Python, Node, browser, CLI, Server, and Studio paths do not own semantic
   lowering.
5. Exact contract and compiler versions fail closed before compilation.
6. Secrets, product grants, provider credentials, and runtime state never
   enter `FrontendGraph`.

## Delivery sequence

### B0 — Freeze baseline and conformance corpus

Deliverables:

- inventory all Python and TypeScript graph DTOs, AIR printers, CLI subprocess
  calls, compile HTTP DTOs, execution helpers, bundlers, and runtime bridges;
- classify each path as target owner, generated consumer, build tool,
  runtime-internal bridge, or deletion candidate;
- capture positive and negative equivalent Python/TypeScript programs,
  including Conversational Agent, Agent Facade Hooks, explicit context flow,
  Skills Discovery Capabilities, executable Capabilities,
  resources, and handlers;
- record current platform and publish workflows without changing them.

Gate B0: every current path has one target disposition and every conformance
fixture names its expected semantic graph rather than unstable formatting.

### B1 — Own `apxm.frontend-graph.v1`

Dependencies: APXM master A0 owner descriptor and C0b contract-development
bundle.

Deliverables:

- define the Rust-owned graph schema, canonical serialization, digest, limits,
  extension rules, and typed diagnostics;
- generate Python and TypeScript DTOs or prove mechanical equivalence;
- include exact Program reference, Hook binding, explicit context/state,
  Capability, Skill-discovery, handler, resource, and artifact references;
- reject unknown security- or semantics-relevant fields and versions.

Gate B1: Rust, Python, and TypeScript pass identical positive/negative vectors;
serialization is deterministic; no frontend has a handwritten competing DTO.

### B2 — Harden the Rust compiler library boundary

Dependencies: [ADR-0007](../adr/0007-rust-embedding-uses-focused-libraries-and-injected-adapters.md)
and RLIB-1-RLIB-4 of the
[Rust embedding plan](rust-embedding-library-hardening-plan.md).

Deliverables:

- expose a stable in-process compile request/result/diagnostic API;
- accept complete bytes/values and explicit options rather than paths or
  ambient process state;
- eliminate compiler dependencies on runtime, Server, OS, provider adapters,
  storage, and network according to D-004C;
- make cancellation, resource limits, source maps, and deterministic outputs
  explicit;
- prove the CLI and compile service are consumers of this same library API.

Gate B2: a dependency-boundary test passes, two isolated in-process callers
produce digest-identical results, and no compiler API reads ambient endpoints,
credentials, or fallback configuration.

### B3 — Split pure authoring frontends

Dependencies: B1.

Deliverables:

- reduce Python and TypeScript frontend packages to authoring, validation, and
  graph recording;
- move compiler invocation behind separate compiler-bridge packages;
- move handler bundling behind separate build-tool packages;
- remove HTTP/runtime execution, service discovery, CLI invocation, and AIR
  printing from frontend exports;
- keep browser-safe TypeScript free of native imports.

Gate B3: frontend dependency scans contain no runtime/server/client/native
bridge; browser bundling succeeds; equivalent language fixtures emit the same
graph digest.

### B4 — Deliver the Python native bridge

Dependencies: B1-B3 and the coordinator-owned v1 platform/version matrix.

Deliverables:

- expose the Rust compiler API through a minimal PyO3 surface;
- build wheels with maturin for each admitted Python/platform/architecture
  tuple;
- map panics, cancellation, limits, and compiler diagnostics to typed Python
  failures without losing structured fields;
- publish provenance, SBOM, signatures, and Compatibility Set metadata.

Gate B4: wheel install/import/compile tests pass in clean environments; missing
or unsupported native binaries fail clearly; no subprocess or remote fallback
is observable.

### B5 — Deliver the Node native bridge

Dependencies: B1-B3 and the coordinator-owned v1 platform/version matrix.

Deliverables:

- expose the same Rust compiler API through a minimal Node-API surface;
- build native artifacts for each admitted Node/platform/architecture tuple;
- preserve structured diagnostics, cancellation, limits, and source maps;
- prevent the native package from entering browser exports or bundles;
- publish provenance, SBOM, signatures, and Compatibility Set metadata.

Gate B5: clean-install and compile tests pass on every admitted matrix cell;
browser builds prove the native bridge is absent; no subprocess or remote
fallback exists.

### B6 — Deliver explicit remote compilation

Dependencies: B1-B2, the coordinator-owned generated-client contract, and
Studio product-boundary contracts.

Deliverables:

- publish the owning APXM compile service description and generated client;
- bind requests to graph digest, options, tenant/resource scope, compatibility
  set, idempotency key, caller identity, and trace context;
- impose request-size, resource, timeout, retention, and diagnostic-redaction
  policy at admission;
- expose an APXM Studio BFF route that authenticates and authorizes the product
  caller before orchestrating/proxying to private APXM services;
- prohibit browser access to private APXM endpoints and prohibit Studio from
  interpreting or rewriting graph semantics.

Gate B6: browser-to-Studio-to-APXM contract tests pass; direct browser/private
APXM access is denied; retry and mismatch tests are deterministic; network
failure never selects native compilation.

### B7 — Separate handler build tools and runtime bridges

Dependencies: B3 and the versioned handler contract.

Deliverables:

- build handler bundles reproducibly from Source Bundle inputs;
- digest-bind handler code, schemas, resources, toolchain, and target runtime;
- keep build tools out of pure frontend dependency graphs;
- keep Python/JavaScript handler execution bridges private to admitted runtime
  workers and images;
- reject a handler bundle outside the selected Compatibility Set.

Gate B7: reproducible-build fixtures pass; frontend imports do not install a
handler runtime; runtime workers cannot reinterpret frontend graph behavior.

### B8 — Cross-mode conformance and supply-chain evidence

Dependencies: B1-B7.

Deliverables:

- compile the full corpus through Rust-direct, Python-native, Node-native, CLI,
  remote service, and Studio browser paths;
- compare canonical diagnostics, graph digests, artifacts, source maps, and
  contract metadata;
- test unsupported platforms, corrupt binaries, ABI mismatch, version skew,
  request replay, denial, timeout, cancellation, and service failure;
- retain source, dependency, build, signature, SBOM, artifact, and test digests.

Gate B8: all admitted modes are semantically equivalent and every forbidden
fallback and dependency boundary has a negative test.

### B9 — Full-replacement cutover

Dependencies: B0-B8 and the complete APXM Compatibility Set promotion gate.

Deliverables:

- promote all compiler, frontend, bridge, build-tool, client, CLI, Server, and
  product pins atomically;
- remove CLI/PATH compilation, hidden downloads/services, auto fallback,
  frontend execution helpers, competing printers/DTOs, and old docs;
- reject old graph, handler, artifact, and compiler contracts at admission;
- archive migration-only fixtures/tools outside supported runtime artifacts.

Gate B9: repository, package, image, and runtime scans find only the target
paths; rollback restores a complete previous Compatibility Set, never a mixed
bridge generation.

## Dependencies and parallel work

- Schema/vector design may proceed in parallel with read-only baseline and
  platform inventories.
- Implementation waits only for its explicit APXM master-plan baseline and
  input-contract gates.
- B2 consumes the documented D-004C public compiler role and waits for
  RLIB-1-RLIB-4; packaging matrices, compatibility metadata, and generated
  clients consume the coordinator-owned canonical v1 contracts.
- Hook/Context and Conversational Loop contract vectors feed B1 and the shared
  cross-mode corpus.
- Native Python and Node work may proceed in parallel after B1-B3; neither
  defines a semantic contract.

## Rollback and completion

Before cutover, discard a failed candidate. After promotion and before the
declared point of no return, restore one entire previous Compatibility Set and
matching product pins. Never install a prior bridge against a target compiler
or vice versa. After irreversible effects, stop intake and repair forward.

The plan completes only when B9 evidence is tied to exact source, graph,
compiler, native binary, generated client, artifact, image, and product
digests, and all old compilation paths are absent.
