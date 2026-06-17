# Feature Specification: APXM Chat Thin Clients (Server Session SSOT)

**Feature dir:** `specs/0002-apxm-chat-thin-clients/`
**Phase:** specify (next: clarify → plan → tasks → analyze)
**Repo:** `apxm`
**Input:** "Make apxm-server the session SSOT; CLI and Studio become thin protocol
clients. Fix SSE reliability, unify session ledger, typed errors, server-emitted
permission events, session API, generated client, thin CLI pipe."
**Source:** program roadmap charter 0002; architecture review §7 (execution/session
model), §8 (agent-server design). Depends on spec 0001 gate (T073).

This spec freezes the server contract that specs 0003 (Studio) and 0006 (hardening)
build on. Server owns `execution_id` runs and `session_id` ledger/history; clients
deliver input and render output only.

## User Scenarios & Testing

### User Story 1 — Reliable long-lived event streams (Priority: P1)

An operator or agent client opens a long-running conversation stream and receives
every event the server emits for that run — including when the client is slow to
consume or reconnects mid-turn. The stream never silently drops events without a
visible signal.

**Why this priority:** Silent drops corrupt trust in every downstream client
(Studio, CLI, OS workers). Without a reliable stream, session APIs and thin clients
cannot be built on a stable contract.

**Independent Test:** Under deliberate consumer backpressure, a multi-turn session
still delivers a terminal event for every started turn; a reconnecting client can
resume from the last acknowledged event without gaps.

**Acceptance Scenarios:**

1. **Given** a slow consumer on the live execute stream, **When** the server emits
   more events than the buffer accepts, **Then** the client receives an explicit
   overflow or lag signal — never silent loss.
2. **Given** a client that disconnects mid-turn, **When** it reconnects with the
   last acknowledged event id, **Then** it receives the remaining events for that
   turn in order.
3. **Given** a completed multi-turn session, **When** the client reviews the event
   log, **Then** every turn has a matching terminal event (success or typed failure).

### User Story 2 — One session authority (Priority: P1)

Turn caps, tool budgets, and capability grants for a conversation are owned and
enforced by the server keyed on `session_id`. Clients send intent; they do not
maintain a parallel ledger.

**Why this priority:** Duplicate session state between host and server caused drift
in the prior chat model. A single SSOT is prerequisite for thin clients and for
Studio/OS to correlate observability without owning truth.

**Independent Test:** Start a session with a turn cap and tool budget from the
server; exceed the cap on a second client attached to the same `session_id` and
observe the server deny the turn — with no client-side enforcement.

**Acceptance Scenarios:**

1. **Given** a session with a turn cap of N, **When** turn N+1 is submitted,
   **Then** the server rejects it with a typed, recoverable outcome.
2. **Given** a per-tool budget in the session ledger, **When** the budget is
   exhausted mid-session, **Then** further invocations of that tool are denied at
   re-arm/recv without host intervention.
3. **Given** two clients using the same `session_id`, **When** one grants a
   capability for the session, **Then** the other observes the same grant on the
   next turn without re-sending it.

### User Story 3 — Typed, distinguishable errors (Priority: P1)

When a conversation fails, the client can tell whether the agent program faulted
(recoverable, user- or model-facing) versus the server infrastructure faulted
(operator-facing). Errors carry stable machine-readable codes — not only free text.

**Why this priority:** Today all runtime faults collapse to undifferentiated server
errors, forcing clients to parse message text. Typed errors unblock thin clients and
generated SDKs (US6).

**Independent Test:** Trigger a program validation fault and a simulated server
fault; clients receive different status classes and codes without inspecting message
substrings.

**Acceptance Scenarios:**

1. **Given** an invalid agent request (bad artifact, admission denial), **When** the
   server responds, **Then** the client receives a program-fault class with a stable
   code and human-readable detail.
2. **Given** an internal server failure, **When** the server responds, **Then** the
   client receives a server-fault class distinct from program faults.
3. **Given** a typed error on the event stream, **When** the model or UI handles it,
   **Then** it can branch on code without parsing natural-language messages.

### User Story 4 — Server-driven permission prompts (Priority: P2)

When a tool or capability requires human approval, the server emits a permission
event on the run stream and waits for a client response. Clients answer on a
documented response path; the server caches "approve for session" grants in the
session ledger.

