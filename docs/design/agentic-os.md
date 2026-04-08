# APXM as an Agentic Operating System

**Date**: 2026-04-07
**Method**: 16-agent parallel deep investigation (10 OS mapping + 6 session/context/memory)
**Scope**: Full mapping of OS kernel concepts to APXM + session-context-memory coherence analysis

> **See also:**
> - [PXM foundations](../pxm/foundations.md) -- the execution model lineage (von Neumann -> Dataflow -> APXM)
> - [Runtime architecture](../implementation/architecture.md) -- how the "kernel" is structured in code
> - [Dataflow scheduler](../implementation/runtime/dataflow-scheduler.md) -- the process scheduler
> - [Memory hierarchy](../implementation/runtime/memory-hierarchy.md) -- STM/LTM/Episodic tiers
> - [Sessions](../implementation/runtime/sessions.md) -- trace and workspace layout
> - [Agent ontology](agent-ontology.md) -- the AAM (B, G, C) model that the OS executes
> - [vLLM integration](../integrations/vllm.md) -- hardware-abstraction layer for LLM backends

---

## 1. The Thesis

APXM is not merely an agent orchestration framework -- it is a **complete operating system for AI agents**, implementing every major OS subsystem through purpose-built analogues. The whiteboard diagrams capture this insight:

**Diagram 1** (IRQ model): Events/signals -> IRQ -> OS Kernel -> method dispatch -> processes. Agents and human queries are **interrupt sources**, the APXM runtime is the **kernel**, and agent workflows are **processes**.

**Diagram 2** (Fork/exec model): Kernel (Top Agent) -> fork -> child with loader/BSS/PIC -> fd(io)/fopen. The runtime spawns agents via a fork/exec model, with node workspaces as address spaces, skills as shared libraries, and CLAUDE.md as the process image.

**Diagram 3** (OpenMultiAgent): The comparison target -- a typical orchestration framework. APXM already exceeds every component.

---

## 2. The Complete OS-to-APXM Mapping

### 2.1 Kernel Architecture

