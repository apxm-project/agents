---
name: task/merge
description: Merge multiple graph fragments into one composed workflow
user-invocable: true
---

# Task Merge

Combines multiple graph JSON files into a single composed workflow. This is the composition primitive — take independently authored graph fragments and merge them into one graph with proper node ID mapping and optional synchronization points.

Useful when different team members author different parts of a workflow, or when you want to compose reusable graph fragments from `nodes/` into a complete flow.

## Commands

```bash
dekk apxm task merge a.air b.air --name combined              # merge and print
dekk apxm task merge a.air b.air --name combined -o out.air  # merge and write to file
dekk apxm task merge a.air b.air c.air --name pipeline       # merge three fragments
dekk apxm task merge a.air b.air --name combined --json       # machine-readable output
```

## How It Works

1. Loads all input graph JSON files
2. Remaps node IDs to avoid collisions (each graph gets a unique ID range)
3. Adds synchronization points where fragments connect
4. Produces a single valid `ApxmGraph` with the specified name

## Output

**Human-readable**: merge summary showing input graph count, total nodes/edges, and sync node ID (if added).

**JSON** (`--json`):
```json
{
  "merged_graph": { "name": "combined", "nodes": [...], "edges": [...] },
  "stats": {"input_graphs": 2, "total_nodes": 8, "total_edges": 10, "sync_node_id": 9}
}
```

## When to Use

- Composing reusable graph fragments into complete workflows
- Combining independently authored workflow sections
- Building complex pipelines from simpler, tested building blocks
- Merging template-generated graphs with custom additions
