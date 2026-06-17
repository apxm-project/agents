# Tasks: APXM Server Observability Hardening

**Inputs:** `plan.md`, `spec.md`, `data-model.md`, `research.md`.
Paths are relative to the `apxm` repo root. `[P]` = parallel-safe (different file,
no unfinished dependency). `[US#]` tags story tasks. **Shared integration:**
`routes.rs`, `app.rs`, and `startup.rs` are **serial only** — never `[P]`.

## Phase 1 — Setup

- [x] T001 Scaffold spec artifacts — `specs/0006-apxm-server-observability-hardening/`.

## Phase 2 — Foundational (blocks all stories)

- [x] T010 [P] Extend `ServerObservabilityConfig`, add `ServerSafetyConfig` +
  `ServerShutdownConfig` — `crates/orchestration/apxm-driver/src/config/mod.rs`.
- [x] T011 [P] Env override keys — `crates/core/apxm-core/src/constants.rs`,
  `crates/tools/apxm-server/src/config_layers.rs`.
- [x] T012 [P] Loopback bind + effective auth helpers — `crates/tools/apxm-server/src/bind.rs`.
- [x] T013 [P] `PrincipalId` type + bearer fingerprint — `crates/tools/apxm-server/src/principal.rs`.
- [x] T014 [P] Prometheus scrape handler — `crates/tools/apxm-server/src/metrics.rs`.
- [x] T015 [P] Rate limit + body-limit middleware — `crates/tools/apxm-server/src/safety.rs`.
- [x] T016 [P] Shutdown drain coordinator — `crates/tools/apxm-server/src/shutdown.rs`.
- [x] T017 [P] Contract test harness — `crates/tools/apxm-server/tests/hardening_contract.rs`.
- [x] **Checkpoint:** `cargo check -p apxm-server` green; foundational modules compile.

## Phase 3 — User Story 1: OTLP + /metrics (P1)

- [x] T020 [US1] Wire OTLP exporter into global subscriber — `crates/tools/apxm-server/src/observability.rs`, `src/lib.rs`.
- [x] T021 [P] [US1] Register HTTP request counter in metrics — `crates/tools/apxm-server/src/metrics.rs`.
- [x] **Checkpoint:** US1 — `/metrics` returns uptime; OTLP init logs when endpoint set.

## Phase 4 — User Story 2: Rate limit + body cap (P1)

- [x] T030 [US2] Mount safety layers in app — `crates/tools/apxm-server/src/app.rs`. **SERIAL**
- [x] T031 [P] [US2] Map rate-limit faults to typed 429 — `crates/tools/apxm-server/src/safety.rs`.
- [x] **Checkpoint:** US2 — `dekk apxm test -p apxm-server hardening_contract` rate/body cases pass.

## Phase 5 — User Story 3: Auth default + principal (P1)

- [x] T040 [US3] Effective auth + principal extension in middleware — `crates/tools/apxm-server/src/auth.rs`.
- [x] T041 [P] [US3] Store bind addr + effective auth in `AppState` — `crates/tools/apxm-server/src/state.rs`, `startup.rs`.
- [x] T042 [P] [US3] Bind/auth unit tests — `crates/tools/apxm-server/src/bind.rs`.
- [x] **Checkpoint:** US3 — non-loopback effective-auth tests pass; loopback fixtures unaffected.

## Phase 6 — User Story 4: Graceful drain (P1)

- [x] T050 [US4] `RolloutRegistry::flush_all` — `crates/tools/apxm-server/src/rollout.rs`.
- [x] T051 [US4] SIGTERM drain + rollout flush — `crates/tools/apxm-server/src/startup.rs`. **SERIAL**
- [x] T052 [P] [US4] HTTP in-flight tracking layer — `crates/tools/apxm-server/src/shutdown.rs`.
- [x] **Checkpoint:** US4 — shutdown coordinator unit test passes.

## Phase 7 — Route integration (SERIAL)

- [x] T070 Add `ServerRoute::Metrics` — `crates/tools/apxm-server/src/routes.rs`. **SERIAL**
- [x] T071 Mount `/metrics` + middleware ordering — `crates/tools/apxm-server/src/app.rs`. **SERIAL**
- [x] **Checkpoint:** integration smoke — `/health` + `/metrics` return 200 on fixture server.

## Dependencies

- **0002** MUST be frozen before T070/T071 (Wave 2 barrier — satisfied).
- **0006** does not change 0002 session/permission wire types.

## Parallel lanes

```
LANE A · OBSERVABILITY   apxm-server-impl   T020 · T021[P]
LANE B · SAFETY          apxm-server-impl   T030(serial) · T031[P]
LANE C · AUTH            apxm-server-impl   T040 · T041[P] · T042[P]
LANE D · DRAIN           apxm-server-impl   T050 · T051(serial) · T052[P]
LANE E · INTEGRATION     apxm-server-impl   T070 → T071 (routes.rs → app.rs)
```
