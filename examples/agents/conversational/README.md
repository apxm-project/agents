# Conversational reference example

This is the primary APXM authoring reference. Its equivalent
[Python](python/agent.py) and
[TypeScript](src/conversational-agent.ts) sources use only the installed
generic frontend: `Agent`, `Context`, `Hook`, `Tool`, `Model`, ordinary
control flow, and `agent.yield_()`.

Each turn makes the complete conversational state machine visible in source:

1. bind the resumed input into the next loop iteration;
2. call the Model with persistent messages and the current input;
3. if the typed Model response requests the one declared `search_web` Tool,
   invoke it through an authored closed dispatch loop;
4. bind the typed Tool result directly into the next Model request;
5. reject every undeclared Tool or Model-response discriminant;
6. persist the final assistant reply in explicit Context; and
7. yield the reply, binding the resume value as the next input without
   replacing committed Context.

Static before/after Hooks bracket the Tool boundary to enforce a persisted
context window and record Tool-call accounting. They do not choose the Tool,
grant authority, or hide the core message and dispatch transitions. The next
Model request directly depends on the Tool-result SSA value, independently of
any Context change made by those Hooks.

Run the focused build, compile, runtime-entrypoint, parity, and regression
checks from the repository root:

```sh
dekk agents test-frontend-examples
```

`package.json` declares the Python and TypeScript build, compile, and run
entrypoints consumed by the repository control plane.
