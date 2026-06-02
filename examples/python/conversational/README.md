# Conversational agents

A conversational agent in APXM is **two layers**:

1. **The agent body** — an APXM graph authored in this frontend, run *once per
   user message*. The runtime is single-shot by design ("one DAG = one turn"):
   it handles the full per-turn lifetime automatically (pre/post hooks via
   dispatcher middleware, the internal model→tool→model loop, response parsing,
   token accounting).
2. **The host turn-loop** — `apxm chat`, which owns the open-ended conversation:
   it threads a stable `session_id` (so server-side memory accrues across turns)
   and the running transcript back into the graph as the `conversation`
   parameter each turn.

## Run it

```bash
# Author the agent body and emit its AIR
dekk apxm execute examples/python/conversational/chat_agent.py --emit-air > chat.air

# Start a server, then drive the agent conversationally
apxm chat --air chat.air
# or with the built-in single-ASK chat graph:
apxm chat
```

In the REPL:

- type a message → one turn runs against `POST /v1/execute/stream`
- `--tree` renders the per-agent dispatch tree (spawns, tool calls) live
- meta-commands: `/tools` `/skills` `/agents` `/help` `/exit`
- if a turn needs a **write** capability, the REPL prompts
  `grant write capability '<cap>' for this session? [y/N]` and, on approval,
  retries the turn with that grant (the same `admit_capabilities` consent path
  used by `--admit`).

## What `chat_agent.py` demonstrates

The conversational multi-agent authoring surface, all from the frontend. The
example uses constructs that pass `apxm validate` (the text-AIR path that
`apxm chat --air` uses):

| Step | Construct | What it shows |
|------|-----------|---------------|
| plan | `g.reason` | intent planning over the transcript |
| answer | `g.ask(tool_groups=[...])` | tool-using turn (group is self-enabling, least privilege) |
| worker | `g.spawn_agent` | spawning a sub-agent |
| team | `g.spawn_team` | a named team resolved from `~/.apxm/teams.toml` |
| reply | `g.ask` | synthesizing the user-facing answer |

Continuity is handled by the host: `apxm chat` threads the whole transcript as
`conversation` each turn, and the runtime additionally supports session-scoped
QMEM/UMEM (memory keyed by `session_id` — a later turn reads an earlier turn's
write).

For a bounded in-graph refinement loop, use the `g.loop(count=N)` context
manager (see `apxm.Loop`). Open-ended iteration belongs in the host turn-loop.

## End-to-end (mock backend, no GPU)

A repeatable smoke test lives at `.apxm/scratch/smoke_mock_server.sh` (gitignored).
It launches a mock-backed `apxm-server` on `:18800`, then proves:

- a one-turn round-trip (`17 + 25` → the mock's deterministic `The answer is 42.`),
- the `apxm chat` REPL over two turns,
- `/tools` discovery.

A multi-node agent (`reason → ask → ask`) executes with the full per-node
lifecycle visible in the SSE stream (`operation_start/end`, `llm_prompt`,
`token_usage`, `node_metrics`, `scheduler_decision`, `execute_complete`).

### text-AIR round-trip — ALL conversational ops FIXED

Every op the conversational agent uses now round-trips and passes
`apxm validate`: `call_skill`, `qmem`/`umem`, `delegate`, `loop_start`/`loop_end`
(carrying `max_iterations`), `plan`, `fence`, and `negotiate` were reconciled
across `AISOps.td` + the C++ verifiers + the Rust reverse emitter
(`air_builder/emit.rs`) + the generated Python emitter. The one structural
caveat: in-graph `loop` iteration is single-pass (the scheduler is fire-once) —
use the host turn-loop or `AUTONOMOUS` for real iteration.

## Context compaction (host post-hook)

The REPL compacts the transcript after each turn when it exceeds a token budget:
the oldest turns are folded into a running `summary` (cumulative) while the most
recent turns stay verbatim, via a built-in `chat_summarize.air`. `/compact`
forces it. Durable detail can be persisted to session memory (`umem`) before
dropping. This is the runtime's `on_graph_finished` post-hook concept realized at
the host level (where the transcript actually lives).

## Compiler + dispatch as an MCP server (`/v1/mcp`)

Any MCP client (this agent, apxm-studio, Claude Code) can drive the compiler and
runtime over the existing JSON-RPC facade — a thin, DRY layer over the same
handlers as the REST API:

- `apxm_compile` / `apxm_validate` (PURE): compile-check AIR; a compile error is
  a normal result (`ok:false` + diagnostics), not a protocol error.
- `apxm_ops_list` (PURE): the AIS op vocabulary.
- `apxm_run` (side-effecting): compile + run AIR — the safe "agent writes IR →
  dispatch" path; writes require `admit_capabilities`.
- `apxm_dispatch` (side-effecting): the agent emits a **constrained spec**
  (`{sub_agents:[{id,role,prompt,depends_on}]}`); the server validates it
  (breadth cap, dup/dangling/cycle checks) and templates it to a fan-out graph,
  then runs it. Deterministic topology, model-driven decision — no raw codegen in
  the default path.
- `apxm_skill_call` (pre-existing): invoke a vetted installed skill by id.

### Security: one no-widen boundary, enforced at the invoke chokepoint

The write boundary is enforced at the runtime `inv_tool` invoke site (not only
the server's static pre-flight), so it holds for **every** path — raw execute,
`CALL_SKILL` child DAGs, `apxm_dispatch`, and `SPAWN_AGENT`. The effective grant
(`SIDE_EFFECT_POLICY`) is seeded from `admit_capabilities` at the top level and
propagated to children with no-widen (`child ⊆ parent`). Read-only and sandboxed
capabilities are always allowed; a Direct write runs only if the execution's
grant admits it.
