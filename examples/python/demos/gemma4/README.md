# Gemma 4 Demo

Compiled APXM workflows for the strategic Gemma 4 demo. Each case is a
self-contained Python graph that compiles to an `.apxmobj` artifact and runs on
a registered vLLM backend.

## Layout

| Path | Purpose |
| --- | --- |
| `workflows/0N_*.py` | One file per case. Each file is the graph definition **and** the entry point. |
| `shared/` | Cross-case helpers (route aliases, dossier text). |
| `scripts/` | Operator scripts: `run_demo.py` writes the local config, `measure_vllm_hints.py` probes the backend, `render_slides.py` builds the deck. |
| `decks/` | Strategic deck source (`apxm-strategic.md`) plus the python-pptx generator and vendor template. |
| `docs/` | Claim boundaries, evidence rules. Read these before quoting numbers. |
| `research/`, `runbooks/`, `dspy/` | Background material — not on the critical path. |
| `runs/` | (gitignored) Generated evidence under `runs/<UTC-timestamp>-three-cases/`. |

## Three Cases

| Case | Workflow | Claim boundary |
| --- | --- | --- |
| ReviewSynthesis Skill | `workflows/01_review_synthesis_skill.py` | Reusable skill that fans Claude + Codex out and synthesizes through Gemma on vLLM. **Do not** claim O2 speedup or token reduction unless `runtime-o0-o2.csv` shows it. |
| Checkout Context Pruning | `workflows/02_checkout_context_pruning.py` | Compiler removes unreferenced context branches at O2. This is the source of the `6 → 2` LLM-call story when fresh runs reproduce it. |
| vLLM Backend Hints | `workflows/03_vllm_backend_hints.py` | Runtime priority hints survive the lowering and reach vLLM. Direct probe lives in `scripts/measure_vllm_hints.py`. |

See [`docs/claim-boundaries.md`](docs/claim-boundaries.md) for the full rules.

## Compile Once, Run Many

Every case is meant to be measured with `dekk apxm compile` (build one
`.apxmobj` per opt level) and `dekk apxm run` (execute the artifact repeatedly).
`dekk apxm execute` is fine for smoke runs but conflates compile + run timing.

```sh
DEMO=examples/python/demos/gemma4
RUN_DIR="$DEMO/runs/$(date -u +%Y%m%dT%H%M%SZ)-three-cases"

python3 "$DEMO/scripts/run_demo.py" \
    --run-dir "$RUN_DIR" \
    --backend vllm-fork \
    --model google/gemma-4-31B-it
CFG="$RUN_DIR/generated/config.toml"
```

Then run any case as a normal APXM graph:

```sh
python3 examples/python/benchmarks/benchmark_e2e.py \
    --graph "$DEMO/workflows/02_checkout_context_pruning.py" \
    --iterations 3 \
    --precompile-artifacts \
    --emit-compiler-diagnostics \
    --artifact-dir       "$RUN_DIR/context-pruning/artifacts" \
    --diagnostics-dir    "$RUN_DIR/context-pruning/compiler-diagnostics" \
    --output             "$RUN_DIR/context-pruning/runtime-o0-o2.csv" \
    --session-base       "$RUN_DIR/context-pruning/sessions" \
    --backend-label strategic-demo-vllm \
    --target tokens \
    --apxm-config "$CFG" \
    --interleave-opt-levels
```

## Building the Deck

```sh
python3 examples/python/demos/gemma4/scripts/render_slides.py
```

The renderer reads the **most recent** `runs/<UTC>-three-cases/` directory.
Stale paths under `.apxm/demos/...` are no longer supported and the linter
(`scripts/lint_deck.py`) will flag them.

## Evidence Rule

> A claim shown in the deck must be backed by a row in the latest
> `runs/<UTC>-three-cases/*/runtime-o0-o2.csv` or
> `direct-vllm-hints.json`. Anything else is hypothesis.

Run `python3 scripts/lint_deck.py` to verify before presenting.
