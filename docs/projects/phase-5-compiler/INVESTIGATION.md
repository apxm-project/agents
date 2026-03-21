# Phase 5 Investigation Report: Compiler Integration

**Date:** 2026-03-20
**Scope:** Cross-reference Phase 5 plan claims against APXM source code; identify integration test points.

---

## 1. Verified Claims

### 1.1 Compiler Pass System

**Plan claims:** "All four passes already exist in the MLIR-based compiler."

**Verified in `apxm-compiler/src/passes/pipeline.rs`:**

The compiler pipeline at O2 includes 12 passes in this order:

1. `normalize`
2. `build-prompt`
3. `template-specialization`
4. `unconsumed-value-warning`
5. `schema-narrowing`
6. `scheduling`
7. `fuse-ask-ops`
8. `condense-ops`
9. `dead-context-elimination`
10. `canonicalizer`
11. `cse`
12. `symbol-dce`

The four passes referenced in the plan all exist:

| Plan Name | Actual Pass Name | Present? |
|-----------|-----------------|----------|
| FuseAskOps | `fuse-ask-ops` | Yes |
| CSE | `cse` | Yes |
| DCE | `symbol-dce` | Yes (named `symbol-dce`, not just `dce`) |
| Canonicalization | `canonicalizer` | Yes |

**Additional passes not mentioned in the plan:**
- `normalize` -- normalization pass
- `build-prompt` -- prompt construction
- `template-specialization` -- template specialization (O2+)
- `unconsumed-value-warning` -- warning for unused values
- `schema-narrowing` -- schema narrowing (O2+)
- `scheduling` -- scheduling pass
- `condense-ops` -- operation condensation (O2+)
- `dead-context-elimination` -- dead context elimination (O2+)

**Key finding:** The plan understates the compiler's capability. There are 12 unique passes, not 4. The plan documents only reference the 4 most consumer-visible ones.

### 1.2 Optimization Levels

**Verified in `apxm-compiler/src/passes/pipeline.rs`:**

| Level | Passes | Behavior |
|-------|--------|----------|
| O0 | 0 | No optimization (passthrough) |
| O1 | 8 | Basic: normalize, build-prompt, scheduling, fusion, canonicalization, CSE, DCE |
| O2 | 12 | O1 + template-specialization, dead-context-elimination, schema-narrowing, condense-ops |
| O3 | 93 | O2 passes iterated 10 times for fixed-point convergence |

The plan references `apxm compile graph.json -o out.apxmobj -O2` which is correct and uses the standard O2 pipeline.

### 1.3 PassManager Architecture

**Verified in `apxm-compiler/src/passes/manager.rs`:**

- `PassManager` wraps a C++ MLIR pass manager via FFI (`ffi::ApxmPassManager`)
- Named convenience methods for each pass: `fuse_ask_ops()`, `cse()`, `symbol_dce()`, `canonicalizer()`, etc.
- `run_with_metrics()` method exists: runs passes individually, collecting per-pass timing and op-count deltas
- Returns `PipelineDiagnostics` with `initial_ops`, `final_ops`, per-pass `PassMetrics` (duration_ms, ops_before, ops_after, ops_delta), and `total_duration_ms`
- Supports `--emit-diagnostics` for machine-readable optimization reports

### 1.4 Pass Registry (FFI Bridge)

**Verified in `apxm-compiler/src/passes/registry.rs`:**

- `list_passes()` returns all registered passes from the C++ MLIR backend
- `find_pass(name)` looks up a pass by name
- `get_pass_count()` and `get_pass_info(index)` for enumeration
- All registry calls go through FFI to the MLIR C++ side

### 1.5 Artifact Format (.apxmobj)

**Verified in `apxm-compiler/src/codegen/artifact.rs`:**

- Wire format version: 3 (`WIRE_VERSION = 3`)
- Binary format: version (u32) + DAG count (u64) + per-DAG data
- Per-DAG: module name (string) + is_entry (bool) + parameters + nodes + edges + entry_nodes + exit_nodes
- Per-node: id (u64) + op_index (u32, via `from_wire_index`/`to_wire_index`) + attributes + input_tokens + output_tokens + priority (u32) + estimated_latency (optional u64)
- Supports multi-DAG artifacts (flow calls produce multiple DAGs in one artifact)
- Pure-Rust emitter (`emit_wire_dags`) and parser (`parse_wire_dags`) with round-trip tests
- Deterministic byte output (attributes sorted by key)

