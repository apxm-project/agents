# Conversational Agent repository example

This example is the current low-level conformance scaffold for equivalent
Python and TypeScript conversational Agent Programs. Each language records the
same FrontendGraph and lowers to equivalent AIR with `ais.loop`; the
example-local `ConversationalAgent` class is not an APXM package API.

Conceptually, a conversational Agent is an ordinary Agent with a typed Context,
an authored loop, explicit Model and optional Tool/Capability calls, and a
yield/resume boundary. The same generic frontend can add Events, Hooks,
specialists, and structured task groups; no conversation-specific compiler or
runtime feature is required.

It is not the intended teaching surface: the current source exposes recorder
details such as node/region ids and manual source-span binding. The
[source-first authoring guide](../../../docs/guides/creating-an-agent-program.md)
defines the target author experience and the conditions for replacing this
scaffold with ordinary typed agent source.

Run the focused build, compile, and parity checks from the repository root:

```sh
dekk agents test-frontend-examples
```

`package.json` declares the separate Python and TypeScript build, compile, and
run entrypoints consumed by the repository control plane.
