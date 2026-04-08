# Utility and Error Operations

These are the remaining operations not covered by the other AIS reference files. They include identity/passthrough nodes and structured error handling.

## NOP

No-op passthrough with no side effects and no AAM state transition. Passes through its first input unchanged, or returns Null if no inputs. Useful as a placeholder, sync point, or structural node in graph composition.

No fields. **Latency tier:** None. **Category:** Identity.

## IDENTITY

Like NOP but records an AAM identity transition in the execution trace (state unchanged but recorded). Useful for observability when you want to mark a point in the graph without changing state.

No fields. **Latency tier:** None. **Category:** Identity.

## TRY_CATCH

Wraps a try subgraph with a catch recovery subgraph. If any node in the try subgraph fails, execution transfers to the catch subgraph which receives the error context.

| Field | Required | Description |
|-------|----------|-------------|
| `try_subgraph` | yes | Subgraph to try executing |
| `catch_subgraph` | yes | Recovery subgraph on failure |

**Latency tier:** Variable. **Category:** ErrorHandling.

```json
{"id": 2, "op": "TRY_CATCH", "attributes": {"try_subgraph": "3", "catch_subgraph": "4"}}
```

TRY_CATCH scopes are lexical within the DAG. Errors in a nested TRY_CATCH are handled by the innermost enclosing catch. If the catch subgraph itself fails, the error propagates outward.

## ERR

Handles an error by invoking a recovery template. Can update the agent's goals and beliefs to reflect the failure. Typically used inside TRY_CATCH catch subgraphs.

| Field | Required | Description |
|-------|----------|-------------|
| `error_handler` | yes | Error handler to invoke |

**Latency tier:** Variable. **Category:** ErrorHandling.

```json
{"id": 4, "op": "ERR", "attributes": {"error_handler": "retry_with_fallback"}}
```
