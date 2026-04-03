---
title: "History Repeats: From Von Neumann to A-PXM"
description: "The historical narrative of Program Execution Models -- how the same structural problems that plagued early computing now plague agentic AI, and why the same class of solutions (formal execution models, compilers, runtimes) apply again."
---

# History Repeats: From Von Neumann to A-PXM

Every major shift in computing has followed the same arc: a powerful new capability arrives, practitioners wire it together by hand, the resulting systems become opaque and unmanageable, and then someone builds the formal infrastructure -- an execution model, a compiler, a runtime -- that makes the capability programmable at scale. This has happened at least five times in the last eighty years. It is happening again now with agentic AI.

This document traces that arc and positions A-PXM within it.

---

## 1. The First Time: Von Neumann and the Stored-Program Computer

In June 1945, John von Neumann circulated a thirty-page document titled "First Draft of a Report on the EDVAC." It described something that had never existed: a machine that stored its program in the same memory as its data, executing instructions sequentially under the control of a program counter. Before this, "programming" meant rewiring patch cables on the ENIAC. There was no abstraction between the problem and the hardware. Every computation was a physical configuration.

Von Neumann's insight was structural. He defined an **abstract machine** -- a program counter, an accumulator, a memory -- and a contract: if you express your computation as a sequence of instructions that this abstract machine understands, the hardware will execute it faithfully. The specific hardware could change. The instruction encoding could change. But the abstract machine was the stable interface.

This was the first Program Execution Model: a formal specification of how programs are represented, what state they operate on, and how execution proceeds. It separated the *what* (the program) from the *how* (the hardware), and in doing so created the entire field of software.

But von Neumann's model carried a structural limitation that would take decades to fully articulate.

---

## 2. The Bottleneck: Backus Sees the Problem

By the 1970s, the von Neumann model had conquered computing so thoroughly that its limitations had become invisible -- mistaken for laws of nature rather than design choices. In his 1977 Turing Award lecture, John Backus named the problem: the **von Neumann bottleneck**.

> "Surely there must be a less primitive way of making big changes in the store than by pushing vast numbers of words back and forth through the von Neumann bottleneck."

The bottleneck was not merely about bandwidth between CPU and memory. It was about **opacity**. The sequential instruction stream hid the structure of the computation from every layer of the system. The hardware could not see which instructions were independent. The compiler could not reason about the program's intent. The programmer was forced to think in terms of state mutations rather than data transformations.

Backus proposed functional programming as the remedy -- programs as compositions of functions with no side effects, where data flow was explicit and the algebra of programs enabled mechanical transformation. The idea was ahead of its time. But the diagnosis was exact: when the execution model hides structure, no tool in the stack can help.

This diagnosis applies, almost word for word, to agentic AI today. Current agent frameworks execute workflows as opaque Python function calls -- "call-at-a-time" chains where the runtime has no visibility into what operations exist, how they relate, or what state they touch. The agentic von Neumann bottleneck is not about CPU-memory bandwidth. It is about the complete absence of structure in the execution model, which makes optimization, verification, and auditing impossible.

---

## 3. The Contract: Instruction Set Architectures

One of the most consequential abstractions in computing history is the **Instruction Set Architecture** (ISA). An ISA is a contract -- a formal interface between software and hardware. Above the ISA, compilers and programmers express computation in terms of defined operations with specified semantics. Below the ISA, hardware architects implement those operations however they choose: pipelining, out-of-order execution, superscalar issue, SIMD.

The x86 ISA, defined in 1978, is still executing programs written today -- nearly fifty years later -- despite the fact that the underlying microarchitecture has been redesigned from scratch at least a dozen times. The Pentium 4 and a modern Zen 5 core share almost nothing in hardware design, but they execute the same ISA. That stability is not an accident. It is the entire point. The ISA decouples progress above the interface from progress below it. Compiler writers and hardware engineers can innovate independently, each relying on the contract.

A-PXM's **Agent Instruction Set (AIS)** is this contract for agentic AI. It defines the full set of typed operations -- ASK, THINK, REASON, INV, PLAN, REFLECT, VERIFY, QMEM, UMEM, FENCE, and others -- each with specified inputs, outputs, and state transitions on the Agent Abstract Machine. Run `apxm ops list` for the current set. Above the AIS, any frontend (Python SDK, Rust API, visual editor, LLM-driven planner) can emit AIS graphs. Below the AIS, any runtime (local executor, distributed scheduler, hardware accelerator) can execute them.

The parallel is precise:

