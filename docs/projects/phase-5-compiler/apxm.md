# Phase 5: APXM Changes -- Compiler Integration

> Draft v6 -- Revised with investigation findings

> Source: [Plan 1 -- APXM Changes](../plan-1-apxm-changes.md), Phase A8
> Timeline: ~3-4 weeks (within Phase 5, Weeks 29+)
> Dependencies: Phase 4 complete (consumer graphs exist and execute via `apxm execute`), all prerequisites resolved

---

## This Is the Punch Line

Phase 5 is the punch line of the entire A-PXM thesis. Once any agent framework expresses its turn loop as an AIS graph, that graph can be compiled, analyzed, and optimized -- the same way GCC/LLVM transformed C programs. This is what makes APXM a universal execution substrate, not just another runtime.

Instead of imperative loops, consumers express their agent logic as AIS graphs. Those graphs are compiled, analyzed, and optimized. The compiler catches structural errors at compile time (49x faster than runtime detection), fuses redundant operations, eliminates dead code, and extracts parallelism.

---

## Prerequisites (Must Be Resolved Before Phase 5)

> **CRITICAL: GUARD wire index blocker.** GUARD has no wire index. Graphs containing GUARD cannot be compiled to `.apxmobj`. Wire index 26 must be assigned to GUARD before Phase 5 begins. This blocks Gemini-CLI turn graph compilation.

| Item | Severity | Effort | Description |
|------|----------|--------|-------------|
| Assign GUARD wire index | **High** | Small | Map wire index 26 to Guard in `from_wire_index`/`to_wire_index` |
| Assign other Phase 1 ISA wire indices | Medium | Small | UpdateGoal (25), Claim (27), Pause (28), Resume (29) |
| Add required attribute validation | **High** | Medium | Use `OperationSpec.fields` in `validate.rs` to check required attributes |
| Fix BRANCH_ON_VALUE naming in plan docs | Medium | Trivial | Use correct operation name `BRANCH_ON_VALUE` (not `BRANCH`) |
| Verify TRY_CATCH MLIR representation | Medium | Medium | Ensure MLIR dialect supports subgraph error boundaries |
| Update service crate references | Low | Trivial | Replace `apxm-service` with `apxm-server` |

---

## A8.1 Compiler Passes on Consumer Graphs

The compiler has 12 distinct passes at O2 (and 93 pass invocations at O3 via fixed-point iteration). The four most consumer-visible passes are `fuse-ask-ops`, `cse`, `symbol-dce`, and `canonicalizer`. Additional passes include `normalize`, `build-prompt`, `template-specialization`, `unconsumed-value-warning`, `schema-narrowing`, `scheduling`, `condense-ops`, and `dead-context-elimination`.

Phase A8 ensures these passes work on the graph patterns consumers produce:

| Pass | Consumer Benefit |
|------|-----------------|
| **`fuse-ask-ops`** | Merges sequential ASK->ASK chains into one API call (fewer calls, lower cost) |
| **`cse`** | Eliminates duplicate LLM calls with identical inputs (saves dollars and seconds) |
| **`symbol-dce`** | Removes operations whose outputs are never consumed (leaner graphs) |
| **`canonicalizer`** | Normalizes graph patterns for consistent optimization |

**Pre-MLIR graph-level passes** also run before lowering (in `apxm-graph/src/optimize.rs`):
- **`prompt_caching()`** -- detects shared system prompts across ASK/THINK nodes
- **`memoization_hints()`** -- detects duplicate pure operations with identical attributes

### Optimization Levels

| Level | Passes | Behavior |
|-------|--------|----------|
| O0 | 0 | No optimization (passthrough) |
| O1 | 8 | Basic: normalize, build-prompt, scheduling, fusion, canonicalization, CSE, DCE |
| O2 | 12 | O1 + template-specialization, dead-context-elimination, schema-narrowing, condense-ops |
| O3 | 93 | O2 passes iterated 10x for fixed-point convergence |

---

