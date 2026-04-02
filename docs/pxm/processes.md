# Agent Process Model

The APXM process model introduces formal OS-like abstractions for agent lifecycle management. Agents are **processes**; node executions within an agent are **threads**. This document explains what really happens at each stage of an agent's life.

---

## 1. Agent Lifecycle

```
┌──────────┐     ┌─────────────┐     ┌─────────────┐     ┌──────────────┐
│  SPAWN   │────▶│ INITIALIZE  │────▶│ COMMUNICATE │────▶│  TERMINATE   │
│  AGENT   │     │  (ACP)      │     │  (IPC)      │     │  (signals)   │
└──────────┘     └─────────────┘     └─────────────┘     └──────────────┘
```

### Phase 1: Spawn — `SPAWN_AGENT` with `profile` attribute

When `SPAWN_AGENT` includes a `profile` attribute (e.g. `"claude"`), the runtime spawns a real ACP subprocess. Here is exactly what happens at each level.

**What happens at the OS level:**

1. Profile lookup: `AgentRegistry.get("claude")` resolves to `AgentProfile { command: "npx -y @agentclientprotocol/claude-agent-acp@^0.24.2", ... }`.
2. Command parsing: `shell_words::split(command)` splits into `["npx", "-y", "@agentclientprotocol/claude-agent-acp@^0.24.2"]`.
3. Process spawn: `tokio::process::Command::new("npx").args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).current_dir(cwd).spawn()`.
4. This creates a real OS child process with PID, stdin/stdout pipes, and its own address space.

**What happens at the ACP protocol level:**

5. `StdioTransport::new(stdin, stdout)` wraps the pipes as an NDJson transport.
6. Phase 1 — Initialize handshake:
   - Client sends: `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-05","clientCapabilities":{"fs":{"readTextFile":true,"writeTextFile":true},"terminal":true},"clientInfo":{"name":"apxm","version":"..."}}}`
   - Agent responds: `{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-05","serverCapabilities":{...}}}`
7. Phase 2 — Authenticate (if agent requests it):
   - Agent may include `authMethods` in the init response.
   - Client resolves credentials from environment (`ACPX_AUTH_*`).
   - Sends `authenticate` request with matching credential.
8. Phase 3 — Session creation:
   - Client sends: `{"jsonrpc":"2.0","id":3,"method":"session/new","params":{"cwd":"/path/to/repo","mcpServers":[...]}}`
   - Agent creates its workspace, loads MCP tools, responds with `sessionId`.

**What happens at the APXM runtime level:**

9. Session controls applied: `set_mode("architect")` sends `session/set_mode`; `set_model("claude-sonnet-4")` sends `unstable_setSessionModel`.
10. Process registered: `ProcessTable.register_external("reviewer", ...)` creates an `AgentProcess` entry.
11. AAM belief recorded: `_spawned_agent:reviewer` — the parent knows the child exists.
12. STM metadata stored: `_agent_info:reviewer` — profile, session_id, process_id.

Without `profile`, SPAWN_AGENT behaves as before (metadata-only registration for local flow agents).

### Phase 2: Communicate — `COMMUNICATE` with `protocol: "acp"`

**What happens:**

1. Recipient lookup: `ProcessTable.get_by_name("reviewer")` resolves to `AgentProcess { kind: External { session, ... } }`.
2. Message conversion: input Value is converted to a prompt text string.
3. Reverse handler created: `CapabilityReverseHandler::new(capability_system, permission_mode)` — this handles file/terminal requests FROM the agent back to APXM.
4. Session locked: `session.lock().await` — prevents concurrent prompts to same agent.
5. Prompt sent via ACP:
   - Client sends: `{"jsonrpc":"2.0","id":N,"method":"session/prompt","params":{"sessionId":"...","prompt":[{"type":"text","text":"..."}]}}`
6. Response loop — the transport reads messages until matching response ID:
   - **Notifications** (`session/update` with `agent_message_chunk`): Text chunks accumulated in `response_text`.
   - **Notifications** (`usage_update`): Token counts accumulated.
   - **Reverse requests** (`fs/readTextFile`, `fs/writeTextFile`, `terminal/*`): Delegated to `CapabilityReverseHandler` which calls APXM's capability system, then sends response back to agent.
   - **Permission requests** (`requestPermission`): Checked against `PermissionMode` (approve-all, approve-reads, deny-all).
   - **Final response**: `{"jsonrpc":"2.0","id":N,"result":{"stopReason":"end_turn",...}}`
7. Turn count incremented: `session.turn_count += 1`.
8. Result returned: `PromptResult { text, model, stop_reason, token_usage }`.

**What the agent can do during a prompt:**

- Read files from the workspace (via `fs/readTextFile` reverse request).
- Write files (via `fs/writeTextFile` — subject to permission mode).
- Execute terminal commands (via `terminal/create` + `terminal/output`).
- Each of these goes through APXM's capability system, maintaining the sandbox boundary.

### Phase 3: Multi-turn — Multiple COMMUNICATE to same agent

Each subsequent COMMUNICATE to the same `recipient`:

- Reuses the same `AcpSession` (same OS process, same stdin/stdout pipes).
- The agent retains full conversation context from previous turns.
- Turn count increments: turn 1, turn 2, turn 3...
- The agent can reference previous work, files it wrote, commands it ran.

Example workflow:

