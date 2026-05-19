# Pre-Registration: Plan 09 J/req Cell — Plan 01 pin_demo c=32 warm cells D vs B (2026-05-19)

**Authored: 2026-05-19T21:28:01Z**
**Scope: descriptive paired J/req measurement on the Plan 01 pin_demo cohort at c=32, warm prefix cache, INT-04 cells D (APXM-on) vs B (flat-HTTP). Fourth PROMOTABLE-candidate cell of the INT-pack J/req column; **first non-tau2 cell**; **first cell whose underlying wall-time has a positive inferential claim** (INT-04 paired bootstrap-CI [0.076, 0.823] excludes 1.0).**

This cell extends the J/req column beyond τ²-bench into the Plan 01
canonical matrix regime. Three tau2 J/req cells are already
promotable
([retail N=5 demo](../../.apxm/docs/claims/plan09-jreq-methodology-demo-retail-n5.md),
[telecom N=20](../../.apxm/docs/claims/plan09-jreq-telecom-n20.md),
[airline N=20](../../.apxm/docs/claims/plan09-jreq-airline-n20.md)),
but in all four tau2 cells (including the non-promotable telecom N=5)
the J/req ratio is dominated by wall-window variance driven by 1–2
tail tasks, and mean power spans only ~7% across cells. **The
load-bearing question for the INT-pack J/req column is whether a
regime where wall-time CI already excludes 1.0 produces a J/req
signal consistent with that wall-time finding.**

The Plan 01 INT-04 cell D vs cell B paired bootstrap-CI on
`batch_wall_ms` is **[0.076, 0.823]** (claim
[`gptoss120b-concurrent-matrix.md`](../../.apxm/docs/claims/gptoss120b-concurrent-matrix.md)).
Per the meta-finding from the tau2 cells (J/req ratio ≈ wall ratio,
mean_watts ≈ constant across arms within ~7%), the J/req ratio for
this cell SHOULD land inside or close to [0.076, 0.823]. **This
pre-reg locks descriptive single-trace framing per arm — same
methodology as the tau2 J/req cells — but the result is a
falsifiable consistency check against INT-04, not an independent
inferential energy claim.** A follow-up cell with per-iteration
power-CSV slicing (10 paired iterations × 2 arms) would convert the
descriptive number into an inferential paired-bootstrap CI; that is
out of scope for this pre-reg.

## Wire / image / service (locked before run)

- Service: `vllm-gptoss` (Slurm job 57976, host b05u13)
- vLLM image: `apxm-vllm-runtime:overlay-490aaad0-cc1fbf4c-mwon-postrebase-ray`
- Model: `gpt-oss-120b` (TP=8 on 1 × MI300X node, max-num-seqs=64,
  scheduling-policy=priority, --enable-prefix-caching)
- Upstream endpoint (on b05u13): `http://127.0.0.1:8916`
- Workload: `examples/python/benchmarks/workloads/pin_demo.py` (the
  same workload INT-04 / INT-03 used; produces measurable pin
  behaviour at c=32)
- Concurrency: 32
- Iterations per arm: 5 (matches INT-04 protocol)
- Two arms run sequentially via the same `concurrent_matrix.py`
  invocation pattern as INT-04, one arm per process:
  - **apxm-on (cell D)**: `--opt-levels 2 --cell-label D-jreq`
    (graph hints + pin policy + warm prefix cache)
  - **flat-http (cell B)**: `--opt-levels 0 --no-apxm-hints
    --cell-label B-jreq` (warm prefix cache, no APXM hints)
- Cache state: warm for both arms; **no prefix-cache reset between
  arms** (matches the cells-D and B definitions; cold is a separate
  regime not measured here)
- Power capture: rocm-smi `--showpower --csv` sampled every 2 s via
  `srun --jobid=57976 --overlap` for the full per-arm wall window,
  same sidecar template as the tau2 J/req cells
  (`.apxm/evaluation/agentic/_run_template/rocm-smi-sidecar.sh`).

## What gets measured

1. **Per-arm joules consumed**: trapezoidal integration of rocm-smi
   `total_watts` over the per-arm wall window.
2. **Iterations completed per arm**: 5 iterations × 32 concurrent
   tenants = 160 paired invocations per arm.