| Layer | Traditional Computing | Agentic Computing |
|-------|----------------------|-------------------|
| **Software above** | C, Rust, Python source code | Agent workflow definitions (any framework) |
| **Contract** | ISA (x86, ARM, RISC-V) | AIS (ASK, THINK, REASON, INV, ...) |
| **Implementation below** | CPU microarchitecture | A-PXM runtime, future agent hardware |

Without this contract, every framework is vertically integrated: LangChain workflows run only on LangChain's runtime, CrewAI workflows run only on CrewAI's runtime. There is no portability, no shared optimization infrastructure, and no separation of concerns. This is the state of computing before ISAs -- when every program was written for a specific machine.

---

## 4. The Compiler's Role: From FORTRAN to LLVM to A-PXM

Compilers have always occupied the same structural position in the software stack: they bridge the gap between **programmability** and **performance**, translating what the developer wants to express into what the machine can efficiently execute.

### FORTRAN: The First Bridge (1957)

When John Backus and his team at IBM released FORTRAN in 1957, the prevailing wisdom was that no automatic translator could match the performance of hand-written assembly. Programmers had spent years mastering the idiosyncrasies of specific machines, and they were skeptical that a "formula translator" could do better. FORTRAN proved them wrong -- not by generating perfect code, but by generating code that was *good enough* while being orders of magnitude faster to write. The compiler absorbed the tedium of register allocation, instruction selection, and loop optimization. The programmer could think about the problem instead of the machine.

### LLVM: The Universal Infrastructure (2004)

Forty-seven years later, Chris Lattner and Vikram Adve published "LLVM: A Compilation Framework for Lifelong Program Analysis & Transformation." LLVM's insight was that compiler infrastructure could be **shared**. Instead of every language building its own compiler from scratch, languages could emit a common intermediate representation (LLVM IR), and a shared optimizer and code generator could handle the rest.

The result was explosive. Clang (C/C++), Rust (rustc), Swift, Julia, and dozens of other languages now compile through LLVM. Each language team focuses on frontend design and language semantics. LLVM handles optimization and code generation. Progress on either side benefits the entire ecosystem.

### A-PXM: The Agent Compiler (Now)

No compiler exists for agentic AI. When a developer writes a LangChain workflow or a CrewAI crew, there is no compilation step. The Python code executes directly -- every LLM call is an opaque function invocation, every dependency is implicit, every opportunity for optimization is invisible. This is the state of computing before FORTRAN: the programmer manages everything by hand.

A-PXM provides the missing compiler. Agent workflows are expressed as AIS dataflow graphs. The compiler, built on MLIR (the successor to LLVM IR for domain-specific compilation), performs typed analysis and optimization:

| Pass | Historical Analogy | Impact |
|------|-------------------|--------|
| **FuseAskOps** | Loop fusion in FORTRAN | Merges producer-consumer LLM calls into single API calls |
| **CSE** | Common Subexpression Elimination (since the 1960s) | Eliminates duplicate LLM calls with identical inputs |
| **DCE** | Dead Code Elimination | Removes operations whose outputs are never consumed |
| **Canonicalization** | Algebraic simplification | Normalizes graph patterns for consistent optimization |

The economics are different from traditional compilation -- and far more favorable. In traditional compilers, eliminating one instruction saves nanoseconds. In A-PXM, eliminating one LLM call saves **seconds of wall-clock time and cents of real money**. The return on optimization is nine orders of magnitude higher per operation. The case for a compiler has never been stronger.

---

## 5. The Runtime's Role: Implementing Abstract Machines

A compiler translates programs. A runtime **executes** them. Specifically, a runtime implements an abstract machine -- it provides the concrete mechanisms for the state model, execution semantics, and resource management that the abstract machine specifies.

### The Von Neumann Runtime

The simplest runtime is the CPU itself. It implements the von Neumann abstract machine: a program counter that fetches instructions, registers that hold working data, and a memory that stores both program and data. The "runtime" is etched in silicon.

### The JVM Runtime (1995)

The Java Virtual Machine added a layer of indirection. The JVM abstract machine defined a stack-based bytecode interpreter with automatic garbage collection, type safety, and platform independence. The JVM runtime (HotSpot, later GraalVM) implemented this abstract machine in software, adding just-in-time compilation, adaptive optimization, and sophisticated garbage collectors. The abstract machine was the contract; the runtime was the implementation.

### The CUDA Runtime (2007)

NVIDIA's CUDA defined yet another abstract machine: a grid of thread blocks, each containing warps of SIMT threads, with a hierarchy of memory spaces (registers, shared memory, global memory, constant memory). The CUDA runtime mapped this abstract machine onto GPU hardware, handling thread scheduling, memory management, and kernel launch.

