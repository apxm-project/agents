# Memory Operations

Category: **Memory** (QMEM, UMEM, UPDATE_GOAL) and **Synchronization** (FENCE). These operations read and write the agent's memory system and goal state. All have **None** latency tier (microseconds) except that QMEM/UMEM are submitted to the executor.

## QMEM -- Query Memory

Reads from the agent's memory system (STM, LTM, or Episodic). Returns the stored value or null if not found.

| Field | Required | Description |
|-------|----------|-------------|
| `query` | yes | Query string or key to search for |
| `memory_tier` | no | Target tier: `stm`, `ltm`, or `episodic` |

```json
{"id": 2, "op": "QMEM", "attributes": {"query": "user_name", "memory_tier": "stm"}}
```

## UMEM -- Update Memory

Writes a key-value pair to the agent's memory system. Overwrites if the key already exists.

| Field | Required | Description |
|-------|----------|-------------|
| `key` | yes | Key to store the value under |
| `value` | yes | Value to store (supports `{{node_N}}` interpolation) |
| `memory_tier` | no | Target tier: `stm`, `ltm`, or `episodic` |

```json
{"id": 3, "op": "UMEM", "attributes": {"key": "summary", "value": "{{node_2}}", "memory_tier": "stm"}}
```

## UPDATE_GOAL -- Modify Goals at Runtime

Dynamically modifies the agent's goal set. Changes are visible to subsequent REASON and REFLECT nodes.

| Field | Required | Description |
|-------|----------|-------------|
| `goal_id` | yes | Goal identifier (description key for upsert/remove) |
| `action` | no | `set` (default), `remove`, or `clear` |
| `priority` | no | Goal priority (u32, default: 1) |

```json
{"id": 3, "op": "UPDATE_GOAL", "attributes": {"goal_id": "optimize_latency", "action": "set", "priority": 2}}
```

## FENCE -- Memory Barrier

Ensures all preceding UMEM writes are committed before subsequent QMEM reads can execute. Place between UMEM and QMEM when ordering matters.

| Field | Required | Description |
|-------|----------|-------------|
| `ordering` | no | Memory ordering constraint |

No fields are required. FENCE has no example JSON in the spec.

## Ordering Guarantees

| Scenario | Guarantee |
|----------|-----------|
| QMEM after UMEM (same key, data edge) | Read sees write |
| QMEM after UMEM (different keys, no edge) | No guarantee without FENCE |
| UMEM after UMEM (same key) | Last-writer-wins within subgraph ordering |
| Cross-agent memory access | Requires COMMUNICATE; no shared memory |
