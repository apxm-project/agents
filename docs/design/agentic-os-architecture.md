# APXM as an Agentic Operating System

**Date**: 2026-04-07
**Method**: 16-agent parallel deep investigation (10 OS mapping + 6 session/context/memory)
**Scope**: Full mapping of OS kernel concepts to APXM + session-context-memory coherence analysis

---

## 1. The Thesis

APXM is not merely an agent orchestration framework — it is a **complete operating system for AI agents**, implementing every major OS subsystem through purpose-built analogues. The whiteboard diagrams capture this insight precisely:

**Diagram 1** (IRQ model): Events/signals → IRQ → OS Kernel → method dispatch → processes. Agents and human queries are **interrupt sources**, the APXM runtime is the **kernel**, and agent workflows are **processes**.

**Diagram 2** (Fork/exec model): Kernel (Top Agent) → fork → child with loader/BSS/PIC → fd(io)/fopen. The runtime spawns agents via a fork/exec model, with node workspaces as address spaces, skills as shared libraries, and CLAUDE.md as the process image.

**Diagram 3** (OpenMultiAgent): The comparison target — a typical orchestration framework. APXM already exceeds every component.

---

## 2. The Complete OS ↔ APXM Mapping

### 2.1 Kernel Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        AGENTIC OS ARCHITECTURE                         │
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                      USER SPACE                                  │   │
│  │                                                                  │   │
│  │   .apxm graphs    Python SDK    Rust SDK    (future: LangGraph)  │   │
│  │   ──────────────────────────────────────────────────────────────  │   │
│  │   "Source code"    "Languages"   "Languages"  "Frontend adapters"│   │
│  └──────────────────────────────┬──────────────────────────────────┘   │
│                                 │ syscall interface (AIS ISA)          │
│  ┌──────────────────────────────▼──────────────────────────────────┐   │
│  │                      KERNEL SPACE                                │   │
│  │                                                                  │   │
│  │  ┌────────────┐  ┌─────────────┐  ┌──────────────────────────┐  │   │
│  │  │  COMPILER   │  │   LOADER    │  │       RUNTIME            │  │   │
│  │  │  (MLIR)     │  │ (.apxmobj)  │  │                          │  │   │
│  │  │             │  │             │  │  ┌──────────────────┐    │  │   │
│  │  │ Passes:     │  │ MAGIC check │  │  │ DataflowScheduler│    │  │   │
│  │  │ -O0 to -O3  │  │ BLAKE3 hash │  │  │ (process sched.) │    │  │   │
│  │  │ FuseAskOps  │  │ DAG extract │  │  └────────┬─────────┘    │  │   │
│  │  │ CSE, DCE    │  │ Flow reg.   │  │           │              │  │   │
│  │  │ Scheduling  │  │             │  │  ┌────────▼─────────┐    │  │   │
│  │  │             │  │             │  │  │  Worker Threads   │    │  │   │
│  │  │ Emits:      │  │             │  │  │  (CPU cores)      │    │  │   │
│  │  │ SecurityMan │  │             │  │  │  work-stealing    │    │  │   │
│  │  │ ArtifactBin │  │             │  │  └────────┬─────────┘    │  │   │
│  │  └────────────┘  └─────────────┘  │           │              │  │   │
│  │                                    │  ┌────────▼─────────┐    │  │   │
│  │                                    │  │  Dispatcher       │    │  │   │
│  │                                    │  │  (syscall table)  │    │  │   │
│  │                                    │  │  41 AIS ops       │    │  │   │
│  │                                    │  └──────────────────┘    │  │   │
│  │                                    │                          │  │   │
│  │  ┌─────────────────────────────────┴─────────────────────┐    │  │   │
│  │  │              KERNEL SUBSYSTEMS                         │    │  │   │
│  │  │                                                        │    │  │   │
│  │  │  ┌──────────┐ ┌──────────┐ ┌────────┐ ┌───────────┐  │    │  │   │
│  │  │  │ Memory   │ │ Process  │ │Sandbox │ │Capability │  │    │  │   │
│  │  │  │ System   │ │ Table    │ │Registry│ │ System    │  │    │  │   │
│  │  │  │ (VMM)    │ │ (PCB)    │ │(secur.)│ │(drivers)  │  │    │  │   │
│  │  │  └──────────┘ └──────────┘ └────────┘ └───────────┘  │    │  │   │
│  │  │                                                        │    │  │   │
│  │  │  ┌──────────┐ ┌──────────┐ ┌────────┐ ┌───────────┐  │    │  │   │
│  │  │  │ AAM      │ │ LLM      │ │Flow    │ │ Event     │  │    │  │   │
│  │  │  │ (state)  │ │ Registry │ │Registry│ │ Emitter   │  │    │  │   │
│  │  │  │ B,G,C    │ │(backends)│ │(IPC)   │ │(tracing)  │  │    │  │   │
│  │  │  └──────────┘ └──────────┘ └────────┘ └───────────┘  │    │  │   │
│  │  └────────────────────────────────────────────────────────┘    │  │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                      HARDWARE ABSTRACTION                        │   │
│  │                                                                  │   │
│  │  LLM Backends (OpenAI, Anthropic, vLLM, Ollama)  = "CPUs"       │   │
│  │  Agent Subprocesses (Claude, Codex, Gemini)       = "co-procs"   │   │
│  │  MCP Tool Servers                                 = "peripherals"│   │
│  │  Filesystem / Network                             = "I/O devices"│   │
│  └─────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
```

### 2.2 Subsystem-by-Subsystem Mapping

| OS Subsystem | APXM Component | Key Type | Location |
|---|---|---|---|
| **Kernel executive** | `Runtime` | `Runtime::new()` | `runtime.rs:107-158` |
| **Kernel boot** | `Runtime::new()` | Memory + scheduler + PCB init | `runtime.rs:107-158` |
| **Process scheduler** | `DataflowScheduler` | Token-counting dataflow | `scheduler/dataflow.rs:40-156` |
| **CPU cores** | Worker threads (N = num_cpus) | `worker_loop()` | `scheduler/worker.rs:31-171` |
| **Runqueue** | 4-level `PriorityQueue` | Critical/High/Normal/Low | `scheduler/queue.rs:11-96` |
| **Load balancer** | 3-tier work stealing | local→global→peer | `scheduler/work_stealing.rs:56-125` |
| **Backpressure / cgroups** | `ConcurrencyControl` | tokio::Semaphore | `scheduler/concurrency_control.rs` |
| **Watchdog** | Deadlock detector | timeout + running-ops check | `dataflow.rs:225-276` |
| **IRQ system** | Token readiness propagation | `TokenState {ready, value, consumers}` | `scheduler/internal_state.rs` |
| **Syscall table** | `OperationDispatcher` | 41 AIS operations | `executor/dispatcher.rs` |
| **Syscall handlers** | Operation handlers (40 files) | `execute(ctx, node, inputs)` | `executor/handlers/` |
| **Process table** | `ProcessTable` | `DashMap<ProcessId, AgentProcess>` | `process_table.rs:50-340` |
| **PCB (task_struct)** | `AgentProcess` | pid, ppid, kind, state | `process.rs:17-30` |
| **Thread (LWP)** | `AgentThread` | tid, node_id, state | `thread.rs:14-27` |
| **fork() + execve()** | `AcpSession::spawn()` | `Command::spawn()` + ACP init | `acp/session.rs:51-79` |
| **Loader (ld.so)** | ACP INITIALIZE + session/new | Protocol handshake | `acp/session.rs:94-151` |
| **BSS/data segment** | Environment injection | `cmd.env()` + APXM_NODE_WORKSPACE | `spawn_agent.rs:142-154` |
| **Stack frame** | AAM context projection | `project_aam_context()` | `spawn_agent.rs:251-301` |
| **Entry point (_start)** | System preamble (turn 0) | `send_system_preamble()` | `acp/session.rs:176-188` |
| **Shared libraries (.so)** | Skills | `copy_skill_to()` | `session_output.rs:444-456` |
| **Process image** | CLAUDE.md / AGENTS.md | `assemble_claude_md()` | `context_assembler.rs:47-131` |
| **Virtual memory (MMU)** | `MemorySystem` | scope_id prefix isolation | `memory/mod.rs:88-96` |
| **L1-L3 cache** | STM (Short-Term Memory) | In-memory HashMap, O(1) | `memory/stm.rs` |
| **Heap / filesystem** | LTM (Long-Term Memory) | SQLite-backed, persistent | `memory/ltm.rs` |
| **Audit log** | Episodic Memory | VecDeque ring buffer + JSONL | `memory/episodic.rs` |
| **Page table / ASID** | `scope_id` + `scoped_key()` | `__scope__/{id}/{key}` | `memory/mod.rs` |
| **Copy-on-write fork** | `ScopePolicy::Snapshot` | AamContext projection | `core/types/aam.rs` |
| **File descriptors** | Tokens (`TokenId`) | `DashMap<TokenId, TokenState>` | `scheduler/state.rs` |
| **Pipes** | DAG edges | `Edge {from, to, dependency_type}` | `core/types/execution/edge.rs` |
| **Unix sockets** | ACP sessions | stdio transport + JSON-RPC | `acp/session.rs` |
| **Named pipes (FIFO)** | FlowRegistry | `(agent, flow) → DAG` | `capability/flow_registry.rs` |
| **Device drivers** | CapabilitySystem | Tool registry + approval | `capability/mod.rs` |
| **ioctl()** | INV_TOOL operation | Capability invocation | `handlers/inv_tool.rs` |
| **Protection rings** | `IsolationLevel` (7 levels) | None→Remote | `sandbox/types.rs` |
| **seccomp / AppArmor** | Sandbox tiers (T0-T3) | Per-op classification | `sandbox/manifest.rs` |
| **DAC (rwx)** | `PermissionMode` | ApproveAll/ApproveReads/DenyAll | `acp/registry.rs` |
| **MAC / SELinux** | `CapabilityInterceptor` | pre/post hooks + EditArgs | `capability/interceptor.rs` |
| **rlimits** | Process limits | max=32, depth=4 | `process_table.rs:307-316` |
| **Compiler (gcc)** | MLIR pipeline | 9+ passes, O0-O3 | `apxm-compiler/` |
| **ELF binary** | `.apxmobj` artifact | 52-byte header + BLAKE3 + payload | `apxm-artifact/src/lib.rs` |
| **Instruction encoding** | Wire format | `AISOperationType::from_wire_index()` | `ais/operations/definitions.rs` |
| **objdump** | `apxm decompile` | Artifact → graph reverse map | `apxm-artifact/` |
| **CPU scheduler (CFS)** | ModelRouter | Cost/Latency/Quality routing | `model_router/mod.rs` |
| **Process health** | Circuit breaker | Closed→Open→HalfOpen | `model_router/health.rs` |
| **cgroup quotas** | RateLimiter | Token bucket per backend | `llm/rate_limit.rs` |
| **mlock / page pinning** | KV-cache pinning | vLLM graph-aware hints | `llm/backends/vllm/graph_meta.rs` |
| **Checkpointing** | `AamCheckpoint` | Serialize to disk | `aam/mod.rs` |
| **Session tracing** | `--emit-session` | trace.ndjson + live.json | `session_output.rs` |
| **Replay** | `apxm replay` | Timeline from trace | CLI |

---

## 3. The AIS as Syscall Table

The Agent Instruction Set serves as APXM's syscall interface — a well-defined ABI between "user space" (graph authors) and "kernel space" (the runtime). Organized by OS syscall category:

### Process Management (fork, exec, wait, kill)
| AIS Op | Syscall Analogue | What It Does |
|---|---|---|
| `SPAWN_AGENT` | `fork() + execve()` | Create agent subprocess with ACP protocol |
| `SPAWN_TEAM` | `fork() × N` | Spawn all team members |
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
| `ASK` | — | Low-latency Q&A (LLM call) |
| `THINK` | — | Extended reasoning with token budget |
| `REASON` | — | Structured reasoning with belief/goal updates |
| `PLAN` | — | Decompose goal into executable subgraph |
| `REFLECT` | — | Analyze execution trace for self-improvement |
| `VERIFY` | — | Fact-check outputs against evidence |

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
| `MERGE` | `waitpid() × N` | Join parallel branches |
| `WAIT_ALL` | `pthread_join() × N` | Block until all inputs ready |
| `TRY_CATCH` | `setjmp/longjmp` | Structured exception handling |
| `GUARD` | `assert()` | Precondition enforcement |
| `UPDATE_GOAL` | `setpriority()` | Modify runtime goals/priorities |

---

## 4. The Fork/Exec Model for Agent Spawning

The whiteboard's "Kernel (Top Agent) → Fork → child (Loader, BSS, PIC)" maps precisely to the SPAWN_AGENT handler:

```
APXM FORK/EXEC PIPELINE
========================

