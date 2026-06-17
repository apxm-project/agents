# Implementation Plan: APXM Chat Thin Clients

**Phase:** plan (next: tasks) · **Spec:** `./spec.md` · **Evidence base:**
`./research.md` · **Architecture SSOT:** `apxm-studio/docs/infrastructure-architecture-review-2026-06-17.md`
§7, §8 (linked, not forked).

## Summary

Freeze apxm-server as the session SSOT and thin out first-party clients. Fix the
live execute stream's silent drop (review §8 gap #2), unify `SessionLedger` as the
sole authority for caps/budgets/grants (§7), introduce typed program vs server
errors (gap #4), formalize server→client permission events (gap #5), expose session
control APIs (status/cancel/grants/compact/events), generate OpenAPI + typed client
with CI diff-tests (gap #1), and shrink `apxm chat` to a generated-client pipe
(§8 thin-client discipline). Shared integration lands serially in `routes.rs` and
`app.rs`; story work fans out on disjoint files per `tasks.md`.

## Technical Context

- **Language/Version:** Rust (workspace edition); OpenAPI via `utoipa` (or `aide`)
  on `apxm-server` types.
- **Primary Dependencies:** axum, tokio, `apxm-runtime` (SessionLedger, observer
  bus), `apxm-core` (events), serde; new `apxm-client` crate for generated consumer.
- **Storage:** Existing rollout JSONL, IndexDb, checkpoints.sqlite; session history
  via server-owned indexes (no new cross-service store).
- **Testing:** `dekk apxm test`, `dekk apxm test-cli`; contract diff-tests in
  `apxm-server/tests/`; SSE/resume integration tests; SC guards per story checkpoint.
- **Target Platform:** Linux; reference clients — `apxm chat` CLI, HTTP consumers
  (Studio defers to 0003).
- **Project Type:** Multi-crate Rust workspace (`apxm-server`, `apxm-runtime`,
  `apxm-cli`, new `apxm-client`).
- **Performance Goals:** Resumable observer bus remains best-in-class; execute
  stream matches observer durability semantics under backpressure.
- **Constraints:** Transport parity (#1); no client-side ledger (#2); green suites
  (#10); defer OTLP/rate-limit/auth-defaults to 0006 on shared `app.rs`/`startup.rs`.
- **Scale/Scope:** Server + runtime session surfaces + CLI; ~4 crates, no Studio/OS.

## Constitution Check

Gated against `apxm/.spec/memory/constitution.md` v1.0.0.

| Principle | This plan |
|---|---|
| 1. Transport parity | PASS — session API + SSE fixes apply equally to CLI and HTTP |
| 2. Host is a pipe | PASS — US7 removes CLI ledger; cognition stays in program |
| 3. AIR-portability | PASS — no host-side control logic added |
| 4. One Python-handler mechanism | PASS — no new Python invocation path |
| 5. Control not observation | PASS — permission channel is interactive control |
| 6. Named entry | PASS — CLI continues named turn binding via server |
| 7. No dead surface | PASS — wire session API + typed errors; remove opaque 500-only paths |
| 8. Reuse proven substrate | PASS — observer bus, SessionLedger, park/wake reused |
| 9. Waits MUST NOT pin compute | PASS — no change to park/wake semantics |
| 10. Green invariant suites | PASS — SC-009 gate; contract tests additive |

**Shared-file contention (program roadmap §6):** `routes.rs`, `app.rs` are serial
integration only — no `[P]`. Spec 0006 follows after 0002 freezes these files.

## Project Structure

```
crates/tools/apxm-server/src/
  state.rs              # US1: fix TokioChannelEmitter silent drop
  execute.rs            # US1: route execute stream via observer pattern
  runs.rs               # US1: overflow/lag frames on observer bus
  error.rs              # US3: map faults to typed HTTP/stream envelopes
  types/errors.rs       # US3: machine-readable error codes (NEW)
  permissions.rs        # US4: permission event + response handler (NEW)
  sessions.rs           # US5: status/cancel/grants/compact/events (NEW)
  conversations.rs      # US5: align with session API where overlap exists
  routes.rs             # SERIAL: register all new routes
  app.rs                # SERIAL: mount handlers + state
  openapi.rs            # US6: utoipa schema export (NEW)

crates/runtime/apxm-runtime/src/executor/
  session_ledger.rs     # US2: enforce caps/budgets/grants at recv re-arm
  context.rs            # US2: ledger threading + enforcement hooks

crates/tools/apxm-client/   # US6: generated typed client crate (NEW)
  src/lib.rs
  openapi/                  # checked-in schema fixtures

crates/tools/apxm-server/tests/
  contract_errors.rs        # US3 contract tests (NEW)
  contract_session_api.rs   # US5/US6 contract tests (NEW)
  sse_backpressure.rs       # US1 integration (NEW)

crates/tools/apxm-cli/src/commands/
  chat.rs               # US7: thin pipe via apxm-client
  watch.rs              # US7: reuse SseParser for permission frames

specs/0002-apxm-chat-thin-clients/
  contracts/openapi-session-v1.yaml   # exported contract snapshot
  data-model.md, research.md, quickstart.md
```

## Complexity Tracking

No unjustified constitution violations. Native re-use of observer bus for execute
stream is simpler than a parallel transport; generating client from `ServerRoute`
is the codex/opencode borrow (review §8) and avoids hand-written drift.
