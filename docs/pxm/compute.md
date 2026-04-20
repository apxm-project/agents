---
title: "Compute in Program Execution Models"
description: "How compute is defined and separated across classical and modern PXMs, and how A-PXM's typed AIS operations formalize agentic computation."
---

# Compute in Program Execution Models

The most fundamental question a Program Execution Model answers is: *what is a unit of compute?* The answer determines what can be optimized, what can be parallelized, and what the programmer must reason about. This document examines how six foundational PXMs define compute, then shows how A-PXM introduces typed, heterogeneous operations purpose-built for agentic AI.

This is a deep dive on the Compute axis of the [five separations](foundations.md).

---

## 1. Von Neumann: Instructions Fetched by Program Counter

In the von Neumann model, a unit of compute is a single **machine instruction** fetched from memory at the address held by the program counter (PC). Instructions operate on registers and memory: load, store, add, branch.

```
while true:
    instruction = memory[PC]
    execute(instruction)
    PC = next(PC, instruction)
```

**Properties:**
- **Granularity**: Sub-microsecond. Single arithmetic, logical, or memory operation.
- **Typing**: Weak. The ISA distinguishes integer/floating-point/address, but data is largely untyped bytes.
- **Composition**: Sequential by default. Parallelism requires explicit multi-threading or SIMD annotations.
- **Cost uniformity**: Most instructions cost roughly the same (1-5 cycles), except memory accesses.

**Limitation for agents:** Agent "instructions" (LLM calls, tool invocations, memory queries) have latencies measured in **seconds**, not nanoseconds -- six to nine orders of magnitude slower. They are heterogeneous: an LLM reasoning call has fundamentally different resource requirements, failure modes, and cost ($0.01-$0.10 per call) than a tool invocation or a memory read.

---

## 2. Dataflow: Operations That Fire on Token Arrival

In the dataflow model (Manchester Machine, MIT Tagged-Token Architecture), a unit of compute is an **operation node** in a dataflow graph. It fires when **all input tokens** have arrived. There is no program counter. The graph topology IS the program.

```
operation(inputs) -> outputs
    precondition: all input tokens present
    postcondition: output tokens emitted on all outgoing arcs
```

**Properties:**
- **Typing**: Tokens carry type tags in tagged-token architectures.
- **Composition**: Structural. Operations compose by wiring output arcs to input arcs.
- **Parallelism**: Automatic. Independent operations fire concurrently without programmer annotation.

**Influence on A-PXM:** A-PXM adopts the dataflow firing rule directly. The critical difference is that A-PXM operations are **coarse-grained and heterogeneous** (LLM calls, tool invocations, memory accesses), whereas Manchester Machine operations were fine-grained and homogeneous (arithmetic). This coarse granularity makes the scheduling overhead (~7.5 microseconds per operation) negligible against operation latencies of milliseconds to seconds.

---

## 3. LLVM IR / SSA: Operations in Static Single Assignment Form

In LLVM IR, a unit of compute is an **SSA instruction** that defines a single value. Each value is defined exactly once, and uses reference the definition directly.

```llvm
%sum = add i32 %a, %b       ; defines %sum
%result = mul i32 %sum, %c   ; uses %sum, defines %result
```

**Properties:**
- **Typing**: Strong. Every value has a precise type. Type mismatches are compilation errors.
- **Composition**: SSA def-use chains form a value dependency graph. Across blocks, CFG and phi nodes connect definitions.
- **Optimization surface**: SSA form enables CSE, DCE, constant folding, inlining -- because each value has a single definition point and all uses are visible.

**Influence on A-PXM:** A-PXM's MLIR-based compiler IR inherits SSA's optimization properties. AIS operations in MLIR are SSA values: each ASK, THINK, or REASON node defines a single result that downstream operations reference. This enables the [optimization pipeline](../compiler/pipeline.md) that are impossible in frameworks where LLM calls are opaque function calls.

---

## 4. Actor Model: Message-Driven Compute

In the actor model (Hewitt, 1973; Agha, 1986), a unit of compute is a **message handler** -- the code an actor executes in response to receiving a message. An actor processes one message at a time, sequentially, from its mailbox.

