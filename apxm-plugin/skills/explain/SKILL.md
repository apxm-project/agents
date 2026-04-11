---
name: explain
description: Generate a human-readable walkthrough of what a graph does
user-invocable: true
---

# Explain

Produces a narrative walkthrough of a graph's execution flow — what each node does, what category it belongs to, how data flows between nodes, and what the overall workflow accomplishes. This is the "documentation generator" for graphs: useful when onboarding someone to an unfamiliar workflow or when reviewing a graph you haven't seen before.

## Commands

```bash
dekk apxm explain graph.air           # human-readable narrative
dekk apxm explain graph.air --json    # structured JSON output
```

## Output

**Human-readable** shows:
- Graph metadata: name, node/edge count, depth (number of phases)
- Phase-by-phase execution walkthrough
- For each node: name, operation, category, description, latency, notable attributes (template text, capability name, claim, evidence)
- Data flow: which nodes feed into each node and which nodes it feeds
- Summary: max parallelism, critical path steps, estimated runtime

**JSON output** provides structured data per node:
```json
{
  "execution_flow": [
    {"phase": 1, "parallel": false, "nodes": [
      {"id": 1, "name": "ask_question", "op": "ASK", "category": "reasoning",
       "description": "Send a prompt to the LLM and return its response",
       "latency": "slow", "latency_ms": 5000, "produces_output": true,
       "required_attributes": ["template_str"],
       "feeds": [2, 3], "depends_on": []}
    ]}
  ],
  "summary": {"max_parallelism": 3, "critical_path_steps": 4, "estimated_ms": 20000}
}
```

## Related Commands

- `dekk apxm analyze graph.air` — quantitative parallelism analysis and speedup estimates
- `dekk apxm validate graph.air` — structural validation
- `dekk apxm view graph.air` — interactive visual graph explorer
- `dekk apxm ops show ASK` — detailed info about a specific AIS operation

## When to Use

- Understanding an unfamiliar graph written by someone else
- Generating documentation for a workflow
- Reviewing data flow and dependency structure in text form
- As a complement to `view` when you prefer text over visual