### 1.6 Graph-Level Optimization Passes (Pre-MLIR)

**Verified in `apxm-graph/src/optimize.rs`:**

Two graph-level passes run BEFORE MLIR lowering:

1. **`prompt_caching()`** -- detects shared system prompts across ASK/THINK nodes; marks duplicates with `cached_system_prompt: true`
2. **`memoization_hints()`** -- detects duplicate pure operations (QMEM, CONST_STR, VERIFY) with identical attributes; marks with `memoizable: true`

These are not mentioned in the Phase 5 plan but are relevant -- they provide pre-MLIR optimization hints.

### 1.7 MLIR Lowering

**Verified in `apxm-graph/src/lower_mlir.rs`:**

- `lower_to_mlir(graph)` converts `ApxmGraph` to MLIR text representation
- Validates graph before lowering
- Produces AIS dialect MLIR IR

### 1.8 ExecutionDag Lowering

**Verified in `apxm-graph/src/lower_dag.rs`:**

- `lower_to_execution_dag(graph)` converts `ApxmGraph` to `ExecutionDag` (runtime format)
- Validates graph before lowering
- Generates token IDs for edges
- Computes entry_nodes (no input tokens) and exit_nodes (no output tokens)
- Populates `DagMetadata` from graph metadata (name, parameters)

### 1.9 Executor Engine

**Verified in `apxm-runtime/src/executor/engine.rs`:**

- `execute_dag(dag)` is the main entry point
- Tries parallel execution via `DataflowScheduler` first (for DAGs with >1 node)
- Falls back to sequential execution on scheduler failure
- Returns `ExecutionResult` with `results: HashMap<u64, Value>` and `stats: ExecutionStats`

### 1.10 Dispatcher Coverage

**Verified in `apxm-runtime/src/executor/dispatcher.rs`:**

- All 39 operations are dispatched (verified by `all_operations_covered` test which asserts count == 39)
- Every `AISOperationType` variant has a match arm
- Agent and Yield are no-ops (return `Value::Null`)
- Emits `OperationStart`/`OperationEnd` events
- Records episodes in episodic memory
- Checks cancellation token before work

---

## 2. Discrepancies Found

### 2.1 CRITICAL: Operations Without Wire Indices Cannot Be Compiled

**Plan claims:** "Compile consumer-authored AIS graphs to optimized `.apxmobj` artifacts."

**Actual state:** Seven operations have no wire index and CANNOT be serialized to `.apxmobj`:

| Operation | Wire Index | Used in Phase 4 Plans? |
|-----------|-----------|----------------------|
| Agent | None | No (metadata, not in turn graphs) |
| UpdateGoal | None | No |
| **Guard** | **None** | **Yes (Gemini turn graph, first node)** |
| Claim | None | No |
| Pause | None | No |
| Resume | None | No |
| Yield | None | No (internal) |

The `to_wire_index()` method returns `None` for these operations, and `write_node()` in `artifact.rs` calls `node.op_type.to_wire_index().ok_or_else(...)` which would produce an error.

**Impact:** The Gemini-CLI turn graph (which starts with GUARD) cannot be compiled to `.apxmobj`. Wire indices 25-30 are reserved for Phase 1 ISA extensions but currently unmapped. GUARD, UpdateGoal, Claim, Pause, and Resume need wire indices assigned to these reserved slots.

**Recommendation:** Assign wire indices before Phase 5:
- 25: UpdateGoal
- 26: Guard
- 27: Claim
- 28: Pause
- 29: Resume

### 2.2 MODERATE: "FuseAskOps" Name Mismatch

**Plan claims:** "FuseAskOps pass"

**Actual pass name:** `fuse-ask-ops` (kebab-case, as used in `pipeline.rs` and `PassManager::fuse_ask_ops()`)

This is a cosmetic discrepancy -- the plan uses PascalCase while the code uses kebab-case. The pass exists and works. However, anyone calling `PassManager::add_pass("FuseAskOps")` would get an error; the correct name is `"fuse-ask-ops"`.

### 2.3 MODERATE: "DCE" Name Mismatch

**Plan claims:** "DCE (Dead Code Elimination)"

**Actual pass name:** `symbol-dce` (not `dce`)

The plan references "DCE" but the actual pass is `symbol-dce`. Additionally, there is a separate `dead-context-elimination` pass at O2+ that is not mentioned in the plan.

### 2.4 MODERATE: Missing Required Attribute Validation at Compile Time

**Plan claims (A8.3):** "Missing required attributes on AIS operations" caught at compile time.

