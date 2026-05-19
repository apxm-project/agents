# Pre-Registration: Plan 09 J/req Cell — Plan 01 pin_demo c=32 iter=20 cells D vs B, fresh-service start (2026-05-19)

**Authored: 2026-05-19T23:01:13Z**
**Scope: descriptive paired J/req measurement on the Plan 01 pin_demo cohort at c=32, iter=20, two arms launched on a freshly-restarted `vllm-gptoss` service (cold prefix cache at cell start). Re-attempt of the [iter=5 NON-PROMOTABLE cell](../../.apxm/docs/claims/plan09-jreq-plan01-pin-demo-c32.md) that hit two stop conditions: (1) sample_count 25/23 < 30 floor; (2) `pinned_blocks_peak_max=0` for all 5 iters (pin path did not engage after 2.5 h of service warmth). This cell is designed to clear BOTH gates AND, if pin engagement reproduces INT-04's regime, to support the FIRST INFERENTIAL energy claim in the INT-pack J/req column via per-iteration power-CSV slicing.**

The [iter=5 attempt](20260519T212801Z-plan09-jreq-plan01-pin-demo-c32.md) shipped descriptively at apxm-on=1330.9 J/req, flat-http=1306.0 J/req (ratio 1.019, direction-flipped from INT-04). The flip is explained by `pinned_blocks_peak_max=0`: with the prefix cache saturated at 0.965 hit rate after intervening cells, there was no pin work for the scheduler to do — apxm-on carried pure client-side IR overhead and lost. **The flip does NOT reopen INT-04**; it characterizes a regime ([[apxm-phase1-dispatch-trend]]) where pin is dormant.

To reproduce INT-04's regime (`pinned_blocks_peak_max=4451`, paired bootstrap-CI [0.076, 0.823] excluding 1.0), this cell:
1. Runs immediately after a fresh `vllm-gptoss` service start (no intervening warm-up cells; prefix cache empty at cell start).
2. Scales iter from 5 → 20 so the wall window per arm clears the 30-sample sidecar floor (2 s sampling × ~400 s ≈ 200 samples per arm at INT-04 wall rates).
3. Captures per-iteration timestamps in the matrix CSV so the rocm-smi power trace can be sliced into per-iteration joule buckets — converting the descriptive single-trace J/req into an inferential paired bootstrap over 20 paired iterations.

## Wire / image / service (locked before run)

