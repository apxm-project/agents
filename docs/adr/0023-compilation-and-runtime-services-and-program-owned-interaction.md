---
status: accepted
date: 2026-08-15
owner: APXM agents
requires: ADR-0006, ADR-0007, ADR-0008, ADR-0009, ADR-0010, ADR-0011, ADR-0013, ADR-0015, ADR-0016, ADR-0018, ADR-0020
amends: ADR-0011
---

# Compilation and Runtime Services, program-owned interaction, and zero-legacy cutover

## Status and authority

Accepted for the Agents repository. It is subordinate to ADR-0013 (implementations enter through exact Port Bindings) and ADR-0009 (AIR has five public semantic operations). It amends ADR-0011 by restating how the one compilation-and-execution spine is hosted: source-to-artifact and artifact-to-lifecycle are separately owned services joined only by a committed digest-bound artifact. It does not reopen ADR-0011's one-spine semantics, ADR-0010's Program ownership of conversational loops, or ADR-0018's portable Event readiness.

Implementation follows the phase sequence recorded in
[docs/ideas/cli-interaction-runtime-service-migration.md](../ideas/cli-interaction-runtime-service-migration.md).
This ADR freezes architecture and vocabulary. Protocol schemas, crate extraction, and CLI cutover are later phases, not this record.

## Context

The production `apxm` binary currently imports compiler, frontend-codegen, kernel, and execution implementation crates. Canonical compile and execute Composition Roots live in CLI command modules. `EventRef` is a non-empty string, `EventPort` exposes only `await_event`, and public `resume_event` accepts a raw delivered value. Parked `await.event` records `WaitingEvent` then `InvocationParked` with `CommittedYield`. Hidden `Chat`, `Watch`, and `Rollout` parser variants remain as rejecting stubs. `agent build` regenerates integrity metadata rather than a committed executable artifact. Package compile is a Python-only `compile-service-canonical` path.

Those facts contradict the product-neutral machine: agent behavior belongs in compiled Programs; hosts admit, execute, and project; source never enters the runtime.

## Decision

### Separate Compilation and Runtime Services joined only by an artifact

APXM exposes two product-neutral service contracts.

1. **Compilation Service** validates an exact package snapshot, selects the manifest-declared Python or TypeScript frontend, binds the exact toolchain, lowers through the Rust compiler core, and commits one digest-bound executable artifact plus build evidence. It has no runtime, Program Instance, continuation, or provider dependency.
2. **Runtime Service** admits a verified artifact plus Invocation Admission, constructs ports, drives Program Instance and Invocation lifecycle, projects observations, and commits evidence. It accepts no source, FrontendGraph, AIR text, frontend selector, compiler option, or build ref as executable truth.

The thin `apxm` command shell may sequence a Compilation Client call then a Runtime/Interaction Client call. That convenience is not a combined protocol, Composition Root, or failure domain. Compilation failure or artifact-commit uncertainty creates no Program Instance.

Local default: separately supervised child processes over bidirectional stdio. Optional persistent Runtime Service over a Unix socket. Loopback HTTP is an independently owned edge (Phase 10). Non-loopback binding requires a later accepted security ADR and passing conformance; it is not a configuration flag on this record.

### The harness is an Interaction Program

All agentic or conversational behavior is an ordinary compiled APXM Program (the **Interaction Program**). The Runtime Service, terminal renderer, and OpenAI HTTP adapter are host infrastructure. Child agents exist only through authored `program.new` / `program.invoke`. The UI cannot spawn a child from a path, provider name, or `ProgramRef` typed as chat input.

No AIS operation is added. Behavior composes from `model.call`, `capability.invoke`, `program.new`, `program.invoke`, `await.event`, and compiler-owned structural control flow.

### Frontends

Python `apxm_program` and TypeScript `@apxm/frontend` are the complete v1 authoring set. The package manifest selects exactly one. Missing support fails closed with no language, local/remote, bridge, or PATH fallback. There is no Rust authoring frontend and no `frontend = "rust"` selector. Rust owns compiler core, FrontendGraph verification/lowering, AIR/AIS, artifact production, and native Python/Node bridges.

### Canonical lifecycle and Event API

Program Instance, Program Invocation, NodeExecution, Event, continuation, cancellation, outcome, and evidence are runtime-owned types. Native protocol, OpenAI DTOs, transport, and UI state are projections and must not fabricate canonical transitions.