```
+-------------------------------------------------------------------------+
|                        AGENTIC OS ARCHITECTURE                          |
|                                                                         |
|  +---------------------------------------------------------------+     |
|  |                      USER SPACE                                |     |
|  |                                                                |     |
|  |   .apxm graphs    Python SDK    Rust SDK    (future: adapters) |     |
|  |   "Source code"    "Languages"   "Languages"  "Frontends"      |     |
|  +-------------------------------+-------------------------------+     |
|                                  | syscall interface (AIS ISA)         |
|  +-------------------------------v-------------------------------+     |
|  |                      KERNEL SPACE                              |     |
|  |                                                                |     |
|  |  +------------+  +-------------+  +--------------------------+ |     |
|  |  |  COMPILER  |  |   LOADER    |  |       RUNTIME            | |     |
|  |  |  (MLIR)    |  | (.apxmobj)  |  |                          | |     |
|  |  |            |  |             |  |  +------------------+    | |     |
|  |  | Passes:    |  | MAGIC check |  |  |DataflowScheduler |    | |     |
|  |  | O0 to O3   |  | BLAKE3 hash |  |  |(process sched.)  |    | |     |
|  |  | FuseAskOps |  | DAG extract |  |  +--------+---------+    | |     |
|  |  | CSE, DCE   |  | Flow reg.   |  |           |              | |     |
|  |  | Scheduling |  |             |  |  +--------v---------+    | |     |
|  |  |            |  |             |  |  |  Worker Threads   |    | |     |
|  |  | Emits:     |  |             |  |  |  work-stealing    |    | |     |
|  |  | SecurityMan|  |             |  |  +--------+---------+    | |     |
|  |  | ArtifactBin|  |             |  |           |              | |     |
|  |  +------------+  +-------------+  |  +--------v---------+    | |     |
|  |                                   |  |  Dispatcher       |    | |     |
|  |                                   |  |  (syscall table)  |    | |     |
|  |                                   |  |  41 AIS ops       |    | |     |
|  |                                   |  +------------------+    | |     |
|  |                                   |                          | |     |
|  |  +--------------------------------+-----------------------+  | |     |
|  |  |              KERNEL SUBSYSTEMS                         |  | |     |
|  |  |                                                        |  | |     |
|  |  |  +----------+ +----------+ +--------+ +-----------+   |  | |     |
|  |  |  | Memory   | | Process  | |Sandbox | |Capability |   |  | |     |
|  |  |  | System   | | Table    | |Registry| | System    |   |  | |     |
|  |  |  | (VMM)    | | (PCB)    | |(secur.)| |(drivers)  |   |  | |     |
|  |  |  +----------+ +----------+ +--------+ +-----------+   |  | |     |
|  |  |                                                        |  | |     |
|  |  |  +----------+ +----------+ +--------+ +-----------+   |  | |     |
|  |  |  | AAM      | | LLM      | |Flow    | | Event     |   |  | |     |
|  |  |  | (state)  | | Registry | |Registry| | Emitter   |   |  | |     |
|  |  |  | B,G,C    | |(backends)| |(IPC)   | |(tracing)  |   |  | |     |
|  |  |  +----------+ +----------+ +--------+ +-----------+   |  | |     |
|  |  +--------------------------------------------------------+  | |     |
|  +---------------------------------------------------------------+     |
|                                                                         |
|  +---------------------------------------------------------------+     |
|  |                      HARDWARE ABSTRACTION                      |     |
|  |                                                                |     |
|  |  LLM Backends (OpenAI, Anthropic, vLLM, Ollama)  = "CPUs"     |     |
|  |  Agent Subprocesses (Claude, Codex, Gemini)       = "co-procs" |     |
|  |  MCP Tool Servers                                 = "peripherals"    |
|  |  Filesystem / Network                             = "I/O devices"    |
|  +---------------------------------------------------------------+     |
+-------------------------------------------------------------------------+
```

### 2.2 Subsystem-by-Subsystem Mapping

| OS Subsystem | APXM Component | Key Type | Location |
|---|---|---|---|
| **Kernel executive** | `Runtime` | `Runtime::new()` | `runtime.rs` |
| **Process scheduler** | `DataflowScheduler` | Token-counting dataflow | `scheduler/dataflow.rs` |
| **CPU cores** | Worker threads (N = num_cpus) | `worker_loop()` | `scheduler/worker.rs` |
| **Runqueue** | 4-level `PriorityQueue` | Critical/High/Normal/Low | `scheduler/queue.rs` |
| **Load balancer** | 3-tier work stealing | local->global->peer | `scheduler/work_stealing.rs` |
| **Backpressure / cgroups** | `ConcurrencyControl` | tokio::Semaphore | `scheduler/concurrency_control.rs` |
| **Watchdog** | Deadlock detector | timeout + running-ops check | `dataflow.rs` |
| **IRQ system** | Token readiness propagation | `TokenState {ready, value, consumers}` | `scheduler/internal_state.rs` |
| **Syscall table** | `OperationDispatcher` | 41 AIS operations | `executor/dispatcher.rs` |
| **Process table** | `ProcessTable` | `DashMap<ProcessId, AgentProcess>` | `process_table.rs` |
| **PCB (task_struct)** | `AgentProcess` | pid, ppid, kind, state | `process.rs` |
| **fork() + execve()** | `AcpSession::spawn()` | `Command::spawn()` + ACP init | `acp/session.rs` |
| **Loader (ld.so)** | ACP INITIALIZE + session/new | Protocol handshake | `acp/session.rs` |
| **Shared libraries (.so)** | Skills | `copy_skill_to()` | `session_output.rs` |
| **Process image** | CLAUDE.md / AGENTS.md | `assemble_claude_md()` | `context_assembler.rs` |
| **Virtual memory (MMU)** | `MemorySystem` | scope_id prefix isolation | `memory/mod.rs` |
| **L1-L3 cache** | STM (Short-Term Memory) | In-memory HashMap, O(1) | `memory/stm.rs` |
| **Heap / filesystem** | LTM (Long-Term Memory) | SQLite-backed, persistent | `memory/ltm.rs` |
| **Audit log** | Episodic Memory | VecDeque ring buffer + JSONL | `memory/episodic.rs` |
| **Copy-on-write fork** | `ScopePolicy::Snapshot` | AamContext projection | `core/types/aam.rs` |
| **File descriptors** | Tokens (`TokenId`) | `DashMap<TokenId, TokenState>` | `scheduler/state.rs` |
| **Pipes** | DAG edges | `Edge {from, to, dependency_type}` | `core/types/execution/edge.rs` |
| **Unix sockets** | ACP sessions | stdio transport + JSON-RPC | `acp/session.rs` |
| **Protection rings** | `IsolationLevel` (7 levels) | None->Remote | `sandbox/types.rs` |
| **seccomp / AppArmor** | Sandbox tiers (T0-T3) | Per-op classification | `sandbox/manifest.rs` |
| **rlimits** | Process limits | max=32, depth=4 | `process_table.rs` |
| **Compiler (gcc)** | MLIR pipeline | 9+ passes, O0-O3 | `apxm-compiler/` |
| **ELF binary** | `.apxmobj` artifact | 52-byte header + BLAKE3 + payload | `apxm-artifact/src/lib.rs` |
| **objdump** | `apxm decompile` | Artifact -> graph reverse map | `apxm-artifact/` |

