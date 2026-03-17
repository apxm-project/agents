---
title: "Memory Operations"
description: "AIS instruction reference for QMEM, UMEM, and FENCE."
---

# Memory Operations

For the formal memory model (tier semantics, cognitive rationale), see [Memory in PXMs](../../pxm/memory.md).
For backing-store implementation, see [Memory Hierarchy](../runtime/memory-hierarchy.md).

## QMEM -- QueryMemory

```
QMEM(q: String, sid: SessionID, k: Int) -> Value
```

| Operand | Type | Description |
|---------|------|-------------|
| `q` | `String` | Query key or search expression |
| `sid` | `SessionID` | Session scope for the lookup |
| `k` | `Int` | Maximum number of results to return |

```mlir
%user_history = "ais.qmem"(%query, %session, %k) {
  tier_hint = "ltm"
} : (!ais.string, !ais.session_id, i64) -> !ais.value
```

**Attributes:** `tier_hint` (optional) -- bypasses unnecessary tier searches when the target tier is known statically.

## UMEM -- UpdateMemory

```
UMEM(data: Value, sid: SessionID) -> Void
```

| Operand | Type | Description |
|---------|------|-------------|
| `data` | `Value` | The typed value to store |
| `sid` | `SessionID` | Session scope for the write |

```mlir
"ais.umem"(%analysis_result, %session) {
  durable = true,
  belief_key = "latest_analysis"
} : (!ais.value, !ais.session_id) -> ()
```

**Attributes:** `durable` -- when `true`, writes to both STM and LTM atomically (higher latency, ~ms for SQLite commit). `belief_key` -- updates the AAM Beliefs map entry.

## FENCE -- Memory Barrier

```
FENCE() -> Void
```

Synchronization point: blocks until all preceding UMEM operations in the current subgraph have committed, then emits a token for downstream QMEM operations.

```mlir
"ais.umem"(%result_a, %session) : (!ais.value, !ais.session_id) -> ()
"ais.umem"(%result_b, %session) : (!ais.value, !ais.session_id) -> ()
"ais.fence"() : () -> ()
%merged = "ais.qmem"(%merge_query, %session, %k) : (!ais.string, !ais.session_id, i64) -> !ais.value
```

## Ordering Guarantees

| Scenario | Guarantee |
|----------|-----------|
| QMEM after UMEM (same key, same subgraph) | Read sees write (data dependency edge) |
| QMEM after UMEM (different keys) | No guarantee without FENCE |
| UMEM after UMEM (same key) | Last-writer-wins within subgraph ordering |
| Cross-agent memory access | Requires COMM protocol; no shared memory |

## Patterns

**Read-Modify-Write:**
```mlir
%old = "ais.qmem"(%key, %sid, %one) : (...) -> !ais.value
%new = "ais.ask"(%modify_prompt, %old) : (...) -> !ais.future<!ais.string>
"ais.umem"(%new, %sid) { belief_key = "counter" } : (...) -> ()
```

**Bulk Retrieval:**
```mlir
%results = "ais.qmem"(%broad_query, %sid, %ten) {
  tier_hint = "ltm",
  similarity_threshold = 0.8
} : (!ais.string, !ais.session_id, i64) -> !ais.value
```
