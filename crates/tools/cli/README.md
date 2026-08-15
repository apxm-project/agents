# APXM CLI

`apxm` is the local command shell. Invoke it through `dekk agents` so the
managed toolchain and target directory remain consistent.

[ADR-0023](../../../docs/adr/0023-compilation-and-runtime-services-and-program-owned-interaction.md)
owns the architecture: the shell sequences a Compilation Client (`apxm build`)
and an Interaction Client (`apxm run`, TUI, `apxm event`, `apxm resume`).
`--artifact` invokes the Runtime Service without contacting compilation.
`apxm runtime serve` / `--connect` supervise local stdio/Unix transports.

`compile-service-canonical`, `execute-canonical`, `codegen`, and `canonical-air`
live only on the Dekk-owned `apxm-dev` binary in the `apxm-cli-dev` package.
Production `apxm` does not compile those composition roots. `apxm build`
regenerates package integrity and then commits an artifact through the
Compilation Client.

The CLI does not parse `chat`, `watch`, or `rollout`. OpenAI `/v1/chat/completions`
is an independently owned protocol edge (ADR-0024 for non-loopback).