---

## 3. The AIS as Syscall Table

The Agent Instruction Set serves as APXM's syscall interface -- a well-defined ABI between "user space" (graph authors) and "kernel space" (the runtime). See [AIS reference](../pxm/ais.md) for the full specification.

### Process Management (fork, exec, wait, kill)
| AIS Op | Syscall Analogue | What It Does |
|---|---|---|
| `SPAWN_AGENT` | `fork() + execve()` | Create agent subprocess with ACP protocol |
| `SPAWN_TEAM` | `fork() x N` | Spawn all team members |
| `DELEGATE` | `fork() + waitpid()` | Fork task to sub-agent, wait for result |
| `FLOW_CALL` | `fork() + exec() + wait()` | Call named flow on another agent |
| `PAUSE` | `kill(SIGSTOP)` | Suspend execution for human review |
| `RESUME` | `kill(SIGCONT)` | Resume from suspended state |

### Memory Operations (read, write, mmap, mfence)
| AIS Op | Syscall Analogue | What It Does |
|---|---|---|
| `QMEM` | `read()` | Query memory (STM/LTM/Episodic) |
| `UMEM` | `write()` | Update memory |
| `FENCE` | `mfence` | Memory ordering barrier |
| `CHECKPOINT` | `mmap() + sync` | Snapshot AAM state to durable storage |

### IPC (send, recv, pipe, socket)
| AIS Op | Syscall Analogue | What It Does |
|---|---|---|
| `COMMUNICATE` | `sendto() / recvfrom()` | Inter-agent messaging (local/http/acp/broadcast) |
| `NEGOTIATE` | Multi-round `send/recv` | Multi-agent consensus protocol |
| `CLAIM` | `atomic_cmpxchg()` | Atomic task claiming with lease |

### Compute (unique to Agent OS)
| AIS Op | No OS Analogue | What It Does |
|---|---|---|
| `ASK` | -- | Low-latency Q&A (LLM call) |
| `THINK` | -- | Extended reasoning with token budget |
| `REASON` | -- | Structured reasoning with belief/goal updates |
| `PLAN` | -- | Decompose goal into executable subgraph |
| `REFLECT` | -- | Analyze execution trace for self-improvement |
| `VERIFY` | -- | Fact-check outputs against evidence |

