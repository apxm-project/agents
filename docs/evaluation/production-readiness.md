# Production Readiness

Deep dive into testing gaps, error handling, and resilience. Companion to the [evaluation scorecard](readme.md).

**Rating: Beta / Pre-Production**

## Test Coverage

### Suite Summary

| Suite | Tests | Pass | Fail | Ignored |
|-------|-------|------|------|---------|
| Rust workspace | 1261 | 1248 | 0 | 13 |
| Python frontend | 42 | 42 | 0 | 0 |
| Example validation | 53 | 53 | 0 | 0 |
| Policy checks | 4 | 4 | 0 | 0 |
| **Total** | **1360** | **1347** | **0** | **13** |

Source: [archive/test-status-2026-04-08.md](../archive/test-status-2026-04-08.md)

### Handler Test Coverage: 9/40 (22.5%)

Runtime handlers are the execution core — each AIS operation maps to a handler function. Most lack dedicated unit tests:

**Tested handlers** (9): Basic control flow and structural operations covered by server tests (490) and integration tests (176).

**Untested handlers** (31): Most LLM-facing handlers (ASK, THINK, REASON), agent lifecycle (SPAWN_AGENT, DELEGATE), memory operations (QMEM, UMEM, FENCE), tool invocation (INV), and error handling (TRYCATCH).

**Risk**: Handler-level edge cases are not exercised:
- Malformed LLM JSON response → unwrap panic (see Error Handling below)
- Schema validation failure on structured output
- Agent spawn timeout / agent crash during execution
- Tool invocation with side effects + retry
- TRYCATCH upstream failure recovery

**Indirect coverage**: The 490 server tests and 176 integration tests exercise many handlers through end-to-end flows. But they test the happy path — error injection and edge cases need handler-level tests.

### Crate Test Distribution

| Crate | Tests | Notes |
|-------|-------|-------|
| apxm-server | 490 | Largest suite, API endpoint tests |
| apxm-events | 110 | Event system |
| apxm-compiler | 135 | Pass tests |
| apxm-cli | 72 | CLI command tests |
| apxm-tools | 59 | Tool registry |
| apxm-backends | 50 | Backend protocol tests |
| apxm-graph | 39 | Graph construction |
| apxm-core | 31 | Type system, error codes |
| apxm-sandbox | 27 | Sandbox interface |
| apxm-credentials | 25 | Credential management |
| apxm-acp | 19 | Agent protocol |
| apxm-artifact | 7 | Artifact format |
| apxm-driver | 7 | Driver orchestration |
| apxm-ais | 2 | AIS definitions |

## Error Handling Assessment

### What Works Well

**Error codes (50+, E001-E999)** — Well-designed, component-based ranges:
- E001-E099: Parser errors
- E101-E199: Type errors
- E201-E299: MLIR/verification errors
- E301-E399: Optimization errors
- E401-E499: Runtime errors
- E501-E599: Semantic validation
- E900-E999: Generic errors

Builder pattern (`ErrorBuilder`), error context (operation IDs, trace IDs), and error chaining all implemented.

**Retry logic** — Exponential backoff with jitter:
- Initial: 500ms, max: 60s, multiplier: 2.0, jitter: +/-10%
- Classification: Retryable (timeout, 5xx, 429), Permanent (401, 403), Unknown (retry once)
- Full test coverage for backoff calculation and error classification

**Health monitoring** — Per-backend tracking via DashMap:
- States: Healthy (>=90% success), Degraded (50-90%), Unhealthy (<50%), Unknown
- Per-model granularity, sliding window latency (last 10 requests)
- Explicit status override for operational control

### What's Broken

**TRYCATCH is a pass-through:**
```rust
pub async fn execute(_ctx: &ExecutionContext, _node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Pass through inputs - actual exception handling done by scheduler
    Ok(inputs.first().cloned().unwrap_or(Value::Null))
}
```
No actual error catching. Comment says "scheduler handles it" but scheduler doesn't implement recovery either. Users who author TRYCATCH nodes get silent non-behavior.

**~60 `.unwrap()` calls in production paths:**

