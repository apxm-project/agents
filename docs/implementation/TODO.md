# A-PXM Implementation TODOs (Master)

Cross-cutting gap analysis between documentation/specification and actual codebase.
Derived from `docs/pxm/ais.md`, `docs/pxm/aam.md`, and source code inspection.

Domain-specific TODOs live in their own files:
- **Runtime**: `docs/implementation/runtime/TODO.md` -- AAM transitions, scoping, goal hierarchy, tool pipeline
- **Compiler**: `docs/implementation/compiler/TODO.md` -- Phase 1 ops, optimization levels, passes, CLI tooling

This file tracks items that span multiple crates or don't belong in either domain file.

---

## AIS Spec vs Enum Drift

### P2: Missing AIS operations from the spec

The spec (docs/pxm/ais.md) lists 32 ops in 9 categories. Cross-referencing with the enum and dispatcher:

- [ ] **DELEGATE** -- Listed in spec under Coordination category, but not in `AISOperationType` enum. No handler exists. Files: `crates/apxm-ais/src/operations/definitions.rs`, `docs/pxm/ais.md`
- [ ] **NEGOTIATE** -- Listed in spec under Coordination category, but not in enum. No handler exists. Files: `crates/apxm-ais/src/operations/definitions.rs`, `docs/pxm/ais.md`
- [ ] **NOP** -- Listed in spec under Identity category, but not in enum. No handler exists. Files: `crates/apxm-ais/src/operations/definitions.rs`, `docs/pxm/ais.md`
- [ ] **IDENTITY** -- Listed in spec under Identity category, but not in enum. No handler exists. Files: `crates/apxm-ais/src/operations/definitions.rs`, `docs/pxm/ais.md`

Note: The enum has 32 variants but they differ from the spec's 32. The enum includes Agent, Exc, Print, Jump, BranchOnValue, LoopStart, LoopEnd, Return, Switch, FlowCall, Err, UpdateGoal, Guard, Claim, Pause, Resume, ConstStr, Yield -- which are NOT in the spec table. The spec lists DELEGATE, NEGOTIATE, NOP, IDENTITY, COMM, FLOW -- which are NOT in the enum (COMM -> Communicate, FLOW -> FlowCall exist as renames).

---

## Substrate Gaps (for "LLVM for agents" vision)

These items live in `apxm-backends`, `apxm-tools`, and cross-crate concerns not covered by runtime or compiler TODOs.

### P0: Needs implementation

- [ ] **Streaming** -- No `generate_stream` on `LLMBackend` trait. The trait only has `generate()` returning a complete `LLMResponse`. No SSE/token-by-token streaming. The event emitter calls `emit_llm_token` but only after the full response arrives, not during streaming. File: `crates/apxm-backends/src/llm/backends/traits.rs` (trait has only `generate`, `name`, `model`, `health_check`, `list_models`)
- [ ] **Parallel tool dispatch** -- Tool calls in the ASK handler execute sequentially in a `for` loop (`for tool_call in &response.tool_calls`). No `FuturesOrdered`, no `RwLock`-based concurrency control, no per-tool parallelism metadata. File: `crates/apxm-runtime/src/executor/handlers/llm.rs` (lines 708-712)
- [ ] **Sandboxing** -- No OS-level sandboxing (Seatbelt/Landlock/bubblewrap). Only app-level policy exists in `InterceptDecision`. No `apxm-sandbox` crate. Files: `crates/apxm-core/src/error/security.rs` (references sandboxing errors but no implementation)
- [ ] **Codebase indexing** -- No AST extraction, tree-sitter integration, dependency graph, embedding-based retrieval, or relevance ranking. Zero matches for tree-sitter/PageRank/embedding patterns in the crate source. This is needed for Aider/Cursor-style context assembly. File: Not implemented anywhere (would be a new `apxm-index` crate)
- [ ] **Transactional file writes** -- No all-or-nothing multi-file write capability. No conflict detection or rollback mechanism for coordinated file changes. File operations in `apxm-tools` are individual read/write with no transaction support. Files: `crates/apxm-tools/src/`

### P1: Needs enhancement