1. FORK PHASE  [spawn_agent.rs:156-166]
   ├── Check spawn limits (max=32, depth=4)     ← ulimit -u
   ├── Resolve parent PID                        ← getppid()
   ├── Record in AAM beliefs                     ← /proc/pid/status
   └── Command::new(profile.command).spawn()     ← fork() + execve()

2. LOADER PHASE  [session.rs:90-151]
   ├── Extract stdio (stdin/stdout/stderr)       ← fd 0,1,2
   ├── ACP INITIALIZE handshake                  ← ld.so entry point
   ├── Protocol version negotiation              ← ELF interp check
   └── session/new with capabilities             ← dlopen() shared libs

3. ADDRESS SPACE SETUP  [session_output.rs:404-483]
   ├── Create node workspace directory           ← mmap(PROT_READ|WRITE)
   ├── Write node.json metadata                  ← .data segment
   ├── Copy skills to workspace                  ← .so library loading
   └── Generate CLAUDE.md / AGENTS.md            ← Process image (objdump-readable)

4. STACK FRAME INIT  [spawn_agent.rs:251-301]
   ├── project_aam_context()                     ← Copy parent registers
   │   ├── Filter beliefs (drop _-prefixed)      ← Strip kernel state
   │   ├── Copy active goals only                ← Copy instruction pointer
   │   └── Extract capabilities list             ← Copy capability set
   └── render_system_prompt()                    ← Format argv[] + environ[]

