---
title: "PXM Foundations"
description: "Why agent workflows need a formal execution model, and how A-PXM separates Compute, Memory, State, Optimization, and Scheduling -- drawing on five decades of program execution model research."
---

# PXM Foundations

## From Von Neumann to Agents

Computing history reveals a recurring pattern. A new computational paradigm emerges, practitioners build ad-hoc solutions that work well enough at small scale, and then the paradigm hits an opacity wall -- the runtime cannot see what the program is doing, so it cannot optimize, verify, or schedule it. The breakthrough comes when someone defines a **formal execution model**: a contract between programs and machines that makes computation visible to tooling. After that, a compiler+runtime ecosystem follows, and the paradigm scales.

Von Neumann formalized sequential computation (1945): one program counter, one memory, deterministic fetch-decode-execute. That model enabled assemblers, linkers, and eventually the entire systems programming stack. Dataflow formalized parallel computation (Manchester, 1985; MIT Tagged-Token): operations fire when operands arrive, and the graph structure itself is the scheduling specification. CUDA formalized GPU computation (2007): a hierarchy of threads, blocks, and grids mapped onto SIMT hardware, giving programmers a portable abstraction over massively parallel devices. In every case, the formal model was what unlocked the compiler and runtime infrastructure -- without it, optimization was guesswork and correctness was hope.

Agentic AI is at the ad-hoc stage now. Workflows are opaque Python scripts. The runtime cannot see dependencies, cannot verify correctness, cannot optimize cost. A-PXM is the formal execution model for agent computation -- the contract that makes agent workflows visible to compilers, runtimes, and auditing tools, just as prior models did for their respective paradigms.

## The Problem: Opaque Agent Workflows

In 1977, John Backus described the **von Neumann bottleneck**: conventional programming languages force programs through a narrow sequential channel, hiding structure from tools and making optimization impossible. The problem was not just sequentiality -- it was **opacity**. The runtime could not see what the program was doing, so it could not help.

Agentic AI frameworks today suffer from an analogous pathology. We call it the **agentic von Neumann bottleneck**: workflows execute as opaque Python scripts -- "call-at-a-time" chains where every operation is a black-box function call. The runtime has no visibility into what operations exist, how they relate to each other, or what state they touch.

Put more bluntly: most agent development today operates at the **wrong abstraction layer**. Developers wire together API calls, prompt templates, and tool invocations inside imperative code -- the same level of abstraction as the LLM calls themselves. This insight aligns with the file-tree architecture argument (Quantum Quill Lyceum, 2025) -- that agent state should be organized as hierarchical directories navigable by both humans and AI. When a workflow is a folder, a task is a sub-folder, and components are files (prompts, tools, data), the architecture becomes modular and future-proof: if a big model provider ships a capability that replaces an entire workflow, you condense that folder into a single tool definition and the rest of the system is unaffected. The problem is not that current tools are bad; it is that the abstraction layer they occupy conflates execution structure with implementation detail, making optimization, composition, and adaptation impossible. A-PXM addresses this by lifting the abstraction to a formal execution model where the structure is visible to tooling.

```python
# Current frameworks: every step is an opaque function call
context = retrieve(query)                 # What does this read? Write?
analysis = llm.reason(context)            # What state does this depend on?
result_a = tool_a(analysis)              # Is this independent of tool_b?
result_b = tool_b(analysis)              # The runtime cannot tell.
summary = llm.ask(result_a, result_b)    # Implicit dependencies everywhere.
```

### Three Consequences of Opacity

**1. No optimization is possible.** The runtime cannot fuse redundant LLM calls, eliminate dead operations, or schedule independent work concurrently. Every optimization that LLVM does for machine code -- CSE, DCE, fusion, reordering -- is impossible when the program is opaque Python.

**2. No verification is possible.** Type errors, missing dependencies, unreachable operations, and malformed state transitions only surface at runtime -- often after expensive LLM calls have already been made. There is no compile-time analysis because there is nothing to compile.

**3. No auditing is possible.** When an agent produces a wrong answer, tracing the root cause requires reading Python stack traces and log files. There is no formal execution trace, no state transition history, no way to replay or inspect the agent's reasoning path.