Critical examples:
```rust
// LLM handler — crashes on malformed JSON
let output = parse_structured_output(json).unwrap();
let extracted = extract_fenced_json(content).unwrap();
```

Most unwraps are in test code or guarded by checks, but LLM response parsing is a real crash risk. Malformed model output (which happens regularly with smaller models) triggers a panic that kills the entire graph execution.

**No circuit breaker:**
Health monitoring tracks backend status but nothing reads it during dispatch. Unhealthy backends continue receiving requests until they recover or all retries exhaust.

**Node failure recovery is full-graph restart:**
```
Parallel execution fails → Falls back to sequential → Sequential re-runs ALL nodes
```
No checkpoint, no partial recovery, no memoization integration in fallback path. Side-effect nodes (tool calls, agent spawns) may re-execute unsafely.

## Production Readiness Matrix

| Component | Ready? | Confidence | Risk |
|-----------|--------|-----------|------|
| Error codes + classification | Yes | 95% | Low |
| LLM backend retry logic | Yes | 90% | Low |
| Health monitoring | Partial | 70% | Medium — not used in decisions |
| Sandbox interface | Yes | 75% | Low — relies on host |
| Node failure recovery | No | 40% | **High** — full restart, side effects |
| Panic safety | Partial | 60% | **High** — JSON parsing unwraps |
| Credentials handling | Yes | 85% | Low — 0o600, atomic writes |
| Circuit breakers | Missing | 0% | **High** — unhealthy backends still hit |
| TRYCATCH | No | 10% | Medium — silent contract violation |

## High-Risk Scenarios

1. **LLM returns malformed JSON** → unwrap panic in handler → graph execution terminates
2. **Backend enters unhealthy state** → requests keep failing until all retries exhaust → cascading failures
3. **Node fails after side effects (tool call)** → full restart re-executes tool → duplicate actions
4. **User authors TRYCATCH expecting error recovery** → errors pass through uncaught → confusing behavior

## Credential Security

**Good:**
- File permissions: 0o600 enforced at creation and read
- Atomic writes via tempfile + persist()
- API key masking in logs (first 4 + last 4 chars)
- `.gitignore` prevents accidental commits

**Gaps:**
- No encryption at rest (relies on filesystem ACLs)
- No key rotation mechanism
- No audit trail for credential access

## Recommendations

### Tier 1: Before Production

1. **Fix JSON parsing unwraps** — Wrap `parse_structured_output()` and `extract_fenced_json()` in `map_err()`. Return `RuntimeError::LLM` with context (which model, which node, attempt number).
2. **Verify TRYCATCH** — Either implement scheduler-side error recovery or document that TRYCATCH is a no-op and remove it from the AIS operation set.
3. **Add circuit breaker** — Check `HealthStatus` before dispatching LLM nodes. Skip `Unhealthy` backends if alternatives exist.
4. **Fix token metrics** — Wire token count extraction in `backends/src/llm/backends/openai/backend.rs`.

### Tier 2: Pre-Release

5. **Handler test coverage** — Add unit tests for ASK, THINK, REASON, INV, SPAWN_AGENT with error injection. Target 30/40 handlers tested.
6. **Partial recovery** — Checkpoint after each successful node. On failure, resume from last checkpoint instead of restarting.
7. **Idempotency tracking** — Mark nodes as idempotent or not. Only retry idempotent nodes (LLM calls). Skip retry on side-effect operations.

### Tier 3: Post-Release

8. Encrypt credentials at rest
9. Key rotation mechanism
10. Predictive health monitoring (anomaly detection)
11. Distributed tracing integration

## Source Code References

- Error types: `crates/core/apxm-core/src/error/`
- Retry strategy: `crates/runtime/apxm-backends/src/llm/retry/mod.rs`
- Health monitoring: `crates/runtime/apxm-backends/src/llm/registry/health.rs`
- Execution engine: `crates/runtime/apxm-runtime/src/executor/engine.rs`
- TRYCATCH handler: `crates/runtime/apxm-runtime/src/executor/handlers/try_catch.rs`
- Sandbox: `crates/apxm-sandbox/src/backend.rs`
- Credentials: `crates/runtime/apxm-credentials/src/backend.rs`
