# Tasks: APXM Chat Thin Clients

**Inputs:** `plan.md`, `spec.md`, `data-model.md`, `contracts/`, `research.md`.
Paths are relative to the `apxm` repo root. `[P]` = parallel-safe (different file,
no unfinished dependency). `[US#]` tags story tasks. **Shared integration:**
`routes.rs` and `app.rs` are **serial only** — never `[P]`.

## Phase 1 — Setup

- [x] T001 Scaffold `apxm-client` crate (workspace member + empty lib) — `crates/tools/apxm-client/Cargo.toml`, `crates/tools/apxm-client/src/lib.rs`.
- [x] T002 [P] Contract test harness module — `crates/tools/apxm-server/tests/contract/mod.rs`.

## Phase 2 — Foundational (blocks all stories)

- [x] T010 [US1] [US3] Fix `TokioChannelEmitter` silent drop: lag signal or delegate to bounded send — `crates/tools/apxm-server/src/state.rs`.
- [x] T011 [P] [US3] Machine-readable error types (`program_fault` / `server_fault`, stable codes) — `crates/tools/apxm-server/src/types/errors.rs`.
- [x] T012 [P] [US3] Contract tests for typed error envelope — `crates/tools/apxm-server/tests/contract_errors.rs`.
- [ ] **Checkpoint:** `cargo check -p apxm-server` green; contract_errors tests compile.

## Phase 3 — User Story 1: Reliable SSE streams (P1)

**Goal:** no silent drop on live execute stream; resumable ordering preserved.
**Independent Test:** backpressure fixture — 0 silent drops, explicit overflow signal.

- [x] T020 [US1] Route `/v1/execute/stream` through observer-bus pattern (seq + broadcast) — `crates/tools/apxm-server/src/execute.rs`.
- [x] T021 [P] [US1] Emit explicit overflow/lag frames on observer backpressure — `crates/tools/apxm-server/src/runs.rs`.
- [x] T022 [P] [US1] SSE backpressure integration test (SC-001, SC-002) — `crates/tools/apxm-server/tests/sse_backpressure.rs`.
- [x] **Checkpoint:** US1 — `dekk apxm test -p apxm-server sse_backpressure` passes.

## Phase 4 — User Story 2: One session authority (P1)

**Goal:** server-owned ledger for caps, budgets, grants.
**Independent Test:** turn N+1 denied without client-side counting (SC-003).

- [x] T030 [US2] Enforce turn caps and tool budgets at recv re-arm — `crates/runtime/apxm-runtime/src/executor/session_ledger.rs`.
- [x] T031 [P] [US2] Ledger enforcement hooks on `ExecutionContext` — `crates/runtime/apxm-runtime/src/executor/context.rs`.
- [ ] **Checkpoint:** US2 — session ledger unit tests + SC-003 contract scenario green.

## Phase 5 — User Story 3: Typed errors (P1)

**Goal:** distinguish program faults from server faults with stable codes.
**Independent Test:** labeled fault fixtures return distinct classes (SC-004).

- [x] T040 [US3] Map `RuntimeError` and admission faults to typed HTTP responses — `crates/tools/apxm-server/src/error.rs`.
- [x] T041 [P] [US3] Typed error frames on SSE event stream — `crates/tools/apxm-server/src/types/responses.rs`.
- [ ] **Checkpoint:** US3 — `dekk apxm test -p apxm-server contract_errors` passes.

## Phase 6 — User Story 4: Permission events (P2)

**Goal:** server→client permission prompts with response endpoint and session grant cache.
**Independent Test:** approve round-trip + session-grant suppresses re-prompt (SC-005).

- [x] T050 [US4] Permission event emitter, blocker, and response handler — `crates/tools/apxm-server/src/permissions.rs`.
- [x] T051 [P] [US4] Session-scoped approval grant cache (fingerprint → ledger) — `crates/tools/apxm-server/src/permissions/grant_cache.rs`.
- [x] **Checkpoint:** US4 — permission E2E fixture passes (SC-005).

## Phase 7 — User Story 5: Session control API (P2)

**Goal:** status, cancel, grants, compact, events without full-transcript posts.
**Independent Test:** contract tests cover all endpoints (SC-006).

- [x] T060 [US5] Session handlers: status, cancel, grants, compact, events list/stream — `crates/tools/apxm-server/src/sessions.rs`.
- [x] T061 [P] [US5] Align `conversations.rs` turn-input path with session API semantics — `crates/tools/apxm-server/src/conversations.rs`.
- [x] T062 [P] [US5] Session API contract tests — `crates/tools/apxm-server/tests/contract_session_api.rs`.
- [ ] **Checkpoint:** US5 — `dekk apxm test -p apxm-server contract_session_api` passes.

## Phase 8 — Route integration (SERIAL — shared files)

**Goal:** wire new surfaces into the central route table and app mount.
**Independent Test:** all new endpoints reachable; no duplicate route registrations.

