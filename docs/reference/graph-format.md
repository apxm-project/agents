# Graph Format Reference

Complete specification of the `.apxm` graph JSON format. This is the stable public contract consumed by `ApxmGraph::from_json()` and the `apxm validate` CLI command.

For a hands-on tutorial, see [How to Build Your First Graph](../getting-started/first-graph.md).
For the binary wire format used by compiled artifacts, see [Wire Format Contracts](../implementation/internals/contracts.md).

---

## Top-Level Structure

Every `.apxm` file is a JSON object with this shape:

```json
{
  "name": "...",
  "nodes": [],
  "edges": [],
  "parameters": [],
  "metadata": {}
}
```

### Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | string | Yes | -- | Graph name; must not be empty or whitespace-only |
| `nodes` | `GraphNode[]` | Yes | -- | At least one node required |
| `edges` | `GraphEdge[]` | Yes | -- | May be empty (single-node graphs have no edges) |
| `parameters` | `Parameter[]` | No | `[]` | Runtime parameters injected into entry nodes |
| `metadata` | object | No | `{}` | Arbitrary key-value pairs for tooling and extensions |

All field names are case-sensitive and must match exactly: `name`, `nodes`, `edges`, `parameters`, `metadata`.

---

## Nodes (`GraphNode`)

Each node represents one AIS operation in the execution graph.

```json
{"id": 1, "name": "greet", "op": "ASK", "attributes": {"template_str": "Hello"}}
```

### Fields

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `id` | integer (u64) | Yes | -- | Unique within the graph |
| `name` | string | Yes | -- | Human-readable label; must not be empty |
| `op` | string | Yes | -- | AIS operation in `SCREAMING_SNAKE_CASE` |
| `attributes` | object | No | `{}` | Operation-specific key-value pairs |

### Node ID Constraints

- Every `id` must be unique within the graph.
- When using non-sequential IDs, node IDs must not fall in the range `[1, edge_count]` to avoid colliding with runtime token IDs. Sequential IDs starting at 1 are always safe.

### Operation Names

Operations use `SCREAMING_SNAKE_CASE`. Run `apxm ops list` for the complete list grouped by category; run `apxm ops show <OP>` for per-op attributes and an example JSON snippet.

**LLM operations:** `ASK`, `THINK`, `REASON`

**Planning and analysis:** `PLAN`, `REFLECT`, `VERIFY`

**Tool operations:** `INV_TOOL`, `EXC`, `PRINT`

**Control flow:** `JUMP`, `BRANCH_ON_VALUE`, `LOOP_START`, `LOOP_END`, `RETURN`, `SWITCH`, `FLOW_CALL`

**Synchronization:** `MERGE`, `FENCE`, `WAIT_ALL`

**Error handling:** `TRY_CATCH`, `ERR`

**Communication:** `COMMUNICATE`

**Coordination:** `UPDATE_GOAL`, `GUARD`, `CLAIM`, `PAUSE`, `RESUME`, `DELEGATE`, `NEGOTIATE`

**Identity:** `NOP`, `IDENTITY`

**Self-organization:** `SPAWN_AGENT`, `SPAWN_TEAM`, `REGISTER_CAPABILITY`

**Autonomous:** `AUTONOMOUS`

**Durable execution:** `CHECKPOINT`

**Internal (compiler-managed):** `CONST_STR`, `YIELD`

### Common Attributes

These attributes appear across multiple operation types. The `attributes` object is free-form JSON; only certain keys are recognized by each operation. Unrecognized keys are preserved but ignored at runtime.

| Attribute | Used by | Type | Description |
|-----------|---------|------|-------------|
| `template_str` | `ASK`, `THINK`, `REASON`, `PLAN`, `REFLECT`, `VERIFY` | string | Prompt template; supports `{{node_N}}` and `{{param}}` placeholders |
| `system_prompt` | LLM ops | string | Per-node system prompt (overrides `[instruction]` in config) |
| `model` | LLM ops | string | Override model for this node |
| `provider` | LLM ops | string | Override backend/provider for this node |
| `temperature` | LLM ops | float | Sampling temperature |
| `token_budget` | `THINK` | integer | Token budget for extended thinking |
| `output_schema` | LLM ops | object | JSON Schema for structured output |
| `max_schema_retries` | LLM ops | integer | Retries on schema validation failure |
| `value` | `CONST_STR` | string | Constant string value **(required)** |
| `agent_name` | `SPAWN_AGENT` | string | Agent identifier **(required)** |
| `team_name` | `SPAWN_TEAM` | string | Team identifier |
| `recipient` | `COMMUNICATE` | string | Target agent name **(required)** |
| `protocol` | `COMMUNICATE` | string | Communication protocol (`local`, `acp`) |
| `capability` | `INV_TOOL` | string | Tool capability name **(required)** |
| `message` | `PRINT` | string | Output message; supports `{{node_N}}` placeholders |
| `tokens` | `MERGE`, `WAIT_ALL` | array | List of `{{node_N}}` refs to collect |
| `condition` | `GUARD` | string | Guard condition expression |
| `label` | `JUMP` | string | Jump target label |
| `true_label` | `BRANCH_ON_VALUE` | string | Branch label when condition is true |
| `false_label` | `BRANCH_ON_VALUE` | string | Branch label when condition is false |
| `case_labels` | `SWITCH` | object | Map of case values to labels |
| `try_label` | `TRY_CATCH` | string | Label for the try block |
| `catch_label` | `TRY_CATCH` | string | Label for the catch block |
| `goal` | `REASON`, `UPDATE_GOAL` | string | Reasoning goal or goal update |