### Device/Tool Operations (open, ioctl, dlopen)
| AIS Op | Syscall Analogue | What It Does |
|---|---|---|
| `INV_TOOL` | `dlopen() + dlsym()` | Invoke registered capability |
| `EXC` | `seccomp() + fork() + exec()` | Sandboxed code execution |
| `REGISTER_CAPABILITY` | `dlopen()` | Register capability at runtime |
| `PRINT` | `write(stdout)` | Output to stdout |

### Control Flow (jmp, signal, setjmp)
| AIS Op | Analogue | What It Does |
|---|---|---|
| `JUMP` | `jmp` | Unconditional jump |
| `BRANCH_ON_VALUE` | `je/jne` | Conditional branch |
| `SWITCH` | Jump table | Multi-way branch |
| `MERGE` | `waitpid() x N` | Join parallel branches |
| `WAIT_ALL` | `pthread_join() x N` | Block until all inputs ready |
| `TRY_CATCH` | `setjmp/longjmp` | Structured exception handling |
| `GUARD` | `assert()` | Precondition enforcement |
| `UPDATE_GOAL` | `setpriority()` | Modify runtime goals/priorities |

---

## 4. The Fork/Exec Model for Agent Spawning

The whiteboard's "Kernel (Top Agent) -> Fork -> child (Loader, BSS, PIC)" maps to the SPAWN_AGENT handler:

```
APXM FORK/EXEC PIPELINE
========================

1. FORK PHASE  [spawn_agent.rs]
   +-- Check spawn limits (max=32, depth=4)     <- ulimit -u
   +-- Resolve parent PID                        <- getppid()
   +-- Record in AAM beliefs                     <- /proc/pid/status
   +-- Command::new(profile.command).spawn()     <- fork() + execve()

2. LOADER PHASE  [session.rs]
   +-- Extract stdio (stdin/stdout/stderr)       <- fd 0,1,2
   +-- ACP INITIALIZE handshake                  <- ld.so entry point
   +-- Protocol version negotiation              <- ELF interp check
   +-- session/new with capabilities             <- dlopen() shared libs

3. ADDRESS SPACE SETUP  [session_output.rs]
   +-- Create node workspace directory           <- mmap(PROT_READ|WRITE)
   +-- Write node.json metadata                  <- .data segment
   +-- Copy skills to workspace                  <- .so library loading
   +-- Generate CLAUDE.md / AGENTS.md            <- Process image

4. STACK FRAME INIT  [spawn_agent.rs]
   +-- project_aam_context()                     <- Copy parent registers
   |   +-- Filter beliefs (drop _-prefixed)      <- Strip kernel state
   |   +-- Copy active goals only                <- Copy instruction pointer
   |   +-- Extract capabilities list             <- Copy capability set
   +-- render_system_prompt()                    <- Format argv[] + environ[]

5. ENTRY POINT  [session.rs]
   +-- send_system_preamble() (turn 0)           <- _start -> __libc_start_main
   +-- Agent begins executing                    <- main() entered

6. PROCESS REGISTRATION  [process_table.rs]
   +-- register_external(name, parent, session)  <- Add to kernel proc table
   +-- Return ProcessId (UUID v7)                <- Return child PID
```

---

## 5. Memory Architecture

