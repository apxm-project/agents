# Gao repository example

Gao is a TypeScript-only Agent Program example. Its `Gao` class extends the
example-local `ConversationalAgent` from
`@apxm/example-agent-conversational`; both examples author only installed
generic `@apxm/frontend` APIs.

The source records an ordinary structured loop, model and Capability calls,
`program.new`/`program.invoke` specialist composition, static Hooks, Context
flow, and `await.event`. Gao has no package export, compiler branch, runtime
mode, Server route, or privileged identity.

Package-local TypeScript Capability handlers use the separate
`@apxm/agent-packaging` authoring and bundling surface; they are not frontend
or Agent Program APIs.

Run the focused build, compile, and authoring checks from the repository root:

```sh
dekk agents test-frontend-examples
```

`package.json` declares the example's build, compile, run, and test entrypoints
consumed by the repository control plane.
