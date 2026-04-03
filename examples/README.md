# APXM Examples

Learning progression for APXM agent workflows.

## Quick Start Examples

Follow these examples in order to learn APXM from basics to advanced patterns:

### 1. Basics
- **01_hello.ais** - Minimal agent with single ASK operation
- **01_hello_graph.json** - Same agent in JSON graph format
- **02_tool_use.ais** - Tool/capability invocation patterns
- **02_tool_use_graph.json** - Tool usage in graph format

### 2. Control Flow
- **03_multi_flow.ais** - Cross-agent control flow and coordination
- **03_multi_flow_graph.json** - Multi-flow in graph format

### 3. Multi-Agent Patterns
- **04_multi_agent_communicate.ais** - Agent-to-agent communication
- **05_apxm_council.ais** - Council pattern with multiple agents
- **06_code_review_council.ais** - Code review workflow with councils

### 4. Python Integration
- **python/demo_apxm_agents.py** - Python API usage examples

### 5. Flagship Demo
- **flagship/** - Production-ready 3-agent collaborative research pipeline
  - See `flagship/README.md` for details

### 6. ACP Multi-Agent (JSON graph format)
- **07-acp-agents/** - Spawn+Communicate and INV-style ACP workflows
  - See `07-acp-agents/README.md` for details

### 7. Advanced Patterns (JSON graph format)

| Folder | Pattern | Key Ops |
|--------|---------|---------|
| `08-iterative-refine/` | Self-refinement loop (unrolled 3×) | `REFLECT`, `ASK`, `CHECKPOINT`, `VERIFY`, `UMEM` |
| `09-plan-fan-out/` | Plan → parallel sections → synthesize | `PLAN`, `ASK × 3`, `WAIT_ALL`, `THINK`, `VERIFY`, `CHECKPOINT` |
| `10-resilient-acp/` | ACP pipeline with fallback agent | `GUARD`, `SPAWN_AGENT`, `COMMUNICATE`, `CHECKPOINT`, `VERIFY`, `BRANCH_ON_VALUE` |
| `11-worker-pool/` | Parallel workers claiming from a queue | `UPDATE_GOAL`, `CLAIM`, `GUARD`, `THINK × 3`, `WAIT_ALL`, `UMEM` |
| `12-memory-rag/` | Memory-augmented RAG pipeline | `QMEM`, `FENCE`, `REASON`, `VERIFY`, `UMEM`, `CHECKPOINT` |
| `13-multi-agent-negotiate/` | 2-agent debate → consensus synthesis | `SPAWN_AGENT × 2`, `COMMUNICATE`, `WAIT_ALL`, `THINK`, `UMEM` |

## Running Examples

Compile and run an example:

```bash
# Compile AIS source to binary
apxm compile examples/01_hello.ais -o hello.apxm

# Execute the compiled binary
apxm execute hello.apxm

# Or compile from JSON graph
apxm compile examples/01_hello_graph.json -o hello.apxm

# Validate any JSON graph before running
apxm validate examples/09-plan-fan-out/plan-fan-out.json

# Run a JSON graph directly
apxm execute examples/09-plan-fan-out/plan-fan-out.json
```

See `docs/getting-started.md` for detailed tutorials.
