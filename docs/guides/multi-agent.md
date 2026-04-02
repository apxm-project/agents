# Multi-Agent Workflows

APXM orchestrates external agents (Claude Code, Codex, custom tools) as first-class participants in dataflow graphs. This guide covers spawning agents, sending them work, and composing multi-agent patterns.

## 1. Concepts

APXM distinguishes three kinds of participants:

| Participant | What it is | How APXM uses it |
|-------------|-----------|-------------------|
| **Agent** | Self-contained process with its own LLM and tools (Claude Code, Codex, etc.) | Spawned via `SPAWN_AGENT`, messaged via `COMMUNICATE` |
| **LLM** | A model backend you register with APXM | Powers internal `ASK`, `THINK`, `REASON` nodes |
| **Tool** | A stateless capability (shell command, HTTP endpoint, MCP server) | Invoked by `INV` nodes |

Key distinction: agents are autonomous processes that manage their own context, tool access, and execution. APXM does not give agents your LLM credentials -- it communicates with them over ACP (Agent Communication Protocol), sending prompts and receiving results.

## 2. Setup

Register at least one LLM backend for APXM's own nodes, then verify that agents are reachable:

```bash
# Register an LLM backend for ASK/THINK/REASON nodes
apxm llm add my-openai --provider openai --api-key sk-...

# List available agents
apxm agent list

# Test connectivity to specific agents
apxm agent test claude
apxm agent test codex
```

The `apxm agent test` command sends a lightweight ping over ACP and reports the agent's status, version, and supported modes.

## 3. Two Authoring Styles

APXM supports two styles for working with agents. Both produce valid graphs.

> **Note on JSON formats:** Style A examples below use the **runtime token format** (`op_type`, `input_tokens`, `output_tokens`, `entry_nodes`, `exit_nodes`) — this is the format emitted by the compiler and used in `examples/07-acp-agents/`. Style B examples use the **ApxmGraph IR format** (`op`, `name`, `parameters`) — this is the format accepted by `apxm validate` and described in CLAUDE.md's Graph JSON Contract. Both are valid; the runtime handles conversion automatically.

### Style A: SPAWN_AGENT + COMMUNICATE (explicit lifecycle)

You control the agent's full lifecycle: spawn it, send messages, collect results. This style uses the low-level AIS operations `SPAWN_AGENT`, `COMMUNICATE`, `MERGE`, and `PRINT`.

- `SPAWN_AGENT` starts an agent process. The `Control` edge from a spawn node ensures the agent is ready before any `COMMUNICATE` targets it.
- `COMMUNICATE` sends a message to a named agent and returns its response. The `protocol` attribute is always `"acp"`.
- `MERGE` collects multiple token streams into one.

Here is a complete graph that spawns two agents in parallel, asks them the same question, and merges their answers:

```json
{
  "metadata": {
    "name": "default.main",
    "is_entry": true,
    "description": "Parallel: spawn Claude and Codex, compare their analyses"
  },
  "nodes": [
    {
      "id": 1,
      "op_type": "SPAWN_AGENT",
      "attributes": { "agent_name": "claude_analyst", "profile": "claude" },
      "input_tokens": [], "output_tokens": [101], "metadata": {}
    },
    {
      "id": 2,
      "op_type": "SPAWN_AGENT",
      "attributes": { "agent_name": "codex_analyst", "profile": "codex" },
      "input_tokens": [], "output_tokens": [102], "metadata": {}
    },
    {
      "id": 3,
      "op_type": "CONST_STR",
      "attributes": { "value": "Analyze the architecture of this project and suggest improvements." },
      "input_tokens": [], "output_tokens": [103], "metadata": {}
    },
    {
      "id": 4,
      "op_type": "COMMUNICATE",
      "attributes": { "recipient": "claude_analyst", "protocol": "acp" },
      "input_tokens": [103], "output_tokens": [104], "metadata": {}
    },
    {
      "id": 5,
      "op_type": "COMMUNICATE",
      "attributes": { "recipient": "codex_analyst", "protocol": "acp" },
      "input_tokens": [103], "output_tokens": [105], "metadata": {}
    },
    {
      "id": 6,
      "op_type": "MERGE",
      "attributes": {},
      "input_tokens": [104, 105], "output_tokens": [106], "metadata": {}
    },
    {
      "id": 7,
      "op_type": "PRINT",
      "attributes": {},
      "input_tokens": [106], "output_tokens": [107], "metadata": {}
    }
  ],
  "edges": [
    {"from": 1, "to": 4, "dependency": "Control"},
    {"from": 2, "to": 5, "dependency": "Control"},
    {"from": 3, "to": 4, "dependency": "Data"},
    {"from": 3, "to": 5, "dependency": "Data"},
    {"from": 4, "to": 6, "dependency": "Data"},
    {"from": 5, "to": 6, "dependency": "Data"},
    {"from": 6, "to": 7, "dependency": "Data"}
  ],
  "entry_nodes": [1, 2, 3],
  "exit_nodes": [7]
}
```

