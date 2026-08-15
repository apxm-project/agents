# APXM CLI

`apxm` is the local command shell. Invoke it through `dekk agents` so the
managed toolchain and target directory remain consistent.

[ADR-0023](../../../docs/adr/0023-compilation-and-runtime-services-and-program-owned-interaction.md)
owns the target architecture: the shell sequences a Compilation Client
(source package to a committed artifact) and an Interaction Client (admitted
artifact through the Runtime Service). It does not own compiler lowering,
runtime construction, or conversational policy. Interaction harnesses are
compiled Agent Programs.

Until cutover, the tree still exposes `compile-service-canonical` and
`execute-canonical` as current fixture commands. Those are not the accepted
product spine.

The CLI does not own a server chat REPL, rollout archive, deployment fleet,
model zoo, or product UI. Hidden `chat`/`watch`/`rollout` parser variants are
retired and will be deleted rather than aliased. Runtime implementations
enter through exact admitted capability and model ports.
