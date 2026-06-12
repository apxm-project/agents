---
name: apxm-compile-and-execute
group: Domain
description: Use when compiling APXM AIR workflows, running .apxmobj artifacts, or executing AIR/IR through the runtime. Enforces dekk apxm as the authority CLI and correct artifact placement under .apxm/.
user-invocable: true
---

# APXM Compile & Execute

Load `_shared/apxm-development-rules.md` before broad work.

## Authority commands

```bash
dekk apxm validate <workflow.air>      # validate against AIS contract
dekk apxm compile <workflow.air> -o <out>  # → .apxmobj artifact
dekk apxm run <out.apxmobj>            # execute a pre-compiled artifact
dekk apxm execute <workflow.air>       # compile + execute in one step
dekk apxm analyze <workflow.air>       # parallelism + critical path
dekk apxm explain <workflow.air>       # human-readable summary
dekk apxm decompile <out.apxmobj>      # reverse-map back to AIR
```

For multi-step `.apxmw` workflow files:

```bash
dekk apxm workflow validate <wf.apxmw>           # validate the workflow file
dekk apxm workflow analyze <wf.apxmw>            # step graph + parallelism
dekk apxm workflow run <wf.apxmw> --session-root <dir> --json
dekk apxm workflow run <wf.apxmw> --background --session-root <dir> --json
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
dekk apxm validate workflow.air
dekk apxm compile workflow.air -o .apxm/compiled/workflow.apxmobj
dekk apxm run .apxm/compiled/workflow.apxmobj
```

### Inspect parallelism before executing

```bash
dekk apxm analyze workflow.air     # surfaces phases, critical path
dekk apxm explain workflow.air     # readable summary
```

### Iterate against a service allocation

```bash
dekk apxm vllm service-exec <name> -- dekk apxm execute workflow.air
```

## Diagnostics

- `dekk apxm doctor` — env sanity.
- `dekk apxm ops list` — live op surface.
- `dekk apxm validate --verbose` — verbose contract errors.

## Anti-patterns

- Hand-rolling workflow structure outside AIR or the Python frontend
  (`crates/compiler/apxm-frontend/python`).
- Running `compile` after editing a `.td` without re-running
  `build-dialect` + `codegen` — produces stale frontend bindings and
  confusing validation errors.
- Writing `.apxmobj` files anywhere other than `.apxm/`.
