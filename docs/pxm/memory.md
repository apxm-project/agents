---
title: "Memory in Program Execution Models"
description: "Comparative analysis of how memory is separated and formalized across classical and modern PXMs, and how A-PXM's tiered memory hierarchy resolves the agentic memory problem."
status: "historical-v1-analysis"
---

# Memory in Program Execution Models

> **Historical design analysis — non-normative for target APXM.** Sections
> describing QMEM/UMEM, AAM memory tiers, FLOW_CALL, or COMM record the v1
> baseline. The target uses explicit Program Context/local values and admitted
> Capabilities as fixed by [ADR-0009](../adr/0009-air-has-five-public-semantic-operations.md)
> and [ADR-0010](../adr/0010-agent-program-source-owns-context-hooks-and-conversational-loops.md).

Memory is the silent axis of every program execution model (PXM). How a model organizes, isolates, and exposes memory determines what optimizations are possible, what concurrency is safe, and what abstractions the programmer can rely on. This document surveys memory formalization across six established PXMs, then shows how A-PXM introduces a purpose-built memory architecture for agentic AI.

> For implementation details, see [apxm-runtime](../../crates/runtime/engine/README.md).

---

## 1. Von Neumann: Flat Address Space

### Organization and Access

The von Neumann model treats memory as a single, flat, byte-addressable store. The CPU fetches instructions and data from the same linear address space through a shared bus. Modern implementations overlay a **cache hierarchy** (L1/L2/L3) to mask the latency gap, but this hierarchy is transparent to the program -- the ISA presents the illusion of uniform-latency memory.

### Isolation Guarantees

Minimal. Any pointer can alias any other pointer. The hardware enforces no semantic boundaries between data regions.

### Optimization Implications

The flat model enables arbitrary data structures but severely limits what the compiler can prove about aliasing. Cache behavior is implicit and unpredictable from the source program. Concurrency requires explicit synchronization because any two threads might access overlapping addresses.

For agentic systems, this model provides no mechanism to distinguish between working context, long-term knowledge, and execution history.

---

## 2. Dataflow Models: Tokens Replace Addresses

### Organization and Access

Pure dataflow architectures eliminate addressable memory entirely. Data moves as **tokens** along edges of a dataflow graph. An operation fires when all its input tokens are present, consumes them, and produces output tokens. There is no memory bus -- data flows directly from producer to consumer.

To reintroduce controlled state, dataflow research introduced:
- **I-structures** (Arvind & Thomas): write-once storage cells. Safe sharing without write-after-write hazards.
- **M-structures** (Barth, Nikhil): mutable cells with built-in synchronization. Single-reader semantics without external locks.

### Isolation Guarantees

Strong by construction. No shared address space means operations cannot interfere through aliased writes. Data races are structurally impossible in the pure model.

### Optimization Implications

The absence of a central memory bottleneck enables massive parallelism. The compiler can freely reorder, duplicate, or eliminate operations without worrying about side effects on shared state. However, stateful computation requires explicit I/M-structure allocation, and spatial locality is lost.

---

## 3. LLVM IR: Memory as a Typed Side Effect

### Organization and Access

LLVM IR models memory through typed instructions (`alloca`, `load`, `store`) mediated by typed pointers. LLVM's **MemorySSA** pass constructs an SSA-form representation of memory operations, lifting reads and writes into an explicit dependency graph. Each memory write creates a new "memory version," and each read is linked to the specific write it observes.

### Isolation Guarantees

Moderate. LLVM provides multiple alias analysis implementations (BasicAA, ScopedNoAliasAA, TBAA) that let the compiler prove independence for many access patterns.

### Optimization Implications

MemorySSA enables dead store elimination, load-store forwarding, and loop-invariant code motion. The key insight: LLVM does not change the von Neumann memory model at the hardware level, but it imposes enough structure at the IR level to recover optimization opportunities.

---

## 4. Actor Model: Private State, Message Passing

### Organization and Access

In the actor model (Hewitt, 1973), each actor encapsulates **private state** inaccessible to any other actor. Communication is exclusively through asynchronous message passing.

### Isolation Guarantees

Total. No actor can read or write another actor's state. Memory isolation is a structural invariant, not a convention.