5. ENTRY POINT  [session.rs:176-188]
   ├── send_system_preamble() (turn 0)           ← _start → __libc_start_main
   └── Agent begins executing                    ← main() entered

6. PROCESS REGISTRATION  [process_table.rs:136-159]
   ├── register_external(name, parent, session)  ← Add to kernel proc table
   └── Return ProcessId (UUID v7)                ← Return child PID
```

---

## 5. Memory Architecture

The whiteboard's "VLM | PID | 6wm | GLib" maps to the memory hierarchy:

```
AGENTIC MEMORY HIERARCHY
=========================

    Registers     ←→  Tokens (TokenId → Value in DashMap)
         │              O(1) access, per-execution lifetime
         │
    L1-L3 Cache   ←→  STM (Short-Term Memory)
         │              In-memory HashMap, scoped by scope_id
         │              max_entries: 1024 (bounded like stack)
         │
    Heap / RAM    ←→  LTM (Long-Term Memory)
         │              SQLite-backed, persistent across executions
         │              Unbounded, supports vector similarity search
         │
    Audit Log     ←→  Episodic Memory
         │              Ring buffer (10K entries) + JSONL file
         │              Immutable, append-only, for REFLECT queries
         │
    Process State ←→  AAM (Agent Abstract Machine)
         │              Beliefs = heap (mutable state)
         │              Goals = scheduler priority queue
         │              Capabilities = permission bits
         │              Transitions = audit trail
         │
    Address Space ←→  Scope Isolation
                        __scope__/{scope_id}/{key} prefix
                        ScopePolicy: Inherit | Isolate | Snapshot | Filter
                        Snapshot = copy-on-write fork semantics
