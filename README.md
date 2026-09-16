# APXM agents

`agents` is the product-neutral APXM abstract machine: source-first Agent
Programs, the closed AIR/AIS semantics, exact capability and inference ports,
and the runtime that executes admitted artifacts.

It is not a product control plane, Studio, Server, Telegram integration,
deployment fleet, model zoo, or prompt-evaluation repository. Those systems may
bind to these contracts from outside.

## Current spine

```text
Python / TypeScript source
  -> FrontendGraph
  -> Rust verification and AIR lowering
  -> canonical AIS dialect MLIR (verified when the native MLIR toolchain is available)
  -> immutable artifact
  -> exact Invocation Admission and Port bindings
  -> generic execution kernel
  -> monotonic runtime evidence
```

The public semantic operations are exactly `model.call`,
`capability.invoke`, `program.new`, `program.invoke`, and `await.event`.
Compiler-emitted structural operations are separate and are never a raw
frontend builder API.

## Layout

- `crates/machine/ais` — canonical operation definitions and generated AIS.
- `crates/machine/program` — FrontendGraph, AIR, artifact verification.
- `crates/compiler` — Rust compiler and Python/TypeScript bridges.
- `crates/runtime` — exact inference, kernel, execution, and capability ports.
- `crates/tools/cli` — focused compile, validate, inspect, and execute CLI.
- `contracts` — schemas, Port Contracts, and conformance vectors.
- `examples/agents` — small generic source-first examples.
- `docs/pxm` — current theory plus historical PXM lineage.

## Development

Use Dekk for repository commands:

```bash
dekk agents doctor
dekk agents ops list
dekk agents check
dekk agents test-frontend-examples
```

Agent instructions come from `AGENTS.md`. The workspace agent-skill tooling
generates and checks the thin root adapters (`CLAUDE.md`, `CODEX.md`,
`.cursorrules`, `.github/copilot-instructions.md`, `.agents.json`, and
`.claude/skills`) without writing global agent configuration. Edit `AGENTS.md`
and `.agents/skills/` first, then run the adapter check for this checkout.
