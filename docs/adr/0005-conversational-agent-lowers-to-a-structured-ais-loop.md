---
status: superseded
date: 2026-07-15
decision: D-001B
owner: APXM agents
superseded_by: ADR-0010
---

# Conversational Agent lowers to a structured AIS loop

> Superseded by [ADR-0010](0010-agent-program-source-owns-context-hooks-and-conversational-loops.md).
> The target keeps the frontend-authored loop and removes the runtime Turn,
> terminal-outcome, rearm, and conversational contract described below.

## Decision

`ConversationalAgent` is a Python and TypeScript frontend construct that lowers
to the generic, versioned `apxm.conversational-loop.v1` contract in the
compiler-owned `FrontendGraph`, AIS/AIR, executable artifact, and APXM runtime.
It is not a runtime type, service, scheduler, host loop, or agent-id-specific
execution mode.

One conversational session admits one active Turn at a time. Inputs are
accepted durably and ordered within that session. Parallel Nodes may execute
inside the authored Turn graph; independent concurrent conversations require
an explicit session or branch rather than concurrent mutation of one session.

The semantic lifecycle is:

```text
session_start once after session admission

loop:
  durably accept one idempotent input and assign its Turn
  reconstruct initial Program Context
  pre_turn
  execute the authored Turn region and its Node/Model/Capability Hooks
  produce a candidate Turn Outcome
  post_turn
  validate Context Deltas, persistence intents, and compaction intents
  commit the terminal Turn checkpoint and durable outcome exactly once
  rearm
```

The terminal commit durably records or enqueues the Turn Outcome before rearm.
HTTP, SSE, WebSocket, mobile, desktop, or Host delivery may occur or resume
after that commit and does not change Turn semantics. A client disconnect is
therefore not a cancellation or successful delivery claim.

A cancellation or failure before terminal commit produces a typed non-success
outcome and records the state of any already committed external effect. A
request arriving after terminal commit cannot undo that Turn; delivery resumes
from its durable cursor. No failure path fabricates a response, repeats a
persistence intent, or silently rearms an uncheckpointed Turn.

## One behavioral source

Frontend Agent Program source is the sole authoring source for loop structure,
Hooks, Nodes, Capabilities, and Turn behavior. The Agent Program Source Bundle
manifest declares identity, frontend entrypoint, handlers, resources, required
Skills and Capabilities, and compatibility metadata. It does not provide a
second behavior language through `[[hooks]]`, `[runtime.loop]`, string-valued
`converse`/`recv` flags, or equivalent runtime configuration.

The compiler extracts one typed representation into the artifact. Under the
[current event/runtime ownership](0018-event-readiness-and-local-scheduling-are-agents-semantics.md),
Server owns managed durability for occurrence, delivery, target application,
activation, effect-work, schedules, and the Host gateway plus
retry/DLQ/recovery and operational APIs. Studio, CLI, and other product
consumers use current-owner generated bindings to select, admit, invoke, or
observe that artifact; none may redefine its loop.

## Context, authority, and persistence

Each Turn reconstructs its initial Program Context from admitted input, static
Agent Program instructions, pinned Skill Bindings, and permitted session
Memory. A previous Turn's local Program Context is not copied implicitly.

Program Context and Turn Outcome contain information, never authority.
Capability execution still requires admitted authority and the common runtime
enforcement path. Persistence and compaction are typed intents validated and
applied through the runtime's exactly-once terminal protocol; a Hook cannot
write durable state directly.

## Failure and recovery

- Duplicate input admission resolves by session plus idempotency key to the
  existing Turn or one new Turn, never two executions.
- A runtime crash before terminal commit resumes or terminates from durable
  evidence; it does not report success.
- A crash after terminal commit replays outcome delivery without repeating the
  Turn or its persistence intents.
- Unknown loop, graph, artifact, Hook, or Program Context versions fail before
  execution.
- A controlling Hook failure follows the fail-closed behavior in
  `apxm.agent-hook.v1`.
- External effects retain their own committed, failed-before-commit,
  outcome-unknown, or compensated state; the loop checkpoint does not redefine
  effect truth.

## Migration

Current host-language conversation loops, direct `GraphBuilder.autonomous`
construction, manifest loop settings, runtime string attributes, and
registration-order behavior are read-only evidence. The target Compatibility
Set replaces them together and rejects the old semantic generation. There is
no dual loop engine, manifest fallback, alias field, or runtime translation.

## Considered alternatives

### Make Conversational Agent a runtime type

Rejected. It would create a second execution model and make frontend source,
artifacts, local embedding, and Server execution diverge.

### Let each host own the input loop

Rejected. Web, mobile, desktop, Embed, CLI, and Gao would disagree on Turn,
context, persistence, cancellation, and recovery behavior.

### Permit concurrent Turns in one session

Rejected as the default contract. It makes context and memory ordering
ambiguous. Explicit graph parallelism and explicit session branching preserve
concurrency without shared conversational mutation.

### Use one structured AIS loop with durable Turn boundaries

Accepted. It preserves one runtime while making frontend ergonomics,
checkpointing, delivery resume, and recovery deterministic.

## Consequences

- Python and TypeScript must lower equivalent conversational programs to
  equivalent loop records and lifecycle evidence.
- Gao must compose this public construct and remove its direct loop.
- `agent.toml` and equivalent manifests become Source Bundle manifests, not
  alternate behavioral programs.
- APXM runtime and Server must serialize same-session Turns and expose durable
  outcome cursors.
- Studio consumes lifecycle evidence and never runs a product-side loop.

## Plan

See the
[Conversational Agent loop implementation plan](../agents/conversational-agent-loop-implementation-plan.md).
