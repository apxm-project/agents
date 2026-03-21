# Phase 4 Investigation Report: Agent Loop as Graph

**Date:** 2026-03-20
**Scope:** Cross-reference Phase 4 plan claims against APXM source code; identify integration test points.

---

## 1. Verified Claims

### 1.1 AIS Operations Referenced in Phase 4 Plans

The plans reference the following operations: ASK, INV, BRANCH, WAIT_ALL, VERIFY, UMEM, QMEM, FENCE, TRY_CATCH, FLOW_CALL, REFLECT, GUARD, PLAN, MERGE.

**Verification against `apxm-ais/src/operations/definitions.rs`:**

| Operation | Exists? | Wire Index | Runtime Handler |
|-----------|---------|------------|-----------------|
| ASK | Yes | 1 | `handlers/llm.rs` (unified) |
| INV | Yes | 0 | `handlers/inv.rs` |
| BRANCH_ON_VALUE | Yes (see discrepancy below) | 15 | `handlers/branch.rs` |
| WAIT_ALL | Yes | 5 | `handlers/wait_all.rs` |
| VERIFY | Yes | 11 | `handlers/verify.rs` |
| UMEM | Yes | 3 | `handlers/umem.rs` |
| QMEM | Yes | 2 | `handlers/qmem.rs` |
| FENCE | Yes | 7 | `handlers/fence.rs` |
| TRY_CATCH | Yes | 18 | `handlers/try_catch.rs` |
| FLOW_CALL | Yes | 21 | `handlers/flow_call.rs` |
| REFLECT | Yes | 10 | `handlers/reflect.rs` |
| GUARD | Yes | **None** (no wire index) | `handlers/guard.rs` |
| PLAN | Yes | 4 | `handlers/plan.rs` |
| MERGE | Yes | 6 | `handlers/merge.rs` |

All 14 operations referenced in Phase 4 plans exist in the codebase with runtime handlers.

### 1.2 Total Operation Count

The plans reference "39 AIS ops" (from CLAUDE.md). The source confirms 39 operations: 1 metadata (Agent) + 36 public + 2 internal (ConstStr, Yield). Of these, 32 have wire indices for artifact serialization; 7 do not (Agent, UpdateGoal, Guard, Claim, Pause, Resume, Yield).

**Verified:** 39 operations is correct.

### 1.3 Graph System (ApxmGraph)

**Verified in `apxm-graph/src/lib.rs`:**
- `ApxmGraph` struct with `name`, `nodes`, `edges`, `parameters`, `metadata` -- matches the JSON contract
- `GraphNode` has `id: u64`, `name: String`, `op: AISOperationType`, `attributes: HashMap<String, Value>`
- `GraphEdge` has `from: u64`, `to: u64`, `dependency: DependencyType`
- `DependencyType` supports `Data`, `Control`, `Effect`
- `from_json()`, `to_json()`, `validate()`, `to_execution_dag()`, `to_mlir()` methods exist
- `ApxmGraph::merge()` exists and works correctly (merges sub-graphs with ID remapping + WAIT_ALL sync node)
- Tests cover: roundtrip serialization, duplicate ID rejection, merge with ID remapping, merge parameter deduplication

### 1.4 Graph Validation