```

---

## 6. Security Model

```
AGENTIC SECURITY LAYERS
========================

Layer 0: COMPILE-TIME (unique to APXM — no prior art has this)
    SecurityManifest generated from static analysis of graph
    ├── Per-op sandbox tier classification (T0/T1/T2/T3)
    ├── needs_network, needs_filesystem_write, needs_process_spawn
    └── min_isolation: IsolationLevel computed from max tier

Layer 1: PROCESS ISOLATION (like namespaces/cgroups)
    ├── Scope-based memory isolation (scope_id prefix = page table)
    ├── ProcessTable depth limits (max_spawn_depth=4 = fork bomb prevention)
    └── ProcessTable count limits (max_processes=32 = ulimit -u)

Layer 2: PERMISSION MODEL (like DAC + MAC)
    ├── PermissionMode per agent: ApproveAll / ApproveReads / DenyAll
    ├── CapabilityInterceptor: pre/post hooks with Allow/Deny/EditArgs
    └── ApprovalStore: cached decisions with Once/Session/Always scope

Layer 3: SANDBOX EXECUTION (like seccomp/containers)
    IsolationLevel enum (7 levels):
    ├── 0: None          ← Ring 3 (untrusted dev)
    ├── 1: PolicyOnly    ← Application ACL
    ├── 2: OsLevel       ← seccomp/Landlock
    ├── 3: Container     ← Docker/bubblewrap
    ├── 4: Hypervisor    ← Firecracker/gVisor
    ├── 5: Wasm          ← Wasmtime capability model
    └── 6: Remote        ← Air-gapped cloud sandbox

    SandboxBackend trait: host provides implementation
    (APXM defines contract, host implements for their platform)
