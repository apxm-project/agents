# apxm_program — Python Agent authoring frontend

Author an Agent in ordinary Python with decorators and typed values:

```python
from apxm_program import Agent, Model

SummarizerModel = Model[object, object]("model.summary")


@Agent(input="SummaryRequest", output="Summary")
async def Summarizer(agent, request):
    return await SummarizerModel(request)
```

`Agent`, `Context`, `Tool`, and `Model` cover ordinary programs;
`Capability`, `Event`, `Hook`, and `TaskGroup` are focused extensions. Python
parses the module with the host AST, binds recognized APXM symbols and types
into an immutable frontend-internal typed source tree, and deterministically
traverses it into FrontendGraph. It never executes the Agent body to discover
behavior and never prints AIR or MLIR.

The package exposes only these declaration markers. It has no imperative program
builder, region or node recorder, operation constant, or raw graph API.

## Compiling workflows

Source packages that declare a Python `[compile]` entry are compiled through the
explicit Agents compiler bridge:

```bash
dekk agents agent build path/to/package
dekk agents compile-service-canonical path/to/package
```
