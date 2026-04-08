---
title: "Scheduling and Execution in Program Execution Models"
description: "A comparative analysis of scheduling and execution across classical and modern PXMs, and how A-PXM synthesizes these ideas for agent workflows."
---

# Scheduling and Execution in Program Execution Models

A Program Execution Model (PXM) defines three interrelated concerns: how execution order is determined, how parallelism is discovered, and how synchronization enforces correctness. This document examines six foundational PXMs, then shows how A-PXM synthesizes their strongest properties into a scheduling model purpose-built for agentic AI workflows.

This is a deep dive on the Scheduling axis of the [five separations](foundations.md).

> For implementation details (thread pool configuration, session lane guards, backpressure), see [dataflow scheduler implementation](../implementation/runtime/dataflow-scheduler.md).

---

## 1. Von Neumann Sequential Execution

The von Neumann model is defined by the **program counter** (PC). The PC holds the address of the next instruction. After each instruction executes, the PC advances to the next sequential address unless a branch redirects it. Execution order is a total order encoded in the instruction stream.

**Parallelism:** Invisible. The instruction stream specifies a single thread of control. Any parallelism must be introduced explicitly by the programmer.

**Synchronization:** Trivially satisfied by program order. When explicit parallelism is introduced, synchronization must be layered on through locks, semaphores, barriers, or fences.

---

## 2. Dataflow Execution (Manchester Machine)

The Manchester Dataflow Machine (1981) eliminated the program counter entirely. Execution order is determined by **data availability**. Each operation fires when **all** of its input tokens have arrived. Two operations with no data dependency are **inherently concurrent** -- the graph topology is the schedule.

**Parallelism:** Intrinsic and structural. If two operations share no common input tokens, they execute simultaneously.

**Synchronization:** Implicit in the token-matching mechanism. A join point naturally blocks until all inputs are present. No locks, no barriers.

The challenge historically was hardware cost: the associative token store required expensive content-addressable memory. But the model itself -- data-driven execution with structural parallelism -- remains theoretically sound.

---

## 3. Out-of-Order Execution (Tomasulo, Scoreboarding)

Out-of-order (OoO) execution preserves the **illusion** of sequential execution while internally reordering instructions based on data readiness. **Tomasulo's algorithm** (IBM 360/91, 1967) used register renaming via reservation stations to eliminate false dependencies (WAW, WAR), dispatching instructions when operands became available.

**Parallelism:** Discovered by hardware at run time through dependency analysis on a sliding window (reorder buffer, typically 100-300 instructions).

**Synchronization:** In-order retirement via the reorder buffer preserves sequential semantics for interrupts, exceptions, and memory ordering.

---

## 4. Task-Based Parallelism (Cilk, TBB, Tokio)

Task-based systems model computation as a **directed acyclic graph of tasks** where edges represent dependencies:

- **Cilk**: `cilk_spawn` / `cilk_sync` with serial elision
- **TBB**: `task_group`, `parallel_for`, `flow_graph`
- **Tokio**: `async fn` / `.await` with cooperative multitasking

**Parallelism:** Programmer-directed but runtime-scheduled. The key innovation is **work stealing**: each worker maintains a local deque; idle workers steal from others. Cilk's scheduler achieves provable bounds: O(T1/P + T_inf) on P processors.

**Synchronization:** Explicit in the programming model (`cilk_sync`, `.await`, channels, mutexes). The programmer must reason about concurrency.

---

## 5. MapReduce / Spark

MapReduce and Spark implement a **bulk-synchronous** model organized into stages separated by global synchronization barriers (shuffles).

**Parallelism:** Data-parallel within stages (each partition processed by an independent task). Across stages, limited by shuffle barriers.

**Synchronization:** Shuffle barriers -- coarse-grained, global. All upstream tasks must complete before downstream tasks begin. Fault tolerance via lineage.

---

## 6. Petri Nets

A Petri net is a bipartite directed graph of **places** (holding tokens) and **transitions**. A transition fires when all input places contain at least one token, atomically consuming input tokens and producing output tokens.

**Parallelism:** Structural. Two transitions with no common input/output places fire concurrently. Petri nets model true concurrency, not interleaving.

**Synchronization:** Modeled through the place-transition structure: fork (fan-out), join (barrier), mutual exclusion (shared place with single token), choice (multiple output transitions).

