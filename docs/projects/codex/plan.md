# Codex-on-APXM Implementation Plan

Step-by-step plan to build a Codex-class coding agent on A-PXM. Frontends such as AgentMate are optional accelerators, not the architectural target.

## Phase 1: Single-Turn Agent

**Goal**: An agent that takes a coding task, calls the LLM with tools, and produces output.

**Components**:
- A graph authoring layer with `bash`, `read`, `write`, `search_web` tools
- Capability interceptors for policy-driven tool access (OS-level sandbox deferred -- P0 gap)
- Single `ASK` node with tool iteration

**Deliverable**: An APXM-backed coder entry point that can take a concrete repo task and execute a Codex-like single-turn loop.

**Validates**: Basic inference loop + tool dispatch work end-to-end.

## Phase 2: Multi-Turn with Memory

**Goal**: Agent maintains context across turns and persists knowledge.

**Components**:
- `QMEM` at session start to load project context from LTM
- `UMEM` at session end to save discoveries and decisions
- Episodic memory for conversation history
- STM for working context within a session

**Deliverable**: Agent remembers project structure, past fixes, and user preferences across sessions.

**Validates**: Three-tier memory hierarchy is sufficient for coding agent state.

## Phase 3: Multi-Step Workflows

**Goal**: Agent follows structured workflows — read code, propose changes, apply edits, run tests, iterate.

**Components**:
- AIS graph with `ASK` → `INV` → `VERIFY` → `BRANCH` pipeline
- Compiler optimization (fuse sequential ASKs, extract parallelism from independent INVs)
- `REFLECT` for self-evaluation after test failures

**Deliverable**: Agent handles complex multi-file changes with test-driven iteration.

**Validates**: AIS graphs can represent real multi-step coding workflows; compiler optimizations provide measurable value.

## Phase 4: Multi-Agent Coordination

**Goal**: Specialized sub-agents for different tasks (code analysis, test writing, documentation).

**Components**:
- `FLOW_CALL` to invoke sub-agent workflows
- `Communicate` for inter-agent message passing (local, HTTP, and broadcast protocols)
- AAM scoping to isolate agent state (basic snapshot-scoped child execution is implemented; file-backed hierarchical workspace projection remains open)
- `PLAN` for dynamic task decomposition

**Deliverable**: Architecture agent decomposes task → specialist agents execute in parallel → coordinator merges results.

**Validates**: A-PXM's multi-agent coordination primitives scale to real workloads.

## Phase 5: Optimization Integration

**Goal**: Demonstrate that A-PXM's shared optimizations provide measurable value.

**Components**:
- Prompt caching (shared prefixes across `ASK` nodes)
- Response memoization (cache deterministic tool results)
- Model routing (cheap model for classification, expensive for reasoning)
- Token budget enforcement with automatic compaction

**Deliverable**: Measured reduction in latency, token usage, and API cost vs. un-optimized baseline.

**Validates**: The "LLVM advantage" — optimizations that benefit all agents on the platform.

## Success Criteria

| Metric | Target | Hypothesis |
|--------|--------|------------|
| SWE-Bench Lite resolution rate | ≥ 30% (competitive with Codex OSS) | -- (external benchmark) |
| Token usage vs. raw Codex | ≤ 80% (20% reduction from optimizations) | H1 (fusion ≥15%), H3 (prompt caching ≥50% input token reduction) |
| Latency vs. raw Codex | ≤ 90% (10% reduction from parallelism) | H2 (≥2x speedup on N≥3 branches) |
| Lines of agent-specific code | ≤ 500 (everything else is shared substrate) | H7 (≤10% of from-scratch code) |

See `advantages/hypotheses.md` for full measurement plans and current status of each hypothesis.

## Dependencies

- **Requires from A-PXM** (cross-ref: `implementation/TODO.md` P0 items):
  - Streaming -- `LLMBackend` trait needs `generate_stream` method (currently only `generate()` returning complete response)
  - Parallel tool dispatch within ASK tool loop -- currently sequential `for` loop over tool calls in `llm.rs`
  - Configurable `MAX_TOOL_ITERATIONS` -- hardcoded at 10, Codex-class agents need 25+
  - Messages as structured arrays -- `LLMRequest` uses single `prompt: String`, needs `messages: Vec<Message>` with roles
- **Optional reusable pieces from AgentMate or other frontends**:
  - OS-level sandbox (Seatbelt/Landlock)
  - Streaming TUI integration
  - Codebase indexing
- **Requires from both**:
  - AAM checkpoint/restore -- partially implemented (`AamCheckpoint` covers beliefs + goals but not capabilities)
  - Async user approval flow for interceptors -- `InterceptDecision` exists but no interactive approval channel (P1 gap)

Cross-references: These gaps align with hypotheses H7 (shared infrastructure, `advantages/hypotheses.md`) and the P0 substrate gaps in `implementation/TODO.md`.