**Actual state:** The graph validator (`apxm-graph/src/validate.rs`) does NOT validate required attributes per operation type. It validates structural properties (IDs, edges, acyclicity, parameters, providers) but not per-operation attribute requirements. The MLIR compiler may catch some attribute errors during lowering, but this is not a systematic check.

Each `OperationSpec` in `definitions.rs` has a `fields` array with `required: true/false` flags, but `validate.rs` does not use them.

**Impact:** "Missing required attributes on AIS operations" errors may not be caught at compile time as the plan promises. They would surface at runtime when the handler calls `get_string_attribute()` or similar.

### 2.5 LOW: "Type Mismatches on Data Edges" Not Implemented

**Plan claims (A8.3):** "Type mismatches on data edges" caught at compile time.

**Actual state:** The graph validator does not check type compatibility on edges. Data edges connect node IDs, but there is no type annotation on edge tokens that could be checked. The MLIR lowering may introduce type checking at the MLIR level, but the graph-level validator does not.

### 2.6 LOW: "Capability References to Unregistered Tools" Not Checked at Validation

**Plan claims (A8.3):** "Capability references to unregistered tools" caught at compile time.

**Actual state:** The graph validator does not check whether INV nodes reference registered capabilities. Capability resolution happens at runtime via the CapabilitySystem. Compile-time checking would require access to the tool registry at compile time, which is not currently wired up.

### 2.7 LOW: Compiler Pass Count Understated

**Plan claims:** "All four passes already exist"

**Actual state:** There are at least 12 distinct passes, not 4. The plan only highlights the 4 most consumer-relevant passes (fuse-ask-ops, cse, symbol-dce, canonicalizer) but omits 8 others (normalize, build-prompt, template-specialization, unconsumed-value-warning, schema-narrowing, scheduling, condense-ops, dead-context-elimination).

This is a documentation gap rather than a code problem. The plan is accurate about what it states; it just omits additional capability.

### 2.8 LOW: `apxm analyze` Command

**Plan claims:** `apxm analyze graph.json --json` provides parallelism and optimization report.

**Observation:** The `apxm analyze` command is referenced in CLAUDE.md as an existing CLI command. Its implementation was not investigated in detail, but the MCP server (`apxm_mcp.rs`) contains analysis logic that identifies parallel phases and provides speedup estimates.

---

## 3. Integration Test Recommendations for Phase 5

### 3.1 Compilation Tests

**Location:** Integration test crate or `apxm-compiler/tests/`

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_codex_turn_graph_compiles` | Construct Codex turn graph, compile at O2, verify `.apxmobj` produced | P0 |
| `test_gemini_turn_graph_compiles` | Construct Gemini turn graph, compile at O2, verify `.apxmobj` produced | P0 |
| `test_all_optimization_levels` | Compile same graph at O0, O1, O2, O3, verify all produce valid artifacts | P0 |
| `test_compile_with_emit_diagnostics` | Compile with diagnostics, verify PipelineDiagnostics JSON output | P1 |
| `test_compile_empty_graph_error` | Compile empty graph, verify clean error | P1 |
| `test_compile_invalid_graph_error` | Compile graph with cycle, verify clean error | P1 |
| `test_compile_missing_attributes_error` | Compile graph with missing required attributes (once validation added), verify error | P1 |

### 3.2 Optimization Tests

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_fuse_ask_ops_reduces_operations` | Graph with ASK -> ASK chain; after fuse-ask-ops, verify ops_after < ops_before | P0 |
| `test_cse_eliminates_duplicates` | Graph with two identical ASK nodes; after CSE, verify one eliminated | P0 |
| `test_symbol_dce_removes_dead_code` | Graph with unreachable node; after symbol-dce, verify node removed | P0 |
| `test_canonicalizer_normalizes` | Graph with non-canonical patterns; after canonicalizer, verify normalized | P1 |
| `test_o2_vs_o0_fewer_ops` | Same graph at O2 and O0; verify O2 has fewer operations | P0 |
| `test_pass_metrics_reported` | Run with `run_with_metrics()`; verify per-pass timing and delta data | P1 |

