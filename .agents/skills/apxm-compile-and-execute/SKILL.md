---
name: apxm-compile-and-execute
description: Use when compiling APXM graphs, running .apxmobj artifacts, or executing AIR/IR through the runtime. Enforces dekk apxm as the authority CLI and correct artifact placement under .apxm/.
user-invocable: true
---

# APXM Compile & Execute

Load `_shared/apxm-development-rules.md` before broad work.

## Authority commands

```bash
dekk apxm validate <air.json>          # validate against AIS contract
dekk apxm compile <air.json> -o <out>  # → .apxmobj artifact
dekk apxm run <out.apxmobj>            # execute a pre-compiled artifact
dekk apxm execute <air.json>           # compile + execute in one step
dekk apxm analyze <air.json>           # parallelism + critical path
dekk apxm explain <air.json>           # human-readable summary
dekk apxm decompile <out.apxmobj>      # reverse-map back to AIR
```

## Rules

- Compiled artifacts (`*.apxmobj`) belong under `.apxm/`, never under
  `examples/` or `docs/`. Use `RepoLayout` for paths.
- Execution against a vLLM backend goes through a registered service.
  See `apxm-vllm-service` for the service path; never invoke `python
  benchmark.py` against `http://127.0.0.1:<port>/v1` outside a
  `service-exec` call.
- After editing a `.td` op definition, run `dekk apxm build-dialect`
  then `dekk apxm codegen` before invoking `compile`/`execute` — the
  Python frontend bindings drive what `validate` accepts.

## Typical flows

### Compile then run

```bash
dekk apxm validate graph.json
dekk apxm compile graph.json -o .apxm/compiled/graph.apxmobj
dekk apxm run .apxm/compiled/graph.apxmobj
```

### Inspect parallelism before executing

```bash
dekk apxm analyze graph.json     # surfaces phases, critical path
dekk apxm explain graph.json     # readable summary
```

### Iterate against a service allocation

```bash
dekk apxm vllm service-exec <name> -- dekk apxm execute graph.json
```

## Diagnostics

- `dekk apxm doctor` — env sanity.
- `dekk apxm ops list` — live op surface.
- `dekk apxm validate --verbose` — verbose contract errors.

## Anti-patterns

- Hand-rolling JSON for a graph you could have built via the Python
  frontend (`crates/compiler/apxm-frontend/python`).
- Running `compile` after editing a `.td` without re-running
  `build-dialect` + `codegen` — produces stale frontend bindings and
  confusing validation errors.
- Writing `.apxmobj` files anywhere other than `.apxm/`.
