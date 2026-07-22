# apxm_program — generic Python Agent Program frontend

`apxm_program` exposes generic Agent Program, Hook, Context, composition,
structured-control-flow, source-map, and native compiler-bridge APIs. It
records `apxm.frontend-graph.v1`; the Rust compiler alone validates and lowers
that graph to AIR.

Structured authoring uses `branch`, `switch`, `loop`, `parallel`,
`try_catch`, `throw_region`, `return_region`, `yield_region`, and joined
`structured_task` scopes.

## Quick start

```bash
pip install apxm
```

```python
import apxm_program

program = apxm_program.AgentProgram(
    program_id="hello",
    input_type_ref="Input",
    output_type_ref="Output",
    context_type_ref="Context",
)
program.loop(
    "region.loop",
    lambda body: body.model_call("node.greet", "model.default"),
)
program.return_region("region.return")
print(program.canonical_air_json(), end="")
```

## Compiling workflows

Source packages that declare a Python `[compile]` entry are compiled through the
explicit Agents compiler bridge:

```bash
dekk agents agent build path/to/package
dekk agents compile-service-canonical path/to/package
```
