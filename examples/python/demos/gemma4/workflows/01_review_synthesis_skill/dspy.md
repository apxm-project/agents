# DSPy analysis — `01_review_synthesis_skill`

## Did DSPy fire on this workflow? **No.**

Evidence — `O2-precompile.json` for this workflow contains:
- `pass_summary.fired_passes = ["scheduling", "shared-prefix-analysis", "assign-priority"]` — no `dspy-optimize` entry.
- `pass_metrics` lists 9 entries, none named `dspy-optimize`.
- `tokens_saved` is `null` on every pass; `total_tokens_saved = 0`.

So no aspect prompt, no synthesis prompt, no validation prompt was rewritten. The 6 aspect Asks ran with their hand-authored templates verbatim.

## Why it didn't fire — the gating chain

`crates/compiler/apxm-compiler/src/passes/pipeline.rs` injects the `dspy-optimize` pass only when **all** of:

1. `config.pass_list_override` is `None` (true here — the harness doesn't override the pipeline).
2. `config.opt_level != O0` (true at O2).
3. The pass isn't already in the explicit list (true).
4. `optimization_context.prompt_optimization_configured()` returns `Ok(true)`.

Step 4 is what failed. `prompt_optimization_configured()` (in `crates/compiler/apxm-compiler/src/optimization.rs:134-148`) checks the loaded `apxm.toml` config for either:

- `[compiler.dspy].training_data` / `[compiler.dspy].dataset`, or
- `[prompt_tuning].training_data` / `[prompt_tuning].dataset`.

Neither key is set in the user's `~/.apxm/config.toml` for this run, so the pass is silently skipped. No DSPy attrs (`ais.dspy_training_data_path`, `ais.dspy_optimizer`, `ais.dspy_metric`, …) are stamped on the module either, because `apply_transient_module_config()` in `pipeline.rs` is gated by the same predicate.

## What DSPy *would* do for this workflow

The 6 aspect Asks are textbook candidates: same skeleton, six independent (aspect, prompt) variants, structured output (a numbered list of issues). A small trainset would let `MIPROv2` propose better instruction wording for each aspect.

A minimal `dspy-trainset.jsonl` for the aspect prompts would contain rows like:

```jsonl
{"node": "aspect_correctness", "inputs": {"aspect_name": "correctness", "aspect_question": "Are there logical errors?"}, "expected": "1. ..."}
{"node": "aspect_security", "inputs": {"aspect_name": "security", "aspect_question": "Are there input-validation gaps?"}, "expected": "1. ..."}
```

Then in `~/.apxm/config.toml`:

```toml
[compiler.dspy]
training_data = "/path/to/dspy-trainset.jsonl"
optimizer     = "miprov2"   # or "bootstrap" / "copro"
metric        = "token_overlap"   # F1 over tokens; alt: exact_match, contains, llm_judge
auto          = "light"
```

With this config in place at O2, the pass would:

1. Stamp the module with `ais.dspy_training_data_path`, `ais.dspy_optimizer = "miprov2"`, `ais.dspy_metric = "token_overlap"`, `ais.dspy_auto = "light"`, `ais.dspy_cache_dir`, `ais.dspy_backend_json` (resolved per-node backend), `ais.dspy_no_cache = false`.
2. Fork `python -m apxm_dspy` per Ask, hand it the template + trainset + backend spec via stdin JSON.
3. Inside the subprocess (`tools/apxm_dspy/optimizer.py:optimize_single_template`): build a `dspy.Signature` from the placeholders (`{aspect_name}`, `{aspect_question}`), instantiate `MIPROv2(metric=metric_fn, auto="light")`, call `.compile(predictor, trainset=examples, max_bootstrapped_demos=0, max_labeled_demos=0)`, then prepend the optimized instruction string to the original template:
   ```python
   optimized_template = optimized_instruction + "\n\n" + template_str
   ```
   This preserves the user's `{placeholders}` so downstream interpolation still works.
4. Write the result back into the AIR module as `ais.dspy_optimized = "<rewritten template>"` on each Ask op.
5. Cache the result under `~/.cache/apxm/dspy/<blake2b key>` keyed on `(template, trainset_hash, optimizer, model, metric, auto, backend_fingerprint, dspy_version)` — see `tools/apxm_dspy/cache.py`. A second compile with the same inputs is a fast cache hit.

## Which metric would apply

The default in this workflow's domain (free-form aspect critique with reference outputs) is `token_overlap` — F1 over the bag of tokens between the optimized prediction and the trainset expected answer (`tools/apxm_dspy/metrics.py`). For the synthesis Think (which produces a structured report) `contains_match` against expected section headings would be more meaningful. For the validation `reason` (which yields a checklist) `exact_match` on the bullet count would be a strict signal.

The user picks one metric per compile via `[compiler.dspy].metric`; per-op metric overrides would require a future per-op attr (`ais.dspy_metric` is currently module-level only).

## Reading the next sweep

After enabling DSPy you'll see the change in three places:
- `O2-precompile.json::pass_metrics` will gain a `dspy-optimize` entry with non-zero `duration_ms` and `tokens_saved`.
- `pass_summary.fired_passes` will include `"dspy-optimize"`.
- The compiled `.apxmobj` (decompile with `dekk apxm decompile`) will show each Ask op carrying an `ais.dspy_optimized` attribute holding the rewritten prompt.

None of those signals are present in the 2026-04-28 sweep, which is why this file's bottom line is: DSPy did not fire.
