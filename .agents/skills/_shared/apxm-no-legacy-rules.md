# Shared rule — APXM no-legacy / no-fallback contract

APXM enforces a strict hard-fail-at-config-time discipline. The model-zoo
migration removed an entire surface of legacy commands and fallback
chains; the patterns they enabled (silent skips, capability flags
papering over a missing fork, "last resort" resolver branches) hide
errors in benchmarks and produce unreproducible runs.

The lint at `tools/scripts/check_no_legacy_vllm.py` is project policy.
All 12 rules below cause `--strict` failure (and a stderr warning from
the pre-write hook).

## The 12 rules (verbatim from the lint)

1. **`legacy-service-start`** — `dekk apxm vllm service-start` is
   removed. Use `zoo apply` against a `deploy/vllm/zoo*.toml` manifest.
2. **`legacy-service-adopt`** — `service-adopt` is removed. Write a
   `zoo.toml` entry, then `zoo apply`.
3. **`legacy-run-vllm-slurm`** — `run-vllm-slurm.sh` is deleted. Use the
   unified `deploy/vllm/run-vllm.sh`.
4. **`hardcoded-port-8916`** — never the literal `8916` outside the
   allocator-range default. Use `_allocate_port()` or a manifest-supplied
   port.
5. **`apxm-endpoints-available-flag`** — no `apxm_endpoints_available`,
   `apxm_routes_available`, or equivalent capability flags that paper
   over a missing APXM-vLLM fork. Probe-fail at startup.
6. **`resolver-last-resort`** — no resolver "last resort" branch.
7. **`resolver-rr-fallback`** — no silent round-robin fallback when the
   configured policy is missing.
8. **`scheduling-policy-fcfs`** — no FCFS scheduling literal in code
   that should consume the configured policy.
9. **`or-env-or-default-chain`** — no `cfg or env or "default"` chains
   that hide missing required config. Hard-fail at config load.
10. **`already-exists-skipping`** — no silent "skipping, already exists"
    branches. Either reuse explicitly or hard-fail.
11. **`shell-model-specific-default`** — no model-specific defaults
    baked into shell wrappers (`gpt-oss-120b` etc.). Pass via flag.
12. **`hardcoded-dispatch-field-literal`** — dispatch field names must
    come from `graph_attrs::*` constants. No string literals in
    handlers.

## Why these matter

Plan 02 §6.7 ("no legacy, no fallback") is the rule the project uses to
keep benchmarks honest. Past incidents from before the rule:

- A capability flag silently skipped APXM routes, so a "graph-aware"
  benchmark ran on stock vLLM and produced unattributable numbers.
- A resolver fallback round-robined across backends, so a single-backend
  benchmark accidentally split load between two — undetected for weeks.
- A hardcoded port collision caused two services to write to each
  other's logs, contaminating an evaluation run.

When you see any of these patterns, hard-fail at config time with a
loud error message. Do not paper over with a flag, default, or "last
resort" branch.

## When to run the lint

- Before staging a commit: `python3 tools/scripts/check_no_legacy_vllm.py --strict`.
- As part of `apxm-finish`.
- In CI on every PR.
- The pre-write hook runs it per-file and emits a stderr warning so you
  can self-correct before committing.

If a legitimate use of a banned pattern must remain (e.g. an evidence
file citing a literal port `8916`), it goes in `exclude_globs` in
`check_no_legacy_vllm.py` with a comment justifying the exclusion.