3. **J/req per arm**: total_joules ÷ 160.
4. **Wall window per arm**: seconds between sidecar start and stop.
5. **Per-iteration wall_ms** and **pinned_blocks_peak_max** from
   the concurrent_matrix CSV (secondary, for INT-04 reproducibility).

## What gets reported

Single CSV row + JSON record per arm:

| arm | cell_label | n_invocations | total_joules | joules_per_invocation | mean_watts | sample_count | window_seconds |

Plus the **descriptive ratio** `apxm-on.J / flat-http.J` and a
consistency check against INT-04's wall-time CI [0.076, 0.823].

Secondary reproducibility table from the concurrent_matrix CSV:

| arm | cell | iterations | mean batch_wall_ms | pinned_blocks_peak_max | prefix_cache_hit_rate |

## What is NOT being claimed

- **Not claiming "APXM is X% more energy-efficient than flat-HTTP at
  c=32 warm pin_demo."** This is a single power trace per arm; no
  per-iteration joule slicing, no paired bootstrap on energy. The
  descriptive ratio is reported, the consistency vs INT-04 wall-time
  CI is reported, but no inferential energy claim is made from this
  cell alone.
- **Not re-claiming the INT-04 wall-time finding.** Per-iteration
  wall_ms is reported only as a reproducibility check that the
  service is behaving similarly to the 2026-05-18 INT-04 run on this
  workload. Material divergence from INT-04 wall-time would be
  flagged as a service-drift signal, not re-opened as a new finding.

## Honest negatives anticipated

- **Idle power dominates ~50% of the per-sample budget** at this
  service config (~1.52 kW idle vs ~3.07 kW under load — same shape
  as the three promotable tau2 cells).
- **The two arms run sequentially** with no inter-arm reset, so arm
  2 (flat-http) may benefit from warmer prefix cache state than arm
  1 (apxm-on). This is the same ordering risk that applies to the
  tau2 cells; it cannot inflate the apxm-on/flat-http ratio above
  1.0 in the apxm-on favorable direction.
- **Single-trace-per-arm on the J/req axis**: 5 iterations per arm
  but 1 power trace per arm.
- **Per-iteration variance**: like INT-04, individual iteration
  wall_ms can vary by ~10–20% within a cell. The descriptive J/req
  number averages over those iterations.
- **rocm-smi setperflevel not available** in the container; same
  caveat as INT-04 (auto-mode applies uniformly to both arms within
  the run).

## Stop conditions

- If the service becomes unhealthy mid-run: kill the matrix run,
  stop sidecar, report partial result.
- If sidecar samples < 30 per arm: same as the telecom N=5 cell —
  note as insufficient sampling, ship descriptively, do not promote.
  (At c=32 × 5 iterations the wall window will be hundreds of
  seconds, so this stop is unlikely to fire.)
- If sidecar samples ≥ 30 on both arms AND the descriptive J/req
  ratio falls inside or near the INT-04 wall-time CI [0.076, 0.823]:
  **promote into the INT-pack J/req column as the fourth
  methodology-proof cell + first non-tau2 cell + first cell where
  the ratio is consistent with a positive inferential wall-time
  claim.**
- If sidecar samples ≥ 30 on both arms BUT the descriptive J/req
  ratio falls clearly outside [0.076, 0.823]: ship descriptively,
  promote with caveat (note the divergence from INT-04 wall-time
  CI as evidence that single-trace J/req cells alone cannot anchor
  an energy claim even when wall-time is known to be inferentially
  positive).
- If per-iteration wall_ms deviates from INT-04 cell-D/B mean by
  more than ±50%: note as service-drift signal but ship.

## Path forward after this cell

If this cell promotes with ratio inside INT-04 CI:
- Plan 04 J/req cells (Mooncake/ShareGPT/LooGLE at c=32) become the
  remaining work for the full INT-pack column (3 cells).
- A follow-up Plan 01 cell with per-iteration power-CSV slicing
  could convert this descriptive cell into an inferential one
  (paired bootstrap on per-iteration joules).

If this cell promotes with ratio outside INT-04 CI:
- The meta-finding from the tau2 cells ("J/req ratio ≈ wall ratio,
  mean_watts ≈ constant") is challenged; either single-trace J/req
  is even noisier than the tau2 cells suggested, OR the c=32 regime
  exposes power-dimension variance that the tau2 cells did not.
  Either outcome reshapes the inferential-energy-claim path.
