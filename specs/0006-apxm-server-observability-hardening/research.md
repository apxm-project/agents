# Research — 0006 server observability hardening

## R1 — OTLP wiring

**Decision:** Use `tracing-opentelemetry` + `opentelemetry-otlp` (HTTP/protobuf) behind
runtime config. Initialize the global subscriber in `lib.rs` before serving so spans
export when endpoint is set.

**Rationale:** `observability.rs` is already a config hook; review §18 expects the
advertised pipeline to work. Non-fatal init matches existing `warn_init_failure` pattern.

## R2 — /metrics shape

**Decision:** Unversioned `GET /metrics` with hand-rolled Prometheus text (counters +
gauges). No push gateway.

**Rationale:** Matches vLLM/codex scrape pattern; avoids coupling to runtime JSON
`MetricsReport` (post-execution only).

## R3 — HTTP rate limit

**Decision:** Token-bucket per `PrincipalId` (or `"anonymous"` / IP fallback) in
`safety.rs`, configured via `[server.safety]`.

**Rationale:** Reuses apxm-backends token-bucket semantics without pulling LLM provider
limiter into HTTP layer.

## R4 — Auth default for non-loopback

**Decision:** `effective_require_auth(bind, config)` returns `true` for non-loopback
regardless of `require_auth` default; loopback honors explicit config (default off).

**Rationale:** api-contracts.md §3.5; closes gap where auth-ms is on but server is open.

## R5 — Per-principal v0

**Decision:** `PrincipalId` = first 16 hex chars of blake3(bearer); inserted into
request extensions after successful auth.

**Rationale:** Single-tenant v0 per 0001 decision; no apxm-auth schema changes.

## R6 — Graceful drain

**Decision:** axum `with_graceful_shutdown` + explicit `RolloutRegistry::flush_all`
after signal, bounded by `shutdown.drain_timeout_secs`.

**Rationale:** axum already drains HTTP; rollout JSONL close is the missing flush.

## R7 — Shared-file serialization with 0002

**Decision:** 0006 owns `app.rs`/`startup.rs`/`routes.rs` integration after 0002 Wave 2
freeze. Story modules are `[P]` on disjoint files.

**Rationale:** roadmap §6; tasks.md marks T070–T072 serial.
