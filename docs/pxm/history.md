---
title: "History Repeats: From Von Neumann to A-PXM"
description: "The historical narrative of Program Execution Models -- how the same structural problems that plagued early computing now plague agentic AI, and why the same class of solutions apply again."
---

# History Repeats: From Von Neumann to A-PXM

Every major shift in computing has followed the same arc: a powerful new capability arrives, practitioners wire it together by hand, the resulting systems become opaque and unmanageable, and then someone builds the formal infrastructure -- an execution model, a compiler, a runtime -- that makes the capability programmable at scale. This has happened at least five times in the last eighty years. It is happening again now with agentic AI.

---

## 1. The First Time: Von Neumann and the Stored-Program Computer

In June 1945, John von Neumann circulated "First Draft of a Report on the EDVAC." It described a machine that stored its program in the same memory as its data, executing instructions sequentially under the control of a program counter. Before this, "programming" meant rewiring patch cables on the ENIAC. There was no abstraction between the problem and the hardware.

Von Neumann's insight was structural. He defined an **abstract machine** -- a program counter, an accumulator, a memory -- and a contract: if you express your computation as a sequence of instructions that this abstract machine understands, the hardware will execute it faithfully. This was the first Program Execution Model: a formal specification of how programs are represented, what state they operate on, and how execution proceeds. It separated the *what* (the program) from the *how* (the hardware), and in doing so created the entire field of software.

---

## 2. The Bottleneck: Backus Sees the Problem

By the 1970s, the von Neumann model had conquered computing so thoroughly that its limitations had become invisible -- mistaken for laws of nature rather than design choices. In his 1977 Turing Award lecture, John Backus named the problem: the **von Neumann bottleneck**.

> "Surely there must be a less primitive way of making big changes in the store than by pushing vast numbers of words back and forth through the von Neumann bottleneck."

The bottleneck was not merely about bandwidth between CPU and memory. It was about **opacity**. The sequential instruction stream hid the structure of the computation from every layer of the system. When the execution model hides structure, no tool in the stack can help.

This diagnosis applies, almost word for word, to agentic AI today. Current agent frameworks execute workflows as opaque Python function calls -- "call-at-a-time" chains where the runtime has no visibility into what operations exist, how they relate, or what state they touch. The [agentic von Neumann bottleneck](foundations.md) is not about CPU-memory bandwidth. It is about the complete absence of structure in the execution model.

---

## 3. The Contract: Instruction Set Architectures

One of the most consequential abstractions in computing history is the **Instruction Set Architecture** (ISA) -- a contract between software and hardware that lets both sides evolve independently. The x86 ISA, defined in 1978, is still executing programs today despite the underlying microarchitecture being redesigned from scratch at least a dozen times. The ISA decouples progress above the interface from progress below it.

A-PXM's [Agent Instruction Set (AIS)](ais.md) is this contract for agentic AI. It defines typed operations -- ASK, THINK, REASON, INV, PLAN, REFLECT, VERIFY, QMEM, UMEM, FENCE, and others -- each with specified inputs, outputs, and state transitions on the [Agent Abstract Machine](aam.md). Above the AIS, any frontend can emit AIS graphs. Below the AIS, any runtime can execute them. (See [foundations.md](foundations.md) for the full ISA contract analysis.)

| Layer | Traditional Computing | Agentic Computing |
|-------|----------------------|-------------------|
| **Software above** | C, Rust, Python source code | Agent workflow definitions (any framework) |
| **Contract** | ISA (x86, ARM, RISC-V) | AIS (ASK, THINK, REASON, INV, ...) |
| **Implementation below** | CPU microarchitecture | A-PXM runtime, future agent hardware |

Without this contract, every framework is vertically integrated: its workflows run only on its own runtime. There is no portability, no shared optimization infrastructure, and no separation of concerns.

---

## 4. The Compiler's Role: From FORTRAN to LLVM to A-PXM

Compilers bridge the gap between **programmability** and **performance**, translating what the developer wants to express into what the machine can efficiently execute.

### FORTRAN: The First Bridge (1957)

When Backus released FORTRAN in 1957, the prevailing wisdom was that no automatic translator could match hand-written assembly. FORTRAN proved them wrong -- not by generating perfect code, but by generating code that was *good enough* while being orders of magnitude faster to write.

