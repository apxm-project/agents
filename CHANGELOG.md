# Changelog

All notable changes to APXM are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and APXM intends to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) once a `1.0`
contract is committed to.

## [Unreleased]

## [0.1.0] - 2026-06-10

### Added
- `dekk apxm release check`, `release dist`, and `release publish` now provide
  a first-class release path: readiness checks, `.apxm/releases` archives,
  checksums, and explicit GitHub publishing through `gh`.

### Removed
- Removed the last `service-start` compatibility stub from the Dekk vLLM
  surface. The model zoo manifest remains the only service launch path.

### Fixed — fork-side v0.21.0 rebase regression
- `external/vllm` bumped to `cc1fbf4c0` on the `apxm` branch:
  the APXM patch in `vllm/v1/core/sched/scheduler.py` called the
  pre-rename `request.num_cached_tokens` attribute in its debug log,
  crashing the engine on the first APXM-tagged request. Renamed to
  `request.num_computed_tokens` per upstream v0.21.0.

### Added — Plan 01 EVAL canonical 2×2 matrix
- First complete Phase-G 2×2 matrix on `openai/gpt-oss-120b` (TP=8
  on 1 × MI300X, vLLM-fork `cc1fbf4c0`,
  `gpu-memory-utilization=0.9`, `max-num-seqs=64`,
  `--scheduling-policy=priority`, `--enable-prefix-caching`
  per-cell). 4 cells × 10 iter × concurrency 32 on the `pin_demo`
  workload; **0 failed tenants across 1280 invocations**. Headline:
  Cell D (APXM-on + prefix-cache, warm) vs Cell B (flat-HTTP +
  prefix-cache, warm) paired bootstrap-CI **[0.076, 0.823]** cleanly
  excludes 1.0 — APXM-on delivers a 17.7%–92.4% reduction in
  `batch_wall_ms`. Pin engagement verified
  (`pinned_blocks_peak_max=4451`). Honest negative reported:
  cold-cache cell C is 18% slower than cell A on central tendency.
- Default workload for `examples/python/benchmarks/concurrent_matrix.py`
  switched to `workloads/pin_demo.py` — the previous default
  `stress/prefix_fanout_concurrent.py` is documented as
  engine-crashing in
  `examples/python/benchmarks/KNOWN-ISSUES.md`.

### Added — dispatch-IR honesty closure
- **Runtime-time capability gating** — `evaluate_required_capabilities`
  in `apxm-runtime::dispatch::v1` gates graph registration on the backend's
  static capability table. A missed `required` field skips
  `register_graph` for that backend, falls back to flat-HTTP dispatch
  (no `vllm_xargs.apxm` injection), and records the decision in
  `dispatch_ir_metrics.fallbacks[*]` with `fallback_triggered=true`. No
  silent proceed.
- **APXM-side ingestion for `x-apxm-fields-honored`** — the
  OpenAI backend's `generate` path captures the comma-separated header
  into `LLMResponse.metadata["fields_honored"]`. Contract constants
  `APXM_FIELDS_HONORED_HEADER` and `FIELDS_HONORED_RECORD_KEY` ship in
  `apxm_core::constants::llm::apxm`. Fork-side emitter is the
  remaining gate.
- **`SchedulerInfoResponse.dispatch_ir_version`** — forward-
  compatible wire field the fork is expected to advertise; APXM gates
  `supports_dispatch_ir_v1_internal` on the tag matching
  `DISPATCH_IR_V1_VERSION_TAG = "v1"` when present. Older fork builds
  omit the field; the absent-field branch defaults to "routes exist"
  gating until universal advertising is shipped.
- **`DispatchIrV1` JSON wire-format round-trip test** —
  `dispatch_ir_round_trips_through_json` pins serialize → deserialize
  → reserialize as byte-identical. Any new IR field forces serde to
  round-trip or the test fails.
- **`dispatch_ir_accounting_json` enrichment** — new keys:
  `fields_honored` (empty until the fork header ships), `fields_passthrough_only`
  (the v1 scheduler passthrough telemetry list),
  `fallback_triggered`, `fallbacks[*]` (with `backend`,
  `missing_required`, `reason`). The renamed
  `fields_capability_supported_by_backend` (was the misleading
  `fields_honored_by_backend`) makes the static-capability partition
  explicit and leaves `fields_honored` reserved for runtime evidence.
- **Constants module for dispatch field names** —
  `apxm_core::constants::llm::apxm::dispatch_fields` is the single
  source of truth used by producer (`dispatch_fields_sent`) and
  consumer (`BackendGraphCapabilities::field_supported`); 4 sites
  migrated off raw string literals.
- **Lint rule `hardcoded-dispatch-field-literal`** in
  `tools/scripts/check_no_legacy_vllm.py` — rejects raw dispatch-field
  string literals outside the constants module to prevent regression.