**Why this priority:** Interactive approvals are a first-class agent-server concern
(review §8 gap #5). Formalizing the channel lets CLI and Studio share one UX model.

**Independent Test:** Run a session that triggers an approval-gated tool; observe a
permission event on the stream, reply with approve/deny, and see the tool proceed or
skip accordingly — including session-scoped grant caching on a second identical
request.

**Acceptance Scenarios:**

1. **Given** a tool marked ask/approval-required, **When** the agent invokes it,
   **Then** the stream emits a permission event before execution proceeds.
2. **Given** a permission event, **When** the client denies, **Then** the tool does
   not run and the agent receives a recoverable denial outcome.
3. **Given** a permission event, **When** the client approves for the session,
   **Then** a subsequent identical request in the same session does not re-prompt.

### User Story 5 — Session control API (Priority: P2)

Operators and thin clients can query session status, cancel in-flight work, adjust
grants, compact history, and list session events without posting a full conversation
payload each turn.

**Why this priority:** Studio and CLI must stop shipping entire transcripts each
turn; session APIs are how thin clients observe and control long conversations
(review §7 observability split).

**Independent Test:** Create a multi-turn session, then use only session endpoints
to read status, cancel a running turn, add a grant, and fetch event history — without
re-executing prior turns.

**Acceptance Scenarios:**

1. **Given** an active session, **When** status is queried, **Then** the response
   includes turn progress, ledger state, and linked run handles.
2. **Given** a running turn, **When** cancel is requested, **Then** the run stops
   and the stream emits a terminal cancelled outcome.
3. **Given** a long session, **When** compact is requested, **Then** history
   reflects compaction per server policy without client-side transcript management.
4. **Given** a session id, **When** events are listed or streamed, **Then** results
   match the resumable observer ordering keyed by `session_id`.

### User Story 6 — Generated client contract (Priority: P2)

The server's wire surface is described by a generated schema and typed client. CI
fails when server types drift from the checked-in contract — consumers regenerate
instead of hand-editing duplicates.

**Why this priority:** Hand-written routes caused Studio/OS/CLI drift (review §8
gap #1). A generated client is the force multiplier for spec 0003 binding.

**Independent Test:** Change a response field in the server types without updating
the schema fixture; CI contract diff-test fails. Regenerate; CI passes. A sample
consumer compiles against the generated client only.

**Acceptance Scenarios:**

1. **Given** the route and type definitions, **When** the schema is generated,
   **Then** it covers session, run, error, and permission surfaces used by US1–US5.
2. **Given** an intentional wire change, **When** CI runs, **Then** the diff-test
   fails until the schema and client are regenerated.
3. **Given** the generated client, **When** a thin client issues a session status
   call, **Then** it type-checks without hand-rolled request structs.

### User Story 7 — CLI as thin pipe (Priority: P3)

The interactive chat command delivers user input, renders server events, and answers
permission prompts — without enforcing turn caps, budgets, or conversational logic
locally.

**Why this priority:** Proves the contract on the reference CLI host (review §8:
"first-party UIs are already protocol clients"). Depends on US1–US6 landing.

**Independent Test:** Run `apxm chat` against a server-owned session; turn caps and
budgets enforced only server-side; permission prompts round-trip via stream + reply;
no host-side ledger module remains.

**Acceptance Scenarios:**

1. **Given** a server-enforced turn cap, **When** the user exceeds it in the CLI,
   **Then** the CLI surfaces the server's typed denial without local counting.
2. **Given** a permission prompt on the stream, **When** the user answers in the
   terminal, **Then** the reply uses the server response endpoint and the turn
   continues.
3. **Given** the generated client, **When** chat issues API calls, **Then** it uses
   generated types — not parallel hand-rolled wire structs.

### Edge Cases

- Consumer disconnects during permission wait — server times out with typed outcome.
- Two clients approve the same permission concurrently — idempotent resolution.
- Session compact while a turn is in-flight — defined ordering (cancel or wait).
- Reconnect with stale `Last-Event-ID` after compaction — explicit gap signal.
- Empty or unknown `session_id` — predictable not-found or mint path.
- Program fault mid-stream — terminal typed error frame, not truncated silence.

## Requirements

### Functional Requirements

- **FR-001:** The live execute/event stream MUST NOT silently drop events on
  backpressure; overflow MUST surface an explicit client-visible signal.
- **FR-002:** Execute and observer streams MUST support resumption from the last
  client-acknowledged event id without reordering within a run.
- **FR-003:** Turn caps, per-tool budgets, and capability grants MUST be stored and
  enforced by the server keyed on `session_id`.
- **FR-004:** Clients MUST NOT be required to maintain a parallel session ledger for
  caps, budgets, or grants.
- **FR-005:** API and stream errors MUST include stable machine-readable codes and
  distinguish program faults from server faults.
- **FR-006:** Approval-gated actions MUST emit a server→client permission event and
  block until a client response or timeout.
- **FR-007:** Session-scoped approval grants MUST persist in the session ledger and
  suppress repeat prompts for equivalent requests.
- **FR-008:** The server MUST expose session endpoints for status, cancel, grants,
  compact, and events (list/stream) keyed by `session_id`.
- **FR-009:** Cancel MUST apply to every run path that can start a conversation
  turn, not only streaming observer attachments.
- **FR-010:** OpenAPI (or equivalent) schema and a typed client MUST be generated
  from server types and diff-tested in CI.
- **FR-011:** The CLI chat command MUST use the generated client and MUST NOT
  enforce turn caps, tool budgets, or grants locally.
- **FR-012:** Permission replies from the CLI MUST use the documented server
  response endpoint.
- **FR-013:** Session history and run observability MUST remain server-owned; clients
  correlate only via documented APIs (review §7).
- **FR-014:** All delivered capabilities MUST behave identically on CLI and HTTP
  server paths (transport parity).

### Key Entities

- **Session (`session_id`):** Conversation/subject lane across turns and runs;
  owns ledger (caps, budgets, grants) and durable history index.
- **Run (`execution_id`):** One cancellable execution handle for a graph turn.
- **Permission event:** Server→client prompt tied to a run, resolvable via
  response endpoint; may cache grant on session.
- **Typed error:** Machine-readable fault with class (program vs server), code,
  detail, optional recovery hint.
- **Generated client:** Typed consumer derived from server schema; sole wire contract
  for first-party thin clients.

## Success Criteria

### Measurable Outcomes

- **SC-001:** In a backpressure test, 0 silent drops across 100 emitted events;
  every overflow produces a client-visible signal.
- **SC-002:** Reconnect resume delivers 100% of post-ack events in order for 10
  simulated disconnects per session.
- **SC-003:** Turn-cap enforcement on turn N+1 returns typed denial in 100% of
  trials without client-side counting.
- **SC-004:** Program-fault and server-fault scenarios produce distinct error classes
  in 100% of labeled test cases.
- **SC-005:** Permission round-trip (prompt → approve → execute) succeeds in end-to-end
  fixture; session-grant suppresses second prompt.
- **SC-006:** Session status/cancel/grants/compact/events endpoints satisfy their
  acceptance scenarios in contract tests.
- **SC-007:** CI schema diff-test fails on intentional wire drift and passes after
  regeneration.
- **SC-008:** CLI chat exercises server enforcement only; grep of CLI chat sources
  shows no local turn-cap or budget counters.
- **SC-009:** Runtime, server, and compiler invariant suites remain green
  (Constitution #10).

## Assumptions

- Spec 0001 gate (T073) passes before implementation lanes start.
- In-program conversational loop from spec 0001 remains the cognition owner;
  this spec changes the host/server contract, not agent program semantics.
- OpenAPI generation adopts `utoipa`/`aide` or equivalent — choice in `plan.md`.
- Session API shapes follow review §7 semantics (`execution_id`, `session_id`,
  `graph_id`); Studio binding waits for this frozen contract (program roadmap Wave 2).
- Operational hardening (OTLP, rate limits, auth defaults) is deferred to spec 0006
  on shared files (`app.rs`, `error.rs`, `startup.rs`) after this spec lands.

## Clarifications

### Session 2026-06-17

- Q: Merge 0006 hardening into 0002? → A: No — roadmap §6 keeps 0006 Wave 2 after
  0002 freezes shared files.
- Q: Studio changes in scope? → A: No — 0003 consumes the generated client; 0002
  delivers the contract only.
- Q: CLI removes all host loop paths? → A: Legacy host-driven loop may remain for
  non-in-program artifacts; in-program artifacts use thin pipe only (Constitution #2).
