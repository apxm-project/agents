# Phase 5: Compiler Integration -- The Punch Line

> Draft v6 -- Revised with investigation findings

> Instead of a simple plan, you create an apxm-graph that gets compiled and analyzed.
> The graph IS the plan, and the compiler IS the analysis engine.

**Timeline:** Weeks 29+
**Dependencies:** Phase 4 complete (turn loops are graphs), all prerequisites resolved (see below)

---

## Goal

Phase 5 is the punch line of the entire A-PXM thesis. Once any agent framework expresses its turn loop as an AIS graph, that graph can be compiled, analyzed, and optimized -- the same way GCC/LLVM transformed C programs. This is what makes APXM a universal execution substrate, not just another runtime.

Compile consumer-authored AIS graphs to optimized `.apxmobj` artifacts. Run MLIR-based optimization passes (`fuse-ask-ops`, `cse`, `symbol-dce`, `canonicalizer`, and 8 others). Achieve compile-time validation that catches structural errors **49x faster** than runtime detection.

**This is why everything else was built.**

Phases 1 through 4 transform imperative consumer code into AIS graph representations. Phase 5 is where that investment pays off: the compiler takes those graphs and applies the full power of MLIR-based analysis and optimization. Errors that would previously surface only after expensive LLM API calls are now caught in milliseconds at compile time. Redundant operations are fused or eliminated. Parallelism is extracted automatically from the DAG structure.

---

## Pre-Phase-5 Prerequisites

These items must be resolved before Phase 5 work begins:

| Item | Severity | Effort | Description |
|------|----------|--------|-------------|
| Assign GUARD wire index | **High** | Small | GUARD has no wire index. Graphs containing GUARD cannot be compiled to `.apxmobj`. Wire index 26 must be assigned to GUARD before Phase 5 begins. This blocks Gemini-CLI turn graph compilation. |
| Assign other Phase 1 ISA wire indices | Medium | Small | UpdateGoal (25), Claim (27), Pause (28), Resume (29) also need wire indices from the reserved 25-30 range. |
| Add required attribute validation | **High** | Medium | The graph validator does NOT check required attributes from `OperationSpec.fields`. Add validation to `validate.rs` before Phase 5. |
| Fix BRANCH_ON_VALUE naming in plan docs | Medium | Trivial | Use correct operation name `BRANCH_ON_VALUE` (not `BRANCH`). |
| Verify TRY_CATCH MLIR representation | Medium | Medium | Ensure MLIR dialect supports subgraph error boundaries for recovery turns. |
| Update service crate references | Low | Trivial | Replace `apxm-service` references with `apxm-server`. |

---

## What Each Consumer Delivers

### APXM (Infrastructure)

The APXM project provides the compiler infrastructure that makes Phase 5 possible:

- **Compiler passes** validated against consumer graph patterns (`fuse-ask-ops`, `cse`, `symbol-dce`, `canonicalizer`, and 8 others)
- **`apxm compile`** extended to handle consumer-authored graphs with full optimization
- **Compile-time validation** that catches structural errors before any LLM call
- **Diagnostic output** (`--emit-diagnostics`) providing machine-readable optimization reports

See: [apxm.md](apxm.md) | Source: Plan 1, Phase A8

### Codex

- **`codex-turn.apxmobj`** -- compiled and optimized Codex turn graph artifact
- **Measured `fuse-ask-ops` impact** on real Codex workflows (API call reduction)
- **Measured compile-time error detection** rate vs. runtime detection
- **Decompilation** (`apxm decompile codex-turn.apxmobj`) showing optimized graph structure

See: [codex.md](codex.md) | Source: Plan 2, Phase C12

### Gemini-CLI

- **`gemini-turn.apxmobj`** -- compiled and optimized Gemini-CLI turn graph artifact
- **Measured optimizations** across all passes (fusion, CSE, DCE, parallelism)
- **Compile-time validation** catching Gemini-specific structural errors
- **Artifact execution** on the A-PXM dataflow scheduler with optimization metrics

See: [gemini-cli.md](gemini-cli.md) | Source: Plan 3, Phase G13

---

## Optimization Passes

The compiler has 12 distinct passes at O2 (and 93 pass invocations at O3 via fixed-point iteration). The four most consumer-visible passes are `fuse-ask-ops`, `cse`, `symbol-dce`, and `canonicalizer`. Additional passes include `normalize`, `build-prompt`, `template-specialization`, `unconsumed-value-warning`, `schema-narrowing`, `scheduling`, `condense-ops`, and `dead-context-elimination`.

Phase 5 validates these passes work on the graph patterns consumers produce:

