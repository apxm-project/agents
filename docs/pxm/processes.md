# Agent Process Model

> This document covers both conceptual semantics and implementation-level behavior. For additional runtime details, see [apxm-runtime](../../crates/runtime/apxm-runtime/README.md).

The A-PXM process model introduces formal OS-like abstractions for agent lifecycle management. Agents are **processes**; node executions within an agent are **threads**.

---

## 1. Agent Lifecycle

```
+----------+     +-------------+     +-------------+     +--------------+
|  SPAWN   |---->| INITIALIZE  |---->| COMMUNICATE |---->|  TERMINATE   |
|  AGENT   |     |  (ACP)      |     |  (IPC)      |     |  (signals)   |
+----------+     +-------------+     +-------------+     +--------------+
```

### Phase 1: Spawn

When `SPAWN_AGENT` includes a registered `profile` attribute, the runtime
spawns a real ACP subprocess:

1. **Profile lookup**: the agent registry resolves the profile to a command and configuration.
2. **Process spawn**: an OS child process is created with stdin/stdout pipes and its own address space.
3. **ACP handshake**: the initialize/authenticate/session-create protocol establishes a live session.
4. **Registration**: the process is recorded in the ProcessTable, and AAM beliefs are updated to reflect the child agent's existence.

Without `profile`, SPAWN_AGENT performs metadata-only registration for local flow agents.

### Phase 2: Communicate

`COMMUNICATE` dispatches a prompt to a live agent process:

1. **Recipient lookup**: the ProcessTable resolves the named recipient.
2. **Message conversion**: input Value is converted to a prompt text string.
3. **Prompt/response loop**: the ACP session exchanges messages, handling reverse requests (file reads/writes, terminal commands) through A-PXM's capability system.
4. **Result**: the agent's text response, model info, stop reason, and token usage are returned as a typed result.

During a prompt, the agent can read/write files and execute terminal commands, all mediated by A-PXM's capability system and sandbox boundary.

### Phase 3: Multi-turn

Each subsequent COMMUNICATE to the same recipient reuses the same session (same OS process, same pipes). The agent retains full conversation context from previous turns. Turn count increments with each interaction.

### Phase 4: Terminate

Graceful shutdown proceeds through: transport close (EOF signal), grace period, SIGTERM, grace period, SIGKILL if necessary, then process reaping to prevent zombies.

---

## 2. Process vs Thread

| | AgentProcess | AgentThread |
|---|---|---|
| **What** | An agent (local or external) | An operation within an agent |
| **Isolation** | Own AAM scope + own OS process | Shares process AAM |
| **Identity** | Named in ProcessTable | Node ID in DAG |
| **Lifetime** | Spawn to terminate | Node start to complete |
| **Communication** | Via COMMUNICATE op | Via dataflow tokens |
| **Example** | External code reviewer | An ASK node querying the LLM |

An `AgentProcess` is the A-PXM equivalent of an OS process. It encapsulates the agent's session, profile, lifecycle state, and parent relationship. All threads within a process share the process's [AAM](aam.md) state (beliefs, goals, capabilities) and capability system -- analogous to how OS threads share the process address space.

An `AgentThread` tracks a single node execution. Threads are created when a node fires in the [dataflow scheduler](scheduling.md) and completed when the node finishes.

---

## 3. Protocol Dispatch

The COMMUNICATE operation dispatches by protocol. Each protocol has its own transport, lookup mechanism, and isolation boundary:

| Protocol | Transport | Lookup | Isolation |
|----------|-----------|--------|-----------|
| `local` | In-process sub-DAG | FlowRegistry | Snapshot AAM |
| `http` | HTTP POST to `/v1/receive` | URL or agent registry | Network boundary |
| `acp` | ACP JSON-RPC over stdio | ProcessTable | OS process boundary |
| `broadcast` | Fan-out to all agents | FlowRegistry scan | Per-agent snapshot |

The protocol is selected via the `protocol` node attribute. When omitted, `local` is the default.

---

## 4. ProcessTable

The `ProcessTable` is the unified registry of all live agent processes -- analogous to the OS kernel's process table.

**Key operations:**

- **Spawn**: create local process entries or register external ACP subprocesses.
- **Lookup**: resolve recipient names to live processes (used by COMMUNICATE).
- **Threads**: track thread start/completion for observability.
- **Lifecycle**: graceful single-process or full-execution shutdown.
- **Limits**: configurable `max_processes` (default 32) prevents runaway spawning.

---

## 5. INV(acp) vs SPAWN + COMMUNICATE

| Aspect | `INV(acp)` | `SPAWN_AGENT` + `COMMUNICATE` |
|--------|-----------|-------------------------------|
| Identity | Anonymous capability call | Named process in ProcessTable |
| State | Stateless (or manual session handle) | Stateful (persistent session) |
| Multi-turn | Requires explicit session handle | Natural (same recipient name) |
| Isolation | Via CapabilitySystem | OS process boundary |
| Lifecycle | Per-invocation | Spawn-to-terminate |
| Observability | Capability metrics | Full process/thread tracking |

Both approaches remain valid. `INV(acp)` is simpler for one-shot agent calls. `SPAWN_AGENT` + `COMMUNICATE` is preferred for multi-turn conversations, workflows where agent identity matters, and scenarios requiring explicit lifecycle control.
