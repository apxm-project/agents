# Conversational reference example

This is the primary APXM authoring reference. Its equivalent Python and
TypeScript sources use only the installed generic frontend: `Agent`, `Context`,
`Tool`, `Model`, `Hook`, ordinary control flow, and `agent.yield_()`.

The example demonstrates a complete conversational shape without a
conversation-specific runtime or package API:

1. record each resumed user input in persistent Context;
2. call the Model with a bounded context window and a closed Tool inventory;
3. dispatch `SearchWeb` or `ReadPage` only when the Model requests it;
4. append the typed Tool result and re-enter the same authored loop;
5. record and yield a final reply that resumes with the next input; and
6. apply statically bound before/after Hooks for context policy and accounting.

The Model returns Tool requests as typed data. Source owns the closed dispatch,
the Capability invocation, Model re-entry, core message transitions, and the
yield/resume boundary. Hooks compose cross-cutting context management around
those visible operations; they do not hide the conversational state machine.

Coder is a small extension of this reference focused on coding capabilities.

Run the focused build, compile, and parity checks from the repository root:

```sh
dekk agents test-frontend-examples
```

`package.json` declares the Python and TypeScript build, compile, and run
entrypoints consumed by the repository control plane.
