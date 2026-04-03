# Common Workflow Patterns

Every JSON example below is valid input to `apxm validate`.

## 1. Pipeline

Sequential chain via `Data` edges. Use `{{node_N}}` to reference upstream output. Built-in: `apxm template show pipeline --json`

```json
{
  "name": "pipeline",
  "nodes": [
    {"id": 1, "name": "draft",  "op": "ASK",  "attributes": {"template_str": "Write a short blog post about Rust"}},
    {"id": 2, "name": "review", "op": "THINK", "attributes": {"template_str": "Review this draft for clarity and accuracy: {{node_1}}"}},
    {"id": 3, "name": "refine", "op": "ASK",  "attributes": {"template_str": "Improve the draft based on this review feedback: {{node_2}}"}}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}, {"from": 2, "to": 3, "dependency": "Data"}],
  "parameters": [], "metadata": {}
}
```

## 2. Fan-Out / Parallel

Independent tasks run concurrently; `WAIT_ALL` collects all results. Built-in: `apxm template show fan-out --json`
```json
{
  "name": "fan-out",
  "nodes": [
    {"id": 1, "name": "research-a", "op": "ASK", "attributes": {"template_str": "Research topic A"}},
    {"id": 2, "name": "research-b", "op": "ASK", "attributes": {"template_str": "Research topic B"}},
    {"id": 3, "name": "research-c", "op": "ASK", "attributes": {"template_str": "Research topic C"}},
    {"id": 4, "name": "merge", "op": "WAIT_ALL", "attributes": {"tokens": ["{{node_1}}", "{{node_2}}", "{{node_3}}"]}}
  ],
  "edges": [{"from": 1, "to": 4, "dependency": "Data"}, {"from": 2, "to": 4, "dependency": "Data"}, {"from": 3, "to": 4, "dependency": "Data"}],
  "parameters": [], "metadata": {}
}
```

## 3. Map-Reduce

Parallel workers sync via `WAIT_ALL`, then a final step synthesizes. Built-in: `apxm template show map-reduce --json`
```json
{
  "name": "map-reduce",
  "nodes": [
    {"id": 1, "name": "analyze-1",  "op": "ASK",     "attributes": {"template_str": "Analyze aspect 1 of the problem"}},
    {"id": 2, "name": "analyze-2",  "op": "ASK",     "attributes": {"template_str": "Analyze aspect 2 of the problem"}},
    {"id": 3, "name": "analyze-3",  "op": "ASK",     "attributes": {"template_str": "Analyze aspect 3 of the problem"}},
    {"id": 4, "name": "sync",       "op": "WAIT_ALL", "attributes": {"tokens": ["{{node_1}}", "{{node_2}}", "{{node_3}}"]}},
    {"id": 5, "name": "synthesize", "op": "ASK",     "attributes": {"template_str": "Synthesize all analyses into a final report: {{node_4}}"}}
  ],
  "edges": [{"from": 1, "to": 4, "dependency": "Data"}, {"from": 2, "to": 4, "dependency": "Data"}, {"from": 3, "to": 4, "dependency": "Data"}, {"from": 4, "to": 5, "dependency": "Data"}],
  "parameters": [], "metadata": {}
}
```

## 4. Verify (Generate + Fact-Check)

`ASK` produces a claim, `VERIFY` checks it against evidence. Built-in: `apxm template show verify --json`
```json
{
  "name": "verify",
  "nodes": [
    {"id": 1, "name": "generate", "op": "ASK",    "attributes": {"template_str": "State 3 facts about the solar system"}},
    {"id": 2, "name": "check",    "op": "VERIFY", "attributes": {"claim": "{{node_1}}", "evidence": "Common astronomical knowledge"}}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}],
  "parameters": [], "metadata": {}
}
```

## 5. Conditional Branching