There is one source-neutral `EventOccurrence<T>` and one `EventApplication<T>`. Source kinds (human, HTTP/CloudEvents, webhook, broker, schedule, provider, Hook bridge, device, sensor) are provenance and adapter profiles, never runtime Event variants. CloudEvents and W3C WoT are optional edge mappings, not kernel semantics.

Event type, EventRef, source occurrence, target application, source contract, and wait binding are distinct identities. Explicit fan-out is one authorized, idempotent application per target. A source id alone never grants wake authority.

Hook lifecycle stays static (`before`/`after` on existing scopes). A Hook-originated signal becomes an Event only through a declared signaling Capability or an admitted committed-Hook-evidence bridge. Ordinary callbacks, provisional Hook observations, and `HookExecuted` facts do not fulfill EventRefs. No `HookScope::Event` and no dynamic Event listeners.

Structural `yield` commits `CommittedYield` / `InvocationCommitted`. `await.event` parks the same Invocation in `WaitingEvent` until exact wake returns it to `Running`. It must not record `CommittedYield`. Public raw `resume_event` is removed; wake is an internal runner operation that requires a proven terminal Event application.

This implements ADR-0018 without moving managed source ingestion, connector operations, queue durability, retry/DLQ, or multi-tenant delivery into the portable kernel. Those remain whatever the Composition Root binds behind the durable-event Port Contract (ADR-0020).

### Approval broker is an exact Port Contract

Unresolved `Ask` is resolved through an **approval-broker Port Contract** bound by the Runtime Composition Root (ADR-0013). The Runtime Service maps broker requests onto protocol `approval.request` / client resolution. Noninteractive OpenAI serving requires a pre-resolved permission policy and fails closed if execution reaches unresolved `Ask`. Denied or unresolved `Ask` cannot reach Capability execution.

### Native protocol has no Chat entity

The Runtime Service protocol exposes Program Instance, Program Invocation, approval, cancellation, observation, continuation, content, evidence, and canonical Event ingress. It defines no canonical `Chat`, `Conversation`, `Thread`, `Turn`, or `Message` runtime entity.

Inbound OpenAI HTTP (`/v1/models`, `/v1/chat/completions`, bounded `/v1/responses`) is a new protocol adapter over the Runtime Service. It shares no DTO, route, or argument compatibility with retired `apxm chat`. Serving aliases bind already committed artifacts; the edge cannot compile.

### Zero-legacy cutover

Retired `Chat`, `Watch`, and `Rollout` are deleted, not aliased, forwarded, deprecated, or kept as rejecting stubs. Old artifacts and client/session/rollout records are inert: the new decoder reports unsupported format/version and offers no import. Equivalence fixtures exist only on the unreleased implementation branch and are removed before cutover release.

### Crate ownership (frozen names)

```text
apxm-compilation-protocol     crates/compiler/service-protocol
apxm-compilation-service      crates/compiler/service
apxm-compilation-client       crates/tools/compilation-client
apxm-runtime-protocol         crates/runtime/service-protocol
apxm-runtime-service          crates/runtime/service
apxm-interaction-client       crates/tools/interaction-client
apxm-openai-compat            crates/runtime/openai-compat
apxm-event-http               crates/runtime/event-http
```

Compiler-contract codegen currently hosted by `apxm-cli` moves to compiler-development tooling invoked through Dekk. Production `apxm` depends only on the thin shell, package contracts, Compilation Client, Interaction Client, and presentation libraries.

Compilation Service and Runtime Service do not depend on each other. Interaction Client depends on Runtime protocol, not source-port, frontend bridges, compiler, kernel, execution, or backends.

## Terminology