**Properties:**
- **Granularity**: Variable -- microseconds to seconds depending on handler logic.
- **Typing**: Untyped in classical formulations. Akka Typed adds compile-time message type checking.
- **Composition**: Actors compose by sending messages. Communication pattern is implicit, not declared as a graph.
- **Concurrency**: Natural. Actors with independent mailboxes execute concurrently.

**Limitation for agents:** The actor model lacks two properties critical for agent optimization: (1) no compile-time dependency graph, since actors communicate via runtime messages; (2) untyped operations, treating all computation uniformly as "handle this message."

---

## 5. GPU/CUDA: SIMT Parallel Compute

In CUDA's SIMT model, a unit of compute is a **kernel invocation** launching thousands of lightweight threads organized into a grid of blocks. All threads in a warp execute the same instruction simultaneously on different data elements.

**Properties:**
- **Granularity**: Two levels -- the kernel (coarse, launched from host) and the thread (fine, executing within the kernel).
- **Typing**: C++ types within kernels. Memory spaces are type-qualified.
- **Parallelism**: Massive data parallelism. All threads execute the same code on different data.

**Limitation for agents:** SIMT is designed for **homogeneous** data-parallel workloads. Agent workflows are **heterogeneous**: a REASON call, a tool invocation, and a memory query have nothing in common except being part of the same workflow.

---

## 6. Functional: Pure Expressions

In pure functional models (lambda calculus, Haskell, ML), a unit of compute is an **expression evaluation**. Every computation is the evaluation of an expression to a value. There are no statements, no side effects, and no mutable state.

**Properties:**
- **Typing**: Strong, static, with type inference (Hindley-Milner).
- **Referential transparency**: Expressions can be replaced with their values without changing program behavior, enabling deforestation, fusion, and specialization.

**Influence on A-PXM:** A-PXM borrows the functional model's key insight: **making data flow explicit enables optimization**. In Haskell, referential transparency lets the compiler fuse `map f . map g` into `map (f . g)`. In A-PXM, explicit data edges in the DAG let the compiler fuse producer-consumer ASK chains into single API calls.

---

## Comparative Summary

| Property | Von Neumann | Dataflow | LLVM IR | Actor | GPU/CUDA | Functional | **A-PXM** |
|----------|-------------|----------|---------|-------|----------|------------|----------|
| **Unit of compute** | Instruction | Operation node | SSA instruction | Message handler | Kernel/thread | Expression | **AIS operation** |
| **Granularity** | Nanoseconds | Nanoseconds (HW) | Nanoseconds | Variable | Variable | Variable | **Seconds** |
| **Type system** | Weak (ISA types) | Tagged tokens | Strong (IR types) | Untyped/runtime | C++ types | Strong (HM) | **AIS op categories** |
| **Heterogeneity** | Moderate | Low (arithmetic) | Moderate | High | Low (SIMT) | High | **High (LLM/tool/mem/ctrl)** |
| **Optimization** | Hardware (OoO) | Graph structure | SSA passes | None (opaque) | Warp scheduling | Fusion, deforestation | **MLIR passes** |
| **Parallelism** | Manual | Structural | Manual (threads) | Structural (actors) | Data-parallel (SIMT) | Implicit (purity) | **Structural (DAG)** |
| **Cost per op** | ~1ns, ~free | ~1ns | ~1ns | ~1us | ~1ns per thread | ~1ns | **~1-10s, $0.01-$0.10** |

---

## A-PXM: Typed Heterogeneous Operations for Agentic Compute

### The Core Insight

Agent operations are the most expensive "instructions" ever scheduled. A single ASK call costs seconds of wall-clock time and cents of real money. This inverts the optimization calculus: in traditional compilation, eliminating one instruction saves nanoseconds. In A-PXM, eliminating one ASK call saves seconds and dollars. The economic return on optimization is orders of magnitude higher.

### AIS Operations as Typed Compute Primitives

A-PXM defines compute through the [Agent Instruction Set (AIS)](ais.md) -- typed operations organized into categories. Each operation is a node in the dataflow DAG with:

- **Typed inputs and outputs**: not opaque function calls, but values with known types
- **Explicit data edges**: declaring exactly what each operation needs
- **Latency annotations**: ASK ~1s, THINK ~3s, REASON ~10s, enabling priority [scheduling](scheduling.md)
- **Operation-specific semantics**: the runtime dispatches each operation to a category-specific executor

