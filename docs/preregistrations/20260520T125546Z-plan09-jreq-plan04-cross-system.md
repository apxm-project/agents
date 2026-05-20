# Pre-Registration: Plan 09 J/req Cells — Plan 04 cross-system (Mooncake / ShareGPT / LooGLE), c=32 iter=6 per-cell engine-restart (2026-05-20)

**Authored: 2026-05-20T12:55:46Z**
**Scope: descriptive paired J/req measurement on all three Plan 04 workloads (Mooncake / ShareGPT / LooGLE) at c=32, iter=6, per-cell engine-restart matching the wall-time matrix protocol from INT-12 pre-reg `20260519T030358Z-plan04-cross-system.md`. Six cells total: {mooncake, sharegpt, loogle} × {apxm-on, flat-http}. Adds rocm-smi power sidecar per cell with the same per-iteration paired-bootstrap inferential vehicle introduced by the [iter=20 Plan 01 re-attempt pre-reg](20260519T230113Z-plan09-jreq-plan01-pin-demo-c32-iter20-restart.md) (sha `5d10eeaf`). Closes the J/req column for the INT pack (3 promotable τ²-bench cells + the upcoming Plan 01 iter=20 inferential cell + these 3 Plan 04 cells = 7 total).**

The Plan 04 wall-time matrix shipped 2026-05-19 (INT-12, [`plan04-cross-system.md`](../../.apxm/docs/claims/plan04-cross-system.md)) as a scoped honest-null: paired bootstrap-CI on `batch_wall_ms` was Mooncake 1.073 [0.968, 1.211] null, ShareGPT **1.223 [1.081, 1.350] flat-HTTP wins**, LooGLE 1.001 [0.967, 1.038] null. The J/req column needs the same 3-workload coverage to support an honest energy synthesis at the INT-pack level. The descriptive J/req number per cell will be reported; the inferential J/req paired bootstrap will be reported per workload (apxm-on iter 1..6 paired with flat-http iter 1..6 within each workload) using `.apxm/evaluation/agentic/_run_template/per-iter-energy.py`.

## Wire / image / service (locked before run)

- Service: `vllm-gptoss` (fresh Slurm job per cell — same per-cell engine-restart pattern as `tools/scripts/plan04_run_per_cell.sh`)
- vLLM image: `apxm-vllm-runtime:overlay-490aaad0-cc1fbf4c-mwon-postrebase-ray`
- Model: `gpt-oss-120b` (TP=8 on 1 × MI300X node, max-num-seqs=64, scheduling-policy=priority, --enable-prefix-caching)
- Upstream endpoint: `http://127.0.0.1:8916`
- Workloads (3, run as 6 cells with engine restart between cells):
  - **Mooncake**: `examples/python/benchmarks/mooncake_replay.py` (the same 100-row replay used by INT-12)
  - **ShareGPT**: `examples/python/benchmarks/workloads/sharegpt_row.py` driven through `concurrent_matrix.py`
  - **LooGLE**: `examples/python/benchmarks/workloads/loogle_row.py` driven through `concurrent_matrix.py`
- Concurrency: 32
- Iterations per cell: **6** (matches INT-12; n_pair=6 is the bootstrap-power floor for the inferential CI)
- Rows per cell (where applicable): 100 (matches INT-12)
- Per-cell sequence (matching `tools/scripts/plan04_run_per_cell.sh`): scancel prior service → archive state record → `dekk apxm vllm zoo-apply` → wait for "vLLM is ready" on slurm log → start rocm-smi sidecar → run cell → stop sidecar → integrate joules → move to next cell.
- Per-arm execution: apxm-on cell uses `--opt-levels 2`, flat-http cell uses `--opt-levels 0 --no-apxm-hints`. Same cell-label conventions as INT-12 (M-ON / M-OFF / S-ON / S-OFF / L-ON / L-OFF), suffixed with `-jreq` to distinguish from the wall-time matrix cells (e.g., `S-ON-jreq`).
- Power capture: rocm-smi `--showpower --csv` sampled every 2 s via `srun --jobid=<per-cell-jid> --overlap` for the full per-cell wall window. Same sidecar template as the iter=20 Plan 01 cell.
- APXM config env: `APXM_CONFIG=$HOME/.apxm/config.toml` exported in orchestrator subprocess env to bypass [[apxm-config-resolver-does-not-merge]].

## What gets measured

Per cell:
1. **Cell joules consumed** (descriptive): trapezoidal integration of rocm-smi `total_watts` over the cell wall window.
2. **Per-iteration joules** (inferential): power trace sliced into 6 per-iteration buckets via `per-iter-energy.py`.
3. **Iterations completed**: 6 × 32 = 192 paired invocations per cell (where workload is concurrent_matrix-driven) OR 6 × 100 = 600 paired invocations per cell (where workload is mooncake_replay-driven).
4. **J/req per cell** (descriptive): total_joules ÷ invocations.

Per workload (apxm-on cell paired with flat-http cell):
5. **J/req inferential ratio**: paired bootstrap (n_boot=10000, seed=42) on per-iteration `apxm-on.J / flat-http.J` over the 6 iterations.

