---
status: accepted
date: 2026-07-15
adopts: APXM coordinator ADR-0001 / D-004A
---

# Agents publishes focused surfaces in the APXM release family

## Context

APXM workspace release governance owns the cross-repository decision recorded
in `apxm-project/docs/adr/0001-apxm-is-one-release-family-with-distinct-installable-surfaces.md`.
That decision requires one Compatibility Set with distinct Rust libraries,
language authoring frontends, generated remote clients, and applications.

This repository owns the compiler, runtime, Python/TypeScript frontend, and CLI
parts of that family. Under the
[current event/runtime ownership](0018-event-readiness-and-local-scheduling-are-agents-semantics.md),
Server owns its routes and managed occurrence, delivery, target-application,
activation, effect-work, schedule, Host-gateway, retry/DLQ, recovery, and
operational APIs. This repository does not own customer product SDKs, APXM
Studio, registry-wide release governance, or the Compatibility Set schema.

## Adoption decision

The `agents` repository will supply focused release descriptors for:

- stable Rust contracts/types;
- Rust AIS specification/authoring;
- Rust artifact encoding/admission;
- Rust compiler embedding;
- Rust runtime embedding and separately admitted adapter crates/profiles;
- an equivalent Python authoring frontend;
- an equivalent TypeScript authoring frontend; and
- the APXM CLI application.

The Python and TypeScript frontends both produce the compiler-owned
`apxm.frontend-graph.v2`, delegate canonical AIR/MLIR/artifact lowering to the released
Rust compiler boundary, and contain no execution runtime. D-004B is resolved by
[ADR-0006](0006-authoring-frontends-use-explicit-compiler-bridges.md): separate
PyO3/maturin and Node-API native bridges call the Rust compiler in-process,
while browsers use an explicit generated remote compile client. There is no
CLI subprocess, hidden service, or automatic native/remote fallback.

The handwritten `@apxm/client` is not a target contract. Remote DTOs and clients
must be generated from the owning service descriptions and released as
separate client surfaces. D-004C is resolved by
[ADR-0007](0007-rust-embedding-uses-focused-libraries-and-injected-adapters.md):
the repository supplies five focused Rust library roles—contracts/types, AIS,
artifact, compiler embedding, and runtime embedding—plus separately admitted
adapters. The existing `apxm` Rust facade is deleted without an alias.

Every admitted artifact reports its exact contract versions, source revision,
features/platform, and build digest for the coordinator-owned Compatibility
Set. No artifact is published independently as a supported release.

## Boundary consequences

- Compiler-only consumers acquire no runtime, current-owner generated Server
  operational client, Server-managed durability implementation, provider
  backend, or network dependency by default.
- Runtime consumers select explicit backend profiles without changing contract
  or enforcement semantics.
- Frontend packages acquire neither runtime behavior nor another AIR printer.
- CLI remote commands remain generated-client consumers; CLI local commands
  consume public compiler/runtime APIs explicitly.
- Applications are never located or launched as hidden library fallbacks.
- APXM Studio is a separate application owned by the Studio repository and is
  excluded from this repository's release descriptors.

## Migration and failure behavior

Target packages are built in parallel from current source evidence. One complete
Compatibility Set cutover deletes independent package publishers, handwritten
target clients, unadmitted facade paths, and old installation guidance. There
are no package aliases, mixed frontend/compiler combinations, old runtime
fallbacks, or two supported release lines.

A missing compiler, unsupported platform, incompatible contract, or artifact
outside the selected Compatibility Set fails explicitly. A pure frontend may
not search `PATH`, download a compiler, start a service, select a remote mode,
or invoke an old printer. The caller must import/configure a native bridge or
generated remote client explicitly.

## Plan

Implementation is coordinated by
`apxm-project/docs/distribution/apxm-distribution-family-and-library-hardening-plan.md`.
The repository-local compiler-delivery sequence is specified in the
[compiler bridge delivery plan](../agents/compiler-bridge-delivery-plan.md).
The repository-local Rust boundary sequence is specified in the
[Rust embedding library hardening plan](../agents/rust-embedding-library-hardening-plan.md).
Implementation starts from the APXM master plan's contract-freeze gate. The
coordinator-owned v1 family version, registry, platform, and support mechanics
are fixed by coordinator ADR-0001 and that master plan.
