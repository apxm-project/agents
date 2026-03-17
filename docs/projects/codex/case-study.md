---
title: "Case Study: Reconstructing Codex on A-PXM"
description: "How a production coding agent maps onto A-PXM primitives -- and why this matters for every agent, not just Codex."
---

# Case Study: Reconstructing Codex on A-PXM

## The Redundancy Problem

Every coding agent ships its own runtime. Codex has a custom scheduler, custom memory system, custom tool dispatch, and custom sandbox. Claude Code has a different scheduler, different memory, different tools, different sandbox. Aider, Cursor, and every new entrant build the same infrastructure from scratch.

The result is thousands of engineering hours duplicated across projects, each solving the same fundamental problems: how to loop over LLM calls, how to dispatch tools safely, how to manage token budgets, how to persist session state, how to coordinate multiple agents.

This is the compiler world before LLVM. Every programming language built its own code generator. Every optimizer was written from zero. A breakthrough in register allocation benefited one compiler and no others. LLVM changed this by providing a shared substrate: emit LLVM IR, and you get optimization, code generation, and multi-target support for free. An improvement to LLVM benefits every language that targets it.

A-PXM aims to do the same for AI agents.

## What All Coding Agents Need

Despite radically different architectures, Codex, Claude Code, Aider, and Cursor all require the same runtime primitives:

| Primitive | What It Does |
|-----------|-------------|
| **Inference loop** | Call an LLM, process its response, call it again with new context |
| **Tool dispatch** | Execute file operations, shell commands, search, and other capabilities |
| **Context management** | Track token usage, enforce budgets, compact history when it grows too large |
| **Memory** | Maintain session state (what happened this conversation) and persistent knowledge (what the project looks like) |
| **Sandboxing** | Isolate tool execution so a shell command cannot damage the host system |
| **Multi-agent coordination** | Spawn sub-agents, divide work, aggregate results |
| **Streaming output** | Deliver tokens to the user as they arrive, not after the full response completes |

The agents differ in **how they compose** these primitives, not in **what primitives they need**. Codex dispatches tools in parallel with `FuturesOrdered`. Aider parses structured edits from LLM text output instead of using tool calls. Cursor indexes the codebase with embeddings and vector search. But underneath, all of them are doing inference, tool dispatch, context management, memory, and output streaming.

This is exactly the LLVM pattern. C++, Rust, and Swift have radically different semantics, but they all need SSA values, basic blocks, memory operations, and function calls.

## How A-PXM Provides the Substrate

A-PXM's three components -- the Agent Instruction Set (AIS), the Agent Abstract Machine (AAM), and the dataflow runtime -- map directly onto the primitives that every coding agent needs.

### AIS Operations as Agent Primitives

Each primitive that coding agents require corresponds to one or more AIS operations:

| Agent Primitive | AIS Operation | What It Provides |
|-----------------|---------------|-----------------|
| Inference loop | `ASK`, `THINK`, `REASON` | Typed LLM calls with latency budgets and built-in tool iteration |
| Tool dispatch | `INV` | Capability invocation with typed parameters and interceptor pipeline |
| Context management | Token budget attributes on `ASK` | Compiler-visible token constraints that the scheduler enforces |
| Memory read/write | `QMEM`, `UMEM` | Three-tier memory access (STM for session, LTM for project knowledge, Episodic for history) |
| Multi-agent | `FLOW_CALL`, `COMM` | Typed inter-agent communication with scoped state isolation |
| Synchronization | `WAIT_ALL`, `MERGE`, `FENCE` | Dataflow barriers that the scheduler resolves automatically |
| Control flow | `BRANCH`, `SWITCH` | Conditional routing without opaque if/else in Python |

### The Compiler Advantage

Because AIS graphs are data -- not opaque code -- the compiler can analyze and optimize them before execution:

- **Operation fusion**: Two sequential `ASK` nodes with no intervening tool call can be fused into a single LLM call, reducing latency and cost.
- **Dead operation elimination**: An `INV` node whose output is never consumed by any downstream node can be removed entirely.
- **Parallelism extraction**: Two `INV` nodes with no data dependency between them can be scheduled concurrently -- without the developer writing any async/await code.
- **Compile-time verification**: Type mismatches, missing dependencies, and unreachable nodes are caught before any LLM call is made.

No existing coding agent has this. Codex discovers parallelism opportunities manually through per-tool concurrency metadata. A-PXM discovers them automatically from the graph structure.

### The AAM as Formal State

The Agent Abstract Machine provides what no coding agent currently has: a formal model of agent state. At any point during execution, the AAM captures:

- **Beliefs**: What the agent knows (file contents, test results, user preferences)
- **Goals**: What the agent is trying to achieve (fix the bug, implement the feature)
- **Capabilities**: What the agent can do (read files, run shell commands, call sub-agents)

Every AIS instruction is a deterministic state transition on the AAM. This means every step of the agent's execution is auditable -- not through log files and stack traces, but through a formal transition history that can be replayed, inspected, and verified.

## Codex Through the A-PXM Lens

Codex is a natural candidate for reconstruction on A-PXM because its architecture already resembles a dataflow graph, even though it is implemented as imperative Rust code.

### The Agent Loop