`BRANCH_ON_VALUE` routes to exactly one path via `Control` edges. Only the taken branch executes. Built-in: `apxm template show conditional --json`
```json
{
  "name": "conditional",
  "nodes": [
    {"id": 1, "name": "classify",       "op": "ASK",             "attributes": {"template_str": "Is this a technical question? Answer only 'yes' or 'no'"}},
    {"id": 2, "name": "branch",         "op": "BRANCH_ON_VALUE", "attributes": {"token": "{{node_1}}", "value": "yes", "label_true": "3", "label_false": "4"}},
    {"id": 3, "name": "technical-path", "op": "ASK",             "attributes": {"template_str": "Give a detailed technical answer"}},
    {"id": 4, "name": "general-path",   "op": "ASK",             "attributes": {"template_str": "Give a friendly general answer"}}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}, {"from": 2, "to": 3, "dependency": "Control"}, {"from": 2, "to": 4, "dependency": "Control"}],
  "parameters": [], "metadata": {}
}
```

## 6. Error Handling

`TRY_CATCH` wraps a try subgraph with a catch subgraph. On failure, the `ERR` node fires with a recovery strategy.
```json
{
  "name": "error-handling",
  "nodes": [
    {"id": 1, "name": "try-catch",  "op": "TRY_CATCH", "attributes": {"try_subgraph": "2", "catch_subgraph": "3"}},
    {"id": 2, "name": "risky-call", "op": "ASK",       "attributes": {"template_str": "Translate this document to French"}},
    {"id": 3, "name": "fallback",   "op": "ERR",       "attributes": {"message": "Translation failed", "recovery_template": "Return the original text with a note that translation was unavailable"}}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Control"}, {"from": 1, "to": 3, "dependency": "Control"}],
  "parameters": [], "metadata": {}
}
```

## 7. Memory Operations

`QMEM` reads from memory (STM/LTM/Episodic). `UMEM` writes a key-value pair. `FENCE` ensures prior writes are visible before subsequent reads.
```json
{
  "name": "memory-pipeline",
  "nodes": [
    {"id": 1, "name": "recall",  "op": "QMEM",  "attributes": {"query": "user_preferences", "memory_tier": "ltm"}},
    {"id": 2, "name": "respond", "op": "ASK",   "attributes": {"template_str": "Given these preferences: {{node_1}}, recommend a book"}},
    {"id": 3, "name": "store",   "op": "UMEM",  "attributes": {"key": "last_recommendation", "value": "{{node_2}}", "memory_tier": "stm"}},
    {"id": 4, "name": "barrier", "op": "FENCE", "attributes": {}},
    {"id": 5, "name": "confirm", "op": "QMEM",  "attributes": {"query": "last_recommendation", "memory_tier": "stm"}}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}, {"from": 2, "to": 3, "dependency": "Data"}, {"from": 3, "to": 4, "dependency": "Data"}, {"from": 4, "to": 5, "dependency": "Data"}],
  "parameters": [], "metadata": {}
}
```

## 8. Multi-Agent Coordination

`GUARD` enforces preconditions (conditions: `> 0.8`, `!= null`, `not_empty`; on_fail: `halt` or `skip`). `CLAIM` atomically takes a task from a shared queue. `UPDATE_GOAL` modifies the agent's goals at runtime (actions: `set`, `remove`, `clear`).
```json
{
  "name": "coordinated-worker",
  "nodes": [
    {"id": 1, "name": "check-ready", "op": "GUARD",       "attributes": {"condition": "not_null", "on_fail": "halt", "error_message": "No input provided"}},
    {"id": 2, "name": "get-task",     "op": "CLAIM",       "attributes": {"queue": "review_tasks", "lease_ms": 30000}},
    {"id": 3, "name": "do-work",      "op": "ASK",         "attributes": {"template_str": "Review the following submission: {{node_2}}"}},
    {"id": 4, "name": "set-goal",     "op": "UPDATE_GOAL", "attributes": {"goal_id": "review_complete", "action": "set", "priority": 1}}
  ],
  "edges": [{"from": 1, "to": 2, "dependency": "Data"}, {"from": 2, "to": 3, "dependency": "Data"}, {"from": 3, "to": 4, "dependency": "Data"}],
  "parameters": [], "metadata": {}
}
```