---

## Comparative Summary

| Property | Von Neumann | Dataflow | OoO (Tomasulo) | Task-Based | MapReduce/Spark | Petri Nets |
|---|---|---|---|---|---|---|
| **Order** | Program counter | Data availability | Data readiness (HW) | Task DAG + runtime | Stage DAG + shuffle | Token marking |
| **Parallelism** | Manual | Automatic (structural) | Automatic (HW window) | Semi-automatic | Automatic within stage | Structural |
| **Synchronization** | Program order / locks | Token matching | Reorder buffer | async/await, barriers | Shuffle barriers | Firing rule |
| **Granularity** | Instruction | Operation | Instruction | Task (function) | Stage (partition) | Transition |
| **Developer burden** | Full concurrency reasoning | None | None (transparent) | Decomposition + sync | None within stages | Model construction |

---

## How A-PXM Schedules and Executes Agent Workflows

A-PXM synthesizes ideas from the models above into a scheduling system designed for agentic AI workloads: high-latency operations (LLM calls measured in seconds), heterogeneous operation types, and multi-agent coordination.

### Token-Based Dataflow Scheduler

The core mechanism is identical in principle to the Manchester Machine: operations fire when all input data is available. But where the Manchester Machine used expensive associative hardware for token matching, A-PXM uses atomic counters.

Every operation maintains a **pending counter** initialized to its in-degree:

1. When an upstream operation completes, it **produces tokens** on each outgoing edge.
2. Each token **atomically decrements** the downstream consumer's pending counter.
3. When a counter reaches **zero**, the operation is ready and enters the priority queue.

```
on_token_produced(token_id):
    for each consumer of token_id:
        count = atomic_decrement(consumer.pending_count)
        if count == 0:
            mark_ready(consumer)
            enqueue(consumer, priority)
```

This provides **O(1) readiness detection**: a single atomic decrement and comparison. No graph traversal. No dependency resolution at fire time.

### Automatic Parallelism Without async/await

The DAG structure **is** the parallelism specification. If two operations have no edge between them, they are independent and execute concurrently. The developer never writes `async`, `await`, `Promise.all()`, or thread management code.

Consider a fan-out/fan-in pattern:

```
REASON --> INV tool_a (500ms)
       --> INV tool_b (800ms)
       --> INV tool_c (300ms)
                        \
                         --> WAIT_ALL --> ASK (summarize)
```

Sequential wall-clock: 1600ms. A-PXM wall-clock: max(500, 800, 300) = 800ms. Zero developer effort.

This is fundamentally different from task-based systems where the developer must explicitly spawn tasks and await results. In A-PXM, the developer declares data dependencies; the runtime exploits independence.

### Work-Stealing Executor Pool

The execution substrate uses a **work-stealing thread pool** inspired by Cilk. The stealing strategy has three tiers:

1. **Local deque** (O(1), no contention)
2. **Global priority injectors** (low contention, priority-ordered)
3. **Peer worker queues** (round-robin)

This hybrid of dataflow scheduling and work-stealing execution combines the best of both: the token system determines **what** is ready; the work-stealing pool determines **where** it runs.

### Typed Operation Dispatch

Unlike Petri nets (where transitions are untyped), A-PXM operations are **typed** with category-specific executors:

| Executor | Operations | Responsibility |
|---|---|---|
| Memory | QMEM, UMEM, FENCE | Three-tier [memory hierarchy](memory.md) access |
| LLM | ASK, THINK, REASON, PLAN, REFLECT, VERIFY | Model API dispatch with latency budgets |
| Tool | INV | External tool invocation with typed parameter marshalling |
| Control | BRANCH_ON_VALUE, SWITCH | Conditional routing in the DAG |
| Sync | MERGE, WAIT_ALL | Parallel path synchronization |
| Communication | COMM, FLOW_CALL | Cross-agent messaging and flow invocation |

The scheduler uses latency budgets (ASK ~1s, THINK ~3s, REASON ~10s) to prioritize critical-path operations and overlap long-running reasoning with independent fast operations.

### WAIT_ALL and FENCE: Explicit Synchronization in the DAG

**WAIT_ALL** is the fan-in synchronization point. It maintains a pending counter identical to the scheduler's per-node counter. When all tokens arrive, values are collected into an ordered tuple and emitted downstream.

