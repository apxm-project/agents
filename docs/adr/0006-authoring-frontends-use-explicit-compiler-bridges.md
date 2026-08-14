---
status: accepted
date: 2026-07-15
decision: D-004B
owner: APXM agents
---

# Authoring frontends use explicit compiler bridges

## Context

Python and TypeScript are APXM authoring frontends, never execution runtimes.
Both must describe the same Agent Program semantics and reach the same Rust
compiler without reimplementing AIR lowering. The current frontends can invoke
the CLI and can mix authoring with execution or transport concerns. That makes
compiler identity, failure behavior, and compatibility depend on ambient
`PATH`, processes, or network state.

Browser authoring adds a different constraint: a browser cannot load the
native Rust bridge, but it must still submit exactly the graph contract emitted
by local TypeScript authoring. A browser also cannot receive authority to
reach private APXM services directly.

## Decision

APXM adopts `apxm.frontend-graph` as the canonical, versioned interchange
contract between authoring frontends and the compiler.

- Python and TypeScript frontend packages are pure authoring libraries. They
  construct and validate language-native values, record `FrontendGraph`, and
  contain no runtime, hidden server, execution client, or private AIR printer.
- A separate Python native compiler bridge, built with PyO3 and maturin, calls
  the public Rust compiler library in-process.
- A separate Node native compiler bridge, built with Node-API, calls that same
  Rust compiler library in-process. It is not part of the browser bundle.
- Browser TypeScript emits the same `FrontendGraph` contract and uses a
  generated, explicitly configured remote compile client.
- Browser compile traffic is authorized by an admitted downstream product
  gateway, which calls the private APXM compile API through a generated
  client. Browsers never call private APXM service endpoints directly.
- Handler bundlers and source-language build tools are separate installable
  surfaces from the pure authoring frontends. Runtime handler bridges remain
  runtime internals rather than frontend execution features.
- The bridge and compiler require an exact admitted graph/compiler contract
  combination from the active Compatibility Set. Unknown or incompatible
  versions fail before compilation.

No supported frontend may:

- shell out to `apxm`, search `PATH`, or depend on a working directory;
- download a compiler or start a local service implicitly;
- fall back automatically between native and remote compilation;
- contain an independent AIR printer or semantic lowering path;
- execute an Agent Program; or
- treat a WASM compiler as the first target.

WASM compilation is not part of APXM v1 and no placeholder, alternate bundle,
or runtime selection hook is reserved for it.

## Authority and network boundary

Native bridges have no network requirement for compilation. The caller
selects native or remote compilation explicitly by importing/configuring the
corresponding surface. Remote clients carry typed identity, authorization,
tenant/resource scope, idempotency, compatibility, and trace metadata; they do
not discover credentials or endpoints from frontend source.

The APXM compile service owns compilation and compiler diagnostics. APXM
Studio owns product authentication, organization scope, policy, quotas, and
the public product API. Its BFF may validate and forward a compile request,
but it does not copy compiler semantics or rewrite `FrontendGraph`.

## Failure behavior

Missing native binaries, unsupported platforms, graph-contract mismatch,
compiler-contract mismatch, remote unavailability, and denied product policy
are distinct typed failures. None may trigger another compilation mode.

A remote request is digest-bound to its complete graph and compiler options.
Retries require an idempotency key and return the same admitted result or a
typed conflict. A native bridge crash fails the compile call; it cannot start a
CLI or remote request as recovery.

## Migration

The target release introduces the versioned graph schema, separates the pure
frontends and build tooling, ships the two native bridges, and adds the
generated remote compile client as one Compatibility Set. It then deletes CLI
subprocess compilation, implicit mode selection, frontend execution helpers,
and any handwritten target wire model. There is no legacy adapter or dual
supported behavior.

Native platform availability is an explicit release-matrix tradeoff. A
platform without an admitted bridge fails installation or import with a clear
diagnostic; it does not receive a hidden compiler mode.

## Alternatives considered

### CLI subprocess as the common bridge

Rejected. Process discovery, quoting, files, exit codes, and CLI version skew
are an application protocol rather than a library contract.

### Automatic local/remote fallback

Rejected. It makes network, data residency, credentials, cost, and compiler
identity implicit and can change semantics between executions.

### WASM compiler for every environment first

Rejected for version 1. It would make browser constraints determine the local
embedding boundary and add a second compiler packaging target before the Rust
library contract is hardened.

### Native local bridges plus an explicit generated remote client

Accepted. It preserves one Rust compiler and one graph contract while making
platform and network choices observable.

## Consequences

- Python and TypeScript remain semantically equivalent frontends.
- Native binary release coverage and ABI testing become release requirements.
- Browser compilation requires an explicit service and product authorization
  path.
- Compiler delivery is no longer coupled to CLI installation.
- D-004C is fixed by
  [ADR-0007](0007-rust-embedding-uses-focused-libraries-and-injected-adapters.md):
  the bridge consumes the focused compiler embedding role, and the compiler
  has no runtime/provider/network/storage dependency.
- The bridge advertises the repository's current contract versions and rejects
  unknown or incompatible artifacts.

## Plan

See the [frontend implementation guide](../../crates/compiler/frontend/README.md).