- Service: `vllm-gptoss` (fresh Slurm job, host TBD by scheduler at allocation time — recorded in run dir at zoo-apply)
- vLLM image: `apxm-vllm-runtime:overlay-490aaad0-cc1fbf4c-mwon-postrebase-ray`
- Model: `gpt-oss-120b` (TP=8 on 1 × MI300X node, max-num-seqs=64, scheduling-policy=priority, --enable-prefix-caching)
- Upstream endpoint: `http://127.0.0.1:8916`
- Workload: `examples/python/benchmarks/workloads/pin_demo.py` (same workload INT-04 / INT-03 used)
- Concurrency: 32
- Iterations per arm: **20** (vs iter=5 in the prior attempt; 4× wall window for sidecar floor; 20 paired iterations for bootstrap power)
- Service lifecycle: **`dekk apxm vllm zoo-apply` immediately precedes the cell**; service reaches "vLLM is ready" → no warm-up cells in between → orchestrator launches arms within ≤ 60 s of ready signal. Prefix cache is empty at cell start.
- Two arms run sequentially via the same `concurrent_matrix.py` invocation pattern:
  - **apxm-on (cell D)**: `--opt-levels 2 --cell-label D-jreq-iter20-fresh` (graph hints + pin policy; cold prefix cache at start)
  - **flat-http (cell B)**: `--opt-levels 0 --no-apxm-hints --cell-label B-jreq-iter20-fresh` (cold prefix cache at start of arm 2 = warm from arm 1's identical-prefix workload)
- **Note on cache ordering**: arm 1 (apxm-on) starts cold; arm 2 (flat-http) starts with whatever cache state arm 1 left. This matches INT-04's cells D and B definitions and is the same ordering risk noted in the iter=5 pre-reg. It cannot inflate `apxm-on/flat-http` above 1.0 in the apxm-on favorable direction.
- Power capture: rocm-smi `--showpower --csv` sampled every 2 s via `srun --jobid=<new-jid> --overlap` for the full per-arm wall window. Same sidecar template as the iter=5 attempt.
- APXM config env: `APXM_CONFIG=$HOME/.apxm/config.toml` exported in the orchestrator subprocess environment to bypass [[apxm-config-resolver-does-not-merge]].

## What gets measured

1. **Per-arm joules consumed** (descriptive): trapezoidal integration of rocm-smi `total_watts` over the per-arm wall window.
2. **Per-iteration joules** (inferential): the orchestrator records per-iteration start/end timestamps in `matrix.csv`; the joule integrator slices the power trace into 20 per-iteration buckets per arm.
3. **Iterations completed per arm**: 20 iterations × 32 concurrent tenants = 640 paired invocations per arm.
4. **J/req per arm** (descriptive): total_joules ÷ 640.
5. **J/req per arm** (inferential): paired bootstrap over the 20 per-iteration joule buckets; 95 % CI on `apxm-on / flat-http` ratio.
6. **Wall window per arm**: seconds between sidecar start and stop.
7. **Per-iteration `pinned_blocks_peak_max`** from the matrix CSV.

## What gets reported

Single CSV row + JSON record per arm:

| arm | cell_label | n_invocations | total_joules | joules_per_invocation | mean_watts | sample_count | window_seconds |

**Plus**: per-iteration joule table (40 rows = 20 iters × 2 arms) and paired-bootstrap CI on `apxm-on / flat-http` per-iteration J/req ratio.

Plus the **descriptive ratio** `apxm-on.J / flat-http.J`.

Plus pin engagement table:

| arm | iteration | batch_wall_ms | pinned_blocks_peak_max | prefix_cache_hit_rate |

## What is NOT being claimed

- **Not re-claiming the INT-04 wall-time finding.** Per-iteration wall_ms is reported as a reproducibility check; material divergence from INT-04 wall is a service-drift signal, not a new finding.
- **Not claiming generalization beyond pin_demo.** This is one workload at one concurrency; Plan 04 J/req cells (Mooncake/ShareGPT/LooGLE) address the cross-workload generalization separately.
- **Not claiming idle-power-corrected energy savings.** Reported J/req is wall-clock × power including idle baseline.

## Honest negatives anticipated

- **Idle power still dominates ~50 %** of the per-sample budget (~1.52 kW idle vs ~3.07 kW under load — consistent with iter=5 attempt's flat-http mean).
- **Arm ordering**: apxm-on runs first into cold cache; flat-http runs second into a partially warm cache. The pin-engagement requirement makes this ordering necessary (testing the cold→pin engagement path). The ordering bias works AGAINST apxm-on if pin engagement helps it.
- **Per-iteration joule bucketing precision** is bounded by the 2 s sidecar interval. Iterations completing in < 10 s will have noisy joule attribution; INT-04 had ~10 s per-iter wall, so a 2 s interval gives ~5 samples per bucket — usable but not tight. If service runs hot and iterations drop to < 4 s, per-iteration bucketing degrades sharply and the bootstrap CI widens.
- **`rocm-smi setperflevel` not available** in the container; auto-mode applies uniformly to both arms.

## Stop conditions

A. **Service unhealthy mid-run**: kill matrix, stop sidecar, ship partial as non-promotable.

B. **Sidecar samples < 30 on either arm**: ship descriptively, non-promotable. (At iter=20 × INT-04 wall rates this stop is unlikely to fire; if it does, the service is running >2× faster than INT-04 and the cell's premise is invalid.)

C. **`pinned_blocks_peak_max < 1000` on apxm-on (cell D)**: pin path did not engage. Ship descriptively as a SECOND saturated-regime data point, non-promotable. **Add to the apxm_phase1_dispatch_trend evidence ledger** as confirmation that pin engagement requires specific service-state conditions (not just fresh-service start). Recommend abandoning the J/req inferential path on this workload until the pin-engagement state conditions are independently characterized.

D. **`pinned_blocks_peak_max ≥ 1000` on apxm-on AND sample_count ≥ 30 per arm AND per-iteration wall_ms within ±50 % of INT-04 cell-D mean**: **PROMOTE as the FIRST inferential energy cell in the INT-pack J/req column**. Report:
   - Descriptive ratio (consistency check vs INT-04 wall CI [0.076, 0.823])
   - Inferential paired-bootstrap 95 % CI on per-iteration J/req ratio (the load-bearing inferential claim)
   - Pin engagement reproducibility table

E. **`pinned_blocks_peak_max ≥ 1000` on apxm-on AND sample_count ≥ 30 per arm BUT per-iteration wall_ms differs from INT-04 by > ±50 %**: ship as inferential, note as service-drift caveat in the claim header.

## Path forward after this cell

**If this cell promotes inferentially (stop D)**: the INT-pack J/req column has its first inferential anchor. Plan 04 J/req cells (Mooncake/ShareGPT/LooGLE at c=32) become extension work for cross-workload coverage; methodology is proven. Plan 09 closes.

**If this cell ships non-promotable via stop C (saturated regime again)**: the pin-engagement reproducibility problem becomes the load-bearing finding — it implies pin engagement depends on initial-state conditions beyond "fresh service + cold cache", which is itself an important methodological result. Recommend a Plan 01 follow-up pre-reg that varies service-startup parameters (e.g., warmup workload type, intervening cell sequence) to characterize the pin-engagement state envelope. **Do NOT attempt a third J/req cell on this workload without first resolving the pin-engagement reproducibility question.**

**If this cell ships non-promotable via stop A/B**: re-attempt at a later service window; methodology unchanged.