| Term | Owner | Must not own |
| --- | --- | --- |
| Interaction Program | Agent Program source and artifact | Terminal, transport, admission, credentials |
| `apxm` command shell | CLI application boundary | Frontend capture, lowering, runtime construction |
| Compilation Client | Build client library | AST analysis, AIR lowering, runtime calls |
| Compilation Service | Compiler Composition Root | Runtime execution, frontend fallback |
| Authoring frontend | Python or TypeScript package | AIR/AIS, artifact, runtime, CLI |
| Rust compiler core | Compiler embedding | Source UX, runtime construction |
| Executable artifact boundary | Artifact encoding and content store | Mutable source, UI session, continuation |
| Interaction Client | TUI/headless client | Model/tool loop, authority, execution truth |
| Runtime Service | Product-neutral Composition Root | Conversation policy, product control plane |
| Canonical root Event API | Runtime contracts, kernel, execution, evidence | HTTP/broker parsing, raw continuation delivery |
| Event source contract / adapter | Deployment composition and edge | Event lifecycle truth, continuation mutation |
| Hook lifecycle | Compiled Hook bindings | Implicit Hook-to-Event bus |
| OpenAI protocol adapter | Runtime Service HTTP edge | Runtime truth, grants, retired chat routes |
| Client interaction record | Interaction Client | Program Context, continuation, authority |
| Execution Commit / evidence | Runtime | Provisional token rendering |
| Approval broker Port | Runtime Composition Root | UI layout, OpenAI DTO state |

*Build* means source package to committed artifact. *Run* means admitted artifact to lifecycle. No service collapses those verbs.

Closed interaction ingress: next Invocation input; Event occurrence; approval resolution; cancellation/close control; outbound observation. Nothing else.

## Lifecycle ownership

| State | Canonical owner | Allowed projections |
| --- | --- | --- |
| Program Instance / Invocation / NodeExecution | Runtime | Native protocol, OpenAI mapping, UI cards |
| Event reservation, wait, terminal, consumption, wake | Runtime Event API | Native protocol, Event HTTP, authorized observations |
| Continuation / Execution Commit / evidence | Runtime | Inspect/reconcile APIs; never client rewrite |
| OpenAI request/stream/`finish_reason` | OpenAI adapter | Maps to runtime outcomes; not stored as truth |
| Transport connection / stdio framing | Transport adapter | Reconnect; cannot override evidence |
| Editor, history, layout, slash commands | Interaction Client | May call generic Runtime requests only |
| `.apxm/client/` interaction record | Interaction Client | Refs to artifact/instance/cursors only |

## Event gap inventory (current tree)

| Gap | Location | Removal |
| --- | --- | --- |
| `EventRef` is a non-empty `String` with no generation/lineage | `crates/runtime/kernel/src/runtime_ports.rs` | Phases 1–2 |
| `EventPort` exposes only `await_event` | same | Phases 1–2 |
| Public `resume_event(..., Value)` | `crates/runtime/execution/src/driver.rs` | Phases 2–3; protocol never exposes it |
| Parked `await.event` records `CommittedYield` | `driver.rs` `InvocationParked` fact | Phases 1–2 |
| Static declaration name lowered as EventRef | frontend capture | Phases 1–2, authoring in 5 |
| No admitted Hook-signal bridge | runtime + examples | Phases 0–1, 4–5, 10, 12 |
| No universal source envelope | contracts | Phases 0–2, 6, 10, 12 |

IoT: raw samples reduce under a declared source contract before admission. Hard-real-time control remains excluded (ADR-0018). Sequence, freshness/clock quality, content identity, and typed gap outcomes are preserved; accepted occurrences are never silently sampled away.

## CLI import audit (current `apxm-cli`)

Assigned destination (deletion or move phase in parentheses):

| Current dependency / module | Destination |
| --- | --- |
| `apxm-execution`, `apxm-kernel`, `apxm-inference`, `apxm-backends`, `apxm-backend-registry`, `apxm-capability` | Runtime Service (3, 11) |
| `apxm-ais`, `apxm-program`, compiler via `compile_service_canonical` / `canonical_air` | Compilation Service or Dekk compiler-dev tools (3, 11) |
| `crates/tools/cli/src/frontend/` | compiler-development tooling (11) |
| `commands/canonical_execute.rs` `CanonicalRuntime` | Runtime Service; public CLI command deleted (3, 11) |
| `commands/compile_service_canonical.rs` | Compilation Service; public command deleted (3, 11) |
| `commands/cli.rs` `Chat`/`Watch`/`Rollout` | deleted (11) |
| `commands/session.rs` | classify: Session Output vs residue; residue deleted (9, 11) |
| `commands/agent.rs` `Build` integrity regeneration | internal stage of `apxm build`; public meaning removed (7, 11) |
| `Agent` New/Sync/Lint/install, `Org`, `Doctor`, `Backend`, `Ops`, `Process`, `Cache`, `Tokenize` | remain as separately named package/ops commands |
| clap parse, config path, `--json`, `--trace` | thin shell |