## A8.2 `apxm compile` for Consumer Graphs

```bash
apxm compile codex-turn.json -o codex-turn.apxmobj -O2
# Produces optimized artifact with compile-time analysis report

apxm compile codex-turn.json --emit-diagnostics diag.json
# Machine-readable: parallelism opportunities, fused ops, dead code, type errors
```

The `apxm compile` command is extended to handle consumer-authored graph patterns. This includes graphs produced by Codex (`codex-turn.json`) and Gemini-CLI (`gemini-turn.json`) in Phase 4. The compiler applies the full optimization pipeline and emits a `.apxmobj` artifact that can be executed with `apxm run`.

---

## A8.3 Compile-Time Validation

Structural errors caught before any LLM call:

- Unreachable nodes (dead code)
- Cycles in what should be a DAG
- Invalid operation sequences (e.g., MERGE without prior BRANCH_ON_VALUE)

**Not yet implemented (requires prerequisite work):**

- Missing required attributes on AIS operations -- the graph validator does NOT currently check required attributes from `OperationSpec.fields`; add validation to `validate.rs` before Phase 5
- Type mismatches on data edges -- the graph validator does not check type compatibility on edges; no type annotations exist on edge tokens
- Capability references to unregistered tools -- capability resolution happens at runtime via the CapabilitySystem, not at compile time; compile-time checking would require access to the tool registry

---

## A8.4 Wire Format (.apxmobj)

The `.apxmobj` artifact format:

- **Wire format version:** 3 (`WIRE_VERSION = 3`)
- **Multi-DAG support:** FLOW_CALL produces multiple DAGs in one artifact
- **Pure-Rust emitter/parser:** `emit_wire_dags` / `parse_wire_dags` with round-trip tests
- **Deterministic byte output:** attributes sorted by key for reproducible builds
- **Per-node data:** id (u64) + op_index (u32, via `to_wire_index`) + attributes + input/output tokens + priority + estimated_latency

---

## A8.5 Deliverables

| Deliverable | Location |
|-------------|----------|
| Verify passes on consumer graph patterns | `apxm-compiler/src/passes/` (validate, possibly extend) |
| Compile endpoint on service | `apxm-server/src/routes/compile.rs` |
| Diagnostic output for consumers | `apxm-cli/src/commands/compile.rs` (enhance) |

---

## A8.6 Acceptance Criteria

- [ ] `apxm compile agent-graph.json` produces optimized `.apxmobj` from consumer graphs
- [ ] `fuse-ask-ops` measurably reduces API calls on real Codex/Gemini-CLI workflows
- [ ] Compile-time validation catches structural errors before any LLM call
- [ ] `apxm analyze graph.json --json` provides parallelism and optimization report

---

## Phase Summary

| Step | Days (est.) | New Files | Modified Files | New Lines (est.) |
|------|------------|-----------|----------------|-----------------|
| A8: Compiler integration | ~15 | ~2 | ~5 | ~800 |

This is the final APXM phase. All prior phases build toward this:

| Phase | What It Built | Why A8 Needs It |
|-------|--------------|-----------------|
| A1-A4 (Phase 1) | Event vocabulary, streaming backends, HTTP bridge | LLM calls during graph execution |
| A5 (Phase 2) | Capability system, tool registration | INV nodes resolve to registered tools |
| A6 (Phase 3) | Hierarchical AAM, file-backed state | QMEM/UMEM nodes access state |
| A7 (Phase 4) | Consumer graph validation + execution | Graphs must exist before they can be compiled |
| **A8 (Phase 5)** | **Compiler passes on consumer patterns** | **The destination** |

---

## Related Files

- [README.md](README.md) -- Phase 5 overview
- [codex.md](codex.md) -- Codex compiler integration (C12)
- [gemini-cli.md](gemini-cli.md) -- Gemini-CLI compiler integration (G13)
- [../plan-1-apxm-changes.md](../plan-1-apxm-changes.md) -- Full APXM integration plan
