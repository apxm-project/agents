---
name: init
description: Scaffold a new APXM project with standard directory structure
user-invocable: true
---

# Init

Creates a new APXM project directory with the standard layout for organizing agents, workflows, nodes, prompts, and tools. Generates an `apxm.toml` configuration file with sensible defaults.

## Commands

```bash
dekk apxm init my-project         # create my-project/ with scaffolded structure
```

## Created Structure

```
my-project/
  agents/       # agent profile definitions
  flows/        # workflow AIR files
  nodes/        # reusable node fragments
  prompts/      # prompt templates
  tools/        # external tool definitions
  apxm.toml     # project configuration (name, version, build/runtime settings)
```

## When to Use

- Starting a new agent workflow project from scratch
- When you want the conventional directory layout for organizing graph sources, node fragments, prompts, and tools