Edge breakdown:
- `Control` edges from `SPAWN_AGENT` to `COMMUNICATE` ensure agents are alive before messages are sent.
- `Data` edges from `CONST_STR` to `COMMUNICATE` carry the prompt payload.
- `Data` edges into `MERGE` collect both responses.

### Style B: INV with ACP capability (session-managed)

The `INV` operation with `capability: "acp"` handles agent lifecycle automatically. You specify the agent, prompt, and optional session handle in `params_json`. This is more concise and better suited for pipelines where you don't need fine-grained lifecycle control.

```json
{
  "name": "parallel-review",
  "parameters": [{"name": "question", "type_name": "str"}],
  "nodes": [
    {
      "id": 1,
      "name": "ask_codex",
      "op": "INV",
      "attributes": {
        "capability": "acp",
        "params_json": "{\"agent\": \"codex\", \"prompt\": \"Answer: {0}\"}",
        "timeout_ms": 300000
      }
    },
    {
      "id": 2,
      "name": "ask_claude",
      "op": "INV",
      "attributes": {
        "capability": "acp",
        "params_json": "{\"agent\": \"claude\", \"prompt\": \"Answer: {0}\"}",
        "timeout_ms": 300000
      }
    },
    {
      "id": 3,
      "name": "compare",
      "op": "ASK",
      "attributes": {
        "template_str": "Compare these answers:\nCodex: {0}\nClaude: {1}\nWhich is better and why?",
        "model_policy": "fast"
      }
    }
  ],
  "edges": [
    {"from": 1, "to": 3, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Data"}
  ],
  "metadata": {
    "description": "Ask two ACP agents the same question in parallel, then compare their answers"
  }
}
```

Key differences from Style A:
- No explicit `SPAWN_AGENT` -- the runtime manages agent lifecycle.
- `params_json` carries the agent name, prompt, and optional `session_handle` and `mode`.
- `timeout_ms` sets a deadline for the agent response.
- Uses the standard graph contract (`name`, `nodes`, `edges`, `parameters`, `metadata`) rather than the token-based format.

### When to use which

| Style | Best for | Trade-off |
|-------|----------|-----------|
| Style A (SPAWN + COMMUNICATE) | Fine-grained lifecycle, multi-turn conversations, cross-agent data flow | More verbose, explicit control |
| Style B (INV + ACP) | Simple delegation, parallel fan-out, session-managed turns | Less control, cleaner graphs |

## 4. Patterns

### Pattern 1: Parallel Review

