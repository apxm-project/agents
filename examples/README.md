# APXM Examples

## Directory Structure

```
examples/
├── basics/         Single-agent fundamentals (ASK, tool use)
├── multi-agent/    Multi-agent coordination and councils (.ais format)
├── acp-agents/     ACP protocol workflows (SPAWN + COMMUNICATE, INV)
├── patterns/       Advanced graph patterns (JSON format)
│   ├── iterative-refine/       Self-refinement loop
│   ├── plan-fan-out/           Plan → parallel sections → synthesize
│   ├── resilient-acp/          ACP pipeline with fallback agent
│   ├── worker-pool/            Parallel workers claiming from a queue
│   ├── memory-rag/             Memory-augmented RAG pipeline
│   └── multi-agent-negotiate/  2-agent debate → consensus
└── workflows/      Production meta-workflows
```

## Basics

| File | Description |
|------|-------------|
| `basics/hello.ais` | Minimal agent with single ASK operation |
| `basics/hello_graph.json` | Same agent in JSON graph format |
| `basics/tool_use.ais` | Tool/capability invocation patterns |
| `basics/tool_use_graph.json` | Tool usage in graph format |

## Multi-Agent

| File | Description |
|------|-------------|
| `multi-agent/multi_flow.ais` | Cross-agent control flow and coordination |
| `multi-agent/multi_flow_graph.json` | Multi-flow in graph format |
| `multi-agent/multi_agent_communicate.ais` | Agent-to-agent communication via COMMUNICATE |
| `multi-agent/parallel_council_graph.json` | Fan-out/fan-in council pattern |
| `multi-agent/apxm_council.ais` | Council pattern with multiple experts |
| `multi-agent/code_review_council.ais` | Code review workflow with councils |

## ACP Agents

Spawn+Communicate and INV-style ACP workflows. See `acp-agents/README.md`.

## Patterns

| Folder | Pattern | Key Ops |
|--------|---------|---------|
| `patterns/iterative-refine/` | Self-refinement loop (unrolled 3x) | `REFLECT`, `ASK`, `CHECKPOINT`, `VERIFY`, `UMEM` |
| `patterns/plan-fan-out/` | Plan → parallel sections → synthesize | `PLAN`, `ASK x 3`, `WAIT_ALL`, `THINK`, `VERIFY`, `CHECKPOINT` |
| `patterns/resilient-acp/` | ACP pipeline with fallback agent | `GUARD`, `SPAWN_AGENT`, `COMMUNICATE`, `CHECKPOINT`, `VERIFY`, `BRANCH_ON_VALUE` |
| `patterns/worker-pool/` | Parallel workers claiming from a queue | `UPDATE_GOAL`, `CLAIM`, `GUARD`, `THINK x 3`, `WAIT_ALL`, `UMEM` |
| `patterns/memory-rag/` | Memory-augmented RAG pipeline | `QMEM`, `FENCE`, `REASON`, `VERIFY`, `UMEM`, `CHECKPOINT` |
| `patterns/multi-agent-negotiate/` | 2-agent debate → consensus synthesis | `SPAWN_AGENT x 2`, `COMMUNICATE`, `WAIT_ALL`, `THINK`, `UMEM` |

## Workflows

| Folder | Description |
|--------|-------------|
| `workflows/` | 13-agent meta-workflow for building new APXM features |

## Running Examples

```bash
# Compile AIS source to binary
apxm compile examples/basics/hello.ais -o hello.apxm

# Execute the compiled binary
apxm execute hello.apxm

# Compile from JSON graph
apxm compile examples/basics/hello_graph.json -o hello.apxm

# Validate any JSON graph before running
apxm validate examples/patterns/plan-fan-out/plan-fan-out.json

# Run a JSON graph directly
apxm execute examples/patterns/plan-fan-out/plan-fan-out.json
```

See `docs/guides/getting-started.md` for detailed tutorials.