### The Outer Plan / Inner Plan Disconnect

Current systems have two disconnected levels of planning:

| Level | Owner | Representation | Visibility |
|-------|-------|----------------|------------|
| **Outer plan** | Developer | Python code, YAML config | Framework runtime (opaque) |
| **Inner plan** | LLM | Chain-of-thought, tool-use decisions | Model inference (invisible) |

The outer plan cannot constrain the inner plan. The inner plan cannot inform scheduling. There is no shared contract between these levels.

## What We Need

Solving the agentic von Neumann bottleneck requires making agent workflows **visible** to tooling:

1. **Explicit state**: a formal model of what the agent knows, wants, and can do -- not implicit Python variables scattered across closures.
2. **Explicit dependencies**: data edges declaring which operations depend on which results, enabling both analysis and automatic scheduling.
3. **Explicit effects**: side effects (LLM calls, tool invocations, memory writes) as first-class operations with typed inputs and outputs, not opaque function calls.

A-PXM provides all three through the Agent Abstract Machine (state), the Agent Instruction Set (typed operations), and dataflow execution (dependency-driven scheduling).

---

## A-PXM: A Program Execution Model

A-PXM is not a framework, not just a compiler, and not just a runtime. It is a **Program Execution Model** -- a formal specification of how agent programs are represented, optimized, and executed. It is composed of a compiler (compile-time half) and a runtime (execution-time half), working together as a unified system.

The key insight is that A-PXM defines the **ISA contract** for agent computation. In hardware, the ISA (Instruction Set Architecture) is the stable boundary between software and silicon: from the software side, any number of languages and compilers can target it; from the hardware side, any number of microarchitectures can implement it. LLVM IR achieved the same decoupling for compilers -- any frontend (C, Rust, Swift) emits LLVM IR, and any backend (x86, ARM, RISC-V) consumes it. The IR is the contract that lets both sides evolve independently.

A-PXM's Agent Instruction Set (AIS) is that contract for agents. From the software side, any framework, DSL, or API can emit AIS graphs. From the backend side, any runtime -- local, distributed, cloud-native -- can execute them. The contract is what enables the ecosystem:

```
Source Language    Frontend         IR          Optimizer       Backend
─────────────    ─────────         ──          ─────────       ───────
C                Clang             LLVM IR     LLVM passes     x86/ARM
Rust             rustc             LLVM IR     LLVM passes     x86/ARM

Python API       agentmate-py      ApxmGraph   AIS passes      APXM Runtime
Rust API         agentmate-rs      ApxmGraph   AIS passes      APXM Runtime
AIS DSL          AIS parser        ApxmGraph   AIS passes      APXM Runtime
```

Multiple frontends emit a common IR. The compiler optimizes it. The runtime executes it. Framework authors focus on developer experience; the execution model handles correctness, optimization, and scheduling. No frontend needs to know about scheduling. No runtime needs to know about syntax. The AIS contract separates these concerns permanently.

### The Agentic Software Stack

```
┌─────────────────────────────────────────────────────────┐
│  Application Layer                                      │
│  CrewAI  |  LangGraph  |  AgentMate  |  Custom          │
├─────────────────────────────────────────────────────────┤
│  A-PXM Layer                                            │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  │
│  │  AIS IR       │→│  Compiler    │→│  Runtime      │  │
│  │  (typed ops)  │  │  (MLIR)      │  │  (dataflow)  │  │
│  └──────────────┘  └──────────────┘  └──────────────┘  │
├─────────────────────────────────────────────────────────┤
│  Foundation Layer                                       │
│  LLM APIs  |  Tool APIs  |  Memory Stores               │
└─────────────────────────────────────────────────────────┘
```

### A Concrete Example

You want 5 expert agents to analyze a business proposal:

