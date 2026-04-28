# Compiler analysis — `01_review_synthesis_skill`

Workflow source: `../01_review_synthesis_skill.py`
Sweep: 2026-04-28, local Gemma vLLM fork, O0 vs O2.

Diagnostics:
- `/tmp/case01-runs/review-synthesis/compiler-diagnostics/01_review_synthesis_skill/O0-precompile.json`
- `/tmp/case01-runs/review-synthesis/compiler-diagnostics/01_review_synthesis_skill/O2-precompile.json`

## Graph shape that the compiler sees

16 ops. The workflow declares:
- 1 tool registration (`review_corpus_tool`)
- 6 aspect Asks that all share the literal `ASPECT_PREFIX` (correctness / readability / performance / testability / security / docs)
- 2 ACP spawns (`claude_architect`, `codex_reviewer`)
- 1 `g.think(synthesis)` over the 6 aspect outputs + 2 ACP outputs
- 1 `g.reason(validation_checklist)` chained off the synthesis
- 1 `print_metrics_report` + `done`

The 6 aspect ops use the same prompt skeleton; only the `{aspect_name}` and `{aspect_question}` interpolations differ. This is what gives `shared-prefix-analysis` something to do at O2.

## Pass-by-pass effect at O2

From `O2-precompile.json::pass_metrics`:

| Pass | duration_ms | fired | ir_size_delta | ops Δ | tokens_saved |
|---|---|---|---|---|---|
| normalize | 0.20 | 0 | +61 | 0 | – |
| build-prompt | 0.17 | 0 | 0 | 0 | – |
| template-specialization | 0.18 | 0 | 0 | 0 | – |
| dead-context-elimination | 0.17 | 0 | 0 | 0 | – |
| canonicalizer | 0.08 | 0 | 0 | 0 | – |
| symbol-dce | 0.03 | 0 | 0 | 0 | – |
| **scheduling** | 0.24 | **9** | +1645 | 0 | – |
| **shared-prefix-analysis** | 0.24 | **6** | +602 | 0 | – |
| **assign-priority** | 0.34 | **15** | +2465 | 0 | – |

`pass_summary.fired_passes = ["scheduling", "shared-prefix-analysis", "assign-priority"]`.
`total_ops_eliminated = 0`. `total_tokens_saved = 0`.

What this means concretely:

- **No IR mutation.** All 16 ops survive O2 byte-for-byte; the compiler only **stamps metadata** on existing nodes.
- **`scheduling` fired 9 times.** Stamps `tier`, `latency_class`, `stage_index`, `downstream_nodes`, `remaining_path_len`, `parallel_safe`, `fanout_count`, `estimated_cost` on the 9 ops that participate in cross-stage parallelism (the 6 aspects + 2 ACP spawns + the synthesis fan-in).
- **`shared-prefix-analysis` fired 6 times — once per aspect Ask.** Stamps each of the 6 aspect ops with:
  - `shared_prefix_group = "shared_prefix_analysis_0"` (single equivalence class — all 6 share the same `ASPECT_PREFIX`)
  - `shared_prefix_group_size = 6`
  - `shared_prefix_est_tokens` between **84 and 90** (per aspect, depending on how the suffix tokenizes)
- **`assign-priority` fired 15 times.** Stamps every executable op (everything except the tool-registration shim) with a numeric `priority`, used by the runtime priority scheduler.

## What changed in `input.air` (O0 → O2)

The 6 aspect Asks pick up shared-prefix metadata at O2. Spot-check from `examples/python/demos/gemma4/workflows/01_review_synthesis_skill.py` compiled at `-O 2`:

```
ais.ask @aspect_correctness ... {
  shared_prefix_group         = "shared_prefix_analysis_0"
  shared_prefix_group_size    = 6
  shared_prefix_est_tokens    = 89
  priority                    = …
  tier, latency_class, …
}
```

At O0 the same op carries only the user-declared attrs (`name`, `prompt`, `temperature`, `token_budget=600`).

## Where the runtime can use this metadata

- `shared_prefix_*` is what enables the vLLM backend to issue the 6 aspect requests with overlapping `extra_body.apxm_hints.prefix_group`, so vLLM's prefix cache can be reused across the 6 prefills. Whether vLLM actually reports cache hits depends on the backend (see `runtime.md` for the observed `cached_input_tokens` numbers).
- `priority` and `tier` flow into the priority scheduler at execution time. With only 16 nodes and 8 of them being independent fan-out work, the scheduler can run several aspects + both ACP spawns concurrently without contention.

## What did *not* fire (and why)

- **`build-prompt`, `template-specialization`** — `fired=0`. The Asks already have fully-resolved string templates at AIR-emit time; nothing to specialize.
- **`dead-context-elimination`** — `fired=0`. Every aspect output is consumed by the synthesis Think; nothing is dead.
- **`canonicalizer`, `symbol-dce`** — `fired=0`. Nothing to canonicalize or DCE.
- **No DSPy pass** in the pipeline at all. Inspect `pass_metrics` — there is no `dspy-optimize` entry. See `dspy.md` for why.

## Net compiler outcome for this workflow

O2 is **pure metadata enrichment**. The IR has the same 16 ops, the same 10 LLM call sites. What the runtime gets extra is: priority assignment on 15 ops, a 6-member shared-prefix equivalence class on the aspects, and full scheduling metadata. The wall-clock improvement reported in `runtime.md` (~14%) comes from the runtime *acting* on this metadata — not from the compiler removing any work.
