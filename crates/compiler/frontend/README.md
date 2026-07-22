# APXM Compiler Frontends

Python and TypeScript authoring frontends record the same
`apxm.frontend-graph.v1` value. Rust verifies that graph and lowers it to
`apxm.air.v1`; frontend packages do not construct MLIR or expose raw AIR
operation builders.

The Python package is [`python/`](python/) and exports `apxm_program`.
TypeScript is [`typescript/`](typescript/).

```python
import apxm_program

program = apxm_program.AgentProgram(
    program_id="support",
    input_type_ref="SupportRequest",
    output_type_ref="SupportAnswer",
)
program.builder.model_call("node.answer", model_target_ref="model.support")
program.builder.model_requirement("model.support")
program.builder.return_region("region.return")

air = program.lower()
```

The only public semantic operations are `model.call`, `capability.invoke`,
`program.new`, `program.invoke`, and `await.event`. Branching, looping,
yielding, and returning are structural FrontendGraph constructs.
