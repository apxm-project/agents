# Phase 4: Agent Loop as Graph

**Draft v6 -- Revised with investigation findings**
**Timeline:** Weeks 21--28
**Dependencies:** Phases 2 and 3 complete (tools are capabilities, state is AAM)

---

## Goal

Express each consumer's imperative turn loop as a declarative AIS graph. Replace ~200 lines of interleaved concerns with typed, compiler-validated DAGs. Enable automatic parallelism from graph topology.

Instead of imperative functions that interleave termination checks, LLM calls, tool dispatch, confirmation, compression, and loop detection in one function, each concern becomes a separate, independently-optimizable node in a typed graph. The scheduler extracts concurrency automatically from the DAG structure -- no manual `FuturesOrdered` or task spawning.

**Universal graph execution.** Any agent framework can express its turn loop as an AIS graph. The graph is executed by APXM's dataflow scheduler, gaining automatic parallelism, typed tool dispatch, and inspectable execution traces -- without the framework implementing its own scheduler. This is what makes APXM a universal execution substrate: the same runtime, scheduler, and optimization infrastructure serves every framework that targets it.

---

## What Each Consumer Delivers

### APXM: Graph Execution Extensions ([apxm.md](apxm.md))

APXM ensures the runtime can execute graphs **submitted by external consumers** (Codex and Gemini-CLI), not just graphs compiled from internal sources. This includes:

- Strengthened graph validation for consumer-authored graphs (edge cases, hand-authored patterns)
- `POST /v1/execute` endpoint that accepts ApxmGraph JSON and streams `ApxmEvent` via SSE
- Service is `apxm-server` (graph execution, capability system, AAM management)

### Codex: `submission_loop()` as AIS Graph ([codex.md](codex.md))

Codex's core turn loop (`submission_loop()`) becomes an AIS graph where:

- Tool INV nodes run **concurrently** with zero developer effort (replacing manual `FuturesOrdered` + per-tool `RwLock`)
- Multi-agent coordination maps to FLOW_CALL and WAIT_ALL
- Context compaction becomes a visible QMEM + ASK + UMEM subgraph the compiler can optimize

### Gemini-CLI: `executeTurn()` as AIS Graph ([gemini-cli.md](gemini-cli.md))

Gemini-CLI's turn loop (`executeTurn()`) becomes a typed DAG where:

- 11 hook event types map to FENCE operations that enforce ordering structurally
- Recovery turn (`executeFinalWarningTurn()`) becomes a TRY_CATCH subgraph
- Sub-agent delegation becomes FLOW_CALL with AAM scope inheritance

---

## Phase 4 Validation Criteria

### APXM
- [ ] Consumer-authored AIS graphs validate and execute via `apxm execute`
- [ ] `POST /v1/execute` endpoint accepts ApxmGraph JSON and streams events
- [ ] Validation errors provide actionable messages for graph authors
- [ ] Runtime handles graphs with ASK, INV, BRANCH_ON_VALUE, WAIT_ALL, VERIFY, UMEM nodes

### Codex
- [ ] `submission_loop()` expressible as AIS graph with correct topology
- [ ] Parallel tool calls demonstrate automatic parallelism from DAG
- [ ] Multi-agent coordination works via FLOW_CALL and WAIT_ALL
- [ ] Context compaction works as QMEM + ASK + UMEM subgraph
- [ ] No behavioral regression vs imperative implementation

### Gemini-CLI
- [ ] `executeTurn()` expressible as AIS graph with correct topology
- [ ] Parallel tool calls demonstrate automatic parallelism from DAG
- [ ] Hook events fire at correct graph positions via FENCE
- [ ] Recovery turn works as TRY_CATCH subgraph
- [ ] Sub-agent delegation works as FLOW_CALL
- [ ] No behavioral regression vs imperative implementation

---

## Service Note: apxm-server Enters Here

**Phase 4 is where `apxm-server` enters the migration.** In Phases 1-3, each consumer embeds APXM as a library (Codex via Rust API, Gemini-CLI via NAPI module), with no server needed. Phase 4 introduces the server for graph execution because:

- The server compiles AND executes graphs — `POST /v1/execute` does graph validation, MLIR compilation (O1), and dataflow scheduling in one call
- SSE streaming (`POST /v1/execute/stream`) provides real-time execution events
- Multi-consumer coordination becomes possible (MCP + A2A protocols, agent registry)

