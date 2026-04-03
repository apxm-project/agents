# Synchronization Operations

Category: **Synchronization**. These operations bring parallel execution paths back together. All have **None** latency tier (microseconds). Run `apxm ops list --category synchronization` for the current set.

## MERGE

Aggregates tokens from parallel branches into a single output. Used after fan-out patterns or BRANCH/SWITCH reconvergence to collect results.

| Field | Required | Description |
|-------|----------|-------------|
| `tokens` | yes | List of tokens to merge |

Min inputs: 1.

```json
{"id": 6, "op": "MERGE", "attributes": {"tokens": ["{{node_3}}", "{{node_4}}", "{{node_5}}"]}}
```

## WAIT_ALL

Blocks until all listed input tokens are ready. Unlike MERGE, it acts as a pure synchronization barrier -- it does not combine the tokens.

| Field | Required | Description |
|-------|----------|-------------|
| `tokens` | yes | Tokens to wait for |

Min inputs: 1.

```json
{"id": 5, "op": "WAIT_ALL", "attributes": {"tokens": ["{{node_2}}", "{{node_3}}"]}}
```

## CHECKPOINT

Creates a durable execution checkpoint by serializing AAM state and runtime metadata. Emits a manifest token (checkpoint id, timestamp, byte size) for downstream reference. Execution continues immediately -- this is NOT a suspend point (use PAUSE for human-gated suspension).

| Field | Required | Description |
|-------|----------|-------------|
| `checkpoint_id` | yes | Stable identifier for this checkpoint |
| `scope` | no | Snapshot scope: `full` (default) or `local` |
| `storage` | no | Storage backend: `fs` (default), `memory`, or `custom` |
| `ttl_seconds` | no | Time-to-live for the checkpoint in seconds |
| `on_fail` | no | Failure mode: `halt` (default) or `continue` |

Min inputs: 1.

```json
{"id": 4, "op": "CHECKPOINT", "attributes": {"checkpoint_id": "before_analysis"}}
```

## Common Patterns

**Diamond (Branch + Merge):** BRANCH_ON_VALUE fans out to two paths; MERGE reconverges them. Only one path fires.

**Fan-out / Fan-in:** A source node feeds multiple parallel ops; WAIT_ALL or MERGE collects all results before downstream processing.

**Write Barrier:** UMEM writes followed by FENCE followed by QMEM reads. See [memory-ops.md](memory-ops.md) for FENCE details.