### LLVM: The Universal Infrastructure (2004)

Chris Lattner and Vikram Adve showed that compiler infrastructure could be **shared**. Instead of every language building its own compiler, languages emit a common IR (LLVM IR), and a shared optimizer handles the rest. The result was explosive: Clang, Rust, Swift, Julia, and dozens of others compile through LLVM. Each language team focuses on frontend design; LLVM handles optimization and code generation.

### A-PXM: The Agent Compiler (Now)

No compiler exists for agentic AI. When a developer writes a LangChain workflow or a CrewAI crew, there is no compilation step. The Python code executes directly -- every LLM call is opaque, every dependency implicit, every optimization opportunity invisible.

A-PXM provides the missing compiler. Agent workflows are expressed as AIS dataflow graphs. The compiler, built on MLIR, performs typed analysis and optimization. The current pass pipeline is documented in [compiler/pipeline.md](../compiler/pipeline.md).

The economics are different from traditional compilation -- and far more favorable. In traditional compilers, eliminating one instruction saves nanoseconds. In A-PXM, eliminating one LLM call saves **seconds and dollars**. The return on optimization is nine orders of magnitude higher per operation.

---

## 5. The Runtime's Role: Implementing Abstract Machines

A compiler translates programs. A runtime **executes** them -- it provides the concrete mechanisms for the state model, execution semantics, and resource management that the abstract machine specifies.

| Model | Abstract Machine | Runtime Implementation |
|-------|-----------------|----------------------|
| Von Neumann | PC + registers + memory | CPU silicon |
| JVM (1995) | Stack-based bytecode + GC | HotSpot / GraalVM |
| CUDA (2007) | Grid of SIMT thread blocks | GPU hardware + CUDA runtime |
| **A-PXM** | **[AAM](aam.md): (Beliefs, Goals, Capabilities)** | **Dataflow scheduler + three-tier memory** |

The AAM state triple is the formal definition of what an agent *is* at any point in time. Every AIS instruction is a deterministic state transition: `d(AAM, Instr) -> AAM'`. The runtime maintains this state, enforces its invariants, and provides the dataflow [scheduler](scheduling.md) that determines when operations fire.

This is not a metaphor. The AAM is to A-PXM what the von Neumann machine is to x86. Without it, "agent state" is whatever happens to be in Python closures and global variables.

---

## 6. The PXM Lineage: Five Decades of Closing the Gap

Each PXM was designed for the dominant computational challenge of its era:

| PXM | Era | Challenge | Key Abstraction |
|-----|-----|-----------|-----------------|
| Von Neumann | 1945 | Making computation possible | Program counter + stored program |
| Dataflow | 1975 | Exploiting parallelism automatically | Token-driven firing rule |
| Cilk | 1995 | Efficient task parallelism | Work-stealing scheduler |
| CUDA | 2007 | Programmable massively parallel hardware | SIMT threads + memory hierarchy |
| Codelet | 2013 | Energy-efficient exascale computing | Fine-grained non-preemptable codelets |
| **A-PXM** | **2025** | **Orchestrating autonomous agents** | **Typed AIS operations + AAM state + dataflow scheduling** |

The pattern is consistent: each PXM provides the abstractions that make a new class of computation **visible** to tooling. A-PXM does this for agent workflows. The [five separations](foundations.md) (Compute, Memory, State, Optimization, Scheduling) are the concrete form this visibility takes.

---

## 7. The Wrong Abstraction Layer

There is a final historical parallel. In the 1950s, before compilers existed, programmers wrote assembly by hand. They were productive -- they built real systems. But they were building at the wrong layer. Every program was coupled to a specific machine. Every optimization was manual. Every reuse required reimplementation.

FORTRAN did not make assembly programmers obsolete. It made them unnecessary for most tasks by providing a higher-level abstraction backed by formal infrastructure. The result was not just convenience -- it was a qualitative change in what was possible.

Current agent frameworks -- LangChain, CrewAI, AutoGen, LangGraph -- are building at the application layer. They provide excellent developer experience. But beneath them, there is no formal execution model. No typed IR. No compiler. No optimizer. No abstract machine.

