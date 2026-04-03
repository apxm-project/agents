# Tool Operations

Category: **Tools**. These operations invoke external capabilities, execute code, and produce output. Run `apxm ops list --category tools` for the current set.

## INV -- Invoke Tool

Calls a registered capability (tool or function) by name. The capability must be declared in the AGENT node's `capabilities` list or registered in the runtime's CapabilityRegistry.

| Field | Required | Description |
|-------|----------|-------------|
| `capability` | yes | Name of the capability/tool to invoke |
| `parameters` | no | Parameters to pass to the tool (JSON object) |

**Latency tier:** Variable (depends on the external tool).

```json
{"id": 2, "op": "INV", "attributes": {"capability": "web_search", "parameters": {"query": "{{node_1}}"}}}
```

When multiple INV nodes share the same upstream dependency but not each other, the scheduler dispatches them in parallel automatically.

## EXC -- Execute Code

Runs code in a sandboxed environment. The sandbox prevents filesystem access, network calls, and other side effects unless explicitly allowed. The code's stdout/return value becomes the output token.

| Field | Required | Description |
|-------|----------|-------------|
| `code` | yes | Code to execute |
| `sandbox_config` | no | Sandbox configuration |

**Latency tier:** Low (sub-second for simple code).

```json
{"id": 3, "op": "EXC", "attributes": {"code": "print(2 + 2)"}}
```

## PRINT -- Print Output

Writes a message to stdout. Supports `{{node_N}}` template interpolation. The message is also stored as the output token.

| Field | Required | Description |
|-------|----------|-------------|
| `message` | yes | Message to print |

**Latency tier:** None (microseconds).

```json
{"id": 4, "op": "PRINT", "attributes": {"message": "Result: {{node_3}}"}}
```