## AIS audit

The change composes from the existing public AIS operations plus structural control flow. This ADR does not add an operation. If a later phase cannot express required Program behavior without a new op, stop and invoke AIS-op design; do not smuggle an op into a service crate.

## Inconsistency inventory

| Item | Phases |
| --- | --- |
| Buffered inference despite a stream contract | 4, then 7–8 |
| Unresolved `Ask` becomes refusal | 4, then 8 and 10 |
| CLI-owned Composition Root | 3, deleted in 11 |
| Hidden Chat/Watch/Rollout | 11 |
| Direct buffered `execute-canonical` | 3, 7, then 11 |
| Ambiguous Session terminology | 0, 9, then 11 |
| Yield / event-wait conflation | 0–2, demonstrated in 5 |
| Incomplete interaction schemas | 1, 5, 7–8 |
| CLI README lacks client/service distinction | 0, 8, then 11 |
| Minimal EventRef/EventPort/raw `resume_event` | 1–3, projected in 6 and 10 |
| `await.event` recorded as committed yield | 1–3 |
| Static declaration name as EventRef | 1–2, frontend in 5 |
| No Hook-signal bridge | 0–1, 4–5, 10, 12 |
| No universal source envelope / IoT policy | 0–2, 6, 10, 12 |
| Monolithic CLI dependency cone | 0, 3, 6–8, then 11 |
| Python-only package compile | 1, 3, 5–7, then 11 |
| Integrity-only `agent build` | 0–1, 3, 7, then 11 |
| Source-package interaction without artifact boundary | 0–1, 3, 6–8, 12 |
| Frontend/codegen inside CLI | 0, 3, then 11 |
| Single-file source port vs package snapshot | 0–1, 3, 5, 12 |

A phase cannot close with an item marked “later” unless this table already names that later phase.

## Zero-legacy deletion manifest

Every entry is deleted in the named phase. None is adapted, aliased, or migrated.

| Identifier | Kind | Phase |
| --- | --- | --- |
| `Commands::Chat` and `apxm chat` | CLI parser + dispatch + rejecting stub | 11 |
| Chat flags `--agent`, `--air`, `--server`, `--session-id`, `--capability-grant-id`, `--import`, `--backend`, `--model`, `--owner` on Chat | CLI flags | 11 |
| `Commands::Watch` and `apxm watch` | hidden command | 11 |
| `Commands::Rollout` and `RolloutAction` list/replay/archive/compact | hidden command | 11 |
| Retired server-chat routes `POST /v1/agents/{id}/sessions` as Chat meaning | routes/docs | 11 |
| Old rollout JSONL/SQLite index readers/writers in CLI | readers | 9, 11 |
| Old session transcript/message importers | readers | 9, 11 |
| `compile-service-canonical` public command | CLI | 11 (routed via Compilation Service in 3) |
| Public `execute-canonical` | CLI | 11 (service-backed in 3) |
| Public `canonical-air` | CLI | 11 (Dekk compiler-dev) |
| Public `apxm codegen` in production CLI | CLI | 11 |
| Public integrity-only `apxm agent build` meaning | CLI | 11 |
| `CanonicalRuntime` constructed from CLI product path | code | 11 |
| Public `EventPort::await_event` / `EventRef::new(String)` / `resume_event` | API | 2 |
| Feature flags or env switches preserving old chat/compile paths | config | none may be added; any found deleted in 11 |
| Equivalence fixtures spanning old/new | tests | deleted in 11 cutover commit |

Canonical-only reachability tests (`dekk agents test-canonical-only`) must reject retired commands, hidden chat loops, direct CLI compilation/execution, and combined compiler/runtime paths after Phase 11.

## Consequences

- Interactive quality comparable to Crush is a client of compiled Programs, not a second agent runtime.
- Users rebuild source through Compilation Service; old artifacts and records are rejected.
- Rollback is source rollback of the coordinated cutover, not a protocol downgrade or dual-read.

## Boundaries

This ADR does not authorize a managed product control plane, a Rust authoring frontend, Runtime compilation, a combined compiler/runtime protocol, LSP/MCP UI, transcript injection into Context, or execution of retired records.