```

---

## 7. Prior Art Comparison

| Capability | AIOS (Rutgers) | Agent-OS Blueprint | Karpathy LLM OS | DSPy | LangGraph et al. | DynTaskMAS | **APXM** |
|---|---|---|---|---|---|---|---|
| Real compiler (MLIR) | No | No | No | Prompt opt. | No | No | **Yes** |
| Typed ISA | No | Conceptual | No | Signatures | No | No | **Yes — 41 ops** |
| Artifact format | No | No | No | No | No | No | **Yes — .apxmobj** |
| Compile-time type checking | No | No | No | No | No | No | **Yes** |
| Formal abstract machine | No | No | No | No | No | No | **Yes — AAM(B,G,C)** |
| Dataflow scheduling O(1) | No | No | No | No | No | Dynamic graphs | **Yes** |
| 3-tier memory (STM/LTM/Episodic) | Basic | Requirements | No | No | No | Semantic | **Yes** |
| Process model (ProcessTable) | Basic | Requirements | No | No | No | No | **Yes** |
| Compile-time sandbox classification | No | No | No | No | No | No | **Yes — T0-T3** |
| Session tracing + replay | No | Requirements | No | No | Checkpointing | No | **Yes** |

### The Core Novelty

APXM is the **only system that treats agent workflow execution as a compiler problem**. Every other "Agent OS" operates at the runtime level — scheduling requests, managing context, providing APIs. None compile agent workflows into an intermediate representation, apply optimization passes, or emit portable artifacts.

The economic argument is decisive: **in traditional compilers, eliminating one instruction saves nanoseconds; in APXM, eliminating one LLM call saves seconds and dollars.**

Three specific novel contributions:

1. **ISA contract as ecosystem enabler.** AIS is the x86/ARM equivalent for agents — a contract decoupling frontends from backends. Any framework can emit AIS graphs; any runtime can execute them.

2. **Compile-time verification of agent workflows.** Type errors, missing dependencies, and malformed state transitions are caught *before* execution, before any expensive LLM calls.

3. **Formal PXM lineage.** APXM explicitly positions itself in the 80-year lineage: von Neumann → Dataflow → Cilk → CUDA → Codelet → A-PXM. The AAM is a formal abstract machine, not a metaphor. The scheduler uses Manchester Machine-style token counting. The process model has a real ProcessTable. These are systems concepts implemented as systems infrastructure.

---

## 8. The Cache Coherence Problem: Session / Context / Memory

The most critical gap in APXM's agentic OS is the **absence of a unified state coherence protocol** between its three data tracks. In a traditional OS, the MMU + TLB + cache coherence protocol (MESI/MOESI) ensure that CPU caches, RAM, and swap all present a consistent view. In APXM, three independent systems record execution state with **no shared event ID and no cross-referencing**.

### 8.1 The Three Independent Tracks

```
TRACK 1: DATAFLOW TOKENS (In-Memory, Volatile)
  Location: SchedulerState.tokens (DashMap<TokenId, TokenState>)
  Writer:   worker.rs publish_outputs() [line 459]
  Lifetime: Single execution (LOST after completion)
  Content:  Full Value for every token
  Reader:   Next node via collect_inputs()

TRACK 2: SESSION FILES (Disk, Per-Node Workspaces)
  Location: ~/.apxm/sessions/<exec-id>/nodes/<id>_<name>/
  Writer:   SessionEventEmitter.emit_node_output() [session_output.rs:724]
  Lifetime: Persists indefinitely (never auto-deleted)
  Content:  output.json, prompt.txt, response.txt, status.json, CLAUDE.md
  Reader:   ContextAssembler.load_upstream_outputs() [context_assembler.rs:158]
            ContextStack.assemble() [context_stack/mod.rs]

TRACK 3: EPISODIC MEMORY (Ring Buffer + JSONL)
  Location: VecDeque<EpisodicEntry> + ~/.apxm/memory/episodes.jsonl
  Writer:   worker.rs record_event() [line 554]
  Lifetime: Ring buffer (10K entries, FIFO eviction)
  Content:  SPARSE metadata only (node_id, op_type, duration, retries)
            *** NO NODE OUTPUT VALUES ***
  Reader:   REFLECT handler, QMEM(space=episodic)
```

### 8.2 The Disconnections

```
           TRACK 1              TRACK 2              TRACK 3
        (Tokens/DashMap)     (Session Files)      (Episodic Memory)
              │                    │                     │
              │   emit_node_      │                     │
              ├──────output()────►│                     │
              │                    │                     │
              │   record_event()  │                     │
              ├───────────────────┼────────────────────►│
              │                    │                     │
              │                    │         ╳           │
              │                    │    No connection    │
              │                    │         ╳           │
              │                    ◄─────────────────────┤
              │                    │                     │
              │                    │                     │
     ContextAssembler             │                     │
     reads output.json ◄──────────┤                     │
     from disk                    │                     │
              │                    │                     │
     ContextAssembler             │                     │
     CANNOT read episodic ────────┼────────╳───────────┤
              │                    │                     │
     Episodic does NOT             │                     │
     contain output values ───────┼────────╳───────────┤
              │                    │                     │
     No shared event ID ──────────┼────────╳───────────┤
     between tracks               │                     │
