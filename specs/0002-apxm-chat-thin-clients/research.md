# Research — 0002-apxm-chat-thin-clients

Decisions from architecture review §7, §8 and current source inspection.

## R1 — Session SSOT location

**Decision:** Server + runtime `SessionLedger` keyed by `session_id` is the sole
authority for turn caps, tool budgets, and grants.

**Rationale:** Review §7 confirms `SessionLedger` registry
(`session_ledger.rs:1-9`); server exposes history by `session_id`
(`routes.rs:63`). Clients (CLI, Studio) currently risk duplicate host state.

**Alternatives:** Client-side ledger (rejected — violates FR-004, Constitution #2).

## R2 — SSE silent drop root cause

**Decision:** Replace `TokioChannelEmitter::try_send` drop-on-full
(`state.rs:173-175`) with observer-bus routing or explicit overflow frames.

**Rationale:** Review §8 gap #2; observer stream already has seq ring + broadcast +
`Last-Event-ID` + disk fallback — execute stream must match.

**Alternatives:** Larger buffer only (rejected — masks backpressure, no client signal).

## R3 — Typed error model

**Decision:** Introduce `types/errors.rs` with program-fault (4xx) vs server-fault
(5xx) classes; map `RuntimeError` variants to stable codes.

**Rationale:** Review §8 gap #4; required for generated client and thin clients.

**Alternatives:** Continue `{"error": string}` only (rejected — blocks US6/US7).

## R4 — Permission channel shape

**Decision:** Emit permission events on the run/event stream; add HTTP response
endpoint per `permissions.rs`; cache session grants in `SessionLedger`.

**Rationale:** Review §8 gaps #5 — opencode `Deferred` + codex elicitation pattern;
ASK/HITL primitives already exist server-side.

**Alternatives:** CLI-only approval UX (rejected — violates transport parity #1).

## R5 — Generated client toolchain

**Decision:** `utoipa` on `ServerRoute` + response types; export OpenAPI; new
`apxm-client` crate; CI diff-test checked-in `contracts/openapi-session-v1.yaml`.

**Rationale:** Review §8 gap #1 — opencode SDK + codex ts-rs/schemars discipline.

**Alternatives:** Hand-maintained CLI structs (rejected — caused drift).

## R6 — Cancellation scope

**Decision:** Register cancel `Notify` on `/v1/execute`, `/v1/execute/stream`, and
`/a2a` paths as part of session cancel API (FR-009).

**Rationale:** Review §8 gap #3 — currently streaming-only.

**Alternatives:** Observer-only cancel (rejected — FR-009).

## R7 — Shared-file serialization with 0006

**Decision:** 0002 owns `routes.rs`/`app.rs` integration; 0006 hardening waits Wave 2.

**Rationale:** Program roadmap §6 contention notes.

**Alternatives:** Merge specs (rejected — violates wave ordering).