```
AGENTIC MEMORY HIERARCHY
=========================

    Registers     <->  Tokens (TokenId -> Value in DashMap)
         |              O(1) access, per-execution lifetime
         |
    L1-L3 Cache   <->  STM (Short-Term Memory)
         |              In-memory HashMap, scoped by scope_id
         |              max_entries: 1024 (bounded like stack)
         |
    Heap / RAM    <->  LTM (Long-Term Memory)
         |              SQLite-backed, persistent across executions
         |              Unbounded, supports vector similarity search
         |
    Audit Log     <->  Episodic Memory
         |              Ring buffer (10K entries) + JSONL file
         |              Immutable, append-only, for REFLECT queries
         |
    Process State <->  AAM (Agent Abstract Machine)
         |              Beliefs = heap (mutable state)
         |              Goals = scheduler priority queue
         |              Capabilities = permission bits
         |              Transitions = audit trail
         |
    Address Space <->  Scope Isolation
                        __scope__/{scope_id}/{key} prefix
                        ScopePolicy: Inherit | Isolate | Snapshot | Filter
                        Snapshot = copy-on-write fork semantics
```

See [memory hierarchy](../implementation/runtime/memory-hierarchy.md) for the implementation.

---

## 6. Security Model

```
AGENTIC SECURITY LAYERS
========================

Layer 0: COMPILE-TIME (unique to APXM -- no prior art has this)
    SecurityManifest generated from static analysis of graph
    +-- Per-op sandbox tier classification (T0/T1/T2/T3)
    +-- needs_network, needs_filesystem_write, needs_process_spawn
    +-- min_isolation: IsolationLevel computed from max tier

Layer 1: PROCESS ISOLATION (like namespaces/cgroups)
    +-- Scope-based memory isolation (scope_id prefix = page table)
    +-- ProcessTable depth limits (max_spawn_depth=4 = fork bomb prevention)
    +-- ProcessTable count limits (max_processes=32 = ulimit -u)

Layer 2: PERMISSION MODEL (like DAC + MAC)
    +-- PermissionMode per agent: ApproveAll / ApproveReads / DenyAll
    +-- CapabilityInterceptor: pre/post hooks with Allow/Deny/EditArgs
    +-- ApprovalStore: cached decisions with Once/Session/Always scope

Layer 3: SANDBOX EXECUTION (like seccomp/containers)
    IsolationLevel enum (7 levels):
    +-- 0: None          <- Ring 3 (untrusted dev)
    +-- 1: PolicyOnly    <- Application ACL
    +-- 2: OsLevel       <- seccomp/Landlock
    +-- 3: Container     <- Docker/bubblewrap
    +-- 4: Hypervisor    <- Firecracker/gVisor
    +-- 5: Wasm          <- Wasmtime capability model
    +-- 6: Remote        <- Air-gapped cloud sandbox
```

---

## 7. Prior Art Comparison

| Capability | AIOS (Rutgers) | Agent-OS Blueprint | Karpathy LLM OS | DSPy | LangGraph et al. | DynTaskMAS | **APXM** |
|---|---|---|---|---|---|---|---|
| Real compiler (MLIR) | No | No | No | Prompt opt. | No | No | **Yes** |
| Typed ISA | No | Conceptual | No | Signatures | No | No | **Yes -- 41 ops** |
| Artifact format | No | No | No | No | No | No | **Yes -- .apxmobj** |
| Compile-time type checking | No | No | No | No | No | No | **Yes** |
| Formal abstract machine | No | No | No | No | No | No | **Yes -- AAM(B,G,C)** |
| Dataflow scheduling O(1) | No | No | No | No | No | Dynamic graphs | **Yes** |
| 3-tier memory (STM/LTM/Episodic) | Basic | Requirements | No | No | No | Semantic | **Yes** |
| Process model (ProcessTable) | Basic | Requirements | No | No | No | No | **Yes** |
| Compile-time sandbox classification | No | No | No | No | No | No | **Yes -- T0-T3** |
| Session tracing + replay | No | Requirements | No | No | Checkpointing | No | **Yes** |

### The Core Novelty

APXM is the **only system that treats agent workflow execution as a compiler problem**. Every other "Agent OS" operates at the runtime level. None compile agent workflows into an intermediate representation, apply optimization passes, or emit portable artifacts.

The economic argument: **in traditional compilers, eliminating one instruction saves nanoseconds; in APXM, eliminating one LLM call saves seconds and dollars.**