**Verified in `apxm-graph/src/validate.rs`:**
- Node ID uniqueness: Yes
- Edge target validation (no dangling references): Yes
- DAG cycle detection (Kahn's algorithm): Yes
- Parameter validation (names, types, uniqueness): Yes
- Provider validation for LLM nodes: Yes

### 1.5 Dataflow Scheduler

**Verified in `apxm-runtime/src/scheduler/dataflow.rs`:**
- Token-based dataflow execution exists
- Operations execute when all input tokens are ready
- Automatic parallelism from DAG structure
- `DataflowScheduler::execute()` returns results, stats, and scheduler metrics
- Cost budget enforcement, latency overrides, goal priority projection
- Watchdog for deadlock detection
- Worker thread spawning with work stealing (`scheduler/work_stealing.rs`)

### 1.6 FLOW_CALL Handler

**Verified in `apxm-runtime/src/executor/handlers/flow_call.rs`:**
- Looks up target flow in FlowRegistry by agent_name + flow_name
- Creates a child execution context with scope isolation (ScopeSpec::snapshot_all())
- Executes sub-flow DAG via ExecutorEngine
- Maximum recursion depth (100) enforced
- Records flow call in AAM beliefs and episodic memory
- Returns result from sub-flow's exit nodes
- Tests cover: registered flow execution, scope isolation, not-found error, recursion limit

### 1.7 WAIT_ALL Handler

**Verified in `apxm-runtime/src/executor/handlers/wait_all.rs`:**
- Passes through all inputs as a `Value::Array`
- Actual synchronization is handled by the scheduler (not the handler)
- Simple and correct implementation

### 1.8 TRY_CATCH Handler

**Verified in `apxm-runtime/src/executor/handlers/try_catch.rs`:**
- Passes through first input or returns `Value::Null`
- Actual exception handling is delegated to the scheduler
- The handler is a thin passthrough; scheduler manages error routing

### 1.9 FENCE Handler

**Verified in `apxm-runtime/src/executor/handlers/fence.rs`:**
- Ensures ordering -- passes through first input or `Value::Null`
- Ordering enforcement comes from the DAG edge structure, not the handler itself

### 1.10 GUARD Handler

**Verified in `apxm-runtime/src/executor/handlers/guard.rs`:**
- Evaluates condition expressions against input values
- Supports numeric comparisons (`>`, `>=`, `<`, `<=`, `==`, `!=`), keyword conditions (`not_empty`, `is_null`, `not_null`, etc.)
- `on_fail` attribute: `"halt"` (default, returns error) or `"skip"` (passes `false`)
- Records guard evaluation in AAM beliefs

### 1.11 POST /v1/execute Endpoint

**Verified in `apxm-server/src/main.rs`:**
- `POST /v1/execute` endpoint exists (line 616)
- `POST /v1/execute/stream` for SSE streaming also exists (line 617)
- Accepts graph JSON, converts to artifact, executes via runtime
- Returns `ExecuteResponse` with results and stats

### 1.12 Service Naming

**Verified:** The crate is already named `apxm-server` (Cargo.toml line 2: `name = "apxm-server"`). The rename from `apxm-llm-service` is already complete.

---

## 2. Discrepancies Found

### 2.1 CRITICAL: "BRANCH" vs "BRANCH_ON_VALUE"

**Plan claims (gemini-cli.md line 98):**
```typescript
{ id: 4, name: "branch", op: "BRANCH", attributes: { on: "has_tool_calls" } }
```

**Actual operation name:** `BRANCH_ON_VALUE` (serde serialized as `"BRANCH_ON_VALUE"`)

There is no `"BRANCH"` operation in the AIS enum. The `from_str()` parser on `AISOperationType` maps `"branch_on_value"` to `BranchOnValue` but does NOT accept `"branch"` as a valid name. A consumer graph using `"BRANCH"` as the op name would fail deserialization.

**Impact:** Any graph JSON using `"BRANCH"` will fail to parse. The plan's pseudocode example is incorrect and would not work if implemented literally.

**Recommendation:** Either (a) update the plan to use `"BRANCH_ON_VALUE"`, or (b) add `"branch"` as a serde alias for `BranchOnValue` in the `from_str()` parser.

### 2.2 MODERATE: Graph Validation Does Not Check Required Attributes

**Plan claims (apxm.md, A7.1):**
> "Required attributes per operation type (uses `get_operation_spec()` from `apxm-ais`)"

**Actual state:** `validate.rs` does NOT call `get_operation_spec()` for attribute validation. It validates:
- Node ID uniqueness
- Edge references
- DAG acyclicity
- Parameter names/types
- Provider names for LLM nodes

It does NOT validate required attributes per operation type. The `get_operation_spec()` function exists in `apxm-ais` and each `OperationSpec` has a `fields` array, but `validate.rs` only uses `get_operation_spec()` to check `category.requires_llm()` for provider validation.

**Impact:** Consumer-authored graphs with missing required attributes will pass validation but fail at runtime. Phase 4's goal of catching errors at validation time is partially unmet.

**Recommendation:** Add attribute validation to `validate.rs` before Phase 4. Each `OperationSpec.fields` entry has a `required` flag that can be checked.

### 2.3 MODERATE: Service Rename Already Complete

**Plan claims (apxm.md, A7.2):** "Service rename: `apxm-llm-service` -> `apxm-service`"

**Actual state:** The crate is already named `apxm-server` (not `apxm-service` -- note the different name). The rename is already done, but to a different name than the plan specifies.

**Impact:** Low. The rename is complete; the plan just references the old name. Plan documents referencing `apxm-llm-service/src/routes/execute.rs` need updating to `apxm-server/src/main.rs` (routes are defined inline, not in separate files).

### 2.4 LOW: GUARD Has No Wire Index

**Plan claims (gemini-cli.md):** GUARD is used as the first node in the turn graph.

**Actual state:** GUARD exists as an `AISOperationType` variant with a runtime handler, but it has no wire index (`to_wire_index()` returns `None`). This means:
- GUARD nodes can be used in JSON graphs (runtime interprets them)
- GUARD nodes CANNOT be serialized into `.apxmobj` artifacts (no wire index for binary format)
- Compiling a graph with GUARD through the MLIR pipeline will fail at artifact emission

**Impact:** Graphs containing GUARD can be executed via `apxm execute` (JSON path), but cannot be compiled to `.apxmobj` via `apxm compile`. This blocks Phase 5 for any graph using GUARD.

**Recommendation:** Assign wire indices 25-30 (currently reserved for Phase 1 ISA) to UpdateGoal, Guard, Claim, Pause, and Resume before Phase 5.

### 2.5 LOW: Codex Plan References `codex.rs:4138`

**Plan claims (codex.md):** "`submission_loop()` at `codex.rs:4138`"

**Observation:** This line reference cannot be verified against the APXM codebase as Codex is an external consumer. This is a cross-reference to the Codex codebase, not an APXM source claim.

### 2.6 LOW: TRY_CATCH Is a Thin Passthrough

**Plan claims (gemini-cli.md):** "Recovery turn becomes a TRY_CATCH subgraph" with specific semantics (timeout, grace period, forced completion).

**Actual state:** The TRY_CATCH handler is a thin passthrough (`inputs.first().cloned().unwrap_or(Value::Null)`). It comments: "actual exception handling done by scheduler." The scheduler does have error handling infrastructure, but the TRY_CATCH subgraph semantics described in the plan (ASK + GUARD + INV within a try boundary) would require the scheduler to understand subgraph boundaries and route errors to catch handlers.

**Impact:** The plan's TRY_CATCH semantics may not be fully supported by the current scheduler. The handler itself is trivial; the complex behavior must be in the scheduler's error routing, which needs verification.

---

## 3. Integration Test Recommendations for Phase 4

### 3.1 Graph Construction Tests

**Location:** `apxm-graph/src/` (new test module or existing tests in `lib.rs`)

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_codex_turn_graph_validates` | Construct a Codex-style turn graph (ASK -> BRANCH_ON_VALUE -> INV x3 -> WAIT_ALL -> VERIFY -> UMEM) and call `validate()` | P0 |
| `test_gemini_turn_graph_validates` | Construct a Gemini-style turn graph (GUARD -> QMEM -> ASK -> BRANCH_ON_VALUE -> INV x3 -> WAIT_ALL -> VERIFY -> REFLECT) and call `validate()` | P0 |
| `test_turn_graph_with_fence_validates` | Add FENCE nodes at hook positions in the Gemini turn graph | P0 |
| `test_turn_graph_cycle_rejected` | Add a back-edge to a turn graph and verify cycle rejection | P1 |
| `test_turn_graph_dangling_edge_rejected` | Add an edge to a non-existent node ID | P1 |
| `test_branch_fan_out_fan_in_topology` | Verify BRANCH_ON_VALUE -> N parallel INVs -> WAIT_ALL topology is valid | P0 |
| `test_flow_call_subgraph_validates` | PLAN -> FLOW_CALL x2 -> WAIT_ALL -> MERGE | P1 |

### 3.2 Graph Execution Tests

**Location:** New integration test crate or `apxm-runtime/tests/`

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_parallel_inv_execution` | Create a graph with 3 parallel INV nodes (no inter-edges), execute, and verify all 3 ran via the dataflow scheduler | P0 |
| `test_wait_all_collects_parallel_results` | INV x3 -> WAIT_ALL, verify WAIT_ALL receives all 3 outputs | P0 |
| `test_branch_routes_correctly` | ASK -> BRANCH_ON_VALUE with two paths, verify correct path taken | P0 |
| `test_flow_call_executes_subdag` | FLOW_CALL node with a registered sub-DAG, verify sub-DAG executes and result propagates | P0 |
| `test_fence_enforces_ordering` | Insert FENCE between nodes and verify execution order | P1 |
| `test_guard_halts_on_failure` | GUARD with failing condition, verify execution stops | P1 |
| `test_guard_skip_on_failure` | GUARD with `on_fail: "skip"`, verify execution continues | P1 |
| `test_consumer_graph_via_execute_endpoint` | POST a consumer-authored graph JSON to `/v1/execute` and verify response | P0 |
| `test_consumer_graph_via_execute_stream` | POST to `/v1/execute/stream` and verify SSE events | P1 |

### 3.3 Multi-Agent Tests (FLOW_CALL/WAIT_ALL)

| Test | What It Validates | Priority |
|------|------------------|----------|
| `test_multi_agent_fan_out_fan_in` | PLAN -> FLOW_CALL x2 -> WAIT_ALL, verify both sub-flows execute | P0 |
| `test_flow_call_scope_isolation` | Verify child scope beliefs do not leak to parent AAM | P0 |
| `test_flow_call_recursion_limit` | Verify MAX_FLOW_CALL_DEPTH (100) is enforced | P1 |
| `test_flow_call_not_found_error` | FLOW_CALL to unregistered agent/flow, verify error message | P1 |

### 3.4 Phase 4 Boundary Test

**The key integration test for Phase 4 completion:**

```
test_end_to_end_consumer_graph:
  1. Construct a turn-loop graph programmatically (ASK -> BRANCH -> INV x2 -> WAIT_ALL -> VERIFY -> UMEM)
  2. Serialize to JSON
  3. POST to /v1/execute
  4. Verify: graph validates, executes, all nodes complete, results returned
  5. Compare: graph-based execution produces same output as sequential execution of same operations
```

---

## 4. Risk Assessment

### High Risk

1. **Missing attribute validation** (Section 2.2): Consumer-authored graphs may contain structurally valid but semantically broken operations (missing required attributes). These will pass validation but fail at runtime, undermining Phase 4's goal of early error detection.

2. **GUARD wire index gap** (Section 2.4): Graphs using GUARD cannot be compiled to `.apxmobj`, blocking the Phase 4 -> Phase 5 transition. The Gemini turn graph specifically uses GUARD as its first node.

### Medium Risk

3. **BRANCH vs BRANCH_ON_VALUE naming** (Section 2.1): Consumer adapter code that follows the plan document will emit invalid operation names. This is easy to fix but must be caught before implementation begins.

4. **TRY_CATCH subgraph semantics** (Section 2.6): The recovery turn pattern described in the plan (TRY_CATCH wrapping ASK + GUARD + INV) requires scheduler support for subgraph error boundaries. The current TRY_CATCH handler is a passthrough; the scheduler's error routing capability for this pattern is unverified.

### Low Risk

5. **Service naming inconsistency** (Section 2.3): Documentation references `apxm-llm-service` but the crate is `apxm-server`. Cosmetic issue; no code impact.

6. **Route structure** (Section 2.3): Plan references `apxm-llm-service/src/routes/execute.rs` but routes are defined inline in `apxm-server/src/main.rs`. Low impact -- just a file path difference.
