# 10 — Resilient ACP Pipeline (with Fallback)

**Pattern:** `GUARD → SPAWN → COMMUNICATE → CHECKPOINT → VERIFY → BRANCH → fallback COMMUNICATE → synthesize ASK`

## What it demonstrates

- `GUARD` validates input before processing (halts on null)
- Primary Claude agent performs code review via `COMMUNICATE`
- `CHECKPOINT` saves state after the primary analysis (on_fail: continue)
- `VERIFY` validates the review quality against acceptance criteria
- `BRANCH_ON_VALUE` routes to a fallback Codex agent if the primary review is insufficient
- Final `ASK` synthesizes both analyses (primary + fallback) into a consolidated report

## Use case

Code review pipeline that self-heals: if the primary agent produces a substandard review, it automatically falls back to a second agent and combines both perspectives.

## Running

```bash
apxm agent test claude
apxm agent test codex
apxm validate examples/10-resilient-acp/resilient-acp-pipeline.json
apxm execute examples/10-resilient-acp/resilient-acp-pipeline.json
```