```ais
agent ProposalReview {
    @entry flow main(proposal: str) -> str {
        ask("Summarize: " + proposal) -> context

        // 5 independent analyses -- written sequentially, executed based on dataflow
        ask(backend: "claude", prompt: "Financial analysis: " + context) -> financial
        ask(backend: "claude", prompt: "Legal review: " + context) -> legal
        ask(backend: "claude", prompt: "Technical assessment: " + context) -> technical
        ask(backend: "claude", prompt: "Market fit: " + context) -> market
        ask(backend: "claude", prompt: "Risk assessment: " + context) -> risk

        think(
            prompt: "Synthesize these analyses:\n"
                    + "Financial: " + financial + "\n"
                    + "Legal: " + legal + "\n"
                    + "Technical: " + technical + "\n"
                    + "Market: " + market + "\n"
                    + "Risk: " + risk,
            budget_tokens: 2000
        ) -> synthesis

        return synthesis
    }
}
```

The developer writes sequential-looking code. The compiler sees a dataflow graph:

```
context ──┬── financial ──┐
          ├── legal       ├── synthesis
          ├── technical   │
          ├── market      │
          └── risk ──────┘
```

The five analyses depend only on `context`, not on each other. The compiler can verify this statically. The runtime can schedule them based on available resources. The execution trace records every state transition. If the synthesis hallucinates, the full provenance chain is available for audit.

This is not primarily about speed. It is about **making the workflow visible** -- to the compiler for optimization, to the runtime for scheduling, to the developer for debugging, and to the organization for auditing.

---

## The Five Separations

A-PXM achieves visibility by cleanly separating five concerns that current frameworks entangle. Each separation has deep roots in program execution model research.

### 1. Compute

**What operations run.**

| Model | How Compute Is Defined | Limitation |
|-------|----------------------|------------|
| Von Neumann | Instructions fetched by program counter | Sequential; independent ops cannot overlap |
| Dataflow (Manchester, MIT Tagged-Token) | Operations fire when all operands arrive | No program counter; automatic concurrency. Too fine-grained for general programs |
| LLVM IR / SSA | Typed operations, each value defined once | Enables optimization but still sequentially scheduled |
| Actor Model (Erlang, Akka) | Message-driven actors, no shared state | Untyped messages; no compile-time analysis |

In A-PXM, compute = **typed AIS operations** (ASK, THINK, REASON, INV, PLAN, REFLECT, VERIFY, ...). Each is a node in a dataflow graph with typed inputs/outputs, explicit data edges, and latency annotations. The developer declares *what* to compute; the system decides *when* and *where*.

**Unlike von Neumann**: operations are never artificially sequenced.
**Unlike actors**: operations have typed signatures and explicit dependency edges.

### 2. Memory

**Where knowledge lives.**

| Model | Memory Organization | Limitation |
|-------|-------------------|------------|
| Von Neumann | Flat address space; cache as optimization | No semantic structure |
| Dataflow | Tokens carry data; no addressable memory | No persistent state |
| Actor Model | Private state per actor | Cross-actor sharing is hard |
| GPU/CUDA | Registers, shared, global, constant | Programmer must manage manually |

A-PXM provides a **three-tier memory hierarchy** matching how agents use context:

| Tier | Purpose | Backing | Access | Analogy |
|------|---------|---------|--------|---------|
| **STM** | Working memory (current session) | In-memory DashMap | ~us | L1 cache |
| **LTM** | Persistent knowledge | SQLite + FTS5 + vectors | ~ms | Main memory |
| **Episodic** | Execution history (for reflection) | Append-only log | ~ms | Disk archive |

Memory access is through first-class AIS instructions: `QMEM` (read), `UMEM` (write), `FENCE` (barrier). Tiers have semantic meaning, not just speed differences.

### 3. State

**What the agent knows, wants, and can do.**

| Model | What Is "State" | Limitation |
|-------|----------------|------------|
| Von Neumann | PC + registers + memory | Implicit, mutable everywhere |
| BDI (Beliefs-Desires-Intentions) | B, D, I decomposition | Conceptual; not machine-executable |
| FSM | Explicit states and transitions | Too rigid for dynamic agents |

The **Agent Abstract Machine (AAM)** formalizes state as:

```
AAM = (B, G, C)
  B: Beliefs      -- Map<Key, TypedValue>   -- what the agent knows
  G: Goals        -- PriorityQueue<Goal>    -- what it's trying to achieve
  C: Capabilities -- Map<Name, Signature>   -- what it can do
```

