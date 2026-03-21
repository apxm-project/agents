# Phase 4: Codex Changes -- Agent Loop as Graph

**Draft v6 -- Revised with investigation findings**
**Source:** [Plan 2: Codex Changes](../plan-2-codex-changes.md), Phase C10--C11
**Timeline:** ~5 days
**Phase overview:** [README.md](README.md)
**Depends on Plan 1:** AIS graph authoring and validation, dataflow scheduler, FLOW_CALL/WAIT_ALL operations.

---

## Goal

Express Codex's `submission_loop()` as an AIS graph instead of imperative Rust code. The graph enables automatic parallelism, compile-time validation, and sets up Phase 5 (compiler integration).

---

## C10: submission_loop() as AIS Graph (4 days)

The core Codex turn loop (`submission_loop()` at `codex.rs:4138`) follows a repeating pattern:

1. Build prompt from conversation history
2. Call LLM
3. If tool calls in response, execute them (potentially in parallel)
4. Wait for all tool results
5. Guardian reviews tool results
6. Save results to memory
7. Loop back to step 1 (with updated context)

This maps to an AIS graph:

```
                                  +----------+
                     +----------->| INV      |------+
                     |            | tool_a   |      |
+-------+  +----------------+     +----------+      |    +-----------+   +---------+
| ASK   |->| BRANCH_ON_VALUE|                       +--->| WAIT_ALL  |-->| VERIFY  |
| prompt |  | on tool calls  |    +----------+      |    |           |   | approval|
+-------+  |                +---->| INV      |------+    +-----------+   +----+----+
            +----------------+    | tool_b   |                               |
                     |            +----------+                               v
                     |                                                 +---------+
                     +------------------------------------------------>| UMEM    |
                                (no tool calls -- direct response)     | save    |
                                                                       +---------+
```

Key properties of this graph:
- The N `INV` nodes for tool calls share no data edges -- the scheduler runs them **concurrently** with zero developer effort (replacing Codex's manual `FuturesOrdered` + per-tool `RwLock`)
- `VERIFY` runs after `WAIT_ALL` -- tool approval happens only after all tools complete
- `UMEM` saves results regardless of branch taken
- The compiler can validate structural correctness: no tool execution without approval, no approval of a tool that was never called

### Multi-agent coordination

Codex's multi-agent system (`spawn.rs`, `wait.rs`, `resume_agent.rs`, `close_agent.rs`, `send_input.rs`) maps to FLOW_CALL and WAIT_ALL.

FLOW_CALL (verified at `apxm-runtime/src/executor/handlers/flow_call.rs`) supports: sub-flow lookup via FlowRegistry, child scope isolation (`ScopeSpec::snapshot_all()`), max recursion depth (100), and episodic memory recording. Multi-agent coordination:

```
+-------+     +-----------+     +-----------+     +-----------+
| PLAN  |---->| FLOW_CALL |---->| FLOW_CALL |---->| WAIT_ALL  |---> MERGE
| decomp|     | agent_a   |     | agent_b   |     | all agents|
+-------+     +-----------+     +-----------+     +-----------+
```

### C10 Deliverables

- `codex-turn.json` -- AIS graph representing a single Codex turn
- Graph authoring code that emits the turn graph from Codex's configuration
- Integration with APXM's dataflow scheduler for execution
- Validation that graph-based execution produces identical outputs to imperative code

---

## C11: Context Compaction as Graph Nodes (1 day)

Codex's 2-phase context compaction becomes visible in the graph:

```
+---------+     +---------+     +---------+
| QMEM    |---->| ASK     |---->| UMEM    |
| load    |     | compact |     | save    |
| history |     | context |     | summary |
+---------+     +---------+     +---------+
```

The compiler sees this subgraph and can:
- Fuse the QMEM + ASK if the query is a simple lookup
- Schedule compaction in parallel with other non-dependent operations
- Validate that compaction writes do not conflict with concurrent reads

### C11 Deliverables

- Context compaction expressed as QMEM + ASK + UMEM subgraph
- Integration with existing compaction triggers

---

## Phase 4 Summary

| Step | Days | New Files | Modified Files | New Lines (est.) |
|------|------|-----------|----------------|-----------------|
| C10: Turn graph | 4 | 2 | 2 | ~500 |
| C11: Compaction graph | 1 | 0 | 1 | ~80 |
| **Total** | **5** | **2** | **3** | **~580** |

---

## Cross-References

- **APXM execution endpoint:** [apxm.md](apxm.md) -- A7.2 provides the `/v1/execute` endpoint these graphs are submitted to
- **Gemini-CLI parallel effort:** [gemini-cli.md](gemini-cli.md) -- G11 does the same for `executeTurn()`
- **Phase overview:** [README.md](README.md)
- **Next phase:** Phase 5 compiles `codex-turn.json` via `apxm compile codex-turn.json -o codex-turn.apxmobj -O2`