### 3.3 Round-Trip Tests

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_compile_decompile_roundtrip` | Compile graph to `.apxmobj`, parse artifact back, verify node/edge structure matches | P0 |
| `test_artifact_roundtrip_all_op_types` | Artifact with one node per wire-indexed op type; emit + parse round-trip (already exists in `artifact.rs` tests) | P0 |
| `test_multi_dag_artifact_roundtrip` | Artifact with FLOW_CALL producing 2+ DAGs; verify all DAGs survive round-trip | P1 |
| `test_compiled_execution_matches_uncompiled` | Execute graph directly, then compile and execute artifact; compare results | P0 |

### 3.4 Performance Tests

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_compile_time_under_threshold` | Compile a 50-node consumer graph; verify compile time < 1 second | P1 |
| `test_fuse_ask_ops_api_call_reduction` | Graph with 4 sequential ASK nodes; verify fuse-ask-ops reduces to fewer API calls | P0 |
| `test_parallelism_extraction_speedup` | Graph with fan-out/fan-in pattern; verify scheduler reports parallelism > 1 | P1 |
| `test_compile_time_error_detection_faster_than_runtime` | Inject a structural error; compare compile-time detection speed vs runtime detection | P2 |

### 3.5 Phase 5 Boundary Tests

**The key integration test for Phase 5 completion:**

```
test_end_to_end_compile_execute:
  1. Construct a consumer-style turn graph (ASK -> BRANCH_ON_VALUE -> INV x3 -> WAIT_ALL -> VERIFY -> UMEM)
  2. Compile at O2: `apxm compile graph.json -o turn.apxmobj -O2`
  3. Verify artifact produced with optimization report
  4. Execute artifact: `apxm run turn.apxmobj`
  5. Compare: compiled execution produces same results as uncompiled execution
  6. Verify: fuse-ask-ops metrics show reduction (if applicable)
  7. Verify: compile-time validation caught a planted structural error
```

```
test_consumer_graph_compile_stream:
  1. POST consumer graph JSON to compile endpoint (if added per A8.4)
  2. Verify artifact bytes returned
  3. Execute artifact via /v1/execute
  4. Verify results match direct graph execution
```

---

## 4. Risk Assessment

### High Risk

1. **GUARD wire index gap** (Section 2.1): The Gemini-CLI turn graph uses GUARD as its first node, but GUARD has no wire index. Any attempt to compile this graph to `.apxmobj` will fail. This is a hard blocker for Phase 5's Gemini-CLI integration. Wire indices 25-30 are reserved but unmapped.

2. **Missing attribute validation** (Section 2.4): The plan promises compile-time validation of "missing required attributes on AIS operations," but neither the graph validator nor the MLIR lowering path systematically checks required attributes from `OperationSpec.fields`. This undermines the "49x faster error detection" claim for attribute errors specifically.

### Medium Risk

3. **TRY_CATCH compilation path** (related to Phase 4 Section 2.6): If the Gemini-CLI recovery turn uses TRY_CATCH, the compilation path must handle subgraph boundaries within the MLIR representation. The current handler is a passthrough; the MLIR dialect's representation of TRY_CATCH regions needs verification.

4. **Consumer graph patterns vs. pass assumptions**: The plan states "Phase A8 ensures [passes] work on the graph patterns consumers produce." This is the correct approach but requires actual testing with representative consumer graphs. The existing pass test suite uses APXM-internal graph patterns, not consumer-style patterns (fan-out/fan-in tool invocation, hook fences, recovery subgraphs).

### Low Risk

5. **Pass name mismatches** (Sections 2.2, 2.3): Cosmetic -- the passes exist under slightly different names. No code impact, but plan documents should be updated for accuracy.

6. **Additional passes beyond plan scope** (Section 2.7): The compiler has more passes than documented. This is a feature, not a risk, but consumers should be aware of the full pipeline.

---

## 5. Summary of Pre-Phase-5 Prerequisites

Before Phase 5 can begin, the following must be resolved:

| Item | Severity | Effort | Description |
|------|----------|--------|-------------|
| Assign GUARD wire index | High | Small | Map wire index 26 to Guard in `from_wire_index`/`to_wire_index` |
| Assign other Phase 1 ISA wire indices | Medium | Small | Map UpdateGoal (25), Claim (27), Pause (28), Resume (29) |
| Add required attribute validation | Medium | Medium | Use `OperationSpec.fields` in `validate.rs` to check required attributes |
| Fix BRANCH vs BRANCH_ON_VALUE in plan docs | Medium | Trivial | Update plan pseudocode to use correct operation name |
| Verify TRY_CATCH MLIR representation | Medium | Medium | Ensure MLIR dialect supports subgraph error boundaries |
| Update plan references to `apxm-server` | Low | Trivial | Replace `apxm-llm-service` references |