Every AIS instruction is a deterministic state transition: `δ(AAM, Instr) → AAM'`. State is typed, inspectable, and compiler-verifiable -- unlike BDI which remains conceptual, or actors where state is opaque.

### 4. Optimization

**Making workflows faster and cheaper.**

| Model | Optimization Approach | Key Passes |
|-------|---------------------|------------|
| LLVM/GCC | SSA-based pass pipeline | CSE, DCE, inlining, constant folding |
| XLA/TVM/Triton | Graph-level ML optimization | Operator fusion, memory planning |
| JIT (JVM, V8) | Profile-guided speculation | Hot path optimization |

A-PXM uses **MLIR** as its compiler infrastructure:

| Pass | What It Does | Impact |
|------|-------------|--------|
| **FuseAskOps** | Merges producer-consumer ASK chains into one API call | Fewer API calls, lower cost |
| **CSE** | Eliminates duplicate LLM calls with identical inputs | Saves $ and latency |
| **DCE** | Removes operations whose outputs are never consumed | Leaner graphs |
| **Canonicalization** | Normalizes graph patterns | Enables further passes |

**Why this matters more than traditional compilation:** In traditional compilers, optimizing away one instruction saves nanoseconds. In A-PXM, optimizing away one LLM call saves **seconds and dollars**. The economic return on agent-level optimization is orders of magnitude higher.

### 5. Scheduling

**When and where operations execute.**

| Model | How Execution Order Is Determined | Parallelism Discovery |
|-------|----------------------------------|----------------------|
| Von Neumann | Program counter (sequential) | None |
| Dataflow (Manchester) | Operations fire when all tokens arrive | Automatic from graph |
| Task-Based (Cilk, Tokio) | Work-stealing over task DAGs | Explicit via spawn/async |
| Petri Nets | Transitions fire when input places have tokens | Automatic from net |

A-PXM's scheduler is a **token-counting dataflow machine**:

1. Each operation has a `pending` counter = number of input edges
2. When a predecessor completes, counter decrements
3. Counter reaches zero -> operation fires
4. **O(1) readiness detection** -- no graph traversal

The DAG structure IS the scheduling specification. No `async`, no `await`, no `Promise.all`. The scheduler also provides work stealing, priority-based dispatch, and per-session lane guards.

---

## Summary

```
┌──────────────────────────────────────────────────────────────────┐
│                        A-PXM Stack                               │
├──────────────────────────────────────────────────────────────────┤
│  COMPUTE        AIS operations (ASK, THINK, REASON, INV, ...)   │
│                 Typed nodes in a dataflow graph                   │
│                                                                   │
│  MEMORY         Three-tier hierarchy (STM / LTM / Episodic)     │
│                 First-class QMEM/UMEM/FENCE instructions         │
│                                                                   │
│  STATE          AAM = (Beliefs, Goals, Capabilities)             │
│                 Typed, inspectable, compiler-verifiable           │
│                                                                   │
│  OPTIMIZATION   MLIR-based pass pipeline                         │
│                 FuseAskOps, CSE, DCE, Canonicalization           │
│                                                                   │
│  SCHEDULING     Token-counting dataflow, O(1) readiness          │
│                 Dependency-driven execution                       │
└──────────────────────────────────────────────────────────────────┘
```

Each concern is isolated, typed, and independently evolvable. This separation is what makes agent workflows **visible** to tooling -- enabling the compiler to optimize, the runtime to schedule, the developer to debug, and the organization to audit.

---

## Further Reading

- [AAM: Agent Abstract Machine](aam.md) -- formal state model
- [Compute in PXMs](compute.md) -- deep dive on compute separation
- [Memory in PXMs](memory.md) -- deep dive on memory separation
- [Scheduling in PXMs](scheduling.md) -- deep dive on scheduling separation
- [Agent Instruction Set](ais.md) -- the typed operation taxonomy
- [Hierarchical AAM](../implementation/runtime/hierarchical-aam.md) -- scoped state for multi-agent systems (TODO)
- [History: From Von Neumann to Agents](history.md) -- how computing history repeats at the agentic scale