**Pre-Phase 4 prerequisite:** Per-session AAM isolation must be wired into `apxm-server`. Currently all sessions share one AAM (`Arc<RwLock<AamState>>`). The fix: `build_context()` should use `get_or_create_session_aam(session_id)` with `child_scope(ScopeSpec::isolate())`. The scoping infrastructure exists but is not wired into the server. See [Sessions & Server Investigation](../SESSIONS-AND-SERVER-INVESTIGATION.md) and [Session → AAM Mapping Design](../SESSION-AAM-MAPPING.md).

The APXM service crate is `apxm-server`. Routes are defined inline in `apxm-server/src/main.rs`.

---

## Consumer Files

| Consumer | File | Key Steps |
|----------|------|-----------|
| APXM | [apxm.md](apxm.md) | A7: Graph validation, `/v1/execute` |
| Codex | [codex.md](codex.md) | C10: Turn graph, C11: Compaction graph |
| Gemini-CLI | [gemini-cli.md](gemini-cli.md) | G11: Turn graph, G12: Hooks + recovery + delegation |

---

## AIS Operations Verified

All 14 operations referenced in Phase 4 plans exist in the codebase with runtime handlers and tests:

| Operation | Wire Index | Runtime Handler |
|-----------|------------|-----------------|
| ASK | 1 | `handlers/llm.rs` (unified) |
| INV | 0 | `handlers/inv.rs` |
| BRANCH_ON_VALUE | 15 | `handlers/branch.rs` |
| WAIT_ALL | 5 | `handlers/wait_all.rs` |
| VERIFY | 11 | `handlers/verify.rs` |
| UMEM | 3 | `handlers/umem.rs` |
| QMEM | 2 | `handlers/qmem.rs` |
| FENCE | 7 | `handlers/fence.rs` |
| TRY_CATCH | 18 | `handlers/try_catch.rs` |
| FLOW_CALL | 21 | `handlers/flow_call.rs` |
| REFLECT | 10 | `handlers/reflect.rs` |
| GUARD | **None** | `handlers/guard.rs` |
| PLAN | 4 | `handlers/plan.rs` |
| MERGE | 6 | `handlers/merge.rs` |

> **Note:** GUARD currently has no wire index (cannot be serialized to `.apxmobj`). Wire index 26 must be assigned before Phase 5. Phase 4 graphs with GUARD work via the JSON execution path (`apxm execute`) but cannot be compiled.

---

## Phase 4 Prerequisites

The following must be resolved before or during Phase 4 implementation:

1. **BRANCH_ON_VALUE naming in consumer code** -- The AIS parser does NOT accept `"BRANCH"` as a valid operation name. Only `"BRANCH_ON_VALUE"` (or `"branch_on_value"`) is accepted. All graph pseudocode and adapter code must use the canonical name.

2. **GUARD wire index for Phase 5 transition** -- GUARD has no wire index, so graphs containing GUARD cannot be compiled to `.apxmobj`. This blocks the Phase 4 to Phase 5 transition for the Gemini turn graph (which uses GUARD as its first node). Wire index 26 must be assigned before Phase 5.

3. **Attribute validation gap** -- Graph validation (`apxm-graph/validate.rs`) does NOT check per-operation required attributes. It validates node IDs, edges, DAG acyclicity, parameters, and LLM provider names, but a graph with missing required attributes will pass validation and fail at runtime. Adding attribute validation (using `OperationSpec.fields`) is a prerequisite for Phase 5 and a known gap in Phase 4.

---

## Integration Tests

### Graph Construction Tests (7 tests)

| Test | Validates | Priority |
|------|-----------|----------|
| `test_codex_turn_graph_validates` | Codex-style turn graph (ASK -> BRANCH_ON_VALUE -> INV x3 -> WAIT_ALL -> VERIFY -> UMEM) validates | P0 |
| `test_gemini_turn_graph_validates` | Gemini-style turn graph (GUARD -> QMEM -> ASK -> BRANCH_ON_VALUE -> INV x3 -> WAIT_ALL -> VERIFY -> REFLECT) validates | P0 |
| `test_turn_graph_with_fence_validates` | FENCE nodes at hook positions in the Gemini turn graph | P0 |
| `test_turn_graph_cycle_rejected` | Back-edge added to turn graph is rejected | P1 |
| `test_turn_graph_dangling_edge_rejected` | Edge to non-existent node ID is rejected | P1 |
| `test_branch_fan_out_fan_in_topology` | BRANCH_ON_VALUE -> N parallel INVs -> WAIT_ALL topology is valid | P0 |
| `test_flow_call_subgraph_validates` | PLAN -> FLOW_CALL x2 -> WAIT_ALL -> MERGE validates | P1 |

