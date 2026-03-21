# Phase 4: Gemini-CLI Changes -- Agent Loop as Graph

**Draft v6 -- Revised with investigation findings**
**Source:** [Plan 3: Gemini-CLI Changes](../plan-3-gemini-changes.md), Phase G11--G12
**Timeline:** ~6 days
**Phase overview:** [README.md](README.md)
**Prerequisite:** Phases 2 and 3 complete (tools are capabilities, state is AAM).

---

## Goal

Express `executeTurn()` as an AIS graph instead of imperative code. Replace the ~200-line imperative turn loop with a typed, compiler-validated DAG. The `apxm-server` crate handles graph execution, capability system management, and AAM state.

### What Changes (Phase 4)

- `executeTurn()` becomes a declarative AIS graph
- Hooks become FENCE operations in the graph
- Recovery turn becomes a TRY_CATCH subgraph
- Sub-agent delegation becomes FLOW_CALL

### What Is NOT Changed (Phase 4)

- TUI (still renders `ServerGeminiStreamEvent` or ApxmEvent)
- User-facing CLI interface
- Configuration system

---

## G11: Express `executeTurn()` as AIS graph (4 days)

### G11.1 Turn graph structure

The current `executeTurn()` at `local-executor.ts:316` has interleaved concerns -- compression, LLM call, tool dispatch, confirmation, loop detection, recovery -- all in one function. The AIS graph separates each concern into a typed, independently-optimizable node:

```
                     executeTurn() as AIS Graph

  ┌──────────────┐
  |  GUARD        |  <-- checkTermination() + DeadlineTimer
  |  max_turns,   |      (precondition enforcement)
  |  timeout      |
  └──────┬───────┘
         |
         v
  ┌──────────────┐
  |  QMEM + UMEM |  <-- tryCompressChat()
  |  (compress)   |      (context compaction if needed)
  └──────┬───────┘
         |
         v
  ┌──────────────┐                    ┌──────────────────┐
  |  ASK / THINK  |---- tool_calls? -->|  BRANCH_ON_VALUE  |
  |  (LLM call)   |    ┌──────────────|  on calls         |
  └──────────────┘    |              └────┬─────────────┘
                      |                   | no tool calls
                      v                   v
             ┌───────────────┐     ┌──────────────┐
             |  INV tool_a    |     |  UMEM         |
             |  INV tool_b    |-┐   |  save_response|
             |  INV tool_c    | |   └──────────────┘
             └───────────────┘ |
                               v
                      ┌───────────────┐
                      |  WAIT_ALL      |  <-- parallel tool calls join
                      └───────┬───────┘
                              |
                              v
                      ┌───────────────┐
                      |  VERIFY        |  <-- shouldConfirmExecute()
                      |  (optional)    |      per-tool confirmation
                      └───────┬───────┘
                              |
                              v
                      ┌───────────────┐   ┌───────────────┐
                      |  TRY_CATCH     |-->|  REFLECT       |  <-- loop detection
                      |  (recovery)    |   |  (pattern      |      on episodic trace
                      └───────────────┘   |   detection)   |
                                          └───────────────┘
```

The three INV operations run concurrently -- the scheduler sees no edge between them and dispatches them in parallel. The GUARD node at the top enforces max turns and timeout before any LLM call happens.

**Key benefit:** The current `executeTurn()` interleaves termination checks, LLM calls, tool dispatch, confirmation waiting, compression, and loop detection in one function. The AIS graph makes each concern a separate, testable, independently-optimizable node.

### G11.2 Graph construction

The turn graph is constructed at session start (or at configuration change) and submitted to APXM for execution:

```typescript
// Pseudocode -- construct turn graph from agent definition
function buildTurnGraph(definition: LocalAgentDefinition): ApxmGraph {
  return {
    name: `gemini-turn-${definition.name}`,
    nodes: [
      { id: 1, name: "guard", op: "GUARD", attributes: { max_turns: definition.maxTurns, timeout_ms: definition.maxTimeMinutes * 60000 } },
      { id: 2, name: "compress", op: "QMEM", attributes: { action: "check_compress" } },
      { id: 3, name: "ask", op: "ASK", attributes: { prompt: "{{current_message}}" } },
      { id: 4, name: "branch", op: "BRANCH_ON_VALUE", attributes: { on: "has_tool_calls" } },
      // ... tool INV nodes generated dynamically per response
      { id: 10, name: "wait", op: "WAIT_ALL", attributes: {} },
      { id: 11, name: "verify", op: "VERIFY", attributes: { claim: "tool_results" } },
      { id: 12, name: "reflect", op: "REFLECT", attributes: { check: "loop_detection" } },
    ],
    edges: [
      { from: 1, to: 2, dependency: "Control" },
      { from: 2, to: 3, dependency: "Control" },
      { from: 3, to: 4, dependency: "Data" },
      // ... dynamic edges for tool fan-out/fan-in
    ],
    parameters: [{ name: "current_message", type: "str" }],
    metadata: { source: "gemini-cli", version: "1.0" },
  };
}
```

