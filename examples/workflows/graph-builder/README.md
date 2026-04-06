# graph-builder

A meta-workflow: give it any goal, it designs and writes the optimal APXM workflow to achieve it.

## What It Does

1. **Analyzes** your goal — identifies phases, parallelism, critical path
2. **Designs** agent topology — which agents, what roles, what profiles (claude/codex/qwen)
3. **Challenges** the design — a skeptic agent finds over-engineering and missing pieces
4. **Writes** complete `.ais` files — real, runnable APXM workflows
5. **Writes** agent profiles — focused context for each specialized agent

## Usage

```bash
dekk apxm execute examples/workflows/graph-builder/graph-builder.ais
```

It will ask you what you want to build, then design the full workflow infrastructure.

## Output

- Complete `.ais` files for entry points, phases, and sub-workflows
- Agent specialization context for each agent role
- Human-readable summary with usage instructions

## Examples

- "Build me an AI app builder that designs screens before writing code"
- "Create a research pipeline that finds, analyzes, and synthesizes academic papers"
- "Build a code review system that uses multiple models to audit PRs"
- "Create a content generation pipeline for technical blog posts"

## The Meta-Insight

clic-designer was designed by this workflow. The graph-builder itself was designed by thinking about
what makes a good APXM workflow. Every APXM workflow can be bootstrapped by graph-builder.