Three specific novel contributions:

1. **ISA contract as ecosystem enabler.** AIS is the x86/ARM equivalent for agents -- a contract decoupling frontends from backends.

2. **Compile-time verification of agent workflows.** Type errors, missing dependencies, and malformed state transitions are caught *before* execution, before any expensive LLM calls.

3. **Formal PXM lineage.** APXM explicitly positions itself in the 80-year lineage: von Neumann -> Dataflow -> Cilk -> CUDA -> Codelet -> A-PXM. See [PXM history](../pxm/history.md).

---

## 8. The Cache Coherence Problem: Session / Context / Memory

The most critical gap in APXM's agentic OS is the **absence of a unified state coherence protocol** between its three data tracks. In a traditional OS, the MMU + TLB + cache coherence protocol (MESI/MOESI) ensure a consistent view. In APXM, three independent systems record execution state with no shared event ID and no cross-referencing.

### 8.1 The Three Independent Tracks

```
TRACK 1: DATAFLOW TOKENS (In-Memory, Volatile)
  Location: SchedulerState.tokens (DashMap<TokenId, TokenState>)
  Writer:   worker.rs publish_outputs()
  Lifetime: Single execution (LOST after completion)
  Content:  Full Value for every token

TRACK 2: SESSION FILES (Disk, Per-Node Workspaces)
  Location: ~/.apxm/sessions/<exec-id>/nodes/<id>_<name>/
  Writer:   SessionEventEmitter.emit_node_output()
  Lifetime: Persists indefinitely (never auto-deleted)
  Content:  output.json, prompt.txt, response.txt, status.json, CLAUDE.md

TRACK 3: EPISODIC MEMORY (Ring Buffer + JSONL)
  Location: VecDeque<EpisodicEntry> + ~/.apxm/memory/episodes.jsonl
  Writer:   worker.rs record_event()
  Lifetime: Ring buffer (10K entries, FIFO eviction)
  Content:  SPARSE metadata only (node_id, op_type, duration, retries)
            *** NO NODE OUTPUT VALUES ***
```

### 8.2 The Disconnections

Tracks 1 and 2 are connected (token publication writes to session files). Tracks 2 and 3 are NOT connected -- session files and episodic memory share no event ID. ContextAssembler reads only from session files; it cannot query episodic memory.

### 8.3 What This Means in OS Terms

| OS Concept | What APXM Has | What's Missing |
|---|---|---|
| **Cache coherence (MESI)** | Three independent caches | No coherence protocol between tracks |
| **Page table** | scope_id prefix isolation | No unified address-to-track mapping |
| **TLB** | DashMap tokens (fast, volatile) | No TLB miss -> session file fallback |
| **Write-back cache** | Tokens write to session files | Episodic memory never gets output values |
| **Swap space** | Session files persist | No way to reload session -> tokens |
| **/proc filesystem** | Session trace.ndjson | No live view into episodic entries |
| **Core dump** | AAM checkpoints (manual) | No auto-checkpoint on crash |

### 8.4 Concrete Impact

1. **ContextAssembler can only read session files** -- if `--emit-session` is not used, no upstream outputs are available for CLAUDE.md assembly. Agents spawn blind.

2. **Episodic memory is useless for output queries** -- REFLECT can see "node 3 completed ASK in 2.1s" but NOT what the ASK returned.

3. **No cross-execution learning** -- previous execution outputs are in session files, but episodic memory (which persists globally) does not contain them.

4. **No session resumption** -- session folders are audit trails, not recovery points.

5. **ACP session state is volatile** -- turn count, conversation history, and agent state are in-memory only.

### 8.5 The Fix: Unified Event Pipeline (the "MMU")

The agentic OS needs a coherence protocol -- a single event path that feeds all three tracks:

