# Applied Projects

Concrete applications being built on A-PXM. Each project validates a different aspect of the platform thesis.

## Codex & Gemini-CLI Integration (Active)

Migrating both OpenAI Codex (Rust, 74-crate workspace) and Google Gemini-CLI (TypeScript, 7-package monorepo) onto A-PXM as their shared execution substrate. Five phases, bottom-up.

**Private working copies (this machine):**

| Upstream | GitHub (private) | Local path |
|----------|------------------|------------|
| [openai/codex](https://github.com/openai/codex) | [youruser/openai-codex](https://github.com/youruser/openai-codex) | `$HOME/projects/agents/openai/codex` |
| [google-gemini/gemini-cli](https://github.com/google-gemini/gemini-cli) | [youruser/google-gemini-cli](https://github.com/youruser/google-gemini-cli) | `$HOME/projects/agents/google/gemini-cli` |

`origin` on each clone points at the private repo; `upstream` is the public OpenAI/Google repository. Paths in the phase docs below use the `openai/codex/...` and `google/gemini-cli/...` prefixes relative to `$HOME/projects/agents/`.

Bare clones used for mirror maintenance (optional): `$APXM_HOME-cli/` (`openai-codex.git`, `gemini-cli.git`).

- **[Global Integration Plan](plan-global-integration.md)** — Overview, five-phase adoption path, architecture diagrams
- **[Phase 1: LLM Backend](phase-1-llm-backend/)** — Replace LLM transport with apxm-backends
- **[Phase 2: Tool Migration](phase-2-tool-migration/)** — Register consumer tools as APXM capabilities
- **[Phase 3: AAM State Model](phase-3-aam-state/)** — Map agent state to hierarchical (B, G, C)
- **[Phase 4: Agent Loop as Graph](phase-4-agent-loop/)** — Express turn loops as AIS graphs
- **[Phase 5: Compiler Integration](phase-5-compiler/)** — Compile, analyze, optimize (the punch line)

## Consumer Case Studies

### [`codex/`](codex/) — Codex-on-APXM

Reconstruct a Codex-class coding agent on A-PXM. This is the "LLVM-GCC" moment — proving the substrate can represent production agent execution.

- [case-study.md](codex/case-study.md) — The redundancy problem and LLVM-GCC parallel
- [plan.md](codex/plan.md) — 5-phase reconstruction plan
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
- **Advantages** (value): [`../advantages/`](../advantages/) — optimizations, LLVM parallel
