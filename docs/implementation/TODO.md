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

The spec (docs/pxm/ais.md) lists 39 ops in 9 categories. Cross-referencing with the enum and dispatcher:

- [x] **DELEGATE** -- Resolved: enum variant, handler (delegate.rs), dispatcher routing all exist.
- [x] **NEGOTIATE** -- Resolved: enum variant, handler (negotiate.rs), dispatcher routing all exist.
- [x] **NOP** -- Resolved: enum variant, handler (nop.rs), dispatcher routing all exist.
- [x] **IDENTITY** -- Resolved: enum variant, handler (identity.rs), dispatcher routing all exist.

Note: The AIS enum (see `apxm ops list` for current count) has variants that differ from the spec's 32. The enum includes Agent, Exc, Print, Jump, BranchOnValue, LoopStart, LoopEnd, Return, Switch, FlowCall, Err, UpdateGoal, Guard, Claim, Pause, Resume, ConstStr, Yield -- which are NOT in the spec table. The spec lists DELEGATE, NEGOTIATE, NOP, IDENTITY, COMM, FLOW -- which are NOT in the enum (COMM -> Communicate, FLOW -> FlowCall exist as renames).

---

## Substrate Gaps (for "LLVM for agents" vision)

These items live in `apxm-backends`, `apxm-tools`, and cross-crate concerns not covered by runtime or compiler TODOs.

### P0: Needs implementation

- [x] **Streaming** -- Resolved: StreamChunk enum and generate_stream() method added to LLMBackend trait with default impl. MockLLMBackend provides simulated token streaming. Runtime handler wired to consume stream via execute_llm_request_streaming() when event_emitter is present.
- [x] **Parallel tool dispatch** -- ~~Tool calls in the ASK handler execute sequentially in a `for` loop.~~ Implemented in `48970eb`: `execute_tool_calls_parallel()` with `join_all`, per-tool-name `RwLock` for write tools, `read_only` field on `CapabilityMetadata`. File: `crates/apxm-runtime/src/executor/handlers/llm.rs`, `crates/apxm-runtime/src/capability/metadata.rs`
- [x] **Sandboxing** -- Phase 1 MVP resolved: ProcessSandbox with timeout, env restriction, working directory isolation. EXC handler wired to sandbox. Phase 2 (Landlock/seccomp) deferred. Files: `crates/apxm-runtime/src/sandbox/` (policy.rs, process.rs), `crates/apxm-runtime/src/executor/handlers/exc.rs`
- [ ] **Codebase indexing** -- _Deferred from P0 to P1._ Greenfield `apxm-index` crate (~3000-4000 LOC). No AST extraction, tree-sitter integration, dependency graph, embedding-based retrieval, or relevance ranking. Zero matches for tree-sitter/PageRank/embedding patterns in the crate source. Needed for Aider/Cursor-style context assembly: given a task description, automatically select the most relevant files/functions/classes from a codebase to include in the LLM prompt. Would require: tree-sitter parsers for language-level AST extraction, a dependency graph (imports/call sites), embedding-based semantic search (vector store), and a relevance ranker (PageRank-style or learned) to score candidates. File: Not implemented anywhere (would be a new `apxm-index` crate)
- [x] **Transactional file writes** -- Resolved: `atomic_write_with_backup()` for single files, `FileTransaction` struct for multi-file coordinated writes with commit/rollback. Files: `crates/apxm-tools/src/write.rs`

### P1: Needs enhancement