---

## G12: Hooks as FENCE operations, recovery as TRY_CATCH (2 days)

### G12.1 Hooks --> FENCE mapping

Gemini-CLI's 11 hook event types map to FENCE operations that enforce ordering in the graph:

| HookEventName | Graph Position | FENCE Semantics |
|--------------|---------------|-----------------|
| `SessionStart` | Before first GUARD | Session initialization barrier |
| `BeforeAgent` | Before GUARD | Pre-agent setup |
| `BeforeModel` | Before ASK/THINK | Pre-LLM barrier |
| `AfterModel` | After ASK/THINK | Post-LLM barrier |
| `BeforeToolSelection` | Before BRANCH_ON_VALUE | Pre-tool-selection barrier |
| `BeforeTool` | Before each INV | Pre-tool barrier |
| `AfterTool` | After each INV | Post-tool barrier |
| `PreCompress` | Before QMEM+UMEM | Pre-compression barrier |
| `Notification` | After any node | Event emission point |
| `AfterAgent` | After REFLECT | Post-agent cleanup |
| `SessionEnd` | After everything | Session teardown barrier |

FENCE operations do not change the computation -- they enforce ordering and provide hook injection points. The hook system continues to work, but hooks are now structurally positioned in the graph rather than scattered through imperative code.

### G12.2 Recovery turn --> TRY_CATCH subgraph

`executeFinalWarningTurn()` becomes a TRY_CATCH subgraph:

```
Main turn graph fails (timeout, max_turns, protocol violation)
  --> TRY_CATCH boundary
    --> ASK("You have one final chance to complete the task...")
    --> GUARD(timeout=60s)  // grace period
    --> INV(complete_task)   // agent must call complete_task
  <-- TRY_CATCH returns result or null
```

> **Implementation note:** The TRY_CATCH handler (`handlers/try_catch.rs`) is a thin passthrough -- it passes through its first input or `Value::Null`. Actual exception routing is delegated to the scheduler. The recovery turn pattern (TRY_CATCH wrapping ASK + GUARD + INV) requires scheduler support for subgraph error boundaries. Verify this works before implementing the full recovery turn.

### G12.3 Sub-agent delegation --> FLOW_CALL

`LocalAgentExecutor` spawning a sub-agent becomes a **FLOW_CALL** operation. The sub-agent's turn graph is a separate AIS graph invoked by reference. The parent's AAM scope creates a child scope (Inherit policy) for the sub-agent.

---

## Phase 4 Summary

| Task | Days | Key Deliverable |
|------|------|----------------|
| G11: Turn graph construction | 4 | `executeTurn()` as AIS graph |
| G12: Hooks + recovery + delegation | 2 | FENCEs, TRY_CATCH, FLOW_CALL |
| **Total** | **6** | -- |

---

## Phase 4 Validation

- [ ] `executeTurn()` expressible as AIS graph with correct topology
- [ ] Parallel tool calls demonstrate automatic parallelism from DAG
- [ ] Hook events fire at correct graph positions via FENCE
- [ ] Recovery turn works as TRY_CATCH subgraph
- [ ] Sub-agent delegation works as FLOW_CALL
- [ ] No behavioral regression vs imperative implementation

---

## Cross-References

- **APXM execution endpoint:** [apxm.md](apxm.md) -- A7.2 provides the `/v1/execute` endpoint these graphs are submitted to
- **Codex parallel effort:** [codex.md](codex.md) -- C10 does the same for `submission_loop()`
- **Phase overview:** [README.md](README.md)
- **Next phase:** Phase 5 compiles `gemini-turn.json` via `apxm compile gemini-turn.json -o gemini-turn.apxmobj -O2`
