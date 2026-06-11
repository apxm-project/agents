# Domain — execution

Runtime executor, handlers, backend adapters.

## Skills

- **apxm-backend-add** — register a new APXM inference backend.
- **apxm-compile-and-execute** — execute graphs via runtime.
- **apxm-vllm-service** — APXM-vLLM service operation.

## Subsystems

- `crates/runtime/` — executor + handlers.
- `crates/runtime/apxm-backends/` — LLM provider implementations,
  vLLM-fork glue.

## Rules

- Hard-fail at config time. No `or env or default` chains.
- `model.id` must equal vLLM's `served_model_name` exactly — bare
  name, not HF repo. See `feedback_apxm_model_id_must_match_served`.
- Dispatch field names go through `graph_attrs::*` constants.

## Docs

- `docs/backends/model-zoo.md`
- `docs/backends/vllm.md`

## Related rules

- `_shared/apxm-development-rules.md`
