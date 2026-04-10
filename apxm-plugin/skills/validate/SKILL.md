---
name: validate
description: Validate a graph JSON against the AIS contract
user-invocable: true
---

# Validate

Checks a graph JSON file against the AIS (Agent Instruction Set) contract without compiling it. This is a fast, static check that catches structural errors — invalid operations, missing attributes, broken edges, cycles — before you spend time on compilation. Think of it as the "linter" for agent workflows.

Validation is the first thing to run after editing a graph. If `validate` passes, the graph is structurally sound and ready for `compile` or `execute`.

## Commands

```bash
dekk apxm validate graph.ais              # human-readable validation output
dekk apxm validate graph.ais --json       # machine-readable JSON (for tooling)
```

## What Gets Checked

**Errors (fail validation):**
- Graph name must not be empty
- Must contain at least one node
- Node IDs must be non-zero and unique across the graph
- Node names must be non-empty
- Operation type (`op`) must be a valid AIS operation (39 ops — use `dekk apxm ops list` to see them)
- Required attributes must be present per operation (e.g., ASK requires `template_str`)
- Edges must reference existing nodes (no dangling `from`/`to` IDs)
- No self-loop edges (an edge cannot go from a node to itself)
- Dependency types must be `Data`, `Control`, or `Effect`
- The graph must be a DAG — no cycles allowed (checked via Kahn's algorithm)
- Parameter names must be non-empty and unique

**Warnings (pass with notes):**
- Non-standard parameter types (when not one of `str`, `int`, `float`, `bool`, `json`)

## Output Format

**Human-readable** (default):
```
✓ graph.ais — valid
```
or:
```
✗ graph.ais — 2 errors
  ✗ node 'greeting' (id=1, op=ASK) missing required attribute 'template_str'
  ✗ edge 5->10 references non-existent source node 5
```

**JSON** (`--json`):
```json
{
  "file": "graph.ais",
  "valid": false,
  "errors": ["node 'greeting' (id=1, op=ASK) missing required attribute 'template_str'"],
  "warnings": []
}
```

Exit code is 0 if valid, non-zero if errors. Warnings do not affect the exit code.

## Related Commands

- `dekk apxm analyze graph.ais` — parallelism analysis, critical path, speedup estimate
- `dekk apxm explain graph.ais` — human-readable walkthrough of what the graph does
- `dekk apxm ops show ASK` — see required/optional attributes for any operation
- `dekk apxm view graph.ais` — interactive visual graph explorer

## When to Use

- After editing a graph JSON, before compiling
- In CI pipelines as a fast pre-compilation check
- When debugging "why won't my graph compile" — validate first to isolate structural issues
- With `--json` for integration into automated tooling