This means:
- **No portability.** A LangChain workflow cannot run on CrewAI's runtime.
- **No shared optimization.** Each framework implements its own ad-hoc caching.
- **No formal verification.** Type errors surface at runtime.
- **No auditing.** Wrong answers require reading stack traces.

These are not limitations of specific frameworks. They are consequences of the missing infrastructure layer. A-PXM provides that layer so frameworks can focus on developer experience while the infrastructure handles correctness, optimization, and scheduling.

---

## The Arc Completes

The history of computing is the history of making computation visible. The von Neumann model made hardware programmable. ISAs made software portable. Compilers made programs optimizable. Runtimes made abstract machines executable.

Agentic AI is at the "wiring it together by hand" stage. The capability -- autonomous agents that reason, plan, use tools, and collaborate -- is here. The formal infrastructure is not. A-PXM is that infrastructure.

---

## Further Reading

- [PXM Foundations](foundations.md) -- the five separations and the ISA contract
- [Agent Abstract Machine (AAM)](aam.md) -- the formal state model
- [Agent Instruction Set (AIS)](ais.md) -- the typed operation taxonomy
- [Compute in PXMs](compute.md) -- how six PXMs define compute
- [Scheduling in PXMs](scheduling.md) -- dataflow scheduling and work stealing
- [Vision](vision.md) -- the LLVM-for-agents strategy

---

## References

### Foundational Architecture

1. J. von Neumann, "First Draft of a Report on the EDVAC," 1945. DOI: [10.1109/85.238389](https://doi.org/10.1109/85.238389)
2. J. Backus, "Can Programming Be Liberated from the von Neumann Style?," *Communications of the ACM*, 1978. DOI: [10.1145/359576.359579](https://doi.org/10.1145/359576.359579)

### Dataflow Machines

3. J. B. Dennis, "First Version of a Data Flow Procedure Language," in *Programming Symposium*, LNCS vol. 19, 1974. DOI: [10.1007/3-540-06859-7_145](https://doi.org/10.1007/3-540-06859-7_145)
4. J. R. Gurd, C. C. Kirkham, and I. Watson, "The Manchester Prototype Dataflow Computer," *Communications of the ACM*, 1985. DOI: [10.1145/2465.2468](https://doi.org/10.1145/2465.2468)
5. Arvind and D. E. Culler, "Dataflow Architectures," *Annual Reviews in Computer Science*, 1986. DOI: [10.1146/annurev.cs.01.060186.001301](https://doi.org/10.1146/annurev.cs.01.060186.001301)

### Compiler Infrastructure

6. J. W. Backus et al., "The FORTRAN Automatic Coding System," in *Proc. Western Joint Computer Conference*, 1957. DOI: [10.1145/1455567.1455599](https://doi.org/10.1145/1455567.1455599)
7. C. Lattner and V. Adve, "LLVM: A Compilation Framework for Lifelong Program Analysis & Transformation," in *Proc. CGO '04*, 2004. DOI: [10.1109/CGO.2004.1281665](https://doi.org/10.1109/CGO.2004.1281665)
8. C. Lattner et al., "MLIR: Scaling Compiler Infrastructure for Domain Specific Computation," in *Proc. CGO '21*, 2021. DOI: [10.1109/CGO51591.2021.9370308](https://doi.org/10.1109/CGO51591.2021.9370308)

### Task Parallelism and Scheduling

9. R. D. Blumofe and C. E. Leiserson, "Scheduling Multithreaded Computations by Work Stealing," *JACM*, 1999. DOI: [10.1145/324133.324234](https://doi.org/10.1145/324133.324234)

### GPU Computing

10. J. Nickolls and W. J. Dally, "The GPU Computing Era," *IEEE Micro*, 2010. DOI: [10.1109/MM.2010.41](https://doi.org/10.1109/MM.2010.41)

### Codelet Execution Model

11. G. R. Gao, R. Patel, and T. St. John, "The Codelet Program Execution Model," presented at *WiA, ISCA '13*, 2013.

### Agent Models

12. A. S. Rao and M. P. Georgeff, "BDI Agents: From Theory to Practice," in *Proc. ICMAS '95*, pp. 312-319, 1995.
