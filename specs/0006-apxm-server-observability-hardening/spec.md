# Feature Specification: APXM Server Observability Hardening

**Feature dir:** `specs/0006-apxm-server-observability-hardening/`
**Phase:** specify (next: clarify → plan → tasks → analyze)
**Repo:** `apxm`
**Input:** "Wire OTLP exporter + /metrics, rate limiter + DefaultBodyLimit,
auth-on by default for non-loopback + per-principal identity, graceful drain on
SIGTERM."
**Source:** program roadmap charter 0006; architecture review §8 gap #7–#9, §18.
Depends on spec 0001 gate and spec 0002 server contract freeze (Wave 2).

Cross-cutting server robustness that makes apxm-server production-operable without
changing the session/execute contract from 0002.

## User Scenarios & Testing

### User Story 1 — OTLP tracing + Prometheus metrics (Priority: P1)

An operator configures `OTEL_EXPORTER_OTLP_ENDPOINT` and scrapes `/metrics`. The
server exports tracing spans to the collector and exposes process-level Prometheus
text for uptime, HTTP traffic, and admission saturation.

**Independent Test:** With OTLP endpoint set, a health request produces a trace in
the collector fixture; `GET /metrics` returns `200` with `text/plain` exposition
containing `apxm_server_uptime_seconds`.

**Acceptance Scenarios:**

1. **Given** `OTEL_EXPORTER_OTLP_ENDPOINT` is set, **When** the server starts,
   **Then** tracing events flow through an OTLP exporter (non-fatal on init failure).
2. **Given** metrics are enabled, **When** a client calls `GET /metrics`,
   **Then** the response is Prometheus text with server uptime and request counters.
3. **Given** OTLP init fails, **When** the server starts, **Then** it logs a warning
   and continues serving with in-process `tracing` only.

### User Story 2 — Rate limiter + body cap (Priority: P1)

A shared or ingress-exposed server rejects abusive request volume and oversized
payloads before they reach the runtime. Limits are configurable and keyed per
principal when auth is active.

**Independent Test:** Send requests exceeding the configured RPS from one principal;
observe `429` with a stable machine-readable code. POST a body larger than
`max_body_bytes`; observe `413`.

**Acceptance Scenarios:**

1. **Given** `rate_limit_rps` is configured, **When** a principal exceeds burst+RPS,
   **Then** the server returns `429` with code `rate_limit_exceeded`.
2. **Given** `max_body_bytes` is configured, **When** a request body exceeds the cap,
   **Then** axum rejects it before handler execution (`413`).
3. **Given** no rate-limit config, **When** traffic is normal, **Then** middleware is
   a transparent pass-through.

### User Story 3 — Auth-on for non-loopback + per-principal (Priority: P1)

Binding the server on a non-loopback address automatically requires bearer auth on
protected routes. Validated tokens yield a stable per-principal identity used for
rate limiting and structured logs — without multi-tenant storage (single-tenant v0).

**Independent Test:** Resolve bind `0.0.0.0:port` with no explicit auth config;
mutating routes return `401` without bearer. Present a valid bearer; request succeeds
and rate limits apply per token fingerprint.

**Acceptance Scenarios:**

1. **Given** bind address is non-loopback, **When** `require_auth` is unset/false in
   config, **Then** protected routes still require bearer (fail-closed default).
2. **Given** bind address is loopback, **When** `require_auth` is false (default),
   **Then** local dev and contract tests remain unauthenticated.
3. **Given** a valid bearer on a protected route, **When** the handler runs,
   **Then** request extensions carry a stable `PrincipalId` derived from the token.

### User Story 4 — Graceful drain on SIGTERM (Priority: P1)

On SIGTERM the server stops accepting new connections, awaits in-flight HTTP work and
rollout writers up to a configurable timeout, then exits cleanly.

**Independent Test:** Start a long SSE stream, send SIGTERM, observe the stream
completes or times out gracefully and rollout files are closed (no truncated JSONL).

**Acceptance Scenarios:**

1. **Given** in-flight HTTP handlers, **When** SIGTERM arrives, **Then** axum
   graceful shutdown waits for them before exit.
2. **Given** open rollout recorders, **When** shutdown begins, **Then** all writers
   flush and close within `drain_timeout_secs`.
3. **Given** drain exceeds timeout, **When** shutdown proceeds, **Then** the server
   logs a warning and exits (best-effort, not hang forever).

## Functional Requirements

- **FR-001:** Wire OTLP exporter when `observability.otlp_endpoint` or
  `OTEL_EXPORTER_OTLP_ENDPOINT` is set; init failures are non-fatal.
- **FR-002:** Expose `GET /metrics` (unversioned, alongside `/health`) with Prometheus
  text exposition when metrics are enabled.
- **FR-003:** Configurable HTTP rate limit (`safety.rate_limit_rps`, burst) keyed by
  `PrincipalId` or client IP fallback.
- **FR-004:** Configurable `safety.max_body_bytes` via `DefaultBodyLimit` middleware.
- **FR-005:** Auto-enable bearer auth on non-loopback binds; loopback default unchanged.
- **FR-006:** Derive `PrincipalId` from validated bearer (blake3 fingerprint); thread
  through request extensions for limits and tracing.
- **FR-007:** On SIGTERM/ctrl-c, drain in-flight work and flush rollout registry within
  `shutdown.drain_timeout_secs`.

## Success Criteria

- **SC-001:** OTLP fixture receives spans for instrumented requests when endpoint set.
- **SC-002:** `/metrics` scrape succeeds; `apxm_server_uptime_seconds` present.
- **SC-003:** Rate-limit fixture returns `429`/`rate_limit_exceeded` under abuse.
- **SC-004:** Body-cap fixture returns `413` for oversized POST.
- **SC-005:** Non-loopback effective-auth unit test passes; loopback tests unaffected.
- **SC-006:** Shutdown fixture closes rollout writers; no worker hang past timeout.

## Assumptions

- Spec 0002 froze `routes.rs`/`app.rs`; 0006 extends them serially post-freeze.
- Multi-tenant principal storage deferred to spec 0007; v0 is bearer fingerprint only.
- Prometheus scrape is process-local; no push gateway in scope.
- `dekk apxm test -p apxm-server` (or targeted `hardening_contract`) is the gate.

## Out of Scope

- Studio/OS ingress TLS, compose topology (0005).
- Postgres/vault auth storage (0007).
- RFC-7807 or OpenAPI changes to error model (0002 owns contract).
