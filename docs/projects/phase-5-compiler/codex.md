# Phase 5: Codex Changes -- Compiler Integration

> Draft v6 -- Revised with investigation findings

> Source: [Plan 2 -- Codex Changes](../plan-2-codex-changes.md), Phase 5 (C12)
> Timeline: 3+ days (within Phase 5, Weeks 29+)
> Dependencies: Phase 4 complete (`codex-turn.apxm` exists), Plan 1 A8 (APXM compiler passes), all prerequisites resolved

---

## Goal

Compile the Codex turn graph, run optimization passes, and measure improvement. This is the punch line -- the reason everything else was built.

**Depends on Plan 1:** MLIR-based compiler, `fuse-ask-ops` pass, `cse`/`symbol-dce` passes, `.apxmobj` artifact format.

---

## C12: Compile and Optimize

Once `codex-turn.apxm` exists (Phase 4), the compiler becomes available:

```bash
apxm compile codex-turn.apxm -o codex-turn.apxmobj -O2
```

### What the Compiler Provides

| Optimization | What It Does | Expected Impact |
|-------------|-------------|-----------------|
| **`fuse-ask-ops`** | Merges producer-consumer ASK chains into single API calls | Fewer API calls, lower latency |
| **`cse`** | Eliminates duplicate LLM calls with identical inputs | Saves dollars and seconds |
| **`symbol-dce`** | Removes operations whose outputs are never consumed | Leaner graphs |
| **Parallelism extraction** | Infers concurrency from DAG structure | Automatic tool parallelism |
| **Compile-time validation** | Catches structural errors before any LLM call | 49x faster error detection |

### Structural Errors Caught at Compile Time

- Tool execution without approval (INV node with no VERIFY dependency)
- Approval of a tool that was never called (VERIFY with no INV predecessor)
- Unreachable operations (`symbol-dce` detects dead nodes)

**Not yet implemented (requires prerequisite work):**

- Type mismatches between connected nodes -- no type annotations exist on edge tokens
- Missing required attributes on operations -- `validate.rs` does not check `OperationSpec.fields`

### Key Insight

Instead of a simple plan, Codex creates an **apxm-graph that gets compiled and analyzed**. The graph IS the plan, and the compiler IS the analysis engine.

The traditional Codex flow generates a plan, executes tools, and discovers errors at runtime -- after expensive LLM API calls have already been made. With APXM compilation, the graph structure is validated in milliseconds. The compiler can prove that every tool invocation has a corresponding approval step, that no operations produce unused output, and that all data dependencies are satisfiable.

---

## Deliverables

- `apxm compile codex-turn.apxm` produces optimized `.apxmobj`
- Measured reduction in API calls from `fuse-ask-ops` on real Codex workflows
- Measured compile-time error detection rate vs. runtime error detection
- `apxm decompile codex-turn.apxmobj` shows optimized graph structure

---

## Phase 5 Summary

| Step | Days | New Files | Modified Files | New Lines (est.) |
|------|------|-----------|----------------|-----------------|
| C12: Compile + optimize | 3+ | 1 | 1 | ~200 |
| **Total** | **3+** | **1** | **1** | **~200** |

---

## Related Files

- [README.md](README.md) -- Phase 5 overview
- [apxm.md](apxm.md) -- APXM compiler infrastructure (A8)
- [gemini-cli.md](gemini-cli.md) -- Gemini-CLI compiler integration (G13)
- [../plan-2-codex-changes.md](../plan-2-codex-changes.md) -- Full Codex integration plan
