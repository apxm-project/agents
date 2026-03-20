# Applied Projects

Concrete applications being built on A-PXM. Each project validates a different aspect of the platform thesis.

## Projects

### [`codex/`](codex/) — Codex-on-APXM

Reconstruct a Codex-class coding agent on A-PXM. This is the "LLVM-GCC" moment — proving the substrate can represent production agent execution. AgentMate is one possible frontend for authoring graphs, not the strategic center of the effort.

- [plan.md](codex/plan.md) — 5-phase implementation plan
- [architecture.md](codex/architecture.md) — Codex concepts → A-PXM component mapping
- [primitives.md](codex/primitives.md) — The 7 runtime primitives every coding agent needs

### [`agentmate/`](agentmate/) — Frontend SDK

AgentMate is the developer-facing Rust + Python SDK. It is to A-PXM what Clang is to LLVM — a native frontend that can make targeting the substrate ergonomic, but it is not the substrate itself.

- [architecture.md](agentmate/architecture.md) — 13 crates, end-to-end pipeline
- [integration.md](agentmate/integration.md) — APXM crate dependencies and integration points
- [status.md](agentmate/status.md) — Implementation status and remaining gaps

## Relationship to Other Docs

- **Theory** (why): [`../pxm/`](../pxm/) — the formal execution model
- **Implementation** (how): [`../implementation/`](../implementation/) — compiler, runtime, TODOs
- **Advantages** (value): [`../advantages/`](../advantages/) — optimizations, hypotheses, LLVM parallel