**FENCE** is a memory barrier node. It ensures that all UMEM operations preceding it complete before any QMEM operations following it can execute. FENCE carries no data -- it is a pure ordering constraint.

**MERGE** handles reconvergence after conditional branching. Unlike WAIT_ALL (which waits for all N inputs), MERGE expects exactly one token because only one branch fires.

These synchronization primitives are declarative DAG nodes, not imperative runtime calls.

### Comparison with Classical Models

| Property | Manchester Machine | Petri Nets | Cilk / Tokio | A-PXM |
|---|---|---|---|---|
| **Firing rule** | All tokens present | All input places have tokens | Explicit spawn/await | Pending counter == 0 |
| **Operation types** | Untyped | Untyped transitions | Typed functions | Typed AIS instructions with category-specific executors |
| **Parallelism** | Structural | Structural | Programmer-directed | Structural (DAG edges) |
| **Synchronization** | Token matching | Place-transition structure | async/await, barriers | WAIT_ALL, FENCE, MERGE as DAG nodes |
| **Readiness cost** | Associative match (HW) | Marking check | Runtime queue | O(1) atomic decrement |
| **Developer burden** | None (hardware) | Model construction | Task decomposition + sync | None -- DAG is the spec |

### Why This Design

1. **Operations are coarse-grained and high-latency.** The ~7.5us scheduling overhead is negligible against second-scale LLM calls. Software-based token counting is viable where the Manchester Machine needed dedicated hardware.

2. **Parallelism is abundant but not obvious.** Agent workflows frequently involve independent tool calls and parallel research paths. Structural parallelism from the DAG extracts this automatically.

3. **Operations have heterogeneous semantics.** Typed dispatch with category-specific executors applies appropriate strategies: latency budgets for LLM calls, retry policies for tool invocations, transactional semantics for memory writes.

4. **The DAG is the contract.** The dataflow graph is the single source of truth for both execution order and parallelism. The compiler verifies it statically; the runtime executes it faithfully.

---

## References

### Von Neumann Sequential Execution

1. J. von Neumann, "First Draft of a Report on the EDVAC," 1945.
2. J. Backus, "Can Programming Be Liberated from the von Neumann Style?," *Communications of the ACM*, 1978.

### Dataflow Execution (Manchester Machine)

3. J. R. Gurd, C. C. Kirkham, and I. Watson, "The Manchester Prototype Dataflow Computer," *Communications of the ACM*, 1985.
4. J. B. Dennis, "First Version of a Data Flow Procedure Language," in *Programming Symposium*, LNCS vol. 19, 1974.
5. Arvind and D. E. Culler, "Dataflow Architectures," *Annual Review of Computer Science*, 1986.

### Out-of-Order Execution

6. R. M. Tomasulo, "An Efficient Algorithm for Exploiting Multiple Arithmetic Units," *IBM Journal of R&D*, 1967.
7. J. E. Thornton, *Design of a Computer: The Control Data 6600*. Scott, Foresman, 1970.
8. J. E. Smith and G. S. Sohi, "The Microarchitecture of Superscalar Processors," *Proceedings of the IEEE*, 1995.

### Task-Based Parallelism

9. R. D. Blumofe et al., "Cilk: An Efficient Multithreaded Runtime System," in *Proc. PPoPP '95*, 1995.
10. R. D. Blumofe and C. E. Leiserson, "Scheduling Multithreaded Computations by Work Stealing," *JACM*, 1999.
11. Tokio Contributors, "Tokio: An Asynchronous Runtime for the Rust Programming Language," https://tokio.rs/.

### MapReduce / Spark

12. J. Dean and S. Ghemawat, "MapReduce: Simplified Data Processing on Large Clusters," in *Proc. OSDI '04*, 2004.
13. M. Zaharia et al., "Resilient Distributed Datasets," in *Proc. NSDI '12*, 2012.

### Petri Nets

14. C. A. Petri, "Kommunikation mit Automaten," PhD thesis, Universitat Hamburg, 1962.
15. T. Murata, "Petri Nets: Properties, Analysis and Applications," *Proceedings of the IEEE*, 1989.

### A-PXM

16. G. R. Gao, R. Patel, and T. Sterling, "The Codelet Program Execution Model," presented at *WiA, ISCA '13*, 2013.
