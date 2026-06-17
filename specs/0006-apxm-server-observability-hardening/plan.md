# Implementation Plan: APXM Server Observability Hardening

**Phase:** plan (next: tasks) · **Spec:** `./spec.md` · **Evidence base:**
`./research.md` · **Architecture SSOT:** `apxm-studio/docs/infrastructure-architecture-review-2026-06-17.md`
§8 gaps #7–#9, §18 (linked, not forked).

## Summary

Wire production telemetry and operational safety into apxm-server: real OTLP export,
Prometheus `/metrics`, HTTP rate limiting + body caps, auth-on by default for
non-loopback binds with per-principal identity, and SIGTERM graceful drain with
rollout flush. Shared integration lands serially in `routes.rs`, `app.rs`, and
`startup.rs`; story modules fan out on disjoint files per `tasks.md`.

## Technical Context

- **Language/Version:** Rust workspace edition 2024.
- **Primary Dependencies:** axum, tower-http (`limit`, `trace`), tracing +
  tracing-opentelemetry, opentelemetry-otlp, apxm-driver config.
- **Storage:** No new stores; rollout flush uses existing `RolloutRegistry`.
- **Testing:** `dekk apxm test -p apxm-server`; `tests/hardening_contract.rs`.
- **Target Platform:** Linux server process; loopback dev unchanged.
- **Constraints:** Green suites (#10); no 0002 contract wire changes; transport parity (#1).

## Constitution Check

Gated against `apxm/.spec/memory/constitution.md` v1.0.0.

| Principle | This plan |
|---|---|
| 1. Transport parity | PASS — limits/auth apply equally to CLI and HTTP paths |
| 2. Host is a pipe | PASS — no conversational logic added |
| 3. AIR-portability | PASS — server-only operational layers |
| 4. One Python-handler mechanism | PASS — untouched |
| 5. Control not observation | PASS — auth/limits are enforcement |
| 6. Named entry | PASS — untouched |
| 7. No dead surface | PASS — OTLP stub replaced; /metrics is real |
| 8. Reuse substrate | PASS — rollout flush reuses existing writers |
| 9. Waits MUST NOT pin compute | PASS — drain releases after timeout |
| 10. Green invariant suites | PASS — contract tests + existing suites stay green |

## Project Structure

```text
crates/orchestration/apxm-driver/src/config/mod.rs   # safety, shutdown, observability fields
crates/tools/apxm-server/src/
  observability.rs   # OTLP subscriber wiring
  metrics.rs         # GET /metrics
  safety.rs          # rate limit + body limit middleware
  bind.rs            # loopback detection + effective auth
  principal.rs       # PrincipalId from bearer
  shutdown.rs        # drain coordinator
  routes.rs          # Metrics route (SERIAL)
  app.rs             # middleware stack (SERIAL)
  startup.rs         # drain on shutdown (SERIAL)
tests/hardening_contract.rs
```

## Phases

1. **Foundational** — config types, disjoint modules, unit helpers.
2. **US1** — OTLP + /metrics wired.
3. **US2** — rate limit + body cap middleware.
4. **US3** — effective auth + principal identity.
5. **US4** — graceful drain + rollout flush.
6. **Integration (SERIAL)** — routes.rs → app.rs → startup.rs.

## Risks

| Risk | Mitigation |
|---|---|
| OTEL dep weight | Init only when endpoint set; warn on failure |
| Contract test auth breakage | Loopback bind in fixtures; effective auth unit-tested |
| Shutdown hang | Bounded `drain_timeout_secs` with warn + proceed |
