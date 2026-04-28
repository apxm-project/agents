# Compiler analysis — `02_checkout_context_pruning`

Workflow source: `../02_checkout_context_pruning.py`
Sweep: 2026-04-28, local Gemma vLLM fork, O0 vs O2.

Diagnostics:
- `/tmp/case01-runs/context-pruning/compiler-diagnostics/02_checkout_context_pruning/O0-precompile.json`
- `/tmp/case01-runs/context-pruning/compiler-diagnostics/02_checkout_context_pruning/O2-precompile.json`

## Graph shape that the compiler sees

The workflow declares 8 ops at AIR-emit time:
- 5 context Asks (`Checkout_Context`, `API_Context`, `Observability_Context`, `Compliance_Context`, `Rollout_Context`)
- 1 `g.think(decision)` over the 5 context outputs
- 1 `g.print` of the result
- 1 `done`

The decision Think interpolates only `{checkout_context}`. The other four context operands are passed in as inputs but never appear in the prompt. **They are dead-context.**

## Pass-by-pass effect at O2

From `O2-precompile.json::pass_metrics`:

| Pass | duration_ms | fired | ops_before → after | ir_size_delta |
|---|---|---|---|---|
| normalize | … | 0 | 8 → 8 | + |
| build-prompt | … | 0 | 8 → 8 | 0 |
| template-specialization | … | 0 | 8 → 8 | 0 |
| **dead-context-elimination** | … | **1** | 8 → 8 | + |
| **canonicalizer** | … | 0 (active) | **8 → 4** | – |
| symbol-dce | … | 0 | 4 → 4 | 0 |
| **scheduling** | … | **2** | 4 → 4 | + |
| shared-prefix-analysis | … | 0 | 4 → 4 | 0 |
| **assign-priority** | … | **3** | 4 → 4 | + |

`pass_summary.fired_passes = ["dead-context-elimination", "scheduling", "assign-priority"]`.
`pass_summary.active_passes = ["canonicalizer"]` (canonicalizer is reported as "active" rather than "fired" because it deletes ops rather than firing per-op rewrites).
`pass_summary.total_ops_eliminated = 4`.

This is the only one of the three demo workflows where the IR actually shrinks.

## What changed in `input.air` (O0 → O2)

O0 input.air (28 lines, 8 ops, 7 edges) declares all 5 Asks and threads each output into the `decision` Think:

```mlir
%ck   = ais.ask @Checkout_Context      ...
%api  = ais.ask @API_Context           ...
%obs  = ais.ask @Observability_Context ...
%comp = ais.ask @Compliance_Context    ...
%roll = ais.ask @Rollout_Context       ...
%dec  = ais.think @Decision  inputs(%ck, %api, %obs, %comp, %roll) { prompt = "...{checkout_context}..." }
ais.print %dec
ais.done
```

O2 input.air (20 lines, **4 ops, 3 edges**) reads:

```mlir
%ck  = ais.ask @Checkout_Context ... { priority = 90, tier, latency_class, stage_index, downstream_nodes, remaining_path_len, parallel_safe, fanout_count, estimated_cost }
%dec = ais.think @Decision inputs(%ck) { prompt = "...", priority, tier, ... }
ais.print %dec
ais.done
```

The transformation is concrete and verifiable:

1. **`dead-context-elimination` (fired=1)** removes the unused operands `%api, %obs, %comp, %roll` from the `decision` Think's input list.
2. **`canonicalizer` (active)** then sees that `Ask @API_Context`, `Ask @Observability_Context`, `Ask @Compliance_Context`, `Ask @Rollout_Context` produce SSA values with **no remaining users**. They are pure ops in AIR semantics, so canonicalizer DCEs them. `ops_before = 8 → ops_after = 4`. Four ops eliminated.
3. **`scheduling` (fired=2)** stamps the surviving 2 LLM ops (`Checkout_Context` and `Decision`) with parallelism metadata.
4. **`assign-priority` (fired=3)** stamps `priority = 90` on the Asks plus `priority` on the Think and Print.
5. **`shared-prefix-analysis` (fired=0)** — only one Ask remains, so there is no equivalence class to discover.

## Net compiler outcome for this workflow

Real IR mutation, not just metadata stamping:

- 8 → 4 ops (50 % shrink)
- 6 → 2 LLM call sites (4 dead Asks + 1 think input cleanup, see `runtime.md`)
- 7 → 3 graph edges
- All 4 surviving ops carry full scheduling + priority metadata at O2

This is the canonical "context pruning" demo: the workflow author over-declared inputs to a single Think, and the optimizer reclaimed all four.

## Why this transformation is safe

`dead-context-elimination` only removes a Think input if **no downstream user references the corresponding placeholder name in any prompt template**. The pass walks the prompt strings and checks placeholder names against the actual input map. Here, `Decision` declared inputs `(checkout_context, api_context, observability_context, compliance_context, rollout_context)` but its template references only `{checkout_context}`. The other four are unambiguously dead.

The canonicalizer's subsequent op-level DCE is safe because Ask ops in AIR are side-effect-free with respect to graph state (they produce a value; they don't mutate). Once their SSA result has zero users, they can be eliminated. (If an Ask had been declared with `side_effects = true`, canonicalizer would have left it alone.)

## What did *not* fire

- **`build-prompt`, `template-specialization`** — `fired=0`. Templates already resolved.
- **`symbol-dce`** — `fired=0`. No top-level symbol declarations to DCE.
- **`shared-prefix-analysis`** — `fired=0`. Only one Ask survives.
- **No DSPy pass.** Same gating as case 01; see `dspy.md`.
