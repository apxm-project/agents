# Quickstart — 0002-apxm-chat-thin-clients

Validate the server contract and thin CLI after implementation checkpoints.

## Prerequisites

- Spec 0001 gate green (`doctor`, `validate_skills`, `dev-stack` smoke).
- Built server: `dekk apxm build-server`
- Built CLI: `dekk apxm build`

## 1. Start an isolated server

```bash
dekk apxm server --port 18999 --data-dir /tmp/apxm-0002-quickstart
```

## 2. SSE reliability (US1 / SC-001–SC-002)

Run the integration test (slow-consumer fixture):

```bash
dekk apxm test -p apxm-server sse_backpressure
```

Expect: zero silent drops; overflow frames present when consumer stalls.

## 3. Session SSOT (US2 / SC-003)

```bash
# Turn cap enforced server-side — second client, same session_id, should deny turn N+1
curl -s "http://127.0.0.1:18999/v1/sessions/demo-session/status"
```

Use contract test `contract_session_api` for automated SC-003 guard.

## 4. Typed errors (US3 / SC-004)

```bash
dekk apxm test -p apxm-server contract_errors
```

## 5. Permission round-trip (US4 / SC-005)

Drive a fixture agent with an ask-gated tool; observe permission event on stream;
reply via `POST /v1/permissions/{id}/respond`.

## 6. Session API (US5 / SC-006)

```bash
dekk apxm test -p apxm-server contract_session_api
```

## 7. Generated client (US6 / SC-007)

```bash
dekk apxm test -p apxm-client
# Intentional drift should fail CI diff-test against contracts/openapi-session-v1.yaml
```

## 8. Thin CLI (US7 / SC-008)

```bash
dekk apxm chat --server http://127.0.0.1:18999 --session-id qs-0002 \
  --air examples/python/conversational/controllable_agent.air
```

Verify: no local turn-cap counters in `chat.rs`; permission prompts round-trip.

## 9. Green suites (SC-009)

```bash
dekk apxm test
dekk apxm test-cli
```

All invariant suites green before marking spec complete.
