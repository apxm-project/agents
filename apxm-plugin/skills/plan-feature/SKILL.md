---
name: plan-feature
description: Generate detailed implementation plan with gap analysis, risks, and crate ordering
user-invocable: true
---

# Plan Feature

Generates a detailed implementation plan for a feature request. Includes gap analysis, risk analysis, crate ordering (bottom-up dependency order), and concrete action items with file paths.

## What it does

This workflow creates a structured implementation plan:

1. **Architect** (Claude) — reads project structure, produces initial plan
2. **Gap Analysis** (THINK) — what exists vs what's missing
3. **Risk Analysis** (THINK) — what could go wrong, mitigation strategies
4. **Crate Ordering** (THINK) — bottom-up dependency order for implementation
5. **Final Plan** (THINK) — integrates all analyses into actionable plan

## Agents spawned

- `architect` (claude) — reads CLAUDE.md, Cargo.toml, docs/, produces plan

## Parameters

1. `feature` (str): Feature description (e.g., "Add streaming support to LLM backends")

## Usage

### Quick execution:
```bash
dekk apxm execute .agents/skills/plan-feature/plan_feature.air --emit-session "Add streaming support to LLM backends"
```

### Run pre-compiled artifact:
```bash
dekk apxm run .agents/skills/plan-feature/plan_feature.apxmobj --emit-session "Add distributed execution"
```

### Monitor progress:
```bash
ls -lt ~/.apxm/sessions/ | head -2
cat ~/.apxm/sessions/<id>/results.json
dekk apxm replay ~/.apxm/sessions/<id>
```

## Output structure

The session folder will contain:
- `nodes/01_architect/` — initial plan (feature breakdown, affected crates, API changes)
- `nodes/02_gap_analysis/` — existing components vs new components
- `nodes/03_risk_analysis/` — risks + mitigation strategies
- `nodes/04_crate_ordering/` — which crates to modify in which order
- `nodes/05_final_plan/` — integrated plan with file paths, test strategy, effort estimates
- `results.json` — final plan

## Final plan structure

The final plan includes:
- **Summary** — 1-2 sentence overview
- **Implementation Order** — numbered list of crates in dependency order
- **Detailed Steps** — per-crate changes with file paths, types, functions
- **Public API Changes** — breaking vs non-breaking
- **Test Strategy** — unit tests, integration tests, examples
- **Risks & Mitigation** — top 3 risks with mitigation
- **Estimated Effort** — development/testing/documentation time

## Crate dependency order (bottom-up)

The workflow uses APXM's actual dependency order:
1. apxm-core (no dependencies)
2. apxm-events, apxm-ais, apxm-tools, apxm-sandbox
3. apxm-credentials, apxm-backends
4. apxm-graph, apxm-artifact
5. apxm-compiler, apxm-acp
6. apxm-runtime
7. apxm-driver
8. apxm-cli, apxm-server

## When to use

Use this workflow:
- Before starting a major feature
- To estimate effort and identify risks
- To understand which crates are affected
- To plan implementation order for complex changes

## Example features

- "Add streaming support to LLM backends"
- "Add distributed execution"
- "Add Python type stubs"
- "Add GraphQL API"
- "Add checkpoint/resume to runtime"

## Notes

- The architect reads real project files (not hypothetical)
- File paths and function signatures are concrete (not generic)
- Risk analysis includes low/medium/high severity
- Effort estimates are in hours/days