### Added — model zoo
- `dekk apxm vllm zoo-apply | zoo-status | zoo-scale | zoo-cache-warm |
  zoo-logs` — the operator-facing CLI surface for the vLLM service fleet.
  Manifest-driven: `deploy/vllm/zoo.example.toml` (template) +
  `deploy/vllm/zoo.toml` (gitignored operator manifest).
- `docs/backends/model-zoo-quickstart.md` — 15-minute end-to-end
  walkthrough from `export APXM_VLLM_HF_HOME` to a working graph
  execution against the zoo.
- `docs/backends/model-zoo.md` — operator reference (schema table,
  daily flow, failure-mode glossary).
- `tools/scripts/check_no_legacy_vllm.py` (invoked as
  `dekk apxm vllm check-no-legacy`) — CI lint enforcing the no-legacy
  / no-fallback discipline across `tools/`, `crates/`, `deploy/`,
  `docs/`, `examples/`. 12 rules; all green on the current tree.

### Added — plans-as-graphs
- LLM PLAN prompt teaches the `inner_plan.task_dag` schema with three
  worked examples (fan-out, diamond, linear). Backwards-compatible:
  the legacy `plan: [steps]` shape still parses.
- `PLAN_GRAPH_EMITTED` event + `PlanGraphEmittedPayload`
  (`plan_id`, `generating_model`, `node_count`, `task_ids`,
  `parallel_fanout_max`) — lets trace consumers tell apart
  "LLM produced free-text steps" from "LLM produced an executable
  graph" and quantifies the extracted parallelism.
- `TaskDag::validate()` is now called up-front in the PLAN handler so
  malformed LLM-emitted DAGs (cycles, dangling `depends_on`, dup ids)
  fail at the PLAN node context with an actionable error.

### Added — workloads
- `examples/python/benchmarks/workloads/sharegpt_row.py` — ShareGPT
  multi-turn adapter, sequential ASK chain with growing prefix.
- `examples/python/benchmarks/workloads/loogle_row.py` — LooGLE
  long-shared-context adapter, fan-out of N questions against one
  shared document.
- Companion fetchers + vendored smoke samples for both.

### Added — Slurm-fork hardening
- `crates/runtime/apxm-credentials/src/validate.rs` —
  `dekk apxm backend test` now probes `/v1/apxm/scheduler` on the
  `vllm` protocol path; a backend that serves `/v1/models` but is
  missing the APXM-fork routes fails to validate with a clear "register
  under protocol=openai instead" message (not at first request).
- `external/vllm` rebased onto upstream `v0.21.0` (branch
  `apxm-rebase-v0.21.0`); image
  `apxm-vllm-runtime:a0e42ad3-e4e3a9af-post-rebase` is the canonical
  runtime artifact.

### Changed
- `README.md` condensed 282 → 74 lines. Operator runbooks moved to
  `docs/backends/model-zoo*.md`; the top-level README is now a landing
  page, not a manual.
- `CONTRIBUTING.md` vLLM section teaches `zoo-apply`, not
  `service-start`. Adds the `APXM_VLLM_HF_HOME` export and cross-links
  the quickstart.
- `deploy/vllm/run-vllm.sh` is single-node only; multi-node Ray (cross-
  node TP+PP) is intentionally out of scope.

### Removed
- `service-start` / `service-adopt` / public `docker-*` CLI surface.
  The zoo manifest is the sole operator entry point.
- `SchedulingPolicy::FCFS` — single-variant `PRIORITY` only.
- `apxm_endpoints_available` capability flag (replaced by a synchronous
  probe at `GraphAwareVllmBackend::new`).
- "Last resort: return any backend" + "round-robin → first-healthy"
  fallback paths in the registry resolver.
- The whole multi-node Ray code path (`run-vllm.sh` NODES>1 branch,
  `verify_cross_shard_pins.py`, the `vllm-kimi` zoo entry).

## [0.0.1] - 2026-04-30 — Public release scrub

This is the first public-ready iteration of the repository.

### Added
- `VISION.md` at the repo root: APXM positioned as a library system for agent
  skills (compiled, versioned, linkable, governed), with three concrete
  proof-point workflows under `examples/python/demos/gemma4/`.
- MIT `LICENSE` at the repo root and across all crates and the Python
  frontend (the previous mixed Apache-2.0 / MIT-OR-Apache-2.0 signals are
  unified to MIT).
- `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `CHANGELOG.md`,
  `.detect-secrets.cfg`, and `.secrets.baseline` for the public release.
- Workspace `Cargo.toml` now sets `license`, `repository`, `homepage`, and
  `authors`; every crate inherits via `*.workspace = true`.

### Changed
- README and documentation lead with the fragmented-skills problem and the
  skills-as-libraries thesis. PXM theory remains first-class as the substrate
  but is now reached through `VISION.md` / `README.md` rather than being the
  entry point itself.
- `examples/python/demos/gemma4/` rewritten as "three skill-library proof
  points" — the workflow source files are unchanged in intent; only the
  framing, the README, and the `pyproject.toml` description are updated.
- `docs/design/apxm-aware-codex-skill-libraries.md` opens with the
  fragmented-skills problem before the integration scope.