```

### 8.3 What This Means in OS Terms

| OS Concept | What APXM Has | What's Missing |
|---|---|---|
| **Cache coherence (MESI)** | Three independent caches | No coherence protocol between tracks |
| **Page table** | scope_id prefix isolation | No unified address → track mapping |
| **TLB** | DashMap tokens (fast, volatile) | No TLB miss → session file fallback |
| **Write-back cache** | Tokens write to session files | Episodic memory never gets output values |
| **Swap space** | Session files persist | No way to reload session → tokens |
| **/proc filesystem** | Session trace.ndjson | No live view into episodic entries |
| **Core dump** | AAM checkpoints (manual) | No auto-checkpoint on crash |

### 8.4 The Root Cause: Three Independent Writers

```rust
// Writer 1: Token publication (worker.rs:358-368)
publish_outputs(state, node_id, outputs, value.clone()).await;  // → DashMap
if let Some(emitter) = &ctx.event_emitter {
    emitter.emit_node_output(node_id, &value);                  // → disk (output.json)
}

// Writer 2: Episodic recording (worker.rs:554-561)
ctx.memory().record_episodic_event(
    ctx.execution_id.clone(),
    "op_success",                                                // SPARSE: no output value!
    Value::Object(/* node_id, op, attempts, duration_ms */),
).await;

// These two writers share NO event ID, NO foreign key, NO cross-reference
```

### 8.5 Concrete Impact

1. **ContextAssembler can only read session files** — If `--emit-session` is not used, there are NO upstream outputs available for CLAUDE.md assembly. Agents spawn blind.

2. **Episodic memory is useless for output queries** — REFLECT can see "node 3 completed ASK in 2.1s" but NOT what the ASK actually returned.

3. **No cross-execution learning** — Previous execution outputs are in session files, but episodic memory (which persists globally) doesn't contain them. No agent can learn from past outputs.

4. **No session resumption** — Session folders are audit trails, not recovery points. You cannot restart an execution from node N.

5. **ACP session state is volatile** — Turn count, conversation history, and agent state are in-memory only. If APXM crashes, all agent sessions are lost with no recovery.

### 8.6 The Fix: Unified Event Pipeline (the "MMU")

The agentic OS needs a coherence protocol — a single event path that feeds all three tracks:

```
Operation completes
  │
  ▼
UnifiedEventPipeline (the "MMU")
  │
  ├──► Track 1: Token publication (DashMap, volatile)
  │     TokenState { ready: true, value, consumers }
  │
  ├──► Track 2: Session files (disk, persistent)
  │     output.json, trace.ndjson, status.json
  │
  ├──► Track 3: Episodic memory (ring buffer)
  │     EpisodicEntry { ..., node_id, OUTPUT VALUE, session_path }
  │
  └──► Track 4: EventBus (subscribers)  [currently orphaned]
        External consumers, webhooks, progress bars

Properties:
  - Single event_id shared across ALL four sinks
  - Episodic entries include full output values (not just metadata)
  - Episodic entries include back-pointer to session directory
  - Session trace events include episodic entry ID
  - ContextAssembler can read from BOTH session files AND episodic memory
  - REFLECT gets rich data from any source
```

### 8.7 The Context Assembly Pipeline (How fork() Should Work)

Currently, context flows through two parallel but disconnected systems:

```
CURRENT (Disconnected):

  ContextAssembler (driver)          ContextStack (runtime)
  ┌─────────────────────┐           ┌─────────────────────┐
  │ Reads output.json   │           │ Reads output.json   │
  │ from session dir    │           │ from session dir    │
  │ Writes CLAUDE.md    │           │ Assembles frames    │
  │                     │           │ with budget alloc   │
  │ Profile-aware:      │           │ Profile-aware:      │
  │ claude → CLAUDE.md  │           │ claude → depth 3    │
  │ codex → AGENTS.md   │           │ codex → depth 2     │
  └─────────────────────┘           │ reviewer → unlimited│
                                    └─────────────────────┘
            │                                │
            ▼                                ▼
   Node workspace file              AamContext.system_prompt
   (read by agent from cwd)         (sent via ACP preamble turn 0)

NEITHER reads episodic memory. NEITHER reads LTM. NEITHER reads STM.

