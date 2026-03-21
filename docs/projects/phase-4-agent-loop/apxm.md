# Phase 4: APXM Changes -- Graph Execution Extensions

**Draft v6 -- Revised with investigation findings**
**Source:** [Plan 1: APXM Changes](../plan-1-apxm-changes.md), Phase A7
**Timeline:** ~2--3 weeks
**Phase overview:** [README.md](README.md)

---

## A7: Graph Execution Extensions

### Current State

The runtime already executes AIS graphs -- that is its primary function. The dataflow scheduler, token-counting readiness detection, and operation handlers all exist. What Phase 4 requires is ensuring the runtime can execute graphs **submitted by external consumers** (Codex and Gemini-CLI), not just graphs compiled from internal sources.

The dataflow scheduler (verified at `apxm-runtime/src/scheduler/dataflow.rs`) provides: token-based dataflow execution, automatic parallelism from DAG structure, cost budget enforcement, watchdog deadlock detection, and work stealing. These capabilities are available to all consumer-authored graphs without additional scheduler work.

### A7.1 Graph validation for consumer-authored graphs

Consumer-authored graphs may have errors that internally-compiled graphs would not. Strengthen validation:

**Already implemented** (verified in `apxm-graph/src/validate.rs`):
- Node ID uniqueness enforcement
- Edge target validation (no dangling references)
- DAG cycle detection (Kahn's algorithm)
- Parameter name/type validation
- Provider validation for LLM nodes

**Not yet implemented (known gap):**
- Required attributes per operation type -- `validate.rs` does NOT call `get_operation_spec()` for attribute validation. Each `OperationSpec` has a `fields` array with `required` flags, but validation only uses `get_operation_spec()` to check `category.requires_llm()` for provider validation. Consumer-authored graphs with missing required attributes will pass validation but fail at runtime. This is a prerequisite for Phase 5.
- Type compatibility checks on Data edges

### A7.2 Graph execution endpoint

The `apxm-server` crate already provides this endpoint (verified at `apxm-server/src/main.rs`, line 616). Routes are defined inline in `main.rs`:

```
POST /v1/execute          -- accepts ApxmGraph JSON, returns ExecuteResponse
POST /v1/execute/stream   -- accepts ApxmGraph JSON, streams ApxmEvent via SSE
```

Request body:

```json
{
  "graph": { ... },        // ApxmGraph JSON
  "parameters": { ... },   // Graph parameter values
  "workspace_id": "..."    // Optional workspace for AAM state
}
```

Response: `ExecuteResponse` with results and stats, or SSE stream of `ApxmEvent`.

### A7.3 `apxm execute` CLI for consumer graphs

Already exists -- ensure it works with consumer-authored graphs by validating input and providing clear error messages.

### A7.4 Deliverables

| Deliverable | Location | Status |
|-------------|----------|--------|
| Strengthened graph validation | `apxm-graph/src/validate.rs` (modify) | Partial -- attribute validation gap |
| `POST /v1/execute` endpoint | `apxm-server/src/main.rs` (inline routes) | Already exists |
| `POST /v1/execute/stream` endpoint | `apxm-server/src/main.rs` (inline routes) | Already exists |

> **Note:** GUARD currently has no wire index (cannot be serialized to `.apxmobj`). Wire index 26 must be assigned before Phase 5. Phase 4 graphs with GUARD work via the JSON execution path (`apxm execute`) but cannot be compiled.

### A7.5 Acceptance criteria

- [ ] Consumer-authored AIS graphs validate and execute via `apxm execute`
- [ ] `POST /v1/execute` endpoint accepts ApxmGraph JSON and streams events
- [ ] Validation errors provide actionable messages for graph authors
- [ ] Runtime handles graphs with ASK, INV, BRANCH_ON_VALUE, WAIT_ALL, VERIFY, UMEM nodes

---

## Cross-References

- **Codex consumer:** [codex.md](codex.md) -- C10 submits turn graphs to this endpoint
- **Gemini-CLI consumer:** [gemini-cli.md](gemini-cli.md) -- G11 submits turn graphs to this endpoint
- **Phase overview:** [README.md](README.md)