- [ ] **Streaming + tool interleaving** -- Even when streaming is added, the current architecture processes tool calls only after a complete LLM response. Need mid-stream tool dispatch (process tool calls as they arrive, not after full response). File: `crates/apxm-runtime/src/executor/handlers/llm.rs`
- [ ] **CancellationToken** -- No cancellation token hierarchy on `ExecutionContext`. No way to cancel in-flight operations or propagate timeouts through the scheduler. Zero references to CancellationToken in the codebase. File: `crates/apxm-runtime/src/executor/context.rs`
- [ ] **Event emission** -- Only 3 event types (`LlmToken`, `ToolStart`, `ToolEnd`). Substrate analysis says ~15 are needed (operation start/end, planning events, checkpoint events, memory events, timing). File: `crates/apxm-runtime/src/executor/events.rs` (31 lines total)
- [ ] **Permission/approval flow** -- `InterceptDecision` exists but there is no async user interaction (no approval channel, no session caching, no multi-level guardian rules like Codex's ApprovalStore). File: `crates/apxm-runtime/src/capability/interceptor.rs`
- [ ] **Session/checkpoint continuity** -- `execution_id` and `session_id` exist but no file-level checkpoints, no resume-from-session-id for process restarts. Related to runtime TODO Gap 7 (AamCheckpoint excludes capabilities). File: `crates/apxm-runtime/src/aam/mod.rs` (AamCheckpoint has beliefs + goals but not capabilities, line 274-280)
- [ ] **Messages as structured arrays** -- `LLMRequest` uses a single `prompt: String` field. Production agents need `messages: Vec<Message>` with roles, tool_call_id, and content parts. File: `crates/apxm-backends/src/llm/backends/request.rs`
- [ ] **Configurable max_tool_iterations** -- Hardcoded `MAX_TOOL_ITERATIONS = 10` in llm.rs. Should be configurable per-node via attributes (Codex needs 25+). File: `crates/apxm-runtime/src/executor/handlers/llm.rs` (line 73)

---

## Cross-Cutting AAM Gaps

Items here are architectural and span multiple crates. Per-handler and per-scheduler AAM items are in runtime/TODO.md.

### P1: Missing feature

- [ ] **No goal satisfaction detection** -- No mechanism to detect when a goal should be marked complete. No `CompletionPolicy` (AllChildren, AnyChild, Manual, Condition). Relates to runtime TODO Gap 4 (flat goals) but is a distinct missing feature. File: `crates/apxm-runtime/src/aam/mod.rs`

### P2: Hierarchical AAM (from gap-analysis.md vision)

- [ ] **No workspace control plane** -- No WorkspaceManager, ScopeRegistry, Materializer, StateProjector, or PolicyEngine. Requires new crate(s). File: Not implemented anywhere
- [ ] **No WorkflowNode unifying INV + FLOW_CALL + PLAN inner-plan** -- These remain separate code paths with no unified abstraction. Spans runtime handlers, compiler lowering, and core types. Files: `crates/apxm-runtime/src/executor/handlers/inv.rs`, `flow_call.rs`, `plan.rs`; `crates/apxm-core/src/types/execution/node.rs`

---

## Summary

| Category | Total Items | P0 | P1 | P2 |
|----------|-------------|----|----|-----|
| AIS Spec vs Enum Drift | 4 | 0 | 0 | 4 |
| Substrate Gaps | 12 | 5 | 7 | 0 |
| Cross-Cutting AAM | 3 | 0 | 1 | 2 |
| **This file** | **19** | **5** | **8** | **6** |
| Runtime TODO (separate) | 13 | 4 | 5 | 4 |
| Compiler TODO (separate) | 15 | 3 | 6 | 6 |
| **Grand Total** | **47** | **12** | **19** | **16** |

### Key metric: AAM transition coverage

- **Operations with AAM transitions**: 7 of 32 (22%) -- see runtime/TODO.md Gap 8
- **Operations without AAM transitions**: 25 of 32 (78%)
- **Spec claims**: "every AIS instruction is a state transition on the AAM"
- **Reality**: fewer than 1 in 4 operations actually produce a transition

### Key metric: Substrate readiness

- **Production-ready primitives**: 9 of 17 (53%)
- **Needs enhancement**: 7 of 17 (41%)
- **Not implemented**: 5 of 17 (29%, counting streaming as not-implemented since there is no trait method)
