---
name: compile-and-execute
group: Domain
description: Use when compiling APXM AIR workflows, running .apxmobj artifacts, or executing AIR/IR through the runtime. Enforces dekk agents as the authority CLI and correct artifact placement under .apxm/.
user-invocable: true
---

# APXM Compile & Execute

Load `_shared/apxm-development-rules.md` before broad work.

## Authority commands

```bash
dekk agents validate <workflow.air>      # validate against AIS contract
dekk agents compile <workflow.air> -o <out>  # → .apxmobj artifact
dekk agents run <out.apxmobj>            # execute a pre-compiled artifact
dekk agents execute <workflow.air>       # compile + execute in one step
dekk agents analyze <workflow.air>       # parallelism + critical path
dekk agents explain <workflow.air>       # human-readable summary
dekk agents decompile <out.apxmobj>      # reverse-map back to AIR
```

For multi-step `.apxmw` workflow files:

```bash
dekk agents workflow validate <wf.apxmw>           # validate the workflow file
dekk agents workflow analyze <wf.apxmw>            # step graph + parallelism
dekk agents workflow run <wf.apxmw> --session-root <dir> --json
dekk agents workflow run <wf.apxmw> --background --session-root <dir> --json
```

## Rules

- Compiled artifacts (`*.apxmobj`) belong under `.apxm/`, never under
  `examples/` or `docs/`. Use `RepoLayout` for paths.
- Execution against a vLLM backend goes through a registered service.
  See `vllm-service` for the service path; never invoke `python
  benchmark.py` against `http://127.0.0.1:<port>/v1` outside a
  `service-exec` call.
- After editing a `.td` op definition, run `dekk agents build-dialect`
  then `dekk agents codegen` before invoking `compile`/`execute` — the
  Python frontend bindings drive what `validate` accepts.

## Typical flows

### Compile then run

```bash
dekk agents validate workflow.air
dekk agents compile workflow.air -o .apxm/compiled/workflow.apxmobj
dekk agents run .apxm/compiled/workflow.apxmobj
```

### Inspect parallelism before executing

```bash
dekk agents analyze workflow.air     # surfaces phases, critical path
dekk agents explain workflow.air     # readable summary
```

### Iterate against a service allocation

```bash
dekk agents vllm service-exec <name> -- dekk agents execute workflow.air
```

## Diagnostics

- `dekk agents doctor` — env sanity.
- `dekk agents ops list` — live op surface.
- `dekk agents validate --verbose` — verbose contract errors.

## Anti-patterns

- Hand-rolling workflow structure outside AIR or the Python frontend
  (`crates/compiler/frontend/python`).
- Running `compile` after editing a `.td` without re-running
  `build-dialect` + `codegen` — produces stale frontend bindings and
  confusing validation errors.
- Writing `.apxmobj` files anywhere other than `.apxm/`.
