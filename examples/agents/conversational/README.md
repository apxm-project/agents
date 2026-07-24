# Conversational reference example

This is the primary APXM authoring reference. Its equivalent Python and
TypeScript sources use only the installed generic frontend: `Agent`, `Context`,
`Tool`, `Model`, `.new()`/`.invoke()`, an authored loop, and `agent.yield_()`.

The example demonstrates a complete conversational shape without a
conversation-specific runtime or package API:

1. create a short-lived research specialist;
2. invoke it through explicit composition;
3. call the Model with its result;
4. replace explicit Context; and
5. yield a reply that resumes with the next input.

Coder and Gao are small extensions of this reference: Coder focuses on coding
capabilities, while Gao focuses on APXM authoring capabilities.

Run the focused build, compile, and parity checks from the repository root:

```sh
dekk agents test-frontend-examples
```

`package.json` declares the Python and TypeScript build, compile, and run
entrypoints consumed by the repository control plane.
