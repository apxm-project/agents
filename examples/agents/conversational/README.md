# Conversational reference example

This is the primary APXM authoring reference. Its equivalent
[Python](python/agent.py) and
[TypeScript](src/conversational-agent.ts) sources use only the installed
generic frontend: `Agent`, `Context`, `Hook`, `Tool`, `Model`, ordinary
control flow, and `agent.yield_()`.

Each turn makes the complete conversational state machine visible in source:

1. append the resumed input to persistent working messages;
2. call the Model with those messages and the current input;
3. if the typed Model response requests the one declared `search_web` Tool,
   invoke it through an authored closed dispatch loop;
4. append the typed Tool result and call the Model again;
5. append the final assistant reply to explicit Context; and
6. yield the reply, binding the resume value as the next input.

Static before/after Hooks bracket the Tool boundary to enforce a persisted
context window and record Tool-call accounting. They do not choose the Tool,
grant authority, or hide the core message and dispatch transitions.

Run the focused build, compile, runtime-entrypoint, parity, and regression
checks from the repository root:

```sh
dekk agents test-frontend-examples
```

`package.json` declares the Python and TypeScript build, compile, and run
entrypoints consumed by the repository control plane.
