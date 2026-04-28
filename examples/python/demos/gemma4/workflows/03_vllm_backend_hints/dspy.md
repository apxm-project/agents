# DSPy analysis — `03_vllm_backend_hints`

## Did DSPy fire on this workflow? **No.**

`O2-precompile.json::pass_summary.fired_passes = ["scheduling", "assign-priority"]` — no `dspy-optimize`. No `tokens_saved`. The 4 background_audit Asks and the 3 critical-chain ops all ran with their hand-authored prompts verbatim.

## Why it didn't fire

Same gating chain as cases 01 and 02. `prompt_optimization_configured()` returned `false`; no `[compiler.dspy].training_data` (or legacy `[prompt_tuning].training_data`) in `~/.apxm/config.toml`. The pass is silently skipped and no DSPy module attrs are stamped.

See `01_review_synthesis_skill/dspy.md` for the full trace.

## What DSPy *would* do — and why it's a poor fit for this workflow

This workflow is the **least appropriate** of the three for DSPy optimization. Reasons:

1. **The 4 background audits are intentionally *not* shared-prefix.** Each one inlines per-lane `_pressure_facts(lane, …)` at the top of the prompt. A DSPy MIPROv2 optimizer would propose a single rewritten instruction per node, which is fine, but there's no cross-node coupling to exploit (unlike case 01's 6 aspects).

2. **The background audits are designed to be expensive prefills with truncated outputs** (`token_budget=6000` declared, ~15 tokens actually produced). The workflow uses them as backend pressure, not for content quality. DSPy optimizes for *answer quality* against a trainset's expected outputs. There is no quality target here.

3. **The critical chain (`triage → plan → summary`) has only 7 LLM calls total in the entire workflow.** With so few prompts, the cost of the DSPy subprocess fork + cache miss + MIPROv2 trial likely exceeds the per-request token savings.

If you did enable DSPy with a trainset, the most defensible scope would be **only** the critical chain:
- `critical_triage`: structured incident classification — `contains_match` against expected category labels.
- `critical_plan`: structured remediation plan — `llm_judge` (since the plan format is open).
- `critical_summary`: one-paragraph executive summary — `token_overlap`.

Trainset rows would look like:

```jsonl
{"node": "critical_triage", "inputs": {"dossier": "..."}, "expected": "severity=high, category=auth, ..."}
{"node": "critical_summary", "inputs": {"plan": "..."}, "expected": "Mitigation deployed at ..."}
```

There is no reason to optimize the 4 background_audit prompts; they are not consumed for content.

## Interaction with backend hints (the workflow's actual purpose)

DSPy optimization runs at compile time and rewrites the prompt body. It does **not** touch the per-node `priority` attribute, the `extra_body.apxm_hints` payload, or the `pin_policy` settings. So enabling DSPy here would not change the priority-scheduling story documented in `runtime.md` — those are runtime-side concerns. DSPy only changes *what bytes get sent*; the backend hints control *how those bytes get scheduled by vLLM*.

If the goal is to make this workflow's O2 numbers compelling, DSPy is the wrong lever. The right levers are the runtime-side ones called out in `runtime.md`: `concurrency_limit`, vLLM saturation, `pin_policy.mode="prefix"` for the critical chain's small repeated prefix.

## Reading the next sweep

If DSPy were enabled on this workflow you'd see:
- `pass_summary.fired_passes` includes `"dspy-optimize"`
- `pass_metrics["dspy-optimize"].duration_ms` reflects 7 subprocess calls (or 1 if cache-warm) + MIPROv2 trial cost
- `tokens_saved` likely small (hand-authored prompts here are already terse; the criticals' inputs are 68-86 tokens)

For this workflow, the recommendation is: **don't enable DSPy** until you have a runtime change that actually exercises the backend-hint path. Optimizing the prompt bytes won't help if the wall time is bound by vLLM scheduling and request concurrency.
