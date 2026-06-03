# Migration: build-apxm + eval skills evicted from apxm-libs

> **STATUS: APPLIED 2026-06-03.** The apxm-libs repo was dissolved; the build-APXM
> contributor skills that previously lived there as scaffolds were not relocated
> verbatim (their canonical homes in `apxm/.agents` + `apxm-eval/.agents` are
> richer). The unique deltas below were folded in:
> - `apxm-backend-add` ← provider touch-point template + `graph_attrs`/`metrics_keys`
>   constant rule.
> - `apxm-mcp-server` ← cross-surface (REST+MCP+A2A) registration + REST↔MCP
>   fanout + contract-string SSOT + the BaseHTTPMiddleware incident.
> - `apxm-design-docs` ← created net-new in `apxm/.agents/skills/`.
> - `apxm-preregistration` (apxm-eval) ← the new-benchmark scaffold decision tree
>   (host decision, workload-family templates, `build_layout`, paired-arm cache salt).
> - `apxm-orient`/`apxm-author-python`/`apxm-compiler-pass`/`apxm-runtime-backends`/
>   `apxm-server-mcp`/`apxm-quality-eval` were redundant with existing skills — no
>   port. Agent configs regenerated in both repos.

This file remains the per-skill record of what was unique in each evicted pack.

## Destination: `apxm-project/apxm` → `.agents/`

| Evicted pack | Destination skill | Status |
|---|---|---|
| `apxm-orient` | `apxm-context` | **redundant** — delete, no port |
| `apxm-author-python` | `apxm-compile-and-execute` | **redundant** — pointers only |
| `apxm-compiler-pass` | `apxm-mlir-pass-development` | **redundant** — strict subset |
| `apxm-runtime-backends` | `apxm-backend-add` (+ `apxm-fork-vllm-rebase`) | **delta to fold in** |
| `apxm-server-mcp` | `apxm-mcp-server` | **delta to fold in** |
| `apxm-design-docs` | *(new skill)* `apxm-design-docs` under `.agents/skills/` | **net-new — recreate** |

### `apxm-orient` → `apxm-context` (redundant)
`apxm-context` already runs `dekk apxm doctor`, `dekk apxm ops list`, project-
memory recall, and subsystem-ownership surfacing. The only nominal difference was
`apxm-orient`'s structured JSON card (`doctor`/`crate_map`/`recent_commits`/
`companion_repos`) — a packaging of the same deterministic introspection. No port.