### Required Attributes by Operation

Validation rejects nodes missing these attributes:

| Operation | Required attribute |
|-----------|--------------------|
| `CONST_STR` | `value` |
| `SPAWN_AGENT` | `agent_name` |
| `COMMUNICATE` | `recipient` |
| `INV_TOOL` | `capability` |

---

## Edges (`GraphEdge`)

Each edge declares a dependency between two nodes.

```json
{"from": 1, "to": 2, "dependency": "Data"}
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `from` | integer (u64) | Yes | Source node ID (must exist in `nodes`) |
| `to` | integer (u64) | Yes | Destination node ID (must exist in `nodes`) |
| `dependency` | string | Yes | One of `Data`, `Control`, `Effect` |

### Dependency Types

| Type | Meaning | Token passed? |
|------|---------|---------------|
| `Data` | Target needs the source's output value | Yes -- the source output is available as `{{node_N}}` in the target's template |
| `Control` | Target must execute after source completes | No -- only ordering is enforced |
| `Effect` | Target must execute after source's side effects finish | No -- only ordering is enforced |

Dependency type values are case-sensitive: `Data`, `Control`, `Effect`.

---

## Parameters (`Parameter`)

Parameters let you inject values into a graph at execution time. Parameter values are bound to entry nodes (nodes with no incoming edges) before execution starts.

```json
{"name": "topic", "type_name": "str"}
```

### Fields

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | Yes | Parameter name; must not be empty; must be unique |
| `type_name` | string | Yes | One of `str`, `int`, `float`, `bool`, `json` |

### Valid Parameter Types

| Type | Description |
|------|-------------|
| `str` | String value |
| `int` | Integer value |
| `float` | Floating-point value |
| `bool` | Boolean value |
| `json` | Arbitrary JSON value |

### Template Placeholders

Use `{{param_name}}` in `template_str` attributes to reference parameters:

```json
{
  "nodes": [
    {"id": 1, "name": "ask", "op": "ASK", "attributes": {"template_str": "Summarize {{topic}}"}}
  ],
  "parameters": [{"name": "topic", "type_name": "str"}]
}
```

Use `{{node_N}}` to reference the output of node with `id = N`:

```json
{"template_str": "Critique this: {{node_1}}"}
```

---

## Metadata

The `metadata` object is free-form. APXM recognizes one key:

| Key | Type | Description |
|-----|------|-------------|
| `is_entry` | bool | Marks this graph as an entry flow (used in multi-flow compositions) |

Store custom data here rather than overloading node attributes. Unknown keys are preserved through compilation and execution.

---

## Validation Rules

`ApxmGraph::from_json()` and `apxm validate` enforce these rules. Violations produce errors that block compilation and execution.

### Structural Rules (always enforced)

| Rule | Error condition |
|------|-----------------|
| Non-empty graph name | `name` is empty or whitespace-only |
| At least one node | `nodes` array is empty |
| Unique node IDs | Two nodes share the same `id` |
| Non-empty node names | Any node has an empty or whitespace-only `name` |
| Valid edge endpoints | An edge references a node ID not present in `nodes` |
| Acyclic graph (DAG) | The directed edge graph contains a cycle |
| No duplicate parameters | Two parameters share the same `name` |
| Non-empty parameter fields | A parameter has empty `name` or `type_name` |
| Token-space safety | Non-sequential node IDs collide with edge token range `[1, edge_count]` |

### Semantic Rules (always enforced)

| Rule | Error condition |
|------|-----------------|
| Required attributes present | `CONST_STR` missing `value`, `SPAWN_AGENT` missing `agent_name`, `COMMUNICATE` missing `recipient`, `INV_TOOL` missing `capability` |
| Valid provider references | LLM node specifies a `provider` attribute that does not match any built-in provider |
| Unique agent names | Two `SPAWN_AGENT` nodes declare the same `agent_name` |
| Valid agent references | `COMMUNICATE` node references a `recipient` not spawned by any `SPAWN_AGENT` in the graph |
| Valid `{{node_N}}` refs | `MERGE` tokens or `PRINT` message contain `{{node_N}}` where no node with `id = N` exists |

### Tier 2 Rules (environment-dependent, skipped with `--no-check-resources`)

When environment context is available, the semantic validator additionally checks that `model`, `provider`, `capability`, and profile references point to registered resources.

---

## Minimal Valid Graph

The smallest valid graph is a single node with no edges:

```json
{
  "name": "minimal",
  "nodes": [
    {"id": 1, "name": "hello", "op": "ASK", "attributes": {"template_str": "Say hello"}}
  ],
  "edges": []
}
```

The `parameters` and `metadata` fields default to `[]` and `{}` respectively when omitted.

---

## Compatibility Guidance

- Treat operation names (`op`) and attribute keys as case-sensitive.
- Always use `SCREAMING_SNAKE_CASE` for operation names (e.g., `WAIT_ALL`, `CONST_STR`, `SPAWN_AGENT`).
- Prefer explicit `parameters` and `metadata` fields even when empty, for forward compatibility.
- Store extension data in graph-level `metadata` rather than overloading node `attributes`.
- Use sequential node IDs starting at 1 to avoid token-space collisions.

---

## See Also

- [How to Build Your First Graph](../getting-started/first-graph.md) -- step-by-step tutorial covering pipeline, fan-out, parameters, and composition
- [Wire Format Contracts](../implementation/internals/contracts.md) -- binary artifact format and op-kind index table
- [Configuration Reference](config.md) -- backend, routing, and tool configuration
- [Optimization Overview](../optimization/overview.md) -- how `-O0` and `-O2` transform the graph before execution
