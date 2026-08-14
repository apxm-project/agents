---
status: accepted
date: 2026-07-24
owner: APXM agents
amends: ADR-0006, ADR-0007, ADR-0015
---

# Tool authoring and handler execution are separate

## Context

APXM source uses `Tool` to declare a typed, model-callable Capability. A
separate TypeScript package helper has also described local handler bundles.
Those two roles used the same word but had no explicit boundary. That invited
three errors: exposing a handler manifest or worker protocol to authors,
treating Node as an APXM runtime, and adding a second Python packaging surface
before Python package-local handlers existed.

The canonical runtime is Rust. TypeScript and Python are equivalent authoring
frontends, never runtimes. The Rust-owned `apxm.handler-manifest` contract
already carries artifact-local handler source. The runtime receives an exact
admitted Capability implementation through its `CapabilityPort`; it does not
discover a language worker, load a manifest from disk, or execute a source
bundle directly.

## Decision

### 1. One Tool vocabulary has two explicit roles

| Role | Author surface | Owner | May execute? |
| --- | --- | --- | --- |
| Agent Program Tool reference | `Tool[I, O](capability_ref)` / `Tool<I, O>(capabilityRef)` | Python and TypeScript frontends | No; static capture only |
| Package-local Tool implementation | `Tool.define({ ... })` in a handler module | language-specific build tool | No; build and packaging only |
| Handler manifest | `apxm.handler-manifest` | Rust contracts/artifact owner | No; immutable artifact metadata |
| Capability execution | exact admitted `CapabilityPort` implementation | Rust runtime plus injected adapter | Yes |

An Agent Program source file never imports a handler worker, manifest format,
protocol frame, grant, endpoint, runtime profile, or language-runtime object.
A package-local handler never changes Agent Program control flow, compiler
lowering, admission, or authority.

### 2. Package handlers use one small definition object

The package-local TypeScript authoring surface is one `Tool` object:

```typescript
export const PrepareValidation = Tool.define({
  name: "prepare_validation",
  description: "Prepare an APXM validation request.",
  input: Tool.object<PrepareValidationInput>({
    request: Tool.text({ minLength: 1 }),
    plan: Tool.text({ minLength: 1 }),
  }),
  run(input) {
    return Tool.answer({
      validation_request: buildRequest(input),
      next_action: "submit_for_validation",
    });
  },
});
```

`Tool.object` and `Tool.text` generate the internal object schema. `Tool.answer`
accepts exactly one typed business-result object. Authors never write JSON
Schema, NDJSON frames, handler IDs, manifest paths, grants, approval decisions,
or runtime dispatch code. The private worker unwraps the answer envelope before
returning the ordinary typed result value to its adapter boundary.

The generated manifest remains the sole serialized handler representation. It
is build output, is validated by Rust, and is never edited by hand.

### 3. Rust keeps the execution boundary

Node and Python helper processes, when an admitted implementation needs them,
are private handler-worker adapters. They are selected and supplied by the
Composition Root like every other implementation; they are not a frontend
runtime, a default process, a fallback, or a runtime registry.

The runtime accepts the typed Capability result through its injected port and
records the ordinary `capability.invoke` outcome. It never branches on
TypeScript, Python, a handler source path, or a named example. Capability
identity, read-only classification, approval, grants, and resource authority
remain in the capability definition, Auth admission, and exact bound adapter;
a handler cannot self-authorize.

### 4. Language support is explicit

TypeScript package-local handlers may use the definition object above because
the TypeScript bundler already produces `apxm.handler-manifest`. Python
Agent Programs continue to declare the same typed `Tool` references and lower
through the same FrontendGraph.

Python package-local handlers are not implied by Python Agent Program
authoring. They are added only by a separate owner change that supplies an
equivalent Python definition object, deterministic bundler, private admitted
worker adapter, and conformance vectors against the same Rust-owned manifest.
Until then, a Python Agent Program binds admitted Rust/host Capabilities rather
than receiving a partial second packaging API.

### 5. The examples demonstrate the boundary

Conversational remains the primary frontend reference. Coder demonstrates
read-only coding Capability references and typed proposal results. (Gao,
which formerly demonstrated APXM Capability references and typed
planning/validation results, is retired — see ADR-0010's historical Gao
note.) Coder's TypeScript package handlers are packaging fixtures only; they
do not create a Coder runtime and cannot execute outside an admitted
Capability implementation.

## Consequences

- Frontends remain equivalent at static Tool binding and FrontendGraph intent;
  only language-native handler build tools differ.
- `Tool.define` is the one TypeScript package-handler definition form. The old
  raw-schema helper is removed rather than retained as an alias.
- The Rust handler-manifest contract and `CapabilityPort` remain the execution
  authority. Any worker protocol is private implementation transport.
- A handler result is a typed program value, not a user-authored JSON protocol
  envelope. The envelope used by build-worker conformance is not observable to
  Agent Program source or Model-facing Tool consumers.
- A Python handler toolchain is a deliberate future capability, not a required
  duplicate for this TypeScript-only example set.

## Alternatives considered

### Make Node the standard Tool runtime

Rejected. It violates the Rust runtime boundary, creates a hidden process
dependency, and makes execution depend on a source language.

### Give every frontend its own manifest and worker protocol

Rejected. It duplicates the Rust-owned handler contract and allows language
surfaces to drift in authorization, schema, and result behavior.

### Keep raw JSON Schema and worker frames in handler source

Rejected. They are serialization details, not the authoring model. They make
ordinary Tool implementation harder and create an accidental public protocol.

### Add Python package handlers immediately

Rejected. No Python package-handler compiler or admitted worker adapter exists
today. Advertising one would create a partial runtime path and an unsupported
second API.

## Amendment — Python declares handlers; it still cannot execute them

The decision above deferred the whole Python handler surface, authoring
included, because no Python bundler or worker adapter existed. That conflated
two questions. Whether a language *can declare* a shipped Capability is a
question about the authoring surface, which
`contracts/vectors/apxm.frontend-surface.json` states and
`dekk agents check-frontend-surface` proves for every registered language;
whether one *can run* is a question about the packaging and execution path,
which is what §4 was actually protecting. Deferring the first to protect the
second left Python unable to state something true about a package it ships, and
left the gap invisible, because a missing projection is not something a name-set
comparison can see.

`apxm_program.handlers.capability(...)` now declares a shipped Capability in the
same shape `Tool.define` uses, and both return the Capability id they implement
rather than a descriptor that has to agree with a string written elsewhere.
`crates/machine/contracts/src/types/handler_manifest.rs` still admits only
`HandlerLanguage::TypeScript`, and there is still no Python bundler or worker
adapter, so a Python handler declaration does not become an executable handler
in a compiled artifact. That remains a deliberate future capability; what
changed is that the surface no longer pretends the declaration is one too.