PROPOSED (Unified):

  ContextAssembler + ContextStack + MemorySystem
  ┌──────────────────────────────────────────────┐
  │ Sources (priority order):                    │
  │ 1. Session files (output.json)   ← current  │
  │ 2. Episodic memory (with outputs) ← NEW     │
  │ 3. STM (scoped beliefs)          ← NEW      │
  │ 4. LTM (persistent knowledge)    ← NEW      │
  │                                              │
  │ Budget-allocated across sources              │
  │ Profile-specific depth and scope rules       │
  └──────────────────────────────────────────────┘
```

### 8.8 Scope Isolation Model (The "Page Table")

The ScopeRegistry implements virtual address space isolation for AAM state:

```
ScopePolicy enum:
  ┌─────────────────────────────────────────────┐
  │ Inherit   → Shared Arc (writes propagate)   │  ← like shared memory
  │ Snapshot  → Point-in-time copy (COW fork)   │  ← like fork()
  │ Isolate   → Empty state (clean process)     │  ← like exec()
  │ Filter    → Subset copy (selective mmap)    │  ← like partial fork
  └─────────────────────────────────────────────┘

Physical memory key: __scope__/{scope_id}/{logical_key}
                     ^^^^^^^^^^^^^^^^^^^^^^^^
                     Page table translation

ExecutionContext.child() creates new scope:
  - New execution_id (UUID v7)
  - New scope_id (UUID v7)
  - AAM scoped per ScopePolicy
  - Shared systems: memory, LLM, capabilities (Arc clone)
  - Hierarchical cancellation: child token linked to parent