- [x] T070 Wire session, permission, and observer routes in `ServerRoute` — `crates/tools/apxm-server/src/routes.rs`. **SERIAL**
- [x] T071 Mount session + permission handlers and shared state — `crates/tools/apxm-server/src/app.rs`. **SERIAL**
- [ ] **Checkpoint:** integration smoke — session status + permission respond return 200 on fixture server.

## Phase 9 — User Story 6: Generated client (P2)

**Goal:** OpenAPI + typed client with CI diff-test (SC-007).
**Independent Test:** intentional wire drift fails CI until regeneration.

- [x] T080 [US6] `utoipa` OpenAPI export from `ServerRoute` + response types — `crates/tools/apxm-server/src/openapi.rs`.
- [x] T081 [P] [US6] Generate `apxm-client` bindings from exported schema — `crates/tools/apxm-client/src/lib.rs`.
- [x] T082 [P] [US6] CI schema diff-test against `specs/0002-apxm-chat-thin-clients/contracts/openapi-session-v1.yaml` — `crates/tools/apxm-server/tests/contract_openapi_diff.rs`.
- [x] **Checkpoint:** US6 — diff-test passes; sample consumer compiles with `apxm-client` only.

## Phase 10 — User Story 7: CLI thin pipe (P3)

**Goal:** chat delivers input, renders SSE, answers permissions — no local ledger.
**Independent Test:** SC-008 grep gate + live chat against server enforcement.

- [ ] T090 [US7] Refactor `apxm chat` to use `apxm-client`; remove host-side ledger — `crates/tools/apxm-cli/src/commands/chat.rs`.
- [ ] T091 [P] [US7] Render permission events and collect terminal replies in watch path — `crates/tools/apxm-cli/src/commands/watch.rs`.
- [ ] **Checkpoint:** US7 — `dekk apxm test-cli` + SC-008 script green.

## Phase 11 — Polish & Cross-Cutting

- [ ] T100 [P] SC-008 guard: no local turn-cap/budget counters in CLI chat sources — `specs/0002-apxm-chat-thin-clients/scripts/check_cli_no_ledger.sh`.
- [ ] T101 **GATE (serial):** `dekk apxm test` + `dekk apxm test-cli` + quickstart.md SC-001..SC-009 — full green (Constitution #10).

## Dependencies & Execution Order

- **Setup (T001–T002)** → **Foundational (T010–T012, checkpoint)** → story phases
  US1→US2→US3 can fan out in parallel across **disjoint files** after foundational
  lands; US4–US5 need typed errors + ledger; **T070/T071 serial barrier** before
  US6 consumer compile; US7 after US6; **T101 gate last**.
- **0006 hardening** MUST NOT start until T070/T071 freeze `routes.rs`/`app.rs`
  (program roadmap §6).

## Parallel Example

- **After Foundational:** T021 `[P]` `runs.rs` ∥ T031 `[P]` `context.rs` ∥ T041
  `[P]` `types/responses.rs` — three lanes, three files.
- **US4:** T051 `[P]` `permissions/grant_cache.rs` while T050 serial on
  `permissions.rs` completes first (same story, file-disjoint).
- **Never parallel:** T070 `routes.rs`, T071 `app.rs` — single integration lane.

## Implementation Strategy

- **MVP:** US1 + US3 (reliable stream + typed errors) — unblocks client debugging.
- **Contract freeze:** US5 + T070/T071 + US6 — Studio 0003 binding depends on this.
- **CLI proof:** US7 last — demonstrates thin pipe on frozen contract.
- **Parallel-team:** see lane map in
  `apxm-project/apxm-project/specs/0001-apxm-workspace-orchestrator/evidence/lane-map-0002.md`.

## Team assignment (parallel lanes)

```
 LANE 0 · FOUNDATION (serial)              T001 → T010 · T011[P] · T012[P]
            ═══ Foundational checkpoint ═══
 ── fan out (concurrent, file-disjoint) ───────────────────────────────────
 LANE A · SSE           apxm-server-impl   T020 · T021[P] · T022[P]
 LANE B · LEDGER        apxm-server-impl   T030 · T031[P]
 LANE C · ERRORS        apxm-server-impl   T040 · T041[P]
 ── barrier: US1–US3 checkpoints ──────────────────────────────────────────
 LANE D · PERMISSIONS   apxm-server-impl   T050 · T051[P]
 LANE E · SESSION API   apxm-server-impl   T060 · T061[P] · T062[P]
 ── converge (SERIAL) ─────────────────────────────────────────────────────
 LANE F · INTEGRATION   apxm-server-impl   T070 (routes.rs) → T071 (app.rs)
 ── fan out ───────────────────────────────────────────────────────────────
 LANE G · GEN CLIENT    apxm-server-impl   T080 · T081[P] · T082[P]
 LANE H · CLI           apxm-cli-impl      T090 · T091[P]
 ── gate ──────────────────────────────────────────────────────────────────
 LANE I · POLISH        either skill       T100[P] · T101 (serial gate)
```
