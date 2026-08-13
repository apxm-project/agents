# APXM authoring examples

These are the complete, deliberately small authoring examples. They all use
the installed generic APXM frontend—never handwritten FrontendGraph, AIR, a
frontend runtime, or a privileged named-agent API.

| Start here | Purpose | Language |
| --- | --- | --- |
| [Conversational](agents/conversational/README.md) | Primary reference: Context, Tool, Model, composition, loop, and yield/resume | Python and TypeScript |
| [Coder](agents/coder/README.md) | Coding-capability extension: read, edit proposal, and test proposal | TypeScript |

Conversational is the teaching reference. Coder is its focused coding
extension. Both are ordinary Agent Programs, not product features, package
exports, compiler modes, or runtime identities. Product-owned Agent Programs
enter through the same public frontend as external source packages.

## Run the examples

From this repository root:

```sh
dekk agents doctor
dekk agents test-frontend-examples
```

The gate builds each checked-in example with the installed Python or TypeScript
frontend, compiles it through the explicit compiler bridge, and checks the
conversational Python/TypeScript parity. Compile-only validation uses
deterministic test bindings; executing a real Model or Capability requires its
separately admitted deployment and authority.

An external TypeScript package can run the same frontend/compiler conformance
without becoming an Agents-owned example:

```sh
dekk agents test-external-source-package path/to/package
```

## Package tools

Package-local capability handlers use the same small authoring pattern:
`Tool.define` names the tool, `Tool.object` and `Tool.text` describe its typed
input, and `Tool.answer` returns one typed answer object. Authors write ordinary
TypeScript objects; the generated `capabilities/handlers/tools.json` sidecar is
an internal build output and is never edited by hand.

This TypeScript package helper is a build input, not an APXM runtime. The
Rust-owned handler-manifest contract records its output, and the admitted Rust
Capability port owns execution. The Node worker used by package tests only
proves a TypeScript bundle conforms to that contract. A Python package helper
is unnecessary until APXM supports a Python package-local handler bundle.