```

---

## 9. Architecture Decision: Runtime IS the Kernel

The key question: should the "Agentic OS kernel" be the **APXM runtime itself**, or a **new entity above it**?

### Argument for "Runtime IS the kernel" (recommended)

APXM already implements every kernel subsystem:

| Kernel Subsystem | Already Implemented? | Location |
|---|---|---|
| Process scheduler | Yes | DataflowScheduler + workers |
| Process table | Yes | ProcessTable (processes + threads) |
| Syscall dispatch | Yes | OperationDispatcher (41 ops) |
| Memory management | Yes | MemorySystem (STM/LTM/Episodic) |
| Virtual address spaces | Yes | scope_id isolation |
| File descriptors / IPC | Yes | Tokens + edges + ACP sessions |
| Security / permissions | Yes | SecurityManifest + SandboxRegistry |
| Device drivers | Yes | CapabilitySystem |
| Program loader | Yes | Artifact loading + DAG extraction |
| Watchdog | Yes | Deadlock detector |
| Session tracing | Yes | EventEmitter + trace.ndjson |

**The runtime IS the kernel.** No additional layer needed. What's needed is:

1. **Framing** — Position the existing runtime explicitly as an OS kernel
2. **Filling gaps** — A few missing OS concepts (see Section 9)
3. **API surface** — Expose kernel capabilities through a clean "shell" interface

### What a "shell" layer above the kernel would provide

```
┌────────────────────────────────────────┐
│  SHELL (CLI / Server / SDK)            │
│                                        │
│  apxm execute  = run a "program"       │
│  apxm compile  = compile source        │
│  apxm ps       = list processes (NEW)  │
│  apxm top      = live scheduler view   │
│  apxm kill     = cancel execution      │
│  apxm mount    = register backend      │
│  apxm lsof     = list open tokens      │
│  apxm strace   = trace operations      │
│  apxm replay   = replay session        │
│  apxm doctor   = kernel health check   │
│                                        │
│  Server: HTTP API for remote access    │
│  SDK: Programmatic kernel access       │
└────────────────────────────────────────┘
```

---

## 10. Missing OS Concepts & Priority Gaps

What the current APXM runtime does NOT yet have, mapped to OS concepts:

### P0: Unified Event Pipeline (Cache Coherence Protocol)

**OS analogue**: MESI/MOESI cache coherence + write-back protocol

**Current state**: Three independent writers (tokens, session files, episodic) with no shared event ID. Episodic entries contain NO output values. No cross-referencing between tracks.

**Impact**: ContextAssembler blind without `--emit-session`. No cross-execution learning. REFLECT gets sparse metadata only.

**Proposal**: `UnifiedEventPipeline` that:
- Assigns a single `event_id` to each operation completion
- Feeds all 4 sinks (tokens, session files, episodic, EventBus)
- Enriches episodic entries with output values + session_path back-pointer
- Enables ContextAssembler to read from episodic as fallback

### P0: Enrich EpisodicEntry with Output Values

**Current**: `EpisodicEntry { id, timestamp, event_type, payload(sparse), execution_id }`
**Proposed**: Add `node_id: Option<u64>`, `output: Option<Value>`, `session_dir: Option<PathBuf>`

### P1: ContextAssembler Reads Memory (Not Just Files)

**Current**: Only reads `output.json` from session directory on disk.
**Proposed**: Also query episodic memory and STM for richer context, with budget allocation across sources.

### P1: Session Resumption (Process Restart from Checkpoint)

**OS analogue**: CRIU (Checkpoint/Restore In Userspace)

**Current state**: Session folders are audit trails only. No way to restart from node N. AAM checkpoints exist but require manual management.

**Proposal**: `apxm resume <session-dir> --from-node <N>` that:
- Loads AAM checkpoint from session
- Replays completed node outputs from results.json into token DashMap
- Resumes scheduler from node N forward

### P1: ACP Session State Persistence

**OS analogue**: TCP state saved to disk for connection migration

**Current state**: Turn count and conversation state in-memory only. Agent crash = total loss.

**Proposal**: Persist ACP session metadata (turn_count, agent_session_id, profile) to session directory. On agent crash, detect broken transport and auto-respawn with context replay.

### P2: Cross-Execution Agent Pool (Warm Process Pool)

**OS analogue**: `init` process / systemd service management / container warm pools

**Current state**: Each `Runtime::new()` creates a fresh ProcessTable. Agents can't survive across executions. Cold start: 2-10s per agent.

**Proposal**: `AgentPool` struct that persists across executions:
```
AgentPool {
    pools: DashMap<String, Vec<PooledSession>>  // profile → warm sessions
    acquire(profile, aam_context) → session
    release(session) → return to pool
    idle_timeout: Duration
}
```

### P2: Inter-Execution IPC (Daemons / Services)

**OS analogue**: System daemons, D-Bus, systemd services

**Current state**: All communication is intra-execution. No way for two separate `apxm execute` invocations to share agents or communicate.

**Proposal**: APXM server mode (`apxm serve`) as a persistent daemon that:
- Holds the AgentPool across client connections
- Routes inter-execution messages
- Provides the warm pool for agent reuse

### P3: Filesystem Abstraction for Memory

**OS analogue**: VFS (Virtual File System) with mount points

**Current state**: Memory tiers are accessed via QMEM/UMEM with `space` attribute. But there's no unified namespace — it's three separate stores.

**Proposal**: Expose memory as a VFS-like hierarchy:
```
/stm/                    ← Short-term (per-execution)
/stm/{scope_id}/         ← Per-agent scope
/ltm/                    ← Long-term (persistent)
/episodic/               ← Event log
/shared/                 ← Cross-agent shared namespace (NEW)
/tokens/{token_id}       ← Active dataflow tokens
```

### P3: Signal Handling (Async Events)

**OS analogue**: POSIX signals (SIGINT, SIGTERM, SIGUSR1)

**Current state**: Only `CancellationToken` (equivalent to SIGKILL). No way to send async signals to running agents.

**Proposal**: Agent signal system:
- `SIGPAUSE` → trigger PAUSE checkpoint
- `SIGPRIORITY` → dynamically boost/lower priority
- `SIGMEMORY` → trigger memory pressure response
- `SIGGOAL` → inject new goal at runtime

### P3: Job Control (fg/bg/suspend/resume)

**OS analogue**: Shell job control, `fg`, `bg`, `jobs`, Ctrl-Z

**Current state**: PAUSE/RESUME exist as graph-level operations but not as runtime-level controls.

**Proposal**: Runtime-level job control:
- Suspend any running execution (snapshot state)
- Resume from snapshot
- List all running/suspended executions
- Move between foreground/background

### P3: Unified Event Bus (Wire or Remove)

**OS analogue**: `/dev/eventfd`, epoll, kernel event subsystem

**Current state**: EventBus is complete and tested but never instantiated in production.

**Proposal**: Wire EventBus as unified fan-out alongside SessionEventEmitter, enabling external consumers (webhook, progress bar, monitoring).

---

## 11. What This Means

APXM is not evolving *toward* an Agentic OS — **it already is one**. The architecture emerged organically from solving real agent orchestration problems, and it converged on the same abstractions that OS kernels use because those abstractions are the correct solution to the fundamental problems of:

1. **Resource multiplexing** — Multiple agents sharing LLM backends, memory, tools
2. **Process isolation** — Agents that can't corrupt each other's state
3. **Scheduling** — Parallel execution with dependency management
4. **Security** — Graduated trust levels for different operations
5. **Portability** — Compile once, run on any conforming runtime

The next step is not building a new layer — it's **recognizing what already exists** and making the OS metaphor explicit in the API surface, documentation, and developer experience. The kernel is built. Now we need the shell.