### Optimization Implications

Location transparency (actors can run on any node), independent garbage collection, and mailbox optimization. The cost is communication overhead for patterns that are trivial with shared memory.

---

## 5. GPU/CUDA: Explicit Programmer-Managed Hierarchy

### Organization and Access

The CUDA execution model exposes a **multi-level memory hierarchy** that the programmer must manage explicitly:

| Memory Type | Scope | Latency | Lifetime |
|-------------|-------|---------|----------|
| **Registers** | Per-thread | ~1 cycle | Thread |
| **Shared memory** | Per-block | ~5 cycles | Block |
| **L1/L2 cache** | Per-SM / device | ~30-200 cycles | Managed |
| **Global memory** | All threads | ~400-800 cycles | Application |

### Isolation Guarantees

Hierarchical. Registers are private to a thread. Shared memory is private to a block. Global memory is accessible to all threads but requires explicit synchronization.

### Optimization Implications

Exposing memory hierarchy to the programmer enables extreme throughput (coalesced access, bank conflict avoidance, occupancy tuning) at the cost of programming complexity.

---

## 6. Functional Models (Haskell/ML): Immutable Values

### Organization and Access

Pure functional languages model computation over **immutable values**. There is no mutable state. Data structures are persistent -- "modifying" a list produces a new list sharing structure with the original. Controlled side effects (Haskell's `IO` and `ST` monads) encode effects in the type system.

### Isolation Guarantees

Strong in the pure subset. Immutable values eliminate aliasing hazards and data races.

### Optimization Implications

Immutability enables deforestation, sharing, parallel evaluation, and specialization. The cost is allocation pressure.

---

## A-PXM: Tiered Memory for Agentic AI

### The Problem with Existing Models

None of the models above address the memory requirements of agentic AI:

- **Von Neumann flat memory** treats all data uniformly -- agents need to distinguish between ephemeral working context, durable learned knowledge, and historical execution traces.
- **Dataflow tokens** carry data between operations but provide no persistent recall or cross-session continuity.
- **LLVM's typed memory** optimizes compiler transformations but operates far below agent-level semantics.
- **Actor private state** isolates agents but provides no structured memory model within an actor.
- **CUDA's explicit hierarchy** is optimized for throughput computing, not semantic knowledge management.
- **Functional immutability** eliminates mutation hazards but provides no model for an agent's evolving beliefs.

### Three-Tier Memory Hierarchy

A-PXM formalizes agent memory into three tiers, each with distinct semantics and access patterns. This hierarchy is a **semantic partition** of the agent's state, not an implementation optimization like CPU caches. Each tier answers a different question:

| Tier | Question | Semantics | Mutability |
|------|----------|-----------|------------|
| **STM** (Short-Term Memory) | "What am I working on right now?" | Working memory / session scratch -- intermediate results, recent tool output, session-scoped variables | Read-write, volatile |
| **LTM** (Long-Term Memory) | "What do I know?" | Persistent knowledge -- user preferences, cached facts, learned associations that survive across sessions | Read-write, durable |
| **Episodic** | "What have I done?" | Execution traces -- timestamped records of every operation, enabling self-reflection and auditing | Append-only, durable |

The design follows cognitive models of human memory (working, semantic, autobiographical), giving flow authors intuitive semantics when deciding where to store data.

### Memory Operations as First-Class AIS Instructions

Unlike frameworks where memory access is buried in library calls, A-PXM elevates memory operations to first-class instructions in the [Agent Instruction Set](ais.md):

| Instruction | Signature | Semantics |
|-------------|-----------|-----------|
| `QMEM` | `(q: String, sid: SessionID, k: Int) -> Value` | Query memory; dispatches to a single tier (tiered fallthrough is aspirational) |
| `UMEM` | `(data: Value, sid: SessionID) -> Void` | Write to memory; optionally durable (write-through to LTM) |
| `FENCE` | `() -> Void` | Memory barrier: flush pending writes, enforce ordering |

These are **nodes in the dataflow DAG**, not opaque side effects. The compiler can see every memory read and write, reason about their dependencies, and apply transformations:

- **Prove independence**: two QMEM operations on different keys can execute in parallel without a FENCE.
- **Eliminate dead stores**: a UMEM whose key is never subsequently read by any QMEM is dead code.
- **Fuse read-modify-write**: a QMEM-ASK-UMEM chain on the same key can be optimized into a single transactional operation.
- **Preserve memory ordering**: the compiler conservatively declines to merge operations when a UMEM or FENCE intervenes.

### Cross-Agent Memory Isolation

Agents do not share memory. Cross-agent data exchange uses the `COMM` and `FLOW_CALL` instructions, which are explicit message-passing operations in the DAG. This follows the actor model's isolation guarantee: an agent's memory tiers are private to that agent. A receiving agent can write received data into its own memory via UMEM, but the sender's memory is never directly accessible.

| Scenario | Guarantee |
|----------|-----------|
| QMEM after UMEM (same key, same subgraph) | Read sees write (data dependency edge) |
| QMEM after UMEM (different keys) | No guarantee without FENCE |
| UMEM after UMEM (same key) | Last-writer-wins within subgraph ordering |
| Cross-agent memory access | Requires COMM protocol; no shared memory |

### Memory Lifecycle

1. **Execution starts**: STM is initialized empty. LTM and Episodic are opened from their on-disk backing stores (or created if first execution).
2. **During execution**: operations issue QMEM and UMEM as DAG nodes fire. The runtime automatically appends episodic entries at operation boundaries.
3. **Execution ends**: STM is dropped (volatile by design). LTM and Episodic are flushed to disk and survive for future runs.

This lifecycle means STM serves as a session-scoped workspace -- agents do not accumulate unbounded working memory across sessions -- while LTM accumulates knowledge and Episodic accumulates history across the agent's entire lifetime.

### Comparative Summary

| Property | Von Neumann | Dataflow | LLVM IR | Actor | CUDA | Functional | **A-PXM** |
|----------|-------------|----------|---------|-------|------|------------|----------|
| Addressing | Flat byte addresses | No addresses (tokens) | Typed pointers | Per-actor private | Hierarchical spaces | No addresses (values) | **Tier + key** |
| Persistence | None (OS layer) | None | None | Per-actor | None | None (GC) | **LTM + Episodic (durable)** |
| Isolation | None (same address space) | Total (no shared state) | Alias analysis | Total (mailbox only) | Hierarchical (block/device) | Total (immutable) | **Per-agent; COMM for exchange** |
| Semantic retrieval | No | No | No | No | No | No | **Yes (hybrid search)** |
| Memory ops in IR | load/store | Token routing | alloca/load/store | send/receive | ld/st with space qualifiers | bind/pattern match | **QMEM/UMEM/FENCE** |
| Compiler visibility | Alias analysis required | Full (DAG structure) | MemorySSA | Opaque (per-actor) | Partial (space annotations) | Full (purity) | **Full (DAG nodes)** |
| Concurrency control | Locks, atomics, fences | Structural (firing rule) | Undefined (target-dependent) | Mailbox serialization | `__syncthreads()`, atomics | None needed (immutable) | **Per-tier independent locks + FENCE** |

### Design Rationale

A-PXM's memory model follows from three decisions:

1. **Memory is not a von Neumann store.** Agents do not need byte-addressable flat memory. They need structured tiers that match how intelligent systems use context: immediate working memory, accumulated knowledge, and reflective history.

2. **Memory operations are not side effects.** They are typed DAG nodes with explicit data dependencies, enabling the compiler to reason about ordering, eliminate redundancy, and parallelize independent accesses.

3. **Memory has semantics, not just addresses.** The memory system supports meaning-based queries, not just exact-key lookups. This is a fundamental departure from every PXM in the table above.

---

## Memory as Fast Context for LLMs

The three-tier hierarchy maps directly to how LLM-based agents consume and produce context during execution.

**STM as fast memory.** Short-term memory holds the immediate working set -- recent tool outputs, intermediate reasoning results, session-scoped variables -- so that every LLM call receives precisely the context it needs without retrieving irrelevant history.

**LTM as accumulated knowledge.** Long-term memory captures what the agent has learned across sessions -- user preferences, project facts, domain knowledge, cached tool results -- preventing re-discovery on every invocation.

**Episodic memory as execution history.** The append-only log records what the agent has done: which operations fired, what inputs they received, what outputs they produced. This is the substrate for self-reflection and auditing.

**Beliefs, Goals, Capabilities.** The combination of these three tiers provides the **Beliefs** component of the [Agent Abstract Machine](aam.md). STM holds current beliefs about the immediate task. LTM holds durable beliefs about the world. Episodic memory holds beliefs derived from experience. Together with Goals and Capabilities, the AAM's full state is defined.

**Navigability by LLMs.** Because A-PXM's memory is structured as typed key-value stores and queryable logs -- not opaque embeddings or flat text blobs -- it is inherently navigable by LLMs. An agent can read its own STM, query LTM for relevant knowledge, scan episodic logs, and write updated beliefs back through UMEM. External tools and human operators use the same QMEM/UMEM interface.

---

## References

### Von Neumann Architecture

1. J. Backus, "Can Programming Be Liberated from the von Neumann Style?," *Communications of the ACM*, vol. 21, no. 8, pp. 613-641, 1978.
2. J. L. Hennessy and D. A. Patterson, *Computer Architecture: A Quantitative Approach*, 6th ed. Morgan Kaufmann, 2017.

### Dataflow Models

3. Arvind and R. E. Thomas, "I-Structures: An Efficient Data Type for Functional Languages," MIT TM-178, 1980.
4. P. S. Barth and R. S. Nikhil, "M-Structures: Extending a Parallel, Non-Strict Functional Language with State," in *Proc. FPCA '91*, pp. 538-568, 1991.
5. J. R. Gurd, C. C. Kirkham, and I. Watson, "The Manchester Prototype Dataflow Computer," *Communications of the ACM*, vol. 28, no. 1, pp. 34-52, 1985.

### LLVM IR and Compiler Memory Representations

6. C. Lattner and V. Adve, "LLVM: A Compilation Framework for Lifelong Program Analysis & Transformation," in *Proc. CGO '04*, pp. 75-86, 2004.
7. D. Novillo, "Memory SSA -- A Unified Approach for Sparsely Representing Memory Operations," in *Proc. GCC Developers' Summit*, 2007.
8. R. Cytron et al., "Efficiently Computing Static Single Assignment Form and the Control Dependence Graph," *ACM TOPLAS*, vol. 13, no. 4, pp. 451-490, 1991.

### Actor Model

9. C. Hewitt, P. Bishop, and R. Steiger, "A Universal Modular ACTOR Formalism for Artificial Intelligence," in *Proc. IJCAI '73*, pp. 235-245, 1973.
10. G. Agha, *Actors: A Model of Concurrent Computation in Distributed Systems*. MIT Press, 1986.

### GPU/CUDA Programming Model

11. NVIDIA Corporation, "CUDA C++ Programming Guide," v12.x.
12. J. Nickolls, I. Buck, M. Garland, and K. Skadron, "Scalable Parallel Programming with CUDA," *ACM Queue*, vol. 6, no. 2, pp. 40-53, 2008.

### Functional Programming Models

13. S. L. Peyton Jones, *The Implementation of Functional Programming Languages*. Prentice Hall, 1987.
14. P. Wadler, "Deforestation: Transforming Programs to Eliminate Trees," *Theoretical Computer Science*, vol. 73, no. 2, pp. 231-248, 1990.

### Cognitive Memory Models

15. R. C. Atkinson and R. M. Shiffrin, "Human Memory: A Proposed System and Its Control Processes," in *The Psychology of Learning and Motivation*, vol. 2, pp. 89-195, 1968.
16. E. Tulving, "Episodic and Semantic Memory," in *Organization of Memory*, pp. 381-403, 1972.
17. A. D. Baddeley, *Working Memory*. Oxford University Press, 1986.

### Information Retrieval

18. S. Robertson and H. Zaragoza, "The Probabilistic Relevance Framework: BM25 and Beyond," *Foundations and Trends in Information Retrieval*, vol. 3, no. 4, pp. 333-389, 2009.
19. J. Carbonell and J. Goldstein, "The Use of MMR, Diversity-Based Reranking for Reordering Documents and Producing Summaries," in *Proc. SIGIR '98*, pp. 335-336, 1998.
