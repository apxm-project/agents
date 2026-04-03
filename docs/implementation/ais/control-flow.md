# Control Flow Operations

Category: **ControlFlow**. These operations route tokens through different subgraphs based on runtime values. All have **None** latency tier (microseconds) except FLOW_CALL (Variable) and RESUME (High). Run `apxm ops list --category control_flow` for the current set.

## JUMP

Unconditional transfer of control to a target label.

| Field | Required | Description |
|-------|----------|-------------|
| `label` | yes | Target node ID to jump to |

```json
{"id": 5, "op": "JUMP", "attributes": {"label": "7"}}
```

## BRANCH_ON_VALUE

Conditional branch: compares an input token against a value and routes to one of two labels.

| Field | Required | Description |
|-------|----------|-------------|
| `token` | yes | Token to evaluate |
| `value` | yes | Value to compare against |
| `label_true` | yes | Target if comparison is true |
| `label_false` | yes | Target if comparison is false |

Min inputs: 1.

```json
{"id": 5, "op": "BRANCH_ON_VALUE", "attributes": {"token": "{{node_4}}", "value": "yes", "label_true": "6", "label_false": "7"}}
```

## LOOP_START

Marks the beginning of a bounded loop. Must be paired with LOOP_END.

| Field | Required | Description |
|-------|----------|-------------|
| `count_token` | yes | Token containing iteration count |

Min inputs: 1.

```json
{"id": 3, "op": "LOOP_START", "attributes": {"count_token": "3"}}
```

## LOOP_END

Marks the end of a bounded loop started by LOOP_START. Decrements the loop counter and branches back if iterations remain. No fields.

## RETURN

Returns a value from a subgraph or flow. Used as the terminal node in flows invoked via FLOW_CALL.

| Field | Required | Description |
|-------|----------|-------------|
| `token` | yes | Result token to return |

Min inputs: 1.

```json
{"id": 6, "op": "RETURN", "attributes": {"token": "{{node_5}}"}}
```

## SWITCH

Multi-way branch: routes execution based on matching a discriminant against case labels.

| Field | Required | Description |
|-------|----------|-------------|
| `discriminant` | yes | Token to match against case labels |
| `cases` | yes | Array of `{"label": "...", "node_id": N}` pairs |
| `default` | no | Default destination if no case matches |

Min inputs: 1.

```json
{"id": 3, "op": "SWITCH", "attributes": {"discriminant": "{{node_2}}", "cases": [{"label": "math", "node_id": 4}, {"label": "code", "node_id": 5}], "default": "6"}}
```

## FLOW_CALL

Invokes a named flow on a target agent. The target executes its flow graph independently and returns the result. Multiple FLOW_CALL nodes can run in parallel if they have no data dependencies.

| Field | Required | Description |
|-------|----------|-------------|
| `agent_name` | yes | Name of the agent to call |
| `flow_name` | yes | Name of the flow to invoke |
| `args` | no | Arguments to pass to the flow |

**Latency tier:** Variable.

```json
{"id": 4, "op": "FLOW_CALL", "attributes": {"agent_name": "researcher", "flow_name": "analyze", "args": {"topic": "{{node_1}}"}}}
```

## GUARD

Evaluates a condition against the input token. On failure, either halts execution or skips downstream nodes (configurable via `on_fail`).

| Field | Required | Description |
|-------|----------|-------------|
| `condition` | yes | Condition expression: `> 0.8`, `!= null`, `not_empty`, etc. |
| `error_message` | no | Message on failure |
| `on_fail` | no | Failure mode: `halt` (default) or `skip` |

Min inputs: 1.

```json
{"id": 3, "op": "GUARD", "attributes": {"condition": "> 0.8", "on_fail": "skip", "error_message": "Confidence too low"}}
```

## RESUME

Polls the APXM server for a checkpoint until a human resumes it. The human's input (if any) becomes the output token.

| Field | Required | Description |
|-------|----------|-------------|
| `checkpoint` | yes | Checkpoint ID to resume from |
| `poll_max_attempts` | no | Max polling attempts (default 60 x 5s = 5 min) |
| `poll_interval_ms` | no | Interval between polls in ms (default 5000) |
| `server_url` | no | Override `APXM_SERVER_URL` env var |

**Latency tier:** High (human-dependent).

```json
{"id": 6, "op": "RESUME", "attributes": {"checkpoint": "review_checkpoint_1"}}
```
