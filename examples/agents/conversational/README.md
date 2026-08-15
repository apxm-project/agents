# Conversational reference example

This is the primary APXM authoring reference. Its equivalent
[Python](python/agent.py) and
[TypeScript](src/conversational-agent.ts) sources use only the installed
generic frontend: `Agent`, `Capability`, `Context`, `Hook`, `Skill`, `Tool`,
`Model`, ordinary control flow, and `agent.yield_()`.

Each turn makes the complete conversational state machine visible in source:

0. load the declared `persona` and `context-policy` Skills, whose instructions
   the package carries under `skills/`;
1. bind the resumed input into the next loop iteration;
2. call the Model with persistent messages and the current input;
3. if the typed Model response requests the one declared `search_web` Tool,
   invoke it through an authored closed dispatch loop;
4. bind the typed Tool result directly into the next Model request;
5. reject every undeclared Tool or Model-response discriminant;
6. persist the final assistant reply in explicit Context; and
7. yield the reply, binding the resume value as the next input without
   replacing committed Context.

Static before/after Hooks bracket the Tool boundary. Their bodies are captured
as ordinary Agent Program structure, not as opaque handlers the artifact only
names: the before Hook measures the model-visible conversation through the
declared `count_tokens` Capability, hands that measurement to the declared
`model.compaction` Model, and persists the compacted conversation in Context;
the after Hook records which Capability the turn last dispatched. Both the
measurement and the compaction are nodes in the compiled AIR — a
`capability.invoke` and a `model.call` inside the Hook's own region — so a
context budget is workflow structure rather than host code the compiler cannot
see.

Each Capability binding states the permission the *program* requests, in source.
`agent.toml [permissions]` is the deployment layer on top of it: it narrows the
`allow` the source asks for on `count_tokens` to `ask`, and it may never widen
what the source requested.

The Hooks do not choose the Tool, grant authority, or hide the core message and
dispatch transitions. The next Model request directly depends on the Tool-result
SSA value, independently of any Context change made by those Hooks.

Run the focused build, compile, runtime-entrypoint, parity, and regression
checks from the repository root:

```sh
dekk agents test-frontend-examples
```

`package.json` declares the Python and TypeScript build, compile, and run
entrypoints consumed by the repository control plane.
