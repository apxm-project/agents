# DSPy analysis — `02_checkout_context_pruning`

## Did DSPy fire on this workflow? **No.**

`O2-precompile.json::pass_summary.fired_passes = ["dead-context-elimination", "scheduling", "assign-priority"]` — no `dspy-optimize` entry. `total_tokens_saved = 0`. The two surviving prompts (`Checkout_Context` Ask, `Decision` Think) ran with their hand-authored templates verbatim.

## Why it didn't fire

Same gating chain as case 01: `prompt_optimization_configured()` returned `false` because `~/.apxm/config.toml` does not declare `[compiler.dspy].training_data` (or the legacy `[prompt_tuning].training_data`). Without that field set, `apply_transient_module_config()` skips the DSPy attribute stamp and `passes/pipeline.rs` skips injecting the `dspy-optimize` pass.

See `01_review_synthesis_skill/dspy.md` for the full gating-chain trace.

## What DSPy *would* do for this workflow — and the interesting interaction with DCE

This workflow is a less obvious DSPy candidate than case 01:

- Only 2 surviving LLM calls after DCE.
- The two prompts (`Checkout_Context`, `Decision`) have very different shapes — they wouldn't share an optimized instruction.

A trainset would need rows for *both* nodes:

```jsonl
{"node": "Checkout_Context", "inputs": {}, "expected": "..."}
{"node": "Decision", "inputs": {"checkout_context": "..."}, "expected": "ship | hold"}
```

Note the **interaction with `dead-context-elimination`**. DSPy runs at **stage 4** of the pass pipeline, *after* normalize/build-prompt but *before* `dead-context-elimination`. So if DSPy were enabled here, the optimizer would see the **original 6-LLM-call graph** with the 4 dead Ask outputs still threaded into Decision. Two consequences:

1. DSPy would attempt to optimize all 5 context Asks — wasted work, since 4 of them are about to be DCE'd anyway.
2. The Decision Think's optimized instruction might reference `{api_context}`, `{observability_context}`, etc. (since DSPy's signature builder reads the placeholders from the *current* template). After DCE strips those operands, the optimized instruction would still mention them — producing an incoherent prompt.

This ordering bug would manifest as DSPy "saving" tokens on Asks that don't survive O2. The fix is either:
- Move `dspy-optimize` to run **after** `dead-context-elimination`, or
- Have `dead-context-elimination` re-run DSPy on any nodes whose template it modifies.

Currently neither is done; the bug is latent because DSPy doesn't fire on this workflow.

## Which metric would apply

- `Checkout_Context`: free-form context dump. `token_overlap` is the right F1-style metric.
- `Decision`: the workflow expects a strict `ship | hold` token. `exact_match` would be the right metric.

The current `[compiler.dspy].metric` field is module-scoped, so a per-op metric override would require a new `ais.dspy_metric_override` op-level attribute (not currently supported).

## Reading the next sweep

If you enable DSPy on this workflow, watch for:
- `pass_summary.fired_passes` gaining `"dspy-optimize"` ahead of `"dead-context-elimination"`.
- `pass_metrics["dspy-optimize"].duration_ms` non-zero (subprocess fork + MIPROv2 trial).
- `pass_metrics["dspy-optimize"].tokens_saved` reflecting *only* what survives the subsequent DCE pass — currently the diagnostics aggregator double-counts here.

For this workflow specifically, the right answer is probably "don't bother enabling DSPy until the ordering bug is fixed."
