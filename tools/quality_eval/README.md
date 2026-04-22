# quality_eval — Tier-3 Quality-Contract Harness

Tier-3 of the APXM evaluation framework. The first gate in the stack
that hits a real LLM backend, so it answers a different question than
tiers 1–2:

- **Tier 1** (`eval_harness/`): does optimization preserve *semantics*?
  (deterministic golden artifacts, byte-exact)
- **Tier 2** (`ablation/`): does each compiler pass *actually do work*?
  (counters, ablation matrix, conflict matrix)
- **Tier 3** (`quality_eval/`, this harness): does the *output the user
  sees* still satisfy a fixture-defined quality contract?

The two-contracts model: tiers 1–2 enforce the **semantic** contract
(no answer should change); tier 3 enforces the **quality** contract
(the answer must keep meeting a stated rubric, even when models drift
or templates are tweaked).

## Quick start

```bash
# Run all fixtures, -O2, NullJudge (rubric+budget only):
dekk apxm quality-eval -- --all

# Single fixture, three samples, majority pass:
dekk apxm quality-eval -- --fixture qa_factual --samples 3 --threshold 2

# Run with the LLM judge (requires backend configured in ~/.apxm/config.toml):
dekk apxm quality-eval -- --all --judge llm

# Direct invocation (no Rust wrapper):
PYTHONPATH=tools python -m quality_eval --all
```

Exit codes: `0` = every fixture passed, `1` = at least one failed,
`2` = harness misuse (bad args, missing fixture root).

## Layout

```
tools/quality_eval/
├── __main__.py          CLI entry — argparse, prints results, sets exit code
├── runner.py            run_fixture / sample_fixture / format_result
├── rubric.py            Rubric DSL (must_contain / regex_match / min_chars / …)
├── budgets.py           max_llm_calls / max_total_tokens enforcement
├── judge.py             NullJudge (always-pass) and LLMJudge (subprocess ASK)
├── session_parse.py     Pull final_output out of an APXM session dir
├── _keys.py             JSON / TOML / CLI key constants (Rust mirror)
└── tests/               Unit tests — 51 offline tests, no backend required

tests/quality_fixtures/<name>/
├── graph.air            The graph to compile + execute
├── expected.toml        Rubric (must_contain / judge_prompt / min_chars / …)
├── budget.toml          Optional spend caps
└── golden_output.txt    Optional byte-exact contract (deterministic templates only)
```

## Authoring a fixture

```toml
# tests/quality_fixtures/qa_factual/expected.toml
must_contain = ["paris"]
case_sensitive = false
min_chars = 1
max_chars = 200

# tests/quality_fixtures/qa_factual/budget.toml
max_llm_calls = 1
max_total_tokens = 200
```

```mlir
// tests/quality_fixtures/qa_factual/graph.air
module {
  func.func @qa_factual() -> !ais.token attributes {ais.entry} {
    %ans = ais.ask "What is the capital of France? Answer with just the city name." : !ais.token
    ais.print "{ans}" [%ans : !ais.token] {input_names = ["ans"]}
    func.return %ans : !ais.token
  }
}
```

A fixture passes when **all** of:
1. Every `must_contain` substring appears (and every `must_not_contain` does not)
2. Every `regex_match` pattern matches
3. `min_chars <= len(output) <= max_chars` (zero means no bound)
4. If `judge_prompt` is set, judge score `>= judge_threshold`
5. Token usage stays inside `max_llm_calls` and `max_total_tokens`
6. If `golden_output.txt` exists, the output matches it byte-exactly

## Backends

`dekk apxm execute` has no `--backend` flag — backend selection lives in
`~/.apxm/config.toml`. The harness's `--backend` flag is metadata only
(it shows up in result lines so a CI log can attribute a failure to a
specific config). Switching backends in CI means staging the right
`config.toml` before invoking the wrapper.

## Stability sampling

Fixtures whose rubric is loose enough to flicker should be run multiple
times via `--samples N --threshold K`: the fixture passes when at least
`K` of `N` runs satisfy the rubric. Default is `--samples 1`; CI uses
3/2 (majority).

## Constants are mirrored from Rust

`results.json` / `metrics.json` JSON keys live in
`crates/core/apxm-core/src/constants.rs::session::{results_keys, metrics_keys}`.
The Python mirror in `_keys.py` is enforced by
`tests/test_keys_match_rust.py`, which regex-parses the Rust file and
fails the build on drift. When you add a key on the Rust side, mirror
it here and the drift test stays green.

The TOML keys (`must_contain`, `max_llm_calls`, …) are owned by Python
— Rust does not consume `expected.toml` or `budget.toml`. Those keys
live in `_keys.py::RubricKeys` / `BudgetKeys`.

## CI

`.github/workflows/eval-tier3.yml`:
- **harness-tests** runs the offline pytest on every PR that touches
  the harness or its mirrored Rust keys.
- **fixtures** runs every fixture against a real backend on the nightly
  cron and on PRs labelled `tier-3`. Hard-fails when
  `LLM_GATEWAY_KEY` is unset rather than silently degrading.
