# Start authoring an Agent

- Audience: Python and TypeScript Agent Program authors
- Frontend status: source-first authoring is the public frontend direction;
  compile the checked-in references to confirm the checkout you are using
- Authority: [ADR-0014](../adr/0014-conversational-agent-and-gao-are-examples-over-generic-agent-program-apis.md),
  [ADR-0015](../adr/0015-source-first-agent-frontend-vocabulary.md), and
  [ADR-0016](../adr/0016-tool-authoring-and-handler-execution-are-separate.md)

## Start with one ordinary Agent

Start with the [Conversational reference](../../examples/agents/conversational/README.md).
It is the primary, cross-language example: a typed `Agent` reads explicit
`Context`, calls a `Model` and a `Tool`, composes a short-lived specialist, and
uses an ordinary source loop with `agent.yield_()`.

That shape is intentionally not a framework within the framework. APXM has no
package-level `ConversationalAgent`, conversational runtime, or core `Turn`
type. It is one generic Agent Program whose source makes the behavior visible
([ADR-0014](../adr/0014-conversational-agent-and-gao-are-examples-over-generic-agent-program-apis.md)).

Use the smallest vocabulary that describes your program:

```text
Agent     Context     Tool     Model     ordinary language control flow
```

`Capability`, `Event`, `Hook`, and `TaskGroup` are focused additions when the
program actually needs them. Do not start from graph ids, AIR, `ais.*` names,
runtime objects, endpoints, credentials, grants, or a hidden model/tool loop;
those are compiler, runtime, or admission concerns rather than authored
behavior ([ADR-0015](../adr/0015-source-first-agent-frontend-vocabulary.md)).

## Use the focused extensions as patterns

The reference stays the teaching path. Read the focused examples only when
their behavior matches the program you are writing:

| Need | Read | Take from it |
| --- | --- | --- |
| A coding-oriented review/proposal flow | [Coder](../../examples/agents/coder/README.md) | Typed read, edit-proposal, and test-command capabilities; the program proposes work and never mutates files or runs commands itself. |
| An APXM-oriented planning flow | [Gao](../../examples/agents/gao/README.md) | Capability discovery, reviewable workflow planning, and validation preparation over explicit Context, Model calls, and yield/resume. |

Coder and Gao build on the same installed generic frontend as Conversational.
They are not imports, base classes, runtime modes, or privileged identities.
Copy the small behavior pattern you need into a new ordinary Agent; do not make
your program depend on an example name ([ADR-0014](../adr/0014-conversational-agent-and-gao-are-examples-over-generic-agent-program-apis.md)).

## A professional authoring loop

1. Define typed input, output, and only the Context that must survive a
   stateful invocation.
2. Bind exact Models and Tool/Capability references at module scope. Make every
   effect call and every Context replacement explicit in the Agent body.
3. Use normal `if`, `match`/`switch`, loops, `try`/`catch`, and `return` for
   behavior. Use `agent.yield_(output)` only when the Program Instance should
   commit its next Context and resume on its next input; use an `Event` wait to
   park the same invocation instead.
4. If a TypeScript package supplies a local Tool implementation, keep it in a
   handler module using `Tool.define`. The generated handler manifest is build
   output; it is not source to edit and it does not make Node the Agent runtime
   or grant authority ([ADR-0016](../adr/0016-tool-authoring-and-handler-execution-are-separate.md)).
5. Compile and validate references through the repository surface:

   ```sh
   dekk agents test-frontend-examples
   ```

The frontend captures source intent; Rust selects and verifies AIR/AIS; an
admitted runtime executes the artifact. This keeps source readable without
turning Python or TypeScript into a second runtime
([ADR-0015](../adr/0015-source-first-agent-frontend-vocabulary.md)).

## Continue from here

- [Author an Agent](creating-an-agent-program.md) for Python/TypeScript
  declarations, composition, Context, and Tool details.
- [Build a conversational Agent](creating-a-conversational-agent.md) for the
  complete stateful loop, Model/Tool dispatch, events, and Hooks.
- [Compose Agent Programs](composing-agent-programs.md) when one Agent needs a
  one-shot or stateful specialist.