```json
{
  "nodes": [
    {"id": 1, "op": "SPAWN_AGENT", "attributes": {"agent_name": "reviewer", "profile": "claude", "mode": "architect"}},
    {"id": 2, "op": "COMMUNICATE", "attributes": {"recipient": "reviewer", "protocol": "acp"}},
    {"id": 3, "op": "COMMUNICATE", "attributes": {"recipient": "reviewer", "protocol": "acp"}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Control"},
    {"from": 2, "to": 3, "dependency": "Data"}
  ]
}
```

- Node 2 sends first message -> turn 1.
- Node 3 sends second message (with node 2's response as input) -> turn 2.
- Same `AcpSession` instance, same subprocess — the agent retains context between turns.

### Phase 4: Terminate — Execution end or explicit close

**What happens:**

1. `ProcessTable.close("reviewer")` or `ProcessTable.close_all()`.
2. Transport dropped: `session.transport.take()` — closes stdin pipe, which signals EOF to the agent.
3. Grace period: `sleep(close_grace_ms)` — typically 100ms (750ms for qoder).
4. Check if exited: `child.try_wait()` — the process may have exited cleanly.
5. SIGTERM: `libc::kill(pid, SIGTERM)` — ask process to terminate gracefully.
6. Grace period: `sleep(SIGTERM_GRACE_MS)` — wait for clean shutdown (1500ms).
7. Check if exited: `child.try_wait()` again.
8. SIGKILL: `child.kill()` — force kill if still alive.
9. Reap: `child.wait()` — collect exit status, prevent zombie.

The `Drop` impl on `AcpSession` sends SIGKILL as a safety net if `close()` wasn't called.

---

## 2. Process vs Thread

| | AgentProcess | AgentThread |
|---|---|---|
| **What** | An agent (local or external) | An operation within an agent |
| **Isolation** | Own AAM scope + own OS process | Shares process AAM |
| **Identity** | Named in ProcessTable | Node ID in DAG |
| **Lifetime** | Spawn to terminate | Node start to complete |
| **Communication** | Via COMMUNICATE op | Via dataflow tokens |
| **Example** | Claude reviewing code | An ASK node querying the LLM |

An `AgentProcess` is the APXM equivalent of an OS process. It encapsulates the agent's session, profile, lifecycle state, and parent relationship. All threads within a process share the process's AAM state (beliefs, goals, capabilities), memory system, and capability system — just like OS threads share the process address space.

An `AgentThread` tracks a single node execution. Threads are created when a node fires in the dataflow scheduler, and completed when the node finishes.

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

The `ProcessTable` is the unified registry of all live agent processes. It is the process model's core data structure, analogous to the OS kernel's process table.

**Key operations:**

- **Spawn**: `spawn_local(name)` creates a local process entry; `register_external(name, session, profile)` registers an ACP subprocess.
- **Lookup**: `get_by_name(name)` resolves a recipient name to an `AgentProcess`. Used by COMMUNICATE with `protocol: "acp"`.
- **Threads**: `register_thread(pid, node_id, op_type)` tracks thread start; `complete_thread(tid)` tracks thread end.
- **Lifecycle**: `close(name)` for graceful single-process shutdown; `close_all()` for execution cleanup.
- **Observability**: `list_processes()` and `list_threads(pid)` for introspection.
- **Limits**: Configurable `max_processes` (default 32) prevents runaway spawning.

**Extension points:**

The ProcessTable holds injected trait objects for cross-crate integration:

- `AgentSpawner` — implemented in the driver using `apxm-acp`, handles the actual ACP subprocess lifecycle.
- `AgentPrompter` — implemented in the driver, handles sending prompts through the live ACP session.

This design avoids a circular dependency between `apxm-runtime` (which defines ProcessTable) and `apxm-acp` (which implements AcpSession). The traits are defined in `apxm-runtime`; the concrete implementations live in the driver.

---

## 5. Comparison: INV(acp) vs SPAWN + COMMUNICATE

| Aspect | `INV(acp)` | `SPAWN_AGENT` + `COMMUNICATE` |
|--------|-----------|-------------------------------|
| Identity | Anonymous capability call | Named process in ProcessTable |
| State | Stateless (or manual `session_handle`) | Stateful (persistent AcpSession) |
| Multi-turn | Requires explicit `session_handle` | Natural (same recipient name) |
| Isolation | Via CapabilitySystem | OS process boundary |
| Lifecycle | Per-invocation | Spawn-to-terminate |
| Observability | Capability metrics | Full process/thread tracking |
| Shutdown | Pool-managed | Explicit SIGTERM/SIGKILL |

Both approaches remain valid. `INV(acp)` is simpler for one-shot agent calls — fire-and-forget prompts where session state doesn't matter. `SPAWN_AGENT` + `COMMUNICATE` is preferred for multi-turn conversations, workflows where agent identity matters, and scenarios requiring explicit lifecycle control.

The `INV(acp)` path continues to work via the `SessionPool` and `AcpCapability`. The new process model adds a higher-level abstraction on top, not a replacement.

---

## 6. Agent Registration — Like LLMs

The driver registers LLMs via `configure_llm_registry()`. Agent processes follow the same pattern via `configure_agent_registry()`:

```rust
// In RuntimeExecutor::new():
configure_llm_registry(runtime.llm_registry(), &config.apxm_config).await?;
configure_capability_registry(runtime.capability_system_arc(), &config.apxm_config)?;
configure_agent_registry(runtime.process_table(), runtime.capability_system_arc()).await?;
```

This injects the `AcpAgentSpawner` and `AcpAgentPrompter` into the ProcessTable, making SPAWN_AGENT and COMMUNICATE(acp) functional. Without this step, SPAWN_AGENT with a `profile` attribute returns an error explaining that no spawner is configured.