Ask multiple agents the same question in parallel, then synthesize. This is the multi-agent version of the fan-out pattern. See the Style B example in [Section 3](#style-b-inv-with-acp-capability-session-managed) above — nodes 1 and 2 have no edges between them, so the scheduler runs them concurrently, and node 3 waits for both via `Data` edges.

### Pattern 2: Pipeline (Architect, Implement, Review)

Chain agents sequentially. Each step's output feeds the next agent. This uses Style A with explicit lifecycle management and multi-turn communication:

```json
{
  "metadata": {
    "name": "default.main",
    "is_entry": true,
    "description": "Full SDLC: architect designs, coder implements, reviewer reviews"
  },
  "nodes": [
    {
      "id": 1,
      "op_type": "SPAWN_AGENT",
      "attributes": { "agent_name": "architect", "profile": "claude", "mode": "architect" },
      "input_tokens": [], "output_tokens": [101], "metadata": {}
    },
    {
      "id": 2,
      "op_type": "CONST_STR",
      "attributes": { "value": "Design a caching layer for the API. Provide the implementation plan." },
      "input_tokens": [], "output_tokens": [102], "metadata": {}
    },
    {
      "id": 3,
      "op_type": "COMMUNICATE",
      "attributes": { "recipient": "architect", "protocol": "acp" },
      "input_tokens": [102], "output_tokens": [103], "metadata": {}
    },
    {
      "id": 4,
      "op_type": "SPAWN_AGENT",
      "attributes": { "agent_name": "coder", "profile": "claude" },
      "input_tokens": [], "output_tokens": [104], "metadata": {}
    },
    {
      "id": 5,
      "op_type": "COMMUNICATE",
      "attributes": { "recipient": "coder", "protocol": "acp" },
      "input_tokens": [103], "output_tokens": [105], "metadata": {}
    },
    {
      "id": 6,
      "op_type": "COMMUNICATE",
      "attributes": { "recipient": "architect", "protocol": "acp" },
      "input_tokens": [105], "output_tokens": [106], "metadata": {}
    },
    {
      "id": 7,
      "op_type": "PRINT",
      "attributes": {},
      "input_tokens": [106], "output_tokens": [107], "metadata": {}
    }
  ],
  "edges": [
    {"from": 1, "to": 3, "dependency": "Control"},
    {"from": 2, "to": 3, "dependency": "Data"},
    {"from": 3, "to": 4, "dependency": "Control"},
    {"from": 3, "to": 5, "dependency": "Data"},
    {"from": 4, "to": 5, "dependency": "Control"},
    {"from": 5, "to": 6, "dependency": "Data"},
    {"from": 6, "to": 7, "dependency": "Data"}
  ],
  "entry_nodes": [1, 2],
  "exit_nodes": [7]
}
```

The flow is: spawn architect (1) -> ask architect to design (3) -> spawn coder (4) -> send design to coder (5) -> send implementation back to architect for review (6) -> print final review (7). Notice how the `Control` edge from node 3 to node 4 delays spawning the coder until the design is ready.

The same pipeline in Style B (INV with session handles for multi-turn):

```json
{
  "name": "multi-turn-review",
  "parameters": [{"name": "repo", "type_name": "str"}],
  "nodes": [
    {
      "id": 1,
      "name": "analyze",
      "op": "INV",
      "attributes": {
        "capability": "acp",
        "params_json": "{\"agent\": \"claude\", \"prompt\": \"Analyze the codebase at {0} for potential issues\", \"session_handle\": \"reviewer\", \"mode\": \"architect\"}"
      }
    },
    {
      "id": 2,
      "name": "fix",
      "op": "INV",
      "attributes": {
        "capability": "acp",
        "params_json": "{\"agent\": \"claude\", \"prompt\": \"Now fix the top 3 issues you identified\", \"session_handle\": \"reviewer\", \"mode\": \"code\"}"
      }
    },
    {
      "id": 3,
      "name": "verify",
      "op": "INV",
      "attributes": {
        "capability": "acp",
        "params_json": "{\"agent\": \"claude\", \"prompt\": \"Run the tests and verify your fixes work\", \"session_handle\": \"reviewer\"}"
      }
    }
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Data"}
  ],
  "metadata": {
    "description": "Multi-turn review: analyze, fix, and verify using a shared ACP session"
  }
}
```

The `session_handle: "reviewer"` ensures all three turns share the same agent session, preserving context across turns.

### Pattern 3: Cross-Critique

Two agents independently propose solutions, then each critiques the other's proposal. This creates a diamond-shaped dataflow:

```json
{
  "metadata": {
    "name": "default.main",
    "is_entry": true,
    "description": "Cross-critique: two agents propose, then critique each other"
  },
  "nodes": [
    {
      "id": 1,
      "op_type": "SPAWN_AGENT",
      "attributes": { "agent_name": "agent_a", "profile": "claude" },
      "input_tokens": [], "output_tokens": [101], "metadata": {}
    },
    {
      "id": 2,
      "op_type": "SPAWN_AGENT",
      "attributes": { "agent_name": "agent_b", "profile": "codex" },
      "input_tokens": [], "output_tokens": [102], "metadata": {}
    },
    {
      "id": 3,
      "op_type": "CONST_STR",
      "attributes": { "value": "Propose a design for a REST API for a todo application." },
      "input_tokens": [], "output_tokens": [103], "metadata": {}
    },
    {
      "id": 4,
      "op_type": "COMMUNICATE",
      "attributes": { "recipient": "agent_a", "protocol": "acp" },
      "input_tokens": [103], "output_tokens": [104], "metadata": {}
    },
    {
      "id": 5,
      "op_type": "COMMUNICATE",
      "attributes": { "recipient": "agent_b", "protocol": "acp" },
      "input_tokens": [103], "output_tokens": [105], "metadata": {}
    },
    {
      "id": 6,
      "op_type": "COMMUNICATE",
      "attributes": { "recipient": "agent_b", "protocol": "acp" },
      "input_tokens": [104], "output_tokens": [106], "metadata": {}
    },
    {
      "id": 7,
      "op_type": "COMMUNICATE",
      "attributes": { "recipient": "agent_a", "protocol": "acp" },
      "input_tokens": [105], "output_tokens": [107], "metadata": {}
    },
    {
      "id": 8,
      "op_type": "MERGE",
      "attributes": {},
      "input_tokens": [106, 107], "output_tokens": [108], "metadata": {}
    },
    {
      "id": 9,
      "op_type": "PRINT",
      "attributes": {},
      "input_tokens": [108], "output_tokens": [109], "metadata": {}
    }
  ],
  "edges": [
    {"from": 1, "to": 4, "dependency": "Control"},
    {"from": 2, "to": 5, "dependency": "Control"},
    {"from": 3, "to": 4, "dependency": "Data"},
    {"from": 3, "to": 5, "dependency": "Data"},
    {"from": 4, "to": 6, "dependency": "Data"},
    {"from": 5, "to": 7, "dependency": "Data"},
    {"from": 5, "to": 6, "dependency": "Control"},
    {"from": 4, "to": 7, "dependency": "Control"},
    {"from": 6, "to": 8, "dependency": "Data"},
    {"from": 7, "to": 8, "dependency": "Data"},
    {"from": 8, "to": 9, "dependency": "Data"}
  ],
  "entry_nodes": [1, 2, 3],
  "exit_nodes": [9]
}
```

Execution flow:
1. Both agents are spawned in parallel (nodes 1, 2).
2. Both receive the same prompt and produce proposals in parallel (nodes 4, 5).
3. Agent A's proposal goes to Agent B for critique (node 6); Agent B's proposal goes to Agent A (node 7). The `Control` edges from nodes 4 and 5 ensure both proposals are ready before critiques begin.
4. Both critiques merge (node 8) and print (node 9).

## 5. Custom Agent Registration

Register agents that are not built-in:

```bash
# Register a custom agent that speaks ACP
apxm agent add my-agent --command "my-cli --acp" --permissions approve-reads

# List all registered agents
apxm agent list

# Test connectivity
apxm agent test my-agent

# Remove an agent
apxm agent remove my-agent
```

Registered agents become available by name in `SPAWN_AGENT` attributes and `INV` `params_json`. The `--permissions` flag controls what the agent is allowed to do (file reads, writes, shell commands).

## 6. Running and Debugging

### Validate

Check that a multi-agent graph is well-formed before running it:

```bash
apxm validate workflow.json
```

The validator checks DAG constraints, operation names, required attributes, and that all `COMMUNICATE` nodes reference agents that appear in a `SPAWN_AGENT` node (or are registered via `apxm agent add`).

### Analyze

Inspect the execution plan to understand parallelism and the critical path:

```bash
apxm analyze workflow.json
```

For multi-agent graphs, this shows which agents run concurrently and where synchronization points (MERGE, WAIT_ALL) introduce sequential bottlenecks.

### Execute

Compile and run the graph:

```bash
apxm execute workflow.json
```

Add `--trace info` for detailed execution logs showing agent spawn times, message round-trips, and merge points:

```bash
apxm execute workflow.json --trace info
```

Use `--emit-metrics` to write runtime statistics (agent latencies, token counts) to a JSON file:

```bash
apxm execute workflow.json --emit-metrics metrics.json
```

## Quick Reference

| Operation | Purpose | Key Attributes |
|-----------|---------|----------------|
| `SPAWN_AGENT` | Start an agent process | `agent_name`, `profile`, `mode` |
| `COMMUNICATE` | Send message, receive response | `recipient`, `protocol` |
| `MERGE` | Combine multiple token streams | (none) |
| `CONST_STR` | Provide a literal string value | `value` |
| `INV` (ACP) | Session-managed agent call | `capability: "acp"`, `params_json` |

## Next Steps

- Browse the example graphs: `examples/07-acp-agents/`
- See all AIS operations: `apxm ops list`
- Full CLI reference for `apxm agent`, `apxm llm`, `apxm tool`: [CLI Reference](cli-reference.md)
- Learn single-agent patterns: [Common Patterns](common-patterns.md)
- Build your first graph: [First Graph](first-graph.md)