```
Operation completes
  |
  v
UnifiedEventPipeline (the "MMU")
  |
  +--> Track 1: Token publication (DashMap, volatile)
  |
  +--> Track 2: Session files (disk, persistent)
  |
  +--> Track 3: Episodic memory (ring buffer, with output values)
  |
  +--> Track 4: EventBus (subscribers)

Properties:
  - Single event_id shared across ALL four sinks
  - Episodic entries include full output values (not just metadata)
  - Episodic entries include back-pointer to session directory
  - ContextAssembler can read from BOTH session files AND episodic memory
```

---

## 9. Architecture Decision: Runtime IS the Kernel

APXM already implements every kernel subsystem. No additional layer is needed. What IS needed:

1. **Framing** -- position the existing runtime explicitly as an OS kernel
2. **Filling gaps** -- the missing coherence protocol (Section 8)
3. **API surface** -- expose kernel capabilities through a clean "shell" interface

A "shell" layer above the kernel would provide:

```
+----------------------------------------+
|  SHELL (CLI / Server / SDK)            |
|                                        |
|  apxm execute  = run a "program"       |
|  apxm compile  = compile source        |
|  apxm ps       = list processes (NEW)  |
|  apxm top      = live scheduler view   |
|  apxm kill     = cancel execution      |
|  apxm mount    = register backend      |
|  apxm replay   = replay session        |
|  apxm doctor   = kernel health check   |
|                                        |
|  Server: HTTP API for remote access    |
|  SDK: Programmatic kernel access       |
+----------------------------------------+
```

---

## 10. Missing OS Concepts and Priority Gaps

### P0: Unified Event Pipeline (Cache Coherence Protocol)

**OS analogue**: MESI/MOESI cache coherence + write-back protocol

**Current state**: Three independent writers with no shared event ID. Episodic entries contain NO output values.

**Proposal**: `UnifiedEventPipeline` that assigns a single `event_id` per operation completion and feeds all four sinks.

### P0: Enrich EpisodicEntry with Output Values

Add `node_id: Option<u64>`, `output: Option<Value>`, `session_dir: Option<PathBuf>` to episodic entries.

### P1: ContextAssembler Reads Memory (Not Just Files)

Also query episodic memory and STM for richer context, with budget allocation across sources.

### P1: Session Resumption (Process Restart from Checkpoint)

`apxm resume <session-dir> --from-node <N>` that loads AAM checkpoint from session and resumes scheduler.

### P1: ACP Session State Persistence

Persist ACP session metadata to session directory. On agent crash, auto-respawn with context replay.

### P2: Cross-Execution Agent Pool (Warm Process Pool)

`AgentPool` struct that persists across executions, eliminating cold-start latency (2-10s per agent).

### P2: Inter-Execution IPC (Daemons / Services)

APXM server mode (`apxm serve`) as a persistent daemon holding the AgentPool across client connections.

### P3: Filesystem Abstraction for Memory

Expose memory as a VFS-like hierarchy (`/stm/`, `/ltm/`, `/episodic/`, `/tokens/`).

### P3: Signal Handling (Async Events)

Agent signal system: `SIGPAUSE`, `SIGPRIORITY`, `SIGMEMORY`, `SIGGOAL`.

### P3: Unified Event Bus (Wire or Remove)

Wire EventBus as unified fan-out alongside SessionEventEmitter, enabling external consumers.

---

## 11. What This Means

APXM is not evolving *toward* an Agentic OS -- **it already is one**. The architecture emerged organically from solving real agent orchestration problems, and it converged on the same abstractions that OS kernels use because those abstractions are the correct solution to:

1. **Resource multiplexing** -- multiple agents sharing LLM backends, memory, tools
2. **Process isolation** -- agents that cannot corrupt each other's state
3. **Scheduling** -- parallel execution with dependency management
4. **Security** -- graduated trust levels for different operations
5. **Portability** -- compile once, run on any conforming runtime

The kernel is built. Now we need the shell.
