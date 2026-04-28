# Compiler analysis — `03_vllm_backend_hints`

Workflow source: `../03_vllm_backend_hints.py`
Sweep: 2026-04-28, local Gemma vLLM fork, O0 vs O2.

Diagnostics:
- `/tmp/case01-runs/backend-hints/priority-contention/compiler-diagnostics/03_vllm_backend_hints/O0-precompile.json`
- `/tmp/case01-runs/backend-hints/priority-contention/compiler-diagnostics/03_vllm_backend_hints/O2-precompile.json`

## Graph shape that the compiler sees

16 ops:
- 2 tool registrations (`incident_dossier`, `critical_release_gate`)
- 4 background_audit Asks (`background_security`, `background_reliability`, `background_finance`, `background_rollout`) — each with `token_budget=6000` and a per-lane `_pressure_facts(lane, …)` block embedded at the top of the prompt
- 3 critical-chain ops:
  - `critical_triage` Ask (`token_budget=1200`, `benchmark_milestone="critical_triage"`)
  - `critical_plan` think (`benchmark_milestone="critical_plan"`)
  - `critical_summary` reason (`benchmark_milestone="critical_summary"`)
- 1 background-merge `g.think` over the 4 background outputs
- 1 `g.print`
- 1 `g.wait_all`
- 1 `g.done`

The user-declared intent: critical chain runs at high priority on a vLLM backend that is being pressured by 4 long-context background audits.

## Pass-by-pass effect at O2

From `O2-precompile.json::pass_metrics`:

| Pass | duration_ms | fired | ir_size_delta |
|---|---|---|---|
| normalize | 1.01 | 0 | +61 |
| build-prompt | 0.95 | 0 | 0 |
| template-specialization | 1.00 | 0 | 0 |
| dead-context-elimination | 1.01 | 0 | 0 |
| **scheduling** | 1.07 | **9** | +1531 |
| canonicalizer | 0.09 | 0 | 0 |
| symbol-dce | 0.03 | 0 | 0 |
| **shared-prefix-analysis** | 1.03 | **0** | 0 |
| **assign-priority** | 1.09 | **15** | +2391 |

`pass_summary.fired_passes = ["scheduling", "assign-priority"]`.
`pass_summary.total_ops_eliminated = 0`. `total_tokens_saved = 0`.

Note the difference from case 01: **`shared-prefix-analysis` did not fire here.**

## Why `shared-prefix-analysis` fired=0 (vs case 01's fired=6)

Case 01's 6 aspect Asks shared a literal `ASPECT_PREFIX` constant at the top of the prompt — same byte sequence across all 6 ops, only the `{aspect_name}` and `{aspect_question}` placeholders changed *after* the prefix.

Case 03's 4 background_audit Asks are built from `_background_prompt(lane, …)` which inlines `_pressure_facts(lane, …)` **at the top**. Each lane (`security`, `reliability`, `finance`, `rollout`) interpolates lane-specific facts into the head of the prompt, so the byte-level prefixes diverge from token 0. The prefix-analysis pass scans for the longest common prefix across an Ask equivalence class; here that LCP is too short to register as a group.

This is **correct compiler behavior**. The workflow author intended the 4 background prompts to be diverse pressure on the backend; they aren't actually a shared-prefix opportunity.

## What the metadata stamps look like

`scheduling` (fired=9) and `assign-priority` (fired=15) stamp the executable ops. The critical chain is the most interesting:

- `critical_triage`, `critical_plan`, `critical_summary` get high `priority` values (the workflow declares `priority="high"` on these via the AIS attribute). At O0 these attributes are present but the runtime priority scheduler doesn't yet have the supporting metadata to act on them — `tier`, `latency_class`, `stage_index` come from `scheduling`, not from the user.
- The 4 background_audit Asks get lower `priority` (workflow declares `priority="low"`).
- The merge Think and the `wait_all` get default priorities.

`scheduling` also stamps `parallel_safe`, `fanout_count`, `downstream_nodes`, `remaining_path_len`, `estimated_cost` on the same 9 ops it fires on (the 4 background + 3 critical + merge + print).

## What did *not* fire

- **`build-prompt`, `template-specialization`** — `fired=0`. Templates pre-resolved.
- **`dead-context-elimination`** — `fired=0`. Every output is consumed.
- **`canonicalizer`, `symbol-dce`** — `fired=0`. Nothing to fold or DCE.
- **`shared-prefix-analysis`** — `fired=0`. See above.
- **No DSPy pass.** Same gating; see `dspy.md`.

## Net compiler outcome for this workflow

Pure metadata stamping. No IR mutation, no token savings. The intent at O2 is to give the runtime priority scheduler enough metadata to favor the critical chain over the background audits when both compete for the vLLM backend's request slots.

Whether that priority signal **actually changes wall time** at runtime is a separate question — and the answer turns out to be "barely". See `runtime.md`.
