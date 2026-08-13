# Domain — execution

Runtime executor, handlers, backend adapters.

## Skills

- **backend-add** — register a new APXM inference backend.
- **compile-and-execute** — execute graphs via runtime.

## Subsystems

- `crates/runtime/` — executor + handlers.
- `crates/runtime/backends/` — LLM provider implementations.

## Rules

- Hard-fail at config time. No `or env or default` chains.
- Dispatch field names go through `graph_attrs::*` constants.

## Docs

- `docs/agents/portable-core-interface-contract.md`

## Related rules

- `_shared/apxm-development-rules.md`
