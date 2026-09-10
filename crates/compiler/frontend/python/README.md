# apxm_program — Python Agent authoring frontend

Author an Agent in ordinary Python with decorators and typed values:

```python
from apxm_program import Agent, Model
from typing import TypedDict


class SummaryRequest(TypedDict):
    text: str


class Summary(TypedDict):
    text: str


SummarizerModel = Model[SummaryRequest, Summary]("model.summary")


@Agent(input=SummaryRequest, output=Summary, model=SummarizerModel)
async def Summarizer(agent, request):
    return await SummarizerModel(request)
```

`input`, `output`, and `context` are the typed declarations themselves, never
strings naming them — a renamed type renames its reference, and a reference to a
type that does not exist is a `NameError` at the definition site. TypeScript
states the same three as `Agent<Input, Output, Context>`.

`Agent` requires an explicit typed `model` binding invoked by the captured body.
`Workflow` declares general orchestration and needs no model when it makes no
model call. Both return the same sealed `Program[Input, Output, Context]` with
typed `invoke(input)` and `new(context=...)`; the public Protocol is not a raw
graph constructor. Loops, Context, Hooks and yield/resume are shared behavior.

`Agent`, `Workflow`, `Context`, `Tool`, and `Model` cover ordinary programs;
`Capability`, `Event`, `Hook`, `Skill`, and `TaskGroup` are focused extensions.
`Skill` declares instructions the program can load — carried as a package file
with `entry=`, or written in the source with `text=`, never both — and
`await skill.load()` reads them through the skill-reading Capability, so the
artifact declares that authority the way it declares any other. Python
parses the module with the host AST, binds recognized APXM symbols and types
into an immutable frontend-internal typed source tree, and deterministically
traverses it into FrontendGraph. It never executes the Agent body to discover
behavior and never prints AIR or MLIR.

The package exposes only these declaration markers. It has no imperative program
builder, region or node recorder, operation constant, or raw graph API.

## Shipping a Capability

`apxm_program.handlers` is the other half of the surface: the declaration that
implements a Capability rather than referencing one.

```python
from apxm_program import Tool
from apxm_program.handlers import answer, capability, schema, text

edit = capability({
    "name": "edit",
    "description": "Return a before/after proposal without changing a file.",
    "read_only": True,
    "input": schema(
        additional_properties=False,
        properties={
            "file_path": text(required=True, min_length=1),
            "before": text(required=True, min_length=1),
            "after": text(required=True, min_length=1),
        },
    ),
    "run": lambda args: answer({**args, "mutates": False}),
})

# `edit` is the exact Capability reference, so the program that calls the
# handler and the package that ships it cannot name different Capabilities.
ProposeEdit = Tool[dict, dict](edit)
```

`required` is stated on the property rather than in a list beside it, and
`additional_properties` has no default: both are decisions, and a decision made
in one place cannot disagree with itself.

## Compiling workflows

Source packages that declare a Python `[compile]` entry are compiled through the
explicit Agents compiler bridge:

```bash
dekk agents agent build path/to/package
dekk agents compile-service-canonical path/to/package
```