---

## References

### Compute and Architecture

1. J. von Neumann, "First Draft of a Report on the EDVAC," Moore School of Electrical Engineering, University of Pennsylvania, 1945. Reprinted in *IEEE Annals of the History of Computing*, vol. 15, no. 4, pp. 27-75, 1993. DOI: [10.1109/85.238389](https://doi.org/10.1109/85.238389)

2. J. Backus, "Can Programming Be Liberated from the von Neumann Style? A Functional Style and Its Algebra of Programs," *Communications of the ACM*, vol. 21, no. 8, pp. 613-641, 1978. DOI: [10.1145/359576.359579](https://doi.org/10.1145/359576.359579)

3. J. R. Gurd, C. C. Kirkham, and I. Watson, "The Manchester Prototype Dataflow Computer," *Communications of the ACM*, vol. 28, no. 1, pp. 34-52, 1985. DOI: [10.1145/2465.2468](https://doi.org/10.1145/2465.2468)

4. Arvind and D. E. Culler, "Dataflow Architectures," *Annual Reviews in Computer Science*, vol. 1, pp. 225-253, 1986. DOI: [10.1146/annurev.cs.01.060186.001301](https://doi.org/10.1146/annurev.cs.01.060186.001301)

### Compiler Infrastructure

5. C. Lattner and V. Adve, "LLVM: A Compilation Framework for Lifelong Program Analysis & Transformation," in *Proc. CGO '04*, pp. 75-86, IEEE, 2004. DOI: [10.1109/CGO.2004.1281665](https://doi.org/10.1109/CGO.2004.1281665)

6. R. Cytron, J. Ferrante, B. K. Rosen, M. N. Wegman, and F. K. Zadeck, "Efficiently Computing Static Single Assignment Form and the Control Dependence Graph," *ACM TOPLAS*, vol. 13, no. 4, pp. 451-490, 1991. DOI: [10.1145/115372.115320](https://doi.org/10.1145/115372.115320)

7. C. Lattner et al., "MLIR: Scaling Compiler Infrastructure for Domain Specific Computation," in *Proc. CGO '21*, pp. 2-14, IEEE, 2021. DOI: [10.1109/CGO51591.2021.9370308](https://doi.org/10.1109/CGO51591.2021.9370308)

### Concurrency and Scheduling

8. C. Hewitt, P. Bishop, and R. Steiger, "A Universal Modular ACTOR Formalism for Artificial Intelligence," in *Proc. IJCAI '73*, pp. 235-245, 1973.

9. R. D. Blumofe and C. E. Leiserson, "Scheduling Multithreaded Computations by Work Stealing," *JACM*, vol. 46, no. 5, pp. 720-748, 1999. DOI: [10.1145/324133.324234](https://doi.org/10.1145/324133.324234)

10. C. A. Petri, "Kommunikation mit Automaten," PhD thesis, Universitat Hamburg, 1962.

### Agent Models

11. A. S. Rao and M. P. Georgeff, "BDI Agents: From Theory to Practice," in *Proc. ICMAS '95*, pp. 312-319, AAAI Press, 1995.

12. G. R. Gao, R. Patel, and T. St. John, "The Codelet Program Execution Model," presented at *WiA, ISCA '13*, Tel-Aviv, Israel, 2013.

### Memory and Cognition

13. R. C. Atkinson and R. M. Shiffrin, "Human Memory: A Proposed System and Its Control Processes," in *The Psychology of Learning and Motivation*, vol. 2, pp. 89-195, Academic Press, 1968.

14. E. Tulving, "Episodic and Semantic Memory," in *Organization of Memory*, pp. 381-403, Academic Press, 1972.

### Parallel Function Calling

15. S. Kim et al., "An LLM Compiler for Parallel Function Calling," in *Proc. ICML '24*, 2024. arXiv: [2312.04511](https://arxiv.org/abs/2312.04511)

### A-PXM

16. APXM Contributors, "A-PXM: An Agent Program Execution Model," unpublished manuscript, 2025-2026.
