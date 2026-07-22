# apxm_program — Python frontend for APXM

The Python authoring frontend for
[APXM](https://github.com/apxm-project/agents), the canonical compiler and
runtime for Agent Programs.

This package records APXM workflows as the compiler-owned
`apxm.frontend-graph.v1` DTO. The canonical Python bridge lowers that graph
through the Rust compiler in-process and returns `apxm.air.v1`; TypeScript and
Rust follow the same FrontendGraph contract.

## Quick start

```bash
pip install apxm
```

```python
import apxm_program

builder = apxm_program.GraphBuilder()
builder.program(
    "hello",
    "run",
    "Input",
    "Output",
    True,
    "Context",
)
builder.model_call("node.greet", model_target_ref="model.default")
builder.model_requirement("model.default")
builder.region("region.return", "return")

print(apxm_program.canonical_air_json(builder.build()), end="")
```

## What's in the package

| Module | What it does |
| ------ | ------------ |
| `apxm_program` | Canonical FrontendGraph recorder, program constructs, and native bridge helpers |

## Compiling workflows

Source packages that declare a Python `[compile]` entry are compiled through the
explicit Agents compiler bridge:

```bash
dekk agents agent build path/to/package
dekk agents compile-service-canonical path/to/package
```

## Companion repos

- [apxm-project/vllm](https://github.com/apxm-project/vllm) — graph-aware
  vLLM fork used by APXM-vLLM

Provider-agnostic operating skills loaded by `apxm-server` are builtin under
`crates/server/skills/`; deployment-specific skills should be
installed through explicit skill roots.

## License

MIT. See [LICENSE](https://github.com/apxm-project/agents/blob/main/LICENSE).
