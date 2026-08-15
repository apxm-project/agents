# APXM authoring examples

These are the complete, deliberately small authoring examples. They all use
the installed generic APXM frontend—never handwritten FrontendGraph, AIR, a
frontend runtime, or a privileged named-agent API.

| Start here | Purpose | Language |
| --- | --- | --- |
| [Conversational](agents/conversational/README.md) | Primary reference: Context, Tool, Model, composition, loop, and yield/resume | Python and TypeScript |
| [Coder](agents/coder/README.md) | Coding-capability extension: read, edit proposal, and test proposal | TypeScript |
| [Skilled](agents/skilled/README.md) | Agent Skills: declaring instructions a package carries and instructions the program writes, and loading both | Python |

Conversational is the teaching reference. Coder is its focused coding
extension. Skilled is the smallest program that declares an Agent Skill and
reads it. All three are ordinary Agent Programs, not product features, package
exports, compiler modes, or runtime identities. Product-owned Agent Programs
enter through the same public frontend as external source packages.

## Run the examples

From this repository root:

```sh
dekk agents doctor
dekk agents test-frontend-examples
dekk agents test-skill-example
```

The gate builds each checked-in example with the installed Python or TypeScript
frontend, compiles it through the explicit compiler bridge, and checks the
conversational Python/TypeScript parity. Compile-only validation uses
deterministic test bindings; executing a real Model or Capability requires its
separately admitted deployment and authority.

`test-skill-example` goes one step further for Skilled, because a skill that
cannot be read is not a skill: it compiles the package, publishes the skills the
program declares into a local discovery root, and executes the compiled AIR
through `apxm execute-canonical`, so the instructions come back from the
`read_skill` capability rather than from a fixture.

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

These package helpers are build inputs, not an APXM runtime. The Rust-owned
handler-manifest contract records their output, and the admitted Rust
Capability port owns execution. There is one per language — a Node worker and a
Python worker — because a package may ship a handler in either, and both
register through the same port.