### `apxm-author-python` → `apxm-compile-and-execute` (redundant)
Was an unimplemented scaffold. Its content was file pointers
(`crates/compiler/apxm-frontend/python/apxm/__init__.py`, `examples/python/`,
`tests/test_imports.py`) and the `.td → dekk apxm codegen` cadence — all already
in `apxm-compile-and-execute` (which even has the "hand-rolling JSON vs Python
frontend" anti-pattern). No port.

### `apxm-compiler-pass` → `apxm-mlir-pass-development` (redundant)
`apxm-mlir-pass-development` already covers `build_pass_list()` as SSOT, the
`build-dialect` + `codegen` cadence, the AIS-ops-defined-in-`apxm-core` rule, the
canonical-attribute-enum rule, and the `apxm-ais-op-design` gate. Strict subset.
No port.

### `apxm-runtime-backends` → `apxm-backend-add` (DELTA TO FOLD IN)
`apxm-backend-add` is the *operator* surface (register/test backends).
`apxm-runtime-backends` was the *implementer* surface (build the backend layer).
Fold these unique pieces into `apxm-backend-add` (or a sibling
`apxm-backend-implement` skill if the operate/implement split is worth keeping):

- **Provider touch-point template:**
  `crates/runtime/apxm-backends/src/<provider>/{mod.rs,client.rs,config.rs}`,
  `…/registry.rs` (enum + dispatch), `…/apxm-runtime/src/executor.rs` (hint),
  `config/backends.example.toml`, `tests/backends/<provider>_smoke.rs`.
- **Promote contract strings to `graph_attrs::*` / `metrics_keys::*` constants**
  — never a literal `"reuse_group"` in handlers (`feedback_attribute_dual_naming`).
- **vLLM cherry-pick path:** edit the APXM side first; write a cherry-pick plan
  against branch `apxm-rebase-v0.21.0` in `external/vllm`; the `apxm-project/vllm`
  fork holds 5 APXM commits on upstream v0.21.0 (a new commit is the sixth);
  `dekk apxm vllm check-no-legacy --strict` after. (Cross-check
  `apxm-fork-vllm-rebase`, which may already cover the cherry-pick mechanics.)
- **No-legacy rule name** `or-env-or-default-chain` in `check_no_legacy_vllm.py`.

### `apxm-server-mcp` → `apxm-mcp-server` (DELTA TO FOLD IN)
`apxm-mcp-server` covers the FastMCP shim only. `apxm-server-mcp` was the
*cross-surface registration planner* (REST + MCP + A2A). Fold in:

- **Cross-surface touch-point template:**
  `crates/tools/apxm-server/src/routes/<route>.rs`, `…/router.rs`,
  `…/contract.rs` (route-path constant), `tools/scripts/apxm_mcp_install.py`,
  `.dekk.toml` (CLI wrapper), `tests/server/<route>_smoke.rs`,
  `docs/api/<route>.md`.
- **REST↔MCP fanout rule:** a route that should also be an MCP tool needs both
  the handler and the tool wrapper — don't ship REST-only.
- **BaseHTTPMiddleware incident** (`feedback_basehttpmiddleware_breaks_chat`):
  server middleware must use raw ASGI, never Starlette `BaseHTTPMiddleware`
  (its receive-queue treats disconnect polls as disconnects and silently nulls
  chat responses).
- **Contract-string SSOT:** route paths, env names, response markers, and MCP
  tool names live in `crates/tools/apxm-server/src/contract.rs` (server) /
  `apxm.contract` (Python), never as handler literals.

### `apxm-design-docs` → NET-NEW skill in `apxm/.agents/skills/` (RECREATE)
No skill in `apxm/.agents` covers `docs/design/` prose discipline (grep for
`overclaim`/`docs/design` returns nothing). `apxm/.agents/domains/meta/` exists
but holds no equivalent. Recreate as a first-class skill — it is genuinely
unique and high-value. The content to carry over (deterministic, no LLM call):

- **Job:** gate edits to `docs/design/` for two shipped failure modes —
  *overclaim* (present-tense prose about unwired behaviour) and *citation drift*
  (factual claims with no anchor to shipped code).
- **Probe sequence:** diff parse → tense+status check (every present-tense
  factual claim needs a citation; aspirational sections need an explicit
  `Status: design` label) → citation resolution (cited path/commit must exist on
  the branch; stale = flagged) → cross-doc consistency fanout.
- **Hard contracts:** every factual claim cites a path/commit; aspirational
  sections labelled explicitly; the skill gates but does **not** write the edit.

## Destination: `apxm-project/apxm-eval` → `.agents/skills/`

| Evicted pack | Closest destination skill | Status |
|---|---|---|
| `apxm-quality-eval` | `apxm-review-council-bench` / `apxm-evaluation-artifacts` | **mostly redundant** — verify, port any harness specifics |
| `apxm-demos-benchmarks` | `apxm-preregistration` + `apxm-priority-lane-bench` | **delta to fold in** |

### `apxm-quality-eval` (verify, likely redundant)
Was a near-empty scaffold ("drive a fixture through the harness: fixture + budget
+ judge + claim-check"), dependent on a sibling `apxm-eval` clone for the fixture
catalog (`tools/quality_eval/`). Confirm `apxm-review-council-bench` /
`apxm-evaluation-artifacts` cover the fixture→budget→judge→claim-check flow; if a
harness-driving step is missing, port that one step. Otherwise delete.

### `apxm-demos-benchmarks` → `apxm-preregistration` + bench skills (DELTA TO FOLD IN)
Unique templater/decision-tree content not in the prereg skill alone:

- **Decision tree:** `host_repo` = `apxm-eval` (paper-bound, prereg required,
  claim card) vs `apxm` (runtime-internal microbench, no prereg/claim).
  Workload family → template: `latency-paired-arm`, `throughput-sweep`,
  `cache-reuse-paired`, `dispatch-correctness`. Prereg template map under
  `apxm-eval/docs/preregistrations/_templates/`.
- **Output scaffold layout:** `docs/preregistrations/<UTC>-<workload>.md`,
  `examples/python/benchmarks/<workload>/{__init__.py,run.py,workload.py,README.md}`,
  `docs/claims/<workload>.md`.
- **Hard contracts:** paths resolve via
  `apxm.contract.build_layout(__file__).evaluation_scenario(workload_name)`
  (never raw `os.path`); cache salt per `(arm, opt)` for paired-arm benchmarks;
  `dekk apxm vllm check-no-legacy --strict` in the driver CI block; no
  hardcoded allocator-range port literal (use the allocator).