## 9. Multi-Agent (ACP)

`SPAWN_AGENT` starts an external agent (Claude Code, Codex, etc.). `COMMUNICATE` sends a message over ACP and returns the response. `MERGE` combines multiple agent outputs. Use `Control` edges from spawn to communicate to ensure agents are alive before messaging them.

```
SPAWN claude ──Control──► COMMUNICATE(claude) ──Data──► MERGE ──► PRINT
SPAWN codex  ──Control──► COMMUNICATE(codex)  ──Data──┘
CONST_STR    ──Data─────► both COMMUNICATE nodes
```

See `examples/07-acp-agents/parallel-agents.json` for the full graph, or use `INV` with `capability: "acp"` for a more concise session-managed approach.

For the full multi-agent walkthrough including session-managed INV style, cross-critique, and pipeline patterns, see [Multi-Agent Workflows](multi-agent.md).

## 10. Iterative Self-Refinement (Unrolled Loop)

`REFLECT` critiques a draft using the execution trace; `ASK` rewrites it. Repeat N times with `CHECKPOINT` after each version. `VERIFY` validates the final result. Note: LOOP_START/LOOP_END back-edges are not supported in JSON graph format — unroll iterations explicitly.

```json
{
  "name": "iterative-refine",
  "nodes": [
    {"id": 1, "name": "draft",    "op": "ASK",        "attributes": {"template_str": "Write a short blog post about async Rust"}},
    {"id": 2, "name": "ckpt_v0", "op": "CHECKPOINT", "attributes": {"checkpoint_id": "v0", "storage": "fs"}},
    {"id": 3, "name": "reflect",  "op": "REFLECT",    "attributes": {"trace_query": "last_execution", "reflection_prompt": "What is imprecise or unclear? Be specific."}},
    {"id": 4, "name": "refine",   "op": "ASK",        "attributes": {"template_str": "Improve this draft:\n{{node_1}}\n\nBased on: {{node_3}}"}},
    {"id": 5, "name": "verify",   "op": "VERIFY",     "attributes": {"claim": "{{node_4}}", "evidence": "Must cover async/await, Tokio, and common pitfalls"}},
    {"id": 6, "name": "store",    "op": "UMEM",       "attributes": {"key": "async_rust_post", "value": "{{node_4}}", "memory_tier": "ltm"}}
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"}, {"from": 2, "to": 3, "dependency": "Data"},
    {"from": 1, "to": 3, "dependency": "Data"}, {"from": 3, "to": 4, "dependency": "Data"},
    {"from": 1, "to": 4, "dependency": "Data"}, {"from": 4, "to": 5, "dependency": "Data"},
    {"from": 5, "to": 6, "dependency": "Data"}
  ],
  "parameters": [], "metadata": {}
}
```

## 11. Plan → Fan-Out → Synthesize

`PLAN` decomposes a goal into structured steps. Parallel `ASK` nodes each handle one section. `WAIT_ALL` syncs them. `THINK` assembles the final output with extended reasoning. `VERIFY` checks quality.

```
PLAN ──► ASK(section_1) ──► WAIT_ALL ──► THINK ──► VERIFY ──► CHECKPOINT
     ──► ASK(section_2) ──┘
     ──► ASK(section_3) ──┘
```

See `examples/09-plan-fan-out/plan-fan-out.json` for the complete graph.

## 12. Resilient ACP Pipeline (with Fallback)

`GUARD` validates input. Primary agent via `COMMUNICATE`. `CHECKPOINT` saves state (on_fail: continue). `VERIFY` checks output quality. `BRANCH_ON_VALUE` routes to a fallback agent if quality is insufficient. Final `ASK` synthesizes both outputs.