- [x] **Streaming + tool interleaving** -- Resolved: `execute_llm_request_streaming()` now accumulates ToolCallStart/ToolCallDelta chunks via `PendingToolCall` state machine. Accumulated tool calls are finalized and merged into the LLMResponse before returning to the tool loop. 4 unit tests. File: `crates/apxm-runtime/src/executor/handlers/mod.rs`
- [x] **CancellationToken** -- Resolved: Hierarchical `CancellationToken` with parent/child, `cancel_after(Duration)`, 11 unit tests. Integrated into `ExecutionContext` (context.rs:70) and child context creation (context.rs:216). Dispatcher checks cancellation before each op dispatch. File: `crates/apxm-runtime/src/executor/cancellation.rs` (204 lines)
- [x] **Event emission** -- Resolved: ~15 event types defined in events.rs (164 lines) with full trait. OperationStart/End wired in dispatcher.rs, memory events in qmem.rs/umem.rs, planning events in plan.rs, token usage in llm.rs. All emitters have default no-op impls. File: `crates/apxm-runtime/src/executor/events.rs`
- [x] **Permission/approval flow** -- Resolved: ApprovalStore with session caching (Once/Session/Always scopes), ApprovalChannel async trait. File: `crates/apxm-runtime/src/capability/approval.rs`
- [x] **Session/checkpoint continuity** -- Resolved: `AamCheckpoint::save_to_file()` / `load_from_file()` for JSON persistence. `SessionManager` struct with `save_checkpoint()`, `load_checkpoint()`, `list_sessions()`, `delete_checkpoint()`. Full round-trip tested (9 unit tests). Files: `crates/apxm-runtime/src/aam/mod.rs`, `crates/apxm-runtime/src/aam/session.rs`
- [x] **Messages as structured arrays** -- Resolved: `LLMRequest` has `messages: Vec<Message>` with `ContentPart` variants (Text, Image, ToolCall). Full API: `from_messages()`, `with_messages()`, `add_message()`, `has_messages()`, `resolved_messages()`. File: `crates/apxm-backends/src/llm/backends/request.rs` (653 lines)
- [x] **Configurable max_tool_iterations** -- Resolved: `DEFAULT_MAX_TOOL_ITERATIONS = 10` is a fallback; per-node override via `graph_attrs::MAX_TOOL_ITERATIONS` attribute in llm.rs:800-803. File: `crates/apxm-runtime/src/executor/handlers/llm.rs` (line 73)

---

## Cross-Cutting AAM Gaps

Items here are architectural and span multiple crates. Per-handler and per-scheduler AAM items are in runtime/TODO.md.

### P1: Missing feature

- [x] **No goal satisfaction detection** -- Resolved: `CompletionPolicy` enum (AllChildren, AnyChild, Manual), `GoalTree` with parent-child index, `propagate_completion()` method on `Aam`. `add_child_goal()` registers parent-child relationships. 3 unit tests. File: `crates/apxm-runtime/src/aam/mod.rs`

### P2: Hierarchical AAM (from gap-analysis.md vision)

- [~] **No workspace control plane** -- _Partially resolved_: ScopeRegistry and WorkspaceManager foundation added to apxm-runtime (scope tracking, AAM instance management, 9 tests). Still needed: **Materializer** (materialize scoped AAM state into concrete sub-task execution contexts), **StateProjector** (project state changes from child scopes back up to parent scopes after sub-task completion), **PolicyEngine** (enforce scope policies like quota limits, capability restrictions, and isolation boundaries across multi-agent hierarchies). File: `crates/apxm-runtime/src/workspace/mod.rs`
- [x] **No WorkflowNode unifying INV + FLOW_CALL + PLAN inner-plan** -- Resolved: `WorkflowNode` enum added to `apxm-core::types::execution` with four variants (Operation, FlowCall, Invocation, SubWorkflow). Recursive composition supported. 16 unit tests. Existing handlers can adopt incrementally. Files: `crates/apxm-core/src/types/execution/workflow.rs`, `crates/apxm-core/src/types/execution/mod.rs`

---

## Summary

| Category | Total | Done | Open P0 | Open P1 | Open P2 |
|----------|-------|------|---------|---------|---------|
| AIS Spec vs Enum Drift | 4 | 4 | 0 | 0 | 0 |
| Substrate Gaps | 12 | 11 | 1 | 0 | 0 |
| Cross-Cutting AAM | 3 | 2 | 0 | 0 | 1 |
| **This file** | **19** | **17** | **1** | **0** | **1** |
| Runtime TODO (separate) | 13 | 12 | 0 | 0 | 1 |
| Compiler TODO (separate) | 15 | 13 | 0 | 0 | 2 |
| **Grand Total** | **47** | **42** | **1** | **0** | **4** |

### Key metric: AAM transition coverage

- **Operations with AAM transitions**: all ops have transitions (100%) -- fixed in `61d408f`
- ~~Operations without AAM transitions: previously 78%~~
- **Spec claims**: "every AIS instruction is a state transition on the AAM"
- **Reality**: all operations now produce transitions (Gap 8 resolved)

### Key metric: Substrate readiness

- **Production-ready primitives**: 17 of 18 (94%) -- streaming, sandboxing, transactional writes, parallel tool dispatch, approval flow, session checkpoints, per-pass diagnostics added
- **Needs enhancement**: 0 of 18 (0%)
- **Not implemented**: 1 of 18 (6%, codebase indexing deferred)
