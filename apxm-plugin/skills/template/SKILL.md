---
name: template
description: List and emit starter graph templates for common patterns
user-invocable: true
---

# Template

Provides ready-to-use starter graphs for common workflow patterns. Instead of writing a graph from scratch, pick a template, emit it as JSON, and customize it. Templates are valid `ApxmGraph` objects that pass validation out of the box.

## Commands

```bash
dekk apxm template list                   # show all available templates
dekk apxm template list --json            # machine-readable list
dekk apxm template show fan-out           # display template with description and hints
dekk apxm template show fan-out --json    # emit pure JSON (pipe to file or tools)
```

## Built-in Templates

| Template | Pattern | Description |
|----------|---------|-------------|
| `ask` | Single node | Simplest possible graph — one LLM call |
| `pipeline` | Sequential chain | Draft, review, refine — a linear multi-step workflow |
| `fan-out` | Parallel branches | Spawn parallel tasks with synchronization barrier |
| `map-reduce` | Fan-out + aggregate | Parallel execution followed by synthesis/reduce |
| `verify` | Generate + check | Claim generation then fact-checking verification |
| `conditional` | Branch on value | Route execution to different paths based on a condition |

User-defined templates can be added to `~/.apxm/templates.json` and will appear alongside built-in ones.

## Typical Workflow

```bash
dekk apxm template show fan-out --json > workflow.air   # start from template
# edit workflow.air to customize nodes, prompts, tools
dekk apxm validate workflow.air                          # check structure
dekk apxm view workflow.air                              # visualize
dekk apxm execute workflow.air                           # run it
```

## When to Use

- Starting a new graph and you want a proven structure to build on
- Learning APXM — templates show idiomatic patterns for common scenarios
- Quickly prototyping a workflow without memorizing the graph JSON contract