```
GUARD ──► COMMUNICATE(primary) ──► CHECKPOINT ──► VERIFY ──► BRANCH ──► COMMUNICATE(fallback) ──► ASK(synthesize)
                                                                  └──► (valid) ──────────────────────┘
```

See `examples/10-resilient-acp/resilient-acp-pipeline.json`.

## 13. Worker Pool (CLAIM + Parallel Workers)

`UPDATE_GOAL` declares intent. Three parallel `CLAIM` nodes atomically pull tasks from a shared queue. `GUARD` (on_fail: skip) handles empty slots. `THINK` workers process in parallel. `CHECKPOINT` saves intermediate state. `WAIT_ALL` syncs. `ASK` aggregates. `UMEM` persists. `UPDATE_GOAL` marks done.

See `examples/11-worker-pool/worker-pool.json`.

## 14. Memory-Augmented RAG

`QMEM` recalls from LTM + episodic in parallel. `FENCE` orders reads. `GUARD` (on_fail: skip) handles cold-start. `REASON` answers with or without prior context. `VERIFY` validates. `UMEM` persists new knowledge. `FENCE` orders writes. `CHECKPOINT` saves session state.

```
QMEM(ltm) ──► FENCE ──► MERGE ──► GUARD ──► REASON ──► VERIFY ──► UMEM ──► FENCE ──► CHECKPOINT
QMEM(epi) ──┘
```

See `examples/12-memory-rag/memory-rag-pipeline.json`.

## 15. Multi-Agent Negotiation → Consensus

Two agents receive the same topic, propose independently, then cross-critique each other's arguments. `THINK` synthesizes a consensus. `UMEM` persists the result.

```
SPAWN(claude) ──Control──► COMMUNICATE(topic) ──► WAIT_ALL ──► cross-MERGE ──► COMMUNICATE(counter) ──► WAIT_ALL ──► THINK ──► UMEM
SPAWN(codex)  ──Control──► COMMUNICATE(topic) ──┘                                                    ──┘
```

See `examples/13-multi-agent-negotiate/negotiate-consensus.json`.

## Quick Reference

| Pattern | Key Ops | Edge Type | Use Case |
|---------|---------|-----------|----------|
| Pipeline | ASK, THINK | Data | Sequential multi-step processing |
| Fan-out | ASK, WAIT_ALL | Data | Independent parallel tasks |
| Map-reduce | ASK, WAIT_ALL | Data | Parallel analysis + synthesis |
| Verify | ASK, VERIFY | Data | Fact-checking generated content |
| Conditional | BRANCH_ON_VALUE | Data + Control | Routing based on runtime values |
| Error handling | TRY_CATCH, ERR | Control | Recovery from failures |
| Memory | QMEM, UMEM, FENCE | Data | Persistent state across steps |
| Coordination | GUARD, CLAIM, UPDATE_GOAL | Data | Multi-agent work distribution |
| Multi-Agent (ACP) | SPAWN_AGENT, COMMUNICATE, MERGE | Control + Data | External agent orchestration |
| Iterative Refine | ASK, REFLECT, CHECKPOINT, VERIFY | Data | Self-improving draft loop |
| Plan + Fan-out | PLAN, ASK, WAIT_ALL, THINK | Data | Decompose then parallelize |
| Resilient ACP | GUARD, COMMUNICATE, VERIFY, BRANCH | Data + Control | Fault-tolerant ACP pipeline |
| Worker Pool | CLAIM, GUARD, THINK, WAIT_ALL | Data | Distributed queue processing |
| Memory RAG | QMEM, FENCE, REASON, UMEM | Data | Knowledge-augmented answering |
| Negotiation | SPAWN_AGENT, COMMUNICATE, THINK | Control + Data | Debate → consensus |