### The A-PXM Runtime (Now)

The A-PXM runtime implements the **Agent Abstract Machine (AAM)**:

| Von Neumann Machine | JVM | CUDA | A-PXM (AAM) |
|---------------------|-----|------|-------------|
| Program Counter | Bytecode Pointer | Thread ID + Warp PC | Dataflow token counters |
| Registers | Operand Stack | Registers + Shared Mem | Beliefs (typed key-value store) |
| Memory | Heap + GC | Global / Shared / Constant | Three-tier memory (STM / LTM / Episodic) |
| -- | -- | -- | Goals (priority queue) |
| -- | -- | -- | Capabilities (typed function signatures) |

The AAM state triple `(Beliefs, Goals, Capabilities)` is the formal definition of what an agent *is* at any point in time. Every AIS instruction is a deterministic state transition: `δ(AAM, Instr) → AAM'`. The runtime maintains this state, enforces its invariants, and provides the dataflow scheduler that determines when operations fire.

The table above shows the *implementation-level* mapping. At the *conceptual* level, the von Neumann abstractions map more directly onto the agent domain than the "--" rows suggest. The program counter determines *what to do next* -- in the agent domain, that role belongs to **Goals** (a priority queue of objectives that drive execution). Registers hold *working data the processor acts on* -- in the agent domain, that role belongs to **Beliefs** (the typed key-value store of current knowledge). Memory stores *available procedures and data* -- in the agent domain, that role is split between the three-tier memory hierarchy (for data) and **Capabilities** (for available procedures, i.e., typed function signatures the agent can invoke). The structural parallel is that every computing machine -- sequential or agentic -- needs mechanisms for directing execution, holding working state, and accessing stored knowledge and procedures. The AAM makes these mechanisms explicit and formal for agents, just as the von Neumann architecture made them explicit and formal for sequential computation [14].

This is not a metaphor. The AAM is to A-PXM what the von Neumann machine is to x86: a formal specification that the runtime implements and that the compiler targets. Without it, "agent state" is whatever happens to be in Python closures and global variables -- untyped, unstructured, invisible to tooling.

---

## 6. The PXM Lineage: Five Decades of Closing the Gap

The term "Program Execution Model" (PXM) names a pattern that has recurred throughout computing history. A PXM closes the gap between software (what the programmer wants to express) and hardware (what the machine can do) by providing abstractions for compute, memory, state, and scheduling. Each PXM in history was designed for the dominant computational challenge of its era.

### Von Neumann (1945): Sequential Instruction Execution

The original PXM. Compute is instructions fetched by a program counter. Memory is a flat address space. State is registers plus memory. Scheduling is trivially sequential. This model was sufficient when the challenge was making computation possible at all.

### Dataflow (1970s-1980s): Data-Driven Execution

The Manchester Machine, MIT Tagged-Token Architecture, and related systems replaced the program counter with a firing rule: operations execute when their input data arrives. Parallelism was structural -- inherent in the graph topology -- not programmer-directed. These machines were ahead of their time; the hardware cost of associative token matching made them impractical against rapidly improving von Neumann processors. But the model was sound, and it survived as an intellectual foundation for everything that followed.

### Cilk (1995): Work-Stealing Task Parallelism

Cilk, developed at MIT by Charles Leiserson and his students, introduced work-stealing scheduling for task-parallel programs. The programmer decomposed work into tasks with explicit fork-join dependencies. The runtime stole tasks from busy workers to idle ones, achieving provably optimal load balancing. Cilk's insight was that **the runtime, not the programmer, should decide where work executes** -- the programmer declares the parallelism structure, and the scheduler exploits it.

### CUDA (2007): Massively Data-Parallel Execution

NVIDIA's CUDA defined a PXM for GPU computing: thousands of lightweight threads organized into blocks and grids, executing the same kernel on different data (SIMT). The memory hierarchy (registers, shared memory, global memory) was explicit and programmer-managed. CUDA made massively parallel hardware programmable by providing the right abstractions -- thread identity, block synchronization, memory spaces -- that mapped naturally onto the underlying architecture.

### Codelet (2013): Fine-Grained Dataflow for Exascale

Guang Gao's Codelet PXM, proposed for exascale computing, revisited dataflow with modern hardware in mind. Codelets were fine-grained, non-preemptable units of computation that fired when their data dependencies were satisfied. The model explicitly addressed the energy and latency costs of data movement -- the dominant constraint in exascale systems. A-PXM's name and conceptual structure draw directly from this lineage.