### What Makes This Different

**Unlike von Neumann instructions**: AIS operations are coarse-grained (seconds, not nanoseconds), heterogeneous, and expensive. Optimization matters orders of magnitude more.

**Unlike dataflow operations**: AIS operations are typed by category, not uniform arithmetic nodes. The scheduler uses operation type information to make informed dispatch decisions.

**Unlike actor messages**: AIS operations exist in a statically analyzable DAG. The compiler can see every operation and every dependency before execution begins.

**Unlike CUDA kernels**: AIS operations are heterogeneous by design. Each operation type has its own executor.

**Unlike functional expressions**: AIS operations have explicit side effects visible in the DAG as typed nodes, not hidden behind monadic wrappers.

### The Optimization Payoff

Because AIS operations are typed nodes in an SSA-style dataflow graph, A-PXM can apply compiler optimizations that no other agent framework supports. See [compiler/pipeline.md](../compiler/pipeline.md) for the current pass inventory.

Each eliminated operation saves seconds and dollars -- not nanoseconds. This is why compute separation matters more for agents than for any previous computing paradigm.

---

## References

### Von Neumann Architecture

1. J. von Neumann, "First Draft of a Report on the EDVAC," 1945. DOI: [10.1109/85.238389](https://doi.org/10.1109/85.238389)
2. J. Backus, "Can Programming Be Liberated from the von Neumann Style?," *Communications of the ACM*, 1978. DOI: [10.1145/359576.359579](https://doi.org/10.1145/359576.359579)

### Dataflow / Manchester Machine

3. J. R. Gurd, C. C. Kirkham, and I. Watson, "The Manchester Prototype Dataflow Computer," *Communications of the ACM*, 1985. DOI: [10.1145/2465.2468](https://doi.org/10.1145/2465.2468)
4. Arvind and D. E. Culler, "Dataflow Architectures," *Annual Reviews in Computer Science*, 1986. DOI: [10.1146/annurev.cs.01.060186.001301](https://doi.org/10.1146/annurev.cs.01.060186.001301)
5. J. B. Dennis, "First Version of a Data Flow Procedure Language," in *Programming Symposium*, LNCS vol. 19, 1974. DOI: [10.1007/3-540-06859-7_145](https://doi.org/10.1007/3-540-06859-7_145)

### LLVM IR / SSA Form

6. C. Lattner and V. Adve, "LLVM: A Compilation Framework for Lifelong Program Analysis & Transformation," in *Proc. CGO '04*, 2004. DOI: [10.1109/CGO.2004.1281665](https://doi.org/10.1109/CGO.2004.1281665)
7. R. Cytron et al., "Efficiently Computing Static Single Assignment Form and the Control Dependence Graph," *ACM TOPLAS*, 1991. DOI: [10.1145/115372.115320](https://doi.org/10.1145/115372.115320)
8. C. Lattner et al., "MLIR: Scaling Compiler Infrastructure for Domain Specific Computation," in *Proc. CGO '21*, 2021. DOI: [10.1109/CGO51591.2021.9370308](https://doi.org/10.1109/CGO51591.2021.9370308)

### Actor Model

9. C. Hewitt, P. Bishop, and R. Steiger, "A Universal Modular ACTOR Formalism for Artificial Intelligence," in *Proc. IJCAI '73*, 1973.
10. G. Agha, *Actors: A Model of Concurrent Computation in Distributed Systems*, MIT Press, 1986.

### GPU / CUDA

11. J. Nickolls and W. J. Dally, "The GPU Computing Era," *IEEE Micro*, 2010. DOI: [10.1109/MM.2010.41](https://doi.org/10.1109/MM.2010.41)

### Functional Programming

12. A. Church, *The Calculi of Lambda Conversion*, Princeton University Press, 1941.
13. P. Wadler, "Deforestation: Transforming Programs to Eliminate Trees," *Theoretical Computer Science*, 1990. DOI: [10.1016/0304-3975(90)90147-A](https://doi.org/10.1016/0304-3975(90)90147-A)

### A-PXM

14. G. R. Gao, R. Patel, and T. St. John, "The Codelet Program Execution Model," presented at *WiA, ISCA '13*, 2013.
