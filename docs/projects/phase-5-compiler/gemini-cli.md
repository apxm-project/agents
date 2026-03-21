# Phase 5: Gemini-CLI Changes -- Compiler Integration

> Draft v6 -- Revised with investigation findings

> Source: [Plan 3 -- Gemini-CLI Changes](../plan-3-gemini-changes.md), Phase 5 (G13)
> Timeline: 3 days (within Phase 5, Weeks 29+)
> Dependencies: Phase 4 complete (`gemini-turn.json` exists), Plan 1 A8 (APXM compiler passes), all prerequisites resolved

---

## Goal

Compile Gemini-CLI's turn graph to optimized `.apxmobj` artifacts. The compiler analyzes the graph for parallelism, dead operations, and fusion opportunities. This is the punch line -- instead of a simple plan, you create an apxm-graph that gets compiled and analyzed.

**Prerequisite:** Phase 4 complete (turn loop is a graph).

> **CRITICAL: GUARD wire index blocker.** The Gemini-CLI turn graph starts with a GUARD node. GUARD has no wire index. Graphs containing GUARD cannot be compiled to `.apxmobj`. Wire index 26 must be assigned to GUARD before this phase can begin.

---

## G13: Compile and Optimize Turn Graphs (3 days)

### G13.1 Compile Turn Graph

```bash
apxm compile gemini-turn.json -o gemini-turn.apxmobj -O2
```

The compiler produces an optimized `.apxmobj` artifact from the turn graph. Optimization passes include:

| Pass | Effect on Gemini-CLI Turn Graph |
|------|-------------------------------|
| **`fuse-ask-ops`** | If multiple sequential ASK calls exist (e.g., compression summary + main prompt), fuse into single API call |
| **`cse`** (Common Subexpression Elimination) | Eliminate duplicate tool invocations with identical arguments |
| **`symbol-dce`** (Dead Code Elimination) | Remove operations whose outputs are never consumed (e.g., REFLECT node when loop detection is disabled) |
| **Parallelism extraction** | Confirm N INV nodes with no inter-edges are dispatched concurrently |
| **`canonicalizer`** | Normalize graph patterns for consistent downstream optimization |

The compiler has 12 distinct passes at O2 (and 93 pass invocations at O3 via fixed-point iteration). Additional passes beyond the four listed above include `normalize`, `build-prompt`, `template-specialization`, `unconsumed-value-warning`, `schema-narrowing`, `scheduling`, `condense-ops`, and `dead-context-elimination`.

### G13.2 Compile-Time Validation

The compiler catches structural errors before any LLM call:
- Missing edges (unreachable nodes)
- DAG constraint violations (cycles)
- Dead operations that consume resources but produce unused output

**Not yet implemented (requires prerequisite work):**
- Type mismatches (wrong attribute types for operations) -- no type annotations exist on edge tokens
- Missing required attributes per operation -- `validate.rs` does not check `OperationSpec.fields`

This provides **49x faster error detection** (compile time vs runtime) for implemented checks -- errors caught in milliseconds instead of after expensive LLM API calls.

### G13.3 Artifact Execution

```bash
apxm run gemini-turn.apxmobj --emit-metrics metrics.json
```

The compiled artifact runs on the A-PXM dataflow scheduler with the same semantics as the interpreted graph but with optimized operation scheduling and pre-validated structure.

---

## Phase 5 Summary

| Task | Days | Key Deliverable |
|------|------|----------------|
| G13: Compile + optimize + validate | 3 | `.apxmobj` artifacts from Gemini-CLI turn graphs |
| **Total** | **3** | -- |

---

## Phase 5 Validation

- [ ] `apxm compile gemini-turn.json` produces valid `.apxmobj`
- [ ] `fuse-ask-ops` reduces API calls on workflows with sequential LLM calls
- [ ] Compile-time validation catches structural errors before any LLM call
- [ ] Compiled artifact execution produces identical results to uncompiled
- [ ] Optimization metrics (parallelism, fused ops, eliminated ops) reported

---

## Related Files

- [README.md](README.md) -- Phase 5 overview
- [apxm.md](apxm.md) -- APXM compiler infrastructure (A8)
- [codex.md](codex.md) -- Codex compiler integration (C12)
- [../plan-3-gemini-changes.md](../plan-3-gemini-changes.md) -- Full Gemini-CLI integration plan