### A-PXM (2025): The PXM for Agentic AI

A-PXM continues this lineage, designed for the computational challenge of the present era: orchestrating autonomous AI agents. The "instructions" are LLM calls, tool invocations, and memory operations -- coarse-grained, high-latency, expensive, and heterogeneous. The "hardware" is API endpoints, model servers, and tool services.

| PXM | Era | Challenge | Key Abstraction |
|-----|-----|-----------|-----------------|
| Von Neumann | 1945 | Making computation possible | Program counter + stored program |
| Dataflow | 1975 | Exploiting parallelism automatically | Token-driven firing rule |
| Cilk | 1995 | Efficient task parallelism | Work-stealing scheduler |
| CUDA | 2007 | Programmable massively parallel hardware | SIMT threads + memory hierarchy |
| Codelet | 2013 | Energy-efficient exascale computing | Fine-grained non-preemptable codelets |
| **A-PXM** | **2025** | **Orchestrating autonomous agents** | **Typed AIS operations + AAM state + dataflow scheduling** |

The pattern is consistent: each PXM provides the abstractions that make a new class of computation **visible** to tooling -- to compilers for optimization, to runtimes for scheduling, to developers for reasoning. A-PXM does this for agent workflows.

---

## 7. The Wrong Abstraction Layer

There is a final historical parallel that clarifies why current agent frameworks are insufficient, and it is not about any single missing feature. It is about operating at the **wrong layer of the stack**.

In the 1950s, before compilers existed, programmers wrote assembly by hand. They were productive -- they built real systems, solved real problems, and developed sophisticated techniques for managing complexity. But they were building at the wrong layer. Every program was coupled to a specific machine. Every optimization was manual. Every reuse required reimplementation. The infrastructure layer -- the compiler, the runtime, the execution model -- was missing.

The arrival of FORTRAN did not make assembly programmers obsolete. It made them unnecessary for most tasks by providing a higher-level abstraction backed by formal infrastructure. The compiler handled what the programmer used to handle by hand. The result was not just convenience -- it was a qualitative change in what was possible. Programs became portable, optimizable, and verifiable in ways that hand-written assembly could never be.

Current agent frameworks -- LangChain, CrewAI, AutoGen, LangGraph, and their successors -- are building at the application layer. They provide excellent developer experience: convenient APIs, pre-built components, visual editors. But beneath them, there is no formal execution model. No typed intermediate representation. No compiler. No optimizer. No abstract machine with formal state transitions.

This means:

- **No portability.** A LangChain workflow cannot run on CrewAI's runtime, or vice versa. Every framework is a vertical silo.
- **No shared optimization.** Each framework implements its own ad-hoc caching and batching. There is no shared infrastructure for optimization passes that benefit all frameworks.
- **No formal verification.** Type errors, missing dependencies, and malformed state transitions surface at runtime -- after expensive LLM calls have already been made.
- **No auditing.** When an agent produces a wrong answer, tracing the root cause means reading Python stack traces and log files. There is no formal execution trace.

These are not limitations of specific frameworks. They are consequences of the missing infrastructure layer. A-PXM provides that layer -- the execution model, the compiler, the runtime -- so that frameworks can focus on what they do well (developer experience, domain-specific abstractions) while the infrastructure handles correctness, optimization, and scheduling.

The relationship is the same as LLVM's relationship to programming languages: LLVM does not replace Rust or Swift. It provides the compilation infrastructure that Rust and Swift build on. A-PXM does not replace LangChain or CrewAI. It provides the execution infrastructure that agent frameworks should build on.

---

## The Arc Completes

The history of computing is, in large part, the history of making computation visible. The von Neumann model made hardware programmable by defining an abstract machine. ISAs made software portable by defining a contract between languages and hardware. Compilers made programs optimizable by defining intermediate representations. Runtimes made abstract machines executable by providing concrete scheduling and state management.

Each of these innovations followed the same structural pattern: a new capability arrived (general-purpose computation, parallelism, heterogeneous hardware, massive data parallelism), practitioners wired it together by hand, the resulting systems became opaque and unmanageable, and then someone built the formal infrastructure that made the capability programmable at scale.

Agentic AI is at the "wiring it together by hand" stage. The capability -- autonomous agents that reason, plan, use tools, and collaborate -- is here. The formal infrastructure is not. A-PXM is that infrastructure: the execution model, the instruction set, the compiler, and the runtime that make agent workflows visible, optimizable, verifiable, and auditable.

History does not repeat, but it rhymes. The rhyme scheme is remarkably consistent.

---

## Further Reading