## What gets reported

Three tables.

**Descriptive J/req per cell** (6 rows):

| workload | arm | cell_label | n_invocations | window_seconds | sample_count | mean_watts | total_joules | J/req |

**Inferential per-workload paired-bootstrap** (3 rows):

| workload | n_iter | ratio (apxm-on / flat-http) point | ratio 95 % CI | diff J/req point | diff 95 % CI | excludes 1.0 |

**Pin engagement / cache state per cell** (6 rows):

| workload | arm | iter_mean batch_wall_ms | pinned_blocks_peak_max_max | prefix_cache_hit_rate (final) |

## What is NOT being claimed

- **Not claiming "APXM is X% more energy-efficient than flat-HTTP cross-system."** With iter=6 the per-workload bootstrap CI is wide; the honest interpretation will track whatever the CIs say. A null on all three workloads is the expected outcome by analogy with INT-12 wall-time (2 of 3 null, 1 ShareGPT flat-HTTP wins).
- **Not re-claiming the INT-12 wall-time finding.** Per-iteration wall_ms is reported for service-drift reproducibility only.
- **Not claiming generalization to other models or other concurrencies.**
- **Not claiming idle-power-corrected energy savings.** Reported J/req is wall-clock × power including the ~1.5 kW idle baseline.

## Honest negatives anticipated

- **Iter=6 is bootstrap-power-marginal**: with n_pair=6 the paired-bootstrap CI on J/req ratio will be wider than the INT-12 wall-time matrix CIs (which used row-level pairing within iterations). A null CI here does NOT imply equivalence; it implies "this cell did not have power to detect a difference."
- **Per-cell engine restart introduces cold-cache start state per cell**, but the apxm-on and flat-http cells for each workload are independent engine instances. Cold-cache effects are present in both arms; the per-iteration bootstrap absorbs them symmetrically only if iter 1 is excluded from the analysis. **Decision: include iter 1 in the bootstrap** (matches INT-12 protocol; iter=6 is too short to discard 1/6 of data without further-collapsing power).
- **ShareGPT is the cell where INT-12 found a wall-time loss**; J/req here should also tilt against apxm-on if the wall-time loss is the dominant driver. A null J/req CI on ShareGPT despite a positive wall-time CI would be a power-dimension finding worth reporting.
- **rocm-smi `setperflevel` not available** in the container; auto-mode applies uniformly within the run.
- **Sidecar per-iter sample density**: at 2 s interval and typical iter walls of 20–100 s per cell, expect 10–50 samples per iter bucket — adequate for per-iter joule attribution.
- **Cell ordering** (which workload runs first): pre-reg locks order as `mooncake → sharegpt → loogle`, alternating arms within each workload (apxm-on first, then flat-http). Service is restarted before EACH cell so prior-cell warm-state cannot bias subsequent cells.

## Stop conditions

A. **Service unhealthy mid-cell**: kill the cell, mark partial, continue to next cell with a fresh engine. Partial cell ships descriptively only (no inferential bootstrap on that workload).

B. **Sidecar samples < 30 on any cell**: ship that cell descriptively, omit from inferential bootstrap, note as sampling-floor miss.

C. **Mooncake / ShareGPT / LooGLE wall-time deviates from INT-12 by > ±30 %** on either arm: note as service-drift caveat in claim header but ship cell.

D. **All 6 cells complete with sample_count ≥ 30 each AND per-iter bootstrap returns a finite CI on all 3 workloads**: **PROMOTE as the cross-system J/req closure of the INT-pack column**. Report descriptive + inferential tables; mark each per-workload CI's relation to 1.0.

E. **Per-cell pin engagement (`pinned_blocks_peak_max`) = 0 across all 6 cells**: ship; note as confirmation that the saturated-cache regime extends across Plan 04 workloads in addition to Plan 01 (matches [[apxm-phase1-dispatch-trend]]). Does NOT block promotion.

## Path forward after this cell

**If stop D fires with all 3 nulls**: J/req column closes as cross-workload honest-null (parallel to INT-12 wall-time honest-null). Combined with the iter=20 Plan 01 inferential cell (if that promotes via its own stop D), the INT pack has: 3 promotable τ²-bench cells (positive descriptive coverage), 1 inferential Plan 01 anchor (positive inferential at constrained-KV), and 3 cross-system Plan 04 cells (honest null at iter=6 power). That's the closure of Plan 09 path item #4.

**If stop D fires with ShareGPT energy CI excluding 1.0 in the flat-HTTP-wins direction**: this is the load-bearing second finding (after INT-12 wall-time) for Plan 08 motivation — the regression appears in both wall AND energy dimensions, strengthening the case for the bidirectional protocol design.

**If stop D fires with any cell's J/req CI excluding 1.0 in the apxm-on-wins direction**: write a follow-up pre-reg to scale that workload's iter to ≥20 for tighter CI and re-run.

**If any cell stops via A/B/C**: re-attempt later via per-cell re-run script; methodology unchanged.