| Pass | What It Does | Consumer Benefit |
|------|-------------|-----------------|
| **`fuse-ask-ops`** | Merges sequential ASK->ASK chains into one API call | Fewer API calls, lower latency, lower cost |
| **`cse`** | Eliminates duplicate LLM calls with identical inputs | Saves dollars and seconds |
| **`symbol-dce`** | Removes operations whose outputs are never consumed | Leaner graphs |
| **`canonicalizer`** | Normalizes graph patterns for consistent optimization | Reliable downstream passes |

**Pre-MLIR graph-level passes** also run before lowering (in `apxm-graph/src/optimize.rs`):
- **`prompt_caching()`** -- detects shared system prompts across ASK/THINK nodes; marks duplicates with `cached_system_prompt: true`
- **`memoization_hints()`** -- detects duplicate pure operations (QMEM, CONST_STR, VERIFY) with identical attributes; marks with `memoizable: true`

### Optimization Levels

| Level | Passes | Behavior |
|-------|--------|----------|
| O0 | 0 | No optimization (passthrough) |
| O1 | 8 | Basic: normalize, build-prompt, scheduling, fusion, canonicalization, CSE, DCE |
| O2 | 12 | O1 + template-specialization, dead-context-elimination, schema-narrowing, condense-ops |
| O3 | 93 | O2 passes iterated 10x for fixed-point convergence |

---

## Compile-Time Validation

Structural errors caught before any LLM call:

- Unreachable nodes (dead code)
- Cycles in what should be a DAG
- Invalid operation sequences (e.g., MERGE without prior BRANCH_ON_VALUE)
- Tool execution without approval (INV node with no VERIFY dependency)
- Approval of a tool that was never called (VERIFY with no INV predecessor)

**Not yet implemented (requires prerequisite work):**

- Missing required attributes on AIS operations -- the graph validator does NOT currently check required attributes from `OperationSpec.fields`; attribute validation must be added to `validate.rs` before Phase 5 (see prerequisites)
- Type mismatches on data edges -- the graph validator does not check type compatibility on edges; no type annotations exist on edge tokens
- Capability references to unregistered tools -- capability resolution happens at runtime via the CapabilitySystem, not at compile time; compile-time checking would require access to the tool registry

---

## Risk Matrix

| Severity | Risk | Impact |
|----------|------|--------|
| **High** | GUARD wire index gap | Gemini-CLI turn graph uses GUARD as its first node. No wire index means compilation to `.apxmobj` fails. Hard blocker. |
| **High** | Missing attribute validation | Plan promises compile-time validation of required attributes, but `validate.rs` does not check `OperationSpec.fields`. Undermines the "49x faster error detection" claim for attribute errors. |
| Medium | TRY_CATCH compilation path | If Gemini-CLI recovery turn uses TRY_CATCH, the MLIR dialect's representation of subgraph boundaries needs verification. |
| Medium | Consumer graph patterns vs. pass assumptions | Existing pass test suite uses APXM-internal graph patterns, not consumer-style patterns (fan-out/fan-in tool invocation, hook fences, recovery subgraphs). |
| Low | Pass name mismatches | Passes exist under kebab-case names (`fuse-ask-ops`, `symbol-dce`), not PascalCase. No code impact, cosmetic only. |
| Low | Additional passes beyond plan scope | Compiler has 12 passes, not 4. A feature, not a risk, but consumers should be aware. |

---

## Integration Tests

### Compilation Tests

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_codex_turn_graph_compiles` | Construct Codex turn graph, compile at O2, verify `.apxmobj` produced | P0 |
| `test_gemini_turn_graph_compiles` | Construct Gemini turn graph, compile at O2, verify `.apxmobj` produced | P0 |
| `test_all_optimization_levels` | Compile same graph at O0, O1, O2, O3, verify all produce valid artifacts | P0 |
| `test_compile_with_emit_diagnostics` | Compile with diagnostics, verify PipelineDiagnostics JSON output | P1 |
| `test_compile_empty_graph_error` | Compile empty graph, verify clean error | P1 |
| `test_compile_invalid_graph_error` | Compile graph with cycle, verify clean error | P1 |
| `test_compile_missing_attributes_error` | Compile graph with missing required attributes (once validation added), verify error | P1 |

### Optimization Tests

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_fuse_ask_ops_reduces_operations` | Graph with ASK -> ASK chain; after `fuse-ask-ops`, verify ops_after < ops_before | P0 |
| `test_cse_eliminates_duplicates` | Graph with two identical ASK nodes; after `cse`, verify one eliminated | P0 |
| `test_symbol_dce_removes_dead_code` | Graph with unreachable node; after `symbol-dce`, verify node removed | P0 |
| `test_canonicalizer_normalizes` | Graph with non-canonical patterns; after `canonicalizer`, verify normalized | P1 |
| `test_o2_vs_o0_fewer_ops` | Same graph at O2 and O0; verify O2 has fewer operations | P0 |
| `test_pass_metrics_reported` | Run with `run_with_metrics()`; verify per-pass timing and delta data | P1 |

