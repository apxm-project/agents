# apxm_program — Python Agent authoring frontend

- Current implementation: low-level conformance recorder
- Target syntax: design proposal; not implemented at the pinned frontend
  baseline
- Target guide:
  [Author an Agent](../../../../docs/guides/creating-an-agent-program.md)

The target Python experience uses decorators and typed values:

```python
from apxm_program import Agent, Model

SummarizerModel = Model[SummaryRequest, Summary](ExactSummarizerModelRef)


@Agent(input=SummaryRequest, output=Summary)
async def Summarizer(agent, request):
    return await SummarizerModel(request)
```

`Agent`, `Context`, `Tool`, and `Model` cover ordinary programs;
`Capability`, `Event`, `Hook`, and `TaskGroup` are focused extensions. Python
parses the module with the host AST, binds recognized APXM symbols/types into
an immutable frontend-internal typed source tree, and deterministically
traverses it into FrontendGraph. It never executes the Agent body to discover
behavior and never prints AIR/MLIR.

The current package still exposes imperative `AgentProgram` and region/node
recorders used by repository parity fixtures. Those APIs are baseline evidence,
not the intended author surface, and the full replacement keeps no compatibility
alias for them.

## Compiling workflows

Source packages that declare a Python `[compile]` entry are compiled through the
explicit Agents compiler bridge:

```bash
dekk agents agent build path/to/package
dekk agents compile-service-canonical path/to/package
```