- [PXM Foundations](foundations.md) -- the five separations (Compute, Memory, State, Optimization, Scheduling) and why they matter
- [Agent Abstract Machine (AAM)](aam.md) -- the formal state model: Beliefs, Goals, Capabilities
- [Agent Instruction Set (AIS)](ais.md) -- the typed operation taxonomy and the ISA contract
- [Compute in PXMs](compute.md) -- how six PXMs define compute, and how A-PXM differs
- [Scheduling in PXMs](scheduling.md) -- dataflow scheduling, work stealing, and session isolation

---

## References

### Foundational Architecture

1. J. von Neumann, "First Draft of a Report on the EDVAC," Moore School of Electrical Engineering, University of Pennsylvania, 1945. Reprinted in *IEEE Annals of the History of Computing*, vol. 15, no. 4, pp. 27-75, 1993. DOI: [10.1109/85.238389](https://doi.org/10.1109/85.238389)

2. J. Backus, "Can Programming Be Liberated from the von Neumann Style? A Functional Style and Its Algebra of Programs," *Communications of the ACM*, vol. 21, no. 8, pp. 613-641, 1978. DOI: [10.1145/359576.359579](https://doi.org/10.1145/359576.359579)

### Dataflow Machines

3. J. B. Dennis, "First Version of a Data Flow Procedure Language," in *Programming Symposium*, Lecture Notes in Computer Science, vol. 19, pp. 362-376, Springer, 1974. DOI: [10.1007/3-540-06859-7_145](https://doi.org/10.1007/3-540-06859-7_145)

4. J. R. Gurd, C. C. Kirkham, and I. Watson, "The Manchester Prototype Dataflow Computer," *Communications of the ACM*, vol. 28, no. 1, pp. 34-52, 1985. DOI: [10.1145/2465.2468](https://doi.org/10.1145/2465.2468)

5. Arvind and D. E. Culler, "Dataflow Architectures," *Annual Reviews in Computer Science*, vol. 1, pp. 225-253, 1986. DOI: [10.1146/annurev.cs.01.060186.001301](https://doi.org/10.1146/annurev.cs.01.060186.001301)

### Compiler Infrastructure

6. J. W. Backus et al., "The FORTRAN Automatic Coding System," in *Proceedings of the Western Joint Computer Conference*, pp. 188-198, IRE-AIEE-ACM, 1957. DOI: [10.1145/1455567.1455599](https://doi.org/10.1145/1455567.1455599)

7. C. Lattner and V. Adve, "LLVM: A Compilation Framework for Lifelong Program Analysis & Transformation," in *Proc. CGO '04*, pp. 75-86, IEEE, 2004. DOI: [10.1109/CGO.2004.1281665](https://doi.org/10.1109/CGO.2004.1281665)

8. C. Lattner et al., "MLIR: Scaling Compiler Infrastructure for Domain Specific Computation," in *Proc. CGO '21*, pp. 2-14, IEEE, 2021. DOI: [10.1109/CGO51591.2021.9370308](https://doi.org/10.1109/CGO51591.2021.9370308)

### Task Parallelism and Scheduling

9. R. D. Blumofe and C. E. Leiserson, "Scheduling Multithreaded Computations by Work Stealing," *JACM*, vol. 46, no. 5, pp. 720-748, 1999. DOI: [10.1145/324133.324234](https://doi.org/10.1145/324133.324234)

10. R. D. Blumofe, C. F. Joerg, B. C. Kuszmaul, C. E. Leiserson, K. H. Randall, and Y. Zhou, "Cilk: An Efficient Multithreaded Runtime System," in *Proc. PPoPP '95*, pp. 207-216, ACM, 1995. DOI: [10.1145/209936.209958](https://doi.org/10.1145/209936.209958)

### GPU Computing

11. J. Nickolls and W. J. Dally, "The GPU Computing Era," *IEEE Micro*, vol. 30, no. 2, pp. 56-69, 2010. DOI: [10.1109/MM.2010.41](https://doi.org/10.1109/MM.2010.41)

### Codelet Execution Model

12. G. R. Gao, R. Patel, and T. St. John, "The Codelet Program Execution Model," presented at *WiA, ISCA '13*, Tel-Aviv, Israel, 2013.

### Agent Models

13. A. S. Rao and M. P. Georgeff, "BDI Agents: From Theory to Practice," in *Proc. ICMAS '95*, pp. 312-319, AAAI Press, 1995.

### A-PXM

14. APXM Contributors, "A-PXM: An Agent Program Execution Model," unpublished manuscript, 2025-2026.
