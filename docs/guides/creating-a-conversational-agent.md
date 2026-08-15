# Build a conversational Agent

A conversational Agent is an ordinary Agent Program with explicit Context,
Model and Tool calls, and an authored loop. It is a teaching shape, not a
separate runtime, package-level `ConversationalAgent`, or core `Turn` type.

The executable references are:

- [Python example](../../examples/agents/conversational/python/agent.py)
- [TypeScript example](../../examples/agents/conversational/src/conversational-agent.ts)

Both examples lower through the same source-first path:

```text
typed Agent source
  -> FrontendGraph
  -> AIR and registered AIS operations
  -> exact admitted Model and Capability ports
  -> runtime evidence
```

## The small vocabulary

Use `Agent`, `Context`, `Model`, and `Tool` first. Add `Event`, `Hook`, or a
specialist Agent only when the program needs those semantics. The frontend
captures declarations and control flow; it never executes the body while
compiling.

A Capability reference has to name something an implementation exists for — a
built-in id, or an id the package ships a handler for at
`capabilities/<id>/handler.py` or `handler.ts`.
Import the catalogue symbol rather than retyping the string: a misspelled symbol
is an `ImportError` at author time, while a misspelled string is a reference the
compile service refuses later.

```python
from apxm_program.capabilities import SEARCH_WEB

SearchWeb = Tool[SearchWebRequest, SearchWebResult](SEARCH_WEB)
SupportModel = Model[ModelRequest, ModelResponse]("model.target")


@Context
class ConversationContext:
    messages: tuple[ConversationMessage, ...] = ()
    last_reply: str = ""


@Agent(input=ConversationInput, output=ConversationOutput,
       context=ConversationContext)
async def ConversationalExample(agent, incoming):
    while incoming["message"]:
        response = await SupportModel({
            "messages": agent.context.messages,
            "incoming": incoming,
        })
        while response["kind"] == "tool_request":
            if response["tool_request"]["kind"] != "search_web":
                raise ValueError("undeclared tool request")
            result = await SearchWeb(response["tool_request"]["arguments"])
            response = await SupportModel({
                "messages": agent.context.messages,
                "incoming": incoming,
                "tool_result": result,
            })
        if response["kind"] != "final":
            raise ValueError("undeclared model response")
        agent.context = ConversationContext(
            messages=agent.context.messages,
            last_reply=response["reply"]["message"],
        )
        incoming = await agent.yield_(response["reply"])
```

The TypeScript reference has the same structure using the generic
`Agent`, `Context`, `Model`, and `Tool` declarations. A Tool request is an
ordinary authored branch: the program calls a declared Capability, then
explicitly re-enters the Model with the result. There is no hidden model/tool
dispatcher.

## Context and yielding

Context is the durable state selected by the Agent Program. Assign a new
Context value when a step should be committed. `agent.yield_(value)` commits
that state and returns the next input; an Event wait parks the invocation
without inventing a second conversational runtime. Normal language control
flow (`if`, `match`/`switch`, loops, exceptions, and returns) remains the source
of behavior.

## Admission and execution

The source names Model and Tool references, but it does not grant authority or
select a provider. The host admits exact capability bindings and model ports,
then the runtime executes the verified artifact. Provider adapters are
implementation details behind those ports and cannot change the Agent API.

Runtime evidence records generic node executions, capability attempts, model
calls, loop iterations, Context transitions, outputs, and failures. A host may
project that evidence back onto source locations, but the core vocabulary stays
generic.

For the normative details, read the [composition and AIR contract](../agents/agent-program-composition-and-air-contract.md),
the [execution admission contract](../agents/execution-admission-contract.md),
and the [PXM theory](../pxm/theory.md).
