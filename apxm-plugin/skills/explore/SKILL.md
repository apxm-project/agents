---
name: explore
description: AI council with 5 parallel agents for adversarial exploration of technical questions
user-invocable: true
---

# Explore

AI council pattern: 5 agents with different perspectives explore a technical question in parallel, then adversarially synthesize into a balanced recommendation with concrete action plan.

## What it does

This workflow runs 5-way parallel exploration:

1. **5 Agents** (parallel) — each analyzes the question from a different lens:
   - Architect (systems design)
   - Adversary (what breaks, what's over-engineered)
   - Implementer (concrete Rust code)
   - Researcher (literature & industry)
   - User Advocate (user needs)

2. **Synthesis** (THINK) — merges all 5 perspectives, adversary wins on scope reduction

3. **Action Plan** (THINK) — extracts concrete next steps with priorities

## Agents spawned

- `architect` (claude) — APXM architecture perspective
- `adversary` (claude) — finds problems, challenges assumptions
- `implementer` (codex) — Rust implementation perspective
- `researcher` (claude) — industry/literature perspective
- `user_advocate` (claude) — user experience perspective

## Parameters

1. `question` (str): Technical question to explore (e.g., "Should we add distributed execution to APXM?")

## Usage

### Quick execution:
```bash
dekk apxm execute .agents/skills/explore/explore.air --emit-session "Should we add streaming support to LLM backends?"
```

### Run pre-compiled artifact:
```bash
dekk apxm run .agents/skills/explore/explore.apxmobj --emit-session "Should we add distributed execution to APXM?"
```

### Monitor progress:
```bash
ls -lt ~/.apxm/sessions/ | head -2
cat ~/.apxm/sessions/<id>/results.json
dekk apxm replay ~/.apxm/sessions/<id>
```

## Output structure

The session folder will contain:
- `nodes/01-05_<persona>/` — each agent's perspective (architect, adversary, implementer, researcher, user_advocate)
- `nodes/06_synthesis/` — merged view with recommendation (yes/no/maybe/different-approach)
- `nodes/07_action_plan/` — concrete next steps if recommendation is yes/maybe
- `results.json` — final action plan

## Synthesis policy

The synthesis intentionally lets the adversary win on scope reduction:
- If the adversary identifies over-engineering, it's taken seriously
- The final recommendation is balanced but conservative
- "No" or "different-approach" is a valid outcome

## When to use

Use this workflow when:
- Evaluating whether to add a major feature
- Deciding between architectural alternatives
- Questioning existing design decisions
- Need multiple perspectives before committing to a direction

## Example questions

- "Should we add distributed execution?"
- "Should we switch from MLIR to a custom IR?"
- "Should we add a GraphQL API?"
- "Should we support Python 3.8?"
- "Should we merge apxm-compiler and apxm-runtime into one crate?"

## Notes

- Each agent is constrained to 300 words (synthesis: 400 words)
- All 5 agents run in parallel (WAIT_ALL synchronization)
- The workflow is intentionally adversarial to avoid groupthink
- Concrete file paths and function names appear in the action plan