### Round-Trip Tests

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_compile_decompile_roundtrip` | Compile graph to `.apxmobj`, parse artifact back, verify node/edge structure matches | P0 |
| `test_artifact_roundtrip_all_op_types` | Artifact with one node per wire-indexed op type; emit + parse round-trip | P0 |
| `test_multi_dag_artifact_roundtrip` | Artifact with FLOW_CALL producing 2+ DAGs; verify all DAGs survive round-trip | P1 |
| `test_compiled_execution_matches_uncompiled` | Execute graph directly, then compile and execute artifact; compare results | P0 |

### Performance Tests

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_compile_time_under_threshold` | Compile a 50-node consumer graph; verify compile time < 1 second | P1 |
| `test_fuse_ask_ops_api_call_reduction` | Graph with 4 sequential ASK nodes; verify `fuse-ask-ops` reduces to fewer API calls | P0 |
| `test_parallelism_extraction_speedup` | Graph with fan-out/fan-in pattern; verify scheduler reports parallelism > 1 | P1 |
| `test_compile_time_error_detection_faster_than_runtime` | Inject a structural error; compare compile-time detection speed vs runtime detection | P2 |

### Boundary Tests

End-to-end validation for Phase 5 completion:

1. Construct a consumer-style turn graph (ASK -> BRANCH_ON_VALUE -> INV x3 -> WAIT_ALL -> VERIFY -> UMEM)
2. Compile at O2: `apxm compile graph.json -o turn.apxmobj -O2`
3. Verify artifact produced with optimization report
4. Execute artifact: `apxm run turn.apxmobj`
5. Compare: compiled execution produces same results as uncompiled execution
6. Verify: `fuse-ask-ops` metrics show reduction (if applicable)
7. Verify: compile-time validation caught a planted structural error

---

## Phase 5 Validation Criteria

- [ ] `apxm compile agent-graph.json` produces optimized `.apxmobj` from consumer graphs
- [ ] `fuse-ask-ops` measurably reduces API calls on real Codex/Gemini-CLI workflows
- [ ] Compile-time validation catches structural errors before any LLM call
- [ ] `apxm analyze graph.json --json` provides parallelism and optimization report
- [ ] Compiled artifact execution produces identical results to uncompiled graphs
- [ ] Optimization metrics (parallelism, fused ops, eliminated ops) reported for each consumer

---

## Effort Estimates

| Consumer | Days (est.) | New Files | Modified Files | New Lines (est.) |
|----------|------------|-----------|----------------|-----------------|
| APXM (A8) | ~15 | ~2 | ~5 | ~800 |
| Codex (C12) | 3+ | 1 | 1 | ~200 |
| Gemini-CLI (G13) | 3 | 0 | 1 | ~100 |
| **Total** | **~21+** | **~3** | **~7** | **~1100** |

---

## Important Notes

- **MLIR/LLVM required:** The compiler requires MLIR/LLVM dependencies. Phases 1-4 work without them. Use `apxm doctor` to verify the compiler toolchain is available.
- **Reference:** See `apxm/docs/implementation/compiler/overview.md` for compiler architecture details.
- **CLI commands:**
  ```bash
  apxm compile graph.json -o out.apxmobj -O2          # compile with optimizations
  apxm compile graph.json --emit-diagnostics diag.json # machine-readable analysis
  apxm run out.apxmobj --emit-metrics metrics.json     # execute compiled artifact
  apxm decompile out.apxmobj                           # inspect optimized graph
  apxm analyze graph.json --json                       # parallelism + optimization report
  ```

---

## Related Files

- [INVESTIGATION.md](INVESTIGATION.md) -- Investigation report with source code cross-references
- [apxm.md](apxm.md) -- APXM compiler infrastructure changes
- [codex.md](codex.md) -- Codex consumer compilation
- [gemini-cli.md](gemini-cli.md) -- Gemini-CLI consumer compilation
- [../plan-1-apxm-changes.md](../plan-1-apxm-changes.md) -- Full APXM integration plan (Phase A8)
- [../plan-2-codex-changes.md](../plan-2-codex-changes.md) -- Full Codex integration plan (Phase C12)
- [../plan-3-gemini-changes.md](../plan-3-gemini-changes.md) -- Full Gemini-CLI integration plan (Phase G13)
