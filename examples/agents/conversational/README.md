# Conversational Agent repository example

This example implements equivalent Python and TypeScript conversational Agent
Programs over the installed generic frontends. Each language records the same
FrontendGraph and lowers to equivalent AIR with `ais.loop`; the example-local
`ConversationalAgent` class is not an APXM package API.

Run the focused build, compile, and parity checks from the repository root:

```sh
dekk agents test-frontend-examples
```

`package.json` declares the separate Python and TypeScript build, compile, and
run entrypoints consumed by the repository control plane.
