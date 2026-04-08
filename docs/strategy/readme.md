# Unified Agent Execution Strategy

> **Status**: Several strategy documents below describe completed work. See the [master plan](master-plan.md) for current project status and validation methodology.

**Date**: March 31, 2026 (updated April 8, 2026)
**Scope**: APXM + vLLM (ACPX absorbed into APXM)

---

## Documents

| Document | Purpose |
|----------|---------|
| [Vision](vision.md) | The big picture: what we are building and why |
| [Opportunities](opportunities.md) | Top 5 optimization opportunities with effort/impact analysis |
| [Master Plan](master-plan.md) | Validation, timeline, and next steps (supersedes original phased plan) |
| [Architecture Integration](architecture-integration.md) | Code-level integration map: files, APIs, data flows |
| [Patent Alignment](patent-alignment.md) | Maps internal patent techniques to implementation locations |
| [Gap Analysis](gap-analysis.md) | Current vs target state: 15 gaps with priorities |
| [ACPX Absorption](acpx-absorption.md) | Why ACPX disappears into APXM (revised architecture) |
| [Optimization Targets](optimization-targets.md) | Goal-directed compiler targets: -O(tokens), -O(parallel), -O(latency), -O(cost) |
| [vLLM Graph Awareness](vllm-graph-awareness.md) | Making vLLM pipeline-aware: KV-cache pinning, priority scheduling, token pipelining |
| [DSPy Integration](dspy-integration.md) | Prompt optimization as compiler pass via DSPy/MIPROv2 |
| [Parity Matrix](parity-matrix.md) | Frontend API coverage against example inventory |

---

## Key Architectural Decision: ACPX Is Absorbed

ACPX does not exist as a separate orchestration layer. APXM already has DELEGATE, COMMUNICATE, SPAWN_AGENT, INV, EXC, PAUSE/RESUME, 3-tier memory, parallel scheduling, and MLIR optimization. The only thing ACPX adds is an ACP protocol client (~800 lines of Rust as a `CapabilityExecutor`).

**Two tiers of LLM work in one graph:**
- **Direct LLM** (ASK/THINK/REASON): Fast, cheap -- analysis, reasoning, generation
- **Full Agent** (INV with ACP capability): Powerful, slower -- multi-step coding with file editing, tests, iteration

---

## One-Paragraph Summary

An OpenClaw agent receives a natural language request and generates an APXM graph where simple reasoning nodes use direct LLM calls (ASK/THINK/REASON via vLLM) and complex coding tasks dispatch to full agents (Claude Code, Codex, Gemini via ACP). The APXM compiler optimizes the graph with MLIR passes -- model affinity, parallelism extraction, speculation insertion, context budget analysis -- guided by **goal-directed optimization targets** (`--target tokens/parallel/latency/cost`) that tell the compiler *what* to optimize for, not just *how much*. The runtime executes on a parallel dataflow scheduler with dynamic model routing (gpt-5.4, gpt-5.4-nano, local models selected by health/cost/latency), spaghetti-stack context management (34% of flat context), memoization (microsecond cached returns), and speculative execution. The vLLM backend receives graph metadata, pins KV-caches for downstream nodes, and prioritizes critical-path operations. No separate orchestration layer needed -- APXM is the orchestrator, compiler, and runtime.

---

## Quick Reference

### Priority Order

```
P0: Model Router (Weeks 1-3) + ACP Client in APXM (Weeks 4-6)
P1: Graph-Aware vLLM (Weeks 7-11) + Context Stack (parallel)
P2: Patent Implementation + Optimization Targets (Weeks 12-16)
P3: Graph Generation from NL + Feedback Loop (future)
```

### Key Numbers

| Metric | Current | Target | Source |
|--------|---------|--------|--------|
| Multi-agent speedup | 1x (sequential in ACPX) | 10.37x (parallel in APXM) | APXM benchmark |
| Context per stage | 100% (flat) | 34% | Patent figure |
| Memoized response | N/A | ~1 microsecond | Two-tier cache |
| Model failover | Manual / none | Automatic | Circuit breaker |
| Pipeline overlap | 0% | 30-50% | Token pipelining |
| New code for ACP | 0 | ~800 lines Rust | CapabilityExecutor |
| New compiler passes | 7 implemented | 13 total (+6 new) | Goal-directed targets |
| Optimization targets | 1 (level only) | 5 (tokens, parallel, latency, cost, balanced) | --target flag |
| Code eliminated | 0 | ~30K lines TypeScript | ACPX orchestration |

### Systems

| System | Role | Key Extension |
|--------|------|--------------|
| **APXM** | Compiler + Runtime + Orchestrator | ModelRouter, AcpCapability, ContextStack, MemoCache |
| **vLLM** | Model Serving | GraphAwareScheduler, KV-cache pinning |
| **OpenClaw** | Trigger Layer | Graph generation agent |
| ~~ACPX~~ | ~~Separate orchestrator~~ | Absorbed as AcpCapability inside APXM |

---

> See also: [PXM theory](../pxm/) for formal foundations, [design](../design/) for architecture decisions, [implementation](../implementation/) for code-level details, [optimization](../optimization/) for compiler pass documentation.