### Graph Execution Tests (9 tests)

| Test | Validates | Priority |
|------|-----------|----------|
| `test_parallel_inv_execution` | 3 parallel INV nodes with no inter-edges all execute via dataflow scheduler | P0 |
| `test_wait_all_collects_parallel_results` | INV x3 -> WAIT_ALL, WAIT_ALL receives all 3 outputs | P0 |
| `test_branch_routes_correctly` | ASK -> BRANCH_ON_VALUE with two paths, correct path taken | P0 |
| `test_flow_call_executes_subdag` | FLOW_CALL with registered sub-DAG executes and result propagates | P0 |
| `test_fence_enforces_ordering` | FENCE between nodes enforces execution order | P1 |
| `test_guard_halts_on_failure` | GUARD with failing condition stops execution | P1 |
| `test_guard_skip_on_failure` | GUARD with `on_fail: "skip"` continues execution | P1 |
| `test_consumer_graph_via_execute_endpoint` | POST consumer-authored graph JSON to `/v1/execute` returns response | P0 |
| `test_consumer_graph_via_execute_stream` | POST to `/v1/execute/stream` returns SSE events | P1 |

### Multi-Agent Tests (4 tests)

| Test | Validates | Priority |
|------|-----------|----------|
| `test_multi_agent_fan_out_fan_in` | PLAN -> FLOW_CALL x2 -> WAIT_ALL, both sub-flows execute | P0 |
| `test_flow_call_scope_isolation` | Child scope beliefs do not leak to parent AAM | P0 |
| `test_flow_call_recursion_limit` | MAX_FLOW_CALL_DEPTH (100) is enforced | P1 |
| `test_flow_call_not_found_error` | FLOW_CALL to unregistered agent/flow returns error | P1 |

### Phase 4 Boundary Test

The key integration test for Phase 4 completion:

```
test_end_to_end_consumer_graph:
  1. Construct a turn-loop graph programmatically (ASK -> BRANCH_ON_VALUE -> INV x2 -> WAIT_ALL -> VERIFY -> UMEM)
  2. Serialize to JSON
  3. POST to /v1/execute
  4. Verify: graph validates, executes, all nodes complete, results returned
  5. Compare: graph-based execution produces same output as sequential execution of same operations
```

---

## Risk Matrix

### High

1. **Missing attribute validation** -- Consumer-authored graphs may contain structurally valid but semantically broken operations (missing required attributes). These will pass validation but fail at runtime, undermining Phase 4's goal of early error detection.

2. **GUARD wire index gap** -- Graphs using GUARD cannot be compiled to `.apxmobj`, blocking the Phase 4 to Phase 5 transition. The Gemini turn graph uses GUARD as its first node.

### Medium

3. **BRANCH naming** -- Consumer adapter code following the plan document would emit `"BRANCH"` which is not a valid operation name. Must use `"BRANCH_ON_VALUE"`. Easy to fix but must be caught before implementation begins.

4. **TRY_CATCH semantics** -- The recovery turn pattern (TRY_CATCH wrapping ASK + GUARD + INV) requires scheduler support for subgraph error boundaries. The current TRY_CATCH handler is a passthrough; the scheduler's error routing capability for this pattern needs verification.

### Low

5. **Service naming** -- Historical references to `apxm-llm-service` should read `apxm-server`. Cosmetic issue; no code impact.

6. **Route structure** -- Routes are defined inline in `apxm-server/src/main.rs`, not in separate route files. No functional impact.

---

## Source Plans

- [Plan 1: APXM Changes](../plan-1-apxm-changes.md) -- Phase A7
- [Plan 2: Codex Changes](../plan-2-codex-changes.md) -- Phase C10--C11
- [Plan 3: Gemini-CLI Changes](../plan-3-gemini-changes.md) -- Phase G11--G12