Codex's core loop, `run_turn`, does four things: build a prompt from conversation history, call the LLM, extract tool calls from the response, execute those tools, and loop. In A-PXM, this is a single `ASK` node with tool capabilities registered in the AAM. The dataflow scheduler handles the loop mechanics. The `ASK` handler calls the LLM, dispatches tools through the capability system, and iterates until the model stops requesting tools.

### Parallel Tool Dispatch

When Codex's LLM response contains multiple tool calls, Codex dispatches them concurrently using `FuturesOrdered` with per-tool read/write locks. In A-PXM's dataflow model, independent tool invocations are naturally parallel -- the scheduler sees that two `INV` nodes share no data edge and runs them concurrently. No manual concurrency control needed.

### Session State

Codex maintains session state in a `SessionState` struct backed by SQLite, with flat in-memory "memories" for the current session. A-PXM's memory hierarchy provides a more structured equivalent:

| Codex Concept | A-PXM Equivalent |
|---------------|-------------------|
| Working context (current turn) | STM -- per-execution scratch space |
| Session memories | Episodic memory -- append-only event log |
| Persistent memories | LTM -- cross-session knowledge store |
| System prompt / instructions | AAM Beliefs -- typed key-value state |

### Approval and Sandboxing

Codex's `ApprovalStore` and platform sandbox (Seatbelt on macOS, Landlock on Linux) map to A-PXM's capability interceptor pipeline. Interceptors inspect every capability invocation before execution and can allow, deny, or modify the call. This is the same pattern, but formalized: the interceptor decisions become part of the execution trace, making approval logic auditable.

### Multi-Step Workflows

A single Codex turn is a single `ASK` node. But real Codex usage involves multi-turn sessions where the agent builds context, proposes changes, runs tests, and iterates. These multi-step workflows map to multi-node AIS graphs where the compiler can analyze the full workflow structure:

```
[Read files] --> [Propose changes] --> [Apply edits] --> [Run tests]
                                                              |
                                                              v
                                                     [Fix failures] --> [Run tests again]
```

Each node is a typed AIS operation. The compiler sees the dependencies. The scheduler extracts parallelism where the graph allows it. And if two nodes perform redundant work (reading the same file twice), the compiler can eliminate the duplication.

## What A-PXM Adds

Reconstructing Codex on A-PXM is not about replacing Codex's functionality. Everything Codex does today continues to work. The value is in what A-PXM provides on top:

**Compile-time verification.** Codex discovers malformed tool calls at runtime, after the LLM has already been invoked. A-PXM validates the workflow graph before execution -- type mismatches, missing dependencies, and unreachable operations are caught at compile time.

**Automatic parallelism.** Codex's parallel tool dispatch requires manual concurrency annotations (which tools are safe to run concurrently). A-PXM infers parallelism from the graph structure. No annotations needed.

**Operation fusion.** When the LLM produces a plan with sequential steps that could be combined, the compiler can fuse them into fewer, cheaper calls. This is invisible to the agent developer -- it happens automatically during compilation.

**Formal execution traces.** Every state transition, every tool call, every LLM invocation is recorded as a typed event in the AAM transition history. This is not logging -- it is a formal trace that supports replay, root cause analysis, and compliance auditing.

**Shared infrastructure.** This is the key point. Every improvement to A-PXM's runtime -- a better caching strategy, a smarter eviction policy, a new LLM backend, a more efficient scheduling algorithm -- automatically benefits every agent built on the substrate. An optimization written for Codex-on-APXM benefits Claude-Code-on-APXM and Aider-on-APXM and every future agent that targets the platform.

## The LLVM-GCC Parallel

LLVM-GCC was a shim. It took GCC's existing C frontend and modified it to emit LLVM IR instead of GCC's internal representation. It was not elegant. It was not the long-term solution. But it proved something essential: LLVM IR could represent real, production C programs. Once that was established, Clang could be built as a clean, purpose-designed frontend.

Codex-on-APXM serves the same role. It is not the end goal. The end goal is a substrate where any agent can be expressed as an AIS graph, compiled, optimized, and executed on the shared runtime. Codex is the proof that the substrate works -- that AIS graphs can represent the full complexity of a production coding agent's execution.

The sequence mirrors LLVM's adoption history:

| LLVM Milestone | A-PXM Equivalent |
|---------------|-------------------|
| LLVM-GCC proves IR can represent C | Codex-on-APXM proves AIS can represent agent execution |
| Clang built as native frontend | Purpose-built agent SDKs targeting A-PXM |
| Rust chooses LLVM as backend | Second agent adopts A-PXM (proves generality) |
| Community contributes optimization passes | Community improves shared runtime (benefits all agents) |

The first step is always the hardest: prove the IR works. Everything after that is leverage.

## The Broader Implication

The coding agent space is young enough that the infrastructure patterns have not yet consolidated. Every team is still building custom runtimes because there is no shared substrate worth targeting. A-PXM's thesis is that this consolidation is inevitable -- the same way compiler backends consolidated around LLVM -- and that the right time to build the substrate is before the ecosystem calcifies around incompatible ad-hoc solutions.

Codex-on-APXM is the first test of that thesis. If a 150,000-line production agent can be faithfully represented as AIS graphs running on A-PXM's dataflow scheduler, then the substrate is real. And if the substrate is real, then every future agent gets compilation, optimization, formal verification, and shared infrastructure for free.

That is the value proposition: stop rebuilding runtime infrastructure. Build on the shared substrate.
