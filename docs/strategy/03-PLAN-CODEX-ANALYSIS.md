# Codex Analysis: Plan vs Reality

**Date:** April 6, 2026  
**Plan audited:** `docs/strategy/03-PLAN.md` (March 31, 2026)  
**Audit scope:** workspace `Cargo.toml`, all 16 crates under `crates/`, and `external/vllm` for Phase 3 because the plan explicitly includes Python-side vLLM work.

## 1. WHAT WAS BUILT

### Workspace reality

The codebase is materially larger than the plan assumed. The current workspace contains 16 Rust crates:

- `apxm-runtime`, `apxm-backends`, `apxm-driver`, `apxm-cli`, `apxm-compiler`, `apxm-graph`, `apxm-acp`
- `apxm-core`, `apxm-ais`, `apxm-artifact`, `apxm-credentials`, `apxm-events`, `apxm-sandbox`, `apxm-server`, `apxm-tools`, `apxm-task-validation`

In addition, Phase 3 work is partly implemented in `external/vllm`, not only in `crates/`.

### Phase 1: Dynamic Model Router

| Planned deliverable | Status | Evidence | Audit note |
|---|---|---|---|
| ModelRouter with priority-based resolution | **DONE** | `crates/apxm-runtime/src/model_router/mod.rs`, `registry.rs`, `health.rs`, `rate_limit.rs` | The router exists, supports explicit backend/model resolution, per-op policy, tag-based fallback, circuit breakers, and rate limiting. This is broader than the original Phase 1 scope. |
| Health monitoring with circuit breakers | **DONE** | `crates/apxm-runtime/src/model_router/health.rs`; `crates/apxm-backends/src/llm/backends/traits.rs` | Circuit breaker state, success/failure recording, health snapshots, and backend `health_check()` are all present. |
| `~/.apxm/models.toml` configuration | **DONE** | `crates/apxm-runtime/src/model_router/registry.rs` | The loader, defaults, routing tags, and model metadata are implemented. |
| Runtime integration into LLM execution | **DONE** | `crates/apxm-runtime/src/runtime.rs`, `crates/apxm-runtime/src/executor/context.rs`, `crates/apxm-runtime/src/executor/handlers/mod.rs` | The runtime mounts `ModelRouter` into `ExecutionContext`, and LLM dispatch uses it when present. |
| Backward compatibility with existing `model` attribute | **DONE** | `crates/apxm-runtime/src/executor/handlers/llm.rs`; `crates/apxm-runtime/src/model_router/mod.rs` | `llm.rs` still reads `model`, and the router explicitly honors direct model selection before policy routing. |
| Unit tests for routing logic and failover | **DONE** | inline tests in `crates/apxm-runtime/src/model_router/mod.rs`, `health.rs`, `profile_router.rs` | The planned standalone `tests/model_router_tests.rs` file does not exist, but the test coverage does. |
| `apxm models list/health` CLI | **PARTIAL** | `crates/apxm-cli/src/main.rs` | Model-related operations exist through `backend add-model`, routing config, and Ollama sync, but there is no first-class `Models` top-level subcommand in the current CLI. The file header still lists `commands/model.rs` as a refactor target, not current structure. |

**Phase 1 verdict:** functionally built, but the CLI surface did not land in the shape the plan described.

### Phase 2: ACP Client in APXM

| Planned deliverable | Status | Evidence | Audit note |
|---|---|---|---|
| `AcpCapability` implementing `CapabilityExecutor` | **TODO** | absence of `crates/apxm-runtime/src/capability/acp.rs`; comments in `crates/apxm-driver/src/runtime/capabilities.rs`; agent path implemented in `crates/apxm-driver/src/runtime/agents.rs`, `crates/apxm-runtime/src/executor/handlers/spawn_agent.rs`, `communicate.rs` | The plan's exact deliverable was not built. It was superseded by a different architecture: `SPAWN_AGENT` + `COMMUNICATE` + `ProcessTable` + separate `apxm-acp` crate. |
| ACP JSON-RPC 2.0 protocol | **DONE** | `crates/apxm-acp/src/protocol.rs`, `constants.rs`, `session.rs` | `initialize`, `authenticate`, `session/new`, and `session/prompt` are implemented. |
| Agent adapter registry with configurable profiles | **DONE** | `crates/apxm-acp/src/registry.rs` | Built-in templates plus persisted user entries in `~/.apxm/agents.toml` are implemented. |
| Reverse request handling (file I/O, terminal, permissions) | **DONE** | `crates/apxm-acp/src/reverse.rs`, `terminal.rs`, `registry.rs` | Permission modes and reverse-request handling are real, not stubs. |
| `~/.apxm/agents.toml` configuration | **DONE** | `crates/apxm-acp/src/registry.rs`, `crates/apxm-acp/src/constants.rs` | User config loading and persistence are implemented. |
| `apxm agent list/test` CLI commands | **DONE** | `crates/apxm-cli/src/main.rs` (`AgentAction`) | `list`, `add`, `remove`, `test`, and `templates` all exist. |
| End-to-end test: `INV(acp)` -> Claude Code -> code changes | **PARTIAL** | `crates/apxm-acp/tests/integration.rs`; removal of `AcpCapability` references in current tree | There is an end-to-end ACP session lifecycle test via a mock agent, but not the exact planned `INV(acp)` graph path. The repo has deliberately moved away from `INV(acp)` toward `SPAWN_AGENT` + `COMMUNICATE`. |

**Phase 2 verdict:** the capability did not land in the planned form, but the ACP client and agent orchestration goal did land through a better-fitted runtime/process architecture.

### Phase 3: Graph-Aware vLLM Extension

| Planned deliverable | Status | Evidence | Audit note |
|---|---|---|---|
| Graph metadata protocol (Rust + Python) | **DONE** | Rust: `crates/apxm-backends/src/llm/backends/vllm/graph_meta.rs`; Python: `external/vllm/vllm/v1/request.py`, `external/vllm/vllm/entrypoints/openai/apxm/api_router.py` | Both sides exist. |
| GraphAwareVllmBackend in `apxm-backends` | **DONE** | `crates/apxm-backends/src/llm/backends/vllm/backend.rs`, `mod.rs` | The Rust backend can register/release graphs and inject APXM hints into requests. |
| vLLM scheduler extension with priority scheduling | **PARTIAL** | `external/vllm/vllm/v1/core/sched/scheduler.py` | Critical-path priority boost, graph registry, graph status, and pin lifecycle are implemented, but the work landed as a fork patch inside the core scheduler rather than the planned standalone plugin module. |
| KV-cache pinning for downstream-feeding nodes | **PARTIAL** | `external/vllm/vllm/v1/core/sched/scheduler.py`; tests in `external/vllm/tests/v1/core/test_scheduler.py` | Prefix pinning by graph, TTL expiry, and release are real. `downstream_nodes` metadata is carried, but the current scheduler logic is still mostly prefix-pin based rather than clearly downstream-aware scheduling logic. |
| Eager prefill of static contexts | **TODO** | `external/vllm/vllm/v1/request.py` parses `compiler_hints`, but no consuming logic was found in scheduler/runtime | The hint schema exists; the behavior does not. |
| Benchmark pipeline latency with/without graph-aware scheduling | **PARTIAL** | `crates/apxm-backends/benches/vllm_hints.rs` | There are benchmarks for hint serialization/overhead, but not the planned end-to-end latency-improvement benchmark. |

**Important gap inside Phase 3:** the runtime does not currently wire this through end-to-end. I found `with_apxm_hints`, `register_graph`, and `release_graph` in the backend layer, but no runtime or driver call sites outside backend tests/examples.

**Phase 3 verdict:** more was built than the plan text suggests at first glance, but the full APXM-runtime-to-vLLM closed loop is still incomplete.

### Phase 4: Patent Implementation + Optimization Targets

| Planned deliverable | Status | Evidence | Audit note |
|---|---|---|---|
| ContextStack with spaghetti-stack organization | **PARTIAL** | `crates/apxm-runtime/src/context_stack/`, `crates/apxm-cli/src/main.rs`, `crates/apxm-runtime/src/executor/handlers/spawn_agent.rs`, `communicate.rs`, `tests/context_stack_integration.rs` | A real context stack exists and is integrated into ACP prompt assembly, but it is not the full module breakdown or full hierarchical AAM/file-tree realization described in the plan. |
| MemoCache (session + persistent) with speculation | **PARTIAL** | `crates/apxm-runtime/src/executor/memoization.rs` | L1 DashMap, optional SQLite L2, and speculative overlay/commit/rollback exist. What is missing is scheduler-level speculative execution of nodes. |
| Token pipelining between adjacent LLM nodes | **TODO** | no runtime call sites for downstream prefill streaming | The plan's pipelining behavior is not present. |
| Five optimization targets with compiler pass mapping | **TODO** | no `crates/apxm-compiler/src/passes/targets.rs`; no target-aware pipeline builder | The target system in the plan does not exist in the current compiler. |
| New compiler passes for target-driven optimization | **PARTIAL** | `crates/apxm-graph/src/optimize.rs`; `crates/apxm-compiler/src/api/pipeline.rs` | Organic graph-level passes exist (`constant_folding`, `prompt_caching`, `memoization_hints`, `parallelism_analysis`), but the planned MLIR files like `ModelDowngrade.cpp`, `ContextBudget.cpp`, `SpeculationInsertion.cpp`, and `PipelineInsertion.cpp` do not exist. |
| `--target` CLI flag | **TODO** | `crates/apxm-cli/src/main.rs` compile/execute commands expose `--opt-level` but not `--target` | Missing exactly as planned. |
| End-to-end integration test | **PARTIAL** | `crates/apxm-runtime/tests/context_stack_integration.rs`, `crates/apxm-acp/tests/integration.rs`, `crates/apxm-backends/tests/vllm_integration.rs` | Subsystem integration tests exist, but not the single end-to-end target-aware stack test the plan describes. |
| Benchmark suite across all optimization targets | **TODO** | no cross-target benchmark harness found in `crates/` | Not built. |

**Phase 4 verdict:** strong substrate pieces landed early, but the planned target-selection layer and token-pipelining layer are still absent.

## 2. WHAT EMERGED ORGANICALLY

These are the largest shipped systems or pivots that are real in the tree but were not part of the March 31 plan in this form.

1. **Ollama became a first-class backend, not just another provider stub.**  
   Evidence: `crates/apxm-backends/src/llm/backends/ollama/backend.rs`, `crates/apxm-cli/src/main.rs`.  
   The backend now speaks `/api/chat`, supports streaming, tool calls, system prompts, multi-turn message input, health checks, model listing, and CLI-side model sync via `/api/show` and `/api/tags`.

2. **The Agent Team concept shipped.**  
   Evidence: `crates/apxm-runtime/src/team/registry.rs`, `crates/apxm-runtime/src/executor/handlers/spawn_team.rs`, `crates/apxm-cli/src/main.rs` (`TeamAction`).  
   This is a real orchestration concept layered above single-agent spawning and was not in the March 31 phase plan.

3. **`ultrathink` became a forcing function for architecture fixes.**  
   Evidence: commit `3052b0a`, plus current code in `crates/apxm-runtime/src/executor/handlers/llm.rs` and `crates/apxm-driver/src/runtime/mod.rs`.  
   The current tree contains the fixes that made operation-type routing and config-driven operation policies actually work end-to-end.

4. **A route/order validator for agent graphs emerged, but not as a module literally named `route_validator`.**  
   Evidence: `crates/apxm-graph/src/semantic.rs`, `crates/apxm-graph/src/validate.rs`, `crates/apxm-core/src/error/codes.rs`, `crates/apxm-cli/src/main.rs` (`E514` explanation).  
   The shipped implementation is semantic validation of `SPAWN_AGENT`/`COMMUNICATE` reachability and recipient correctness.

5. **Session observability expanded into a full subsystem.**  
   Evidence: `crates/apxm-driver/src/session_output.rs`, `crates/apxm-cli/src/main.rs` (`SessionAction`, `Replay`), `crates/apxm-core/src/types/session/types.rs`.  
   This includes `manifest.json`, `live.json`, per-node workspaces, per-node traces, token capture, session diff/inspect/clean, and live progress tracking.

6. **Model profiles and allowlist governance appeared on top of simple routing.**  
   Evidence: `crates/apxm-core/src/model_profiles.rs`, `crates/apxm-runtime/src/model_router/profile_registry.rs`, `profile_router.rs`, `crates/apxm-compiler/src/passes/validate_model_allowlist.rs`, `validate_model_profile.rs`.  
   This is a much richer model-governance system than the original Phase 1 policy table.

7. **Agent profiles and scoped context rules became a real abstraction.**  
   Evidence: `crates/apxm-core/src/agent_profile.rs`, `crates/apxm-runtime/src/context_stack/policy.rs`, `crates/apxm-driver/src/context_assembler.rs`.  
   The runtime now reasons about profile-specific context exposure rather than only provider/model routing.

8. **The project pivoted hard toward `.ais` source and `.air` canonical IR.**  
   Evidence: current compiler/driver/runtime tree, especially `crates/apxm-compiler/src/api/pipeline.rs`, `crates/apxm-driver/src/lib.rs`, `crates/apxm-runtime/src/workflow/*`.  
   The plan assumed APXM graph artifacts, but the actual repo has already deprecated the legacy JSON `.apxm` path and standardized around `.ais` + `.air`.

9. **Multi-graph workflow execution (`.apxmw`) shipped.**  
   Evidence: `crates/apxm-runtime/src/workflow/def.rs`, `runner.rs`, `template.rs`, `topo.rs`.  
   This is outside the March 31 plan and gives APXM a second orchestration layer above single graphs.

10. **Task intake validation became its own crate and runtime boundary.**  
    Evidence: `crates/apxm-task-validation/src/lib.rs`, `crates/apxm-runtime/tests/invalid_task_rejection.rs`.  
    This was not in the plan, but it is now an architectural guardrail around LLM/task execution.

11. **The vLLM work spilled into a maintained fork under `external/vllm`.**  
    Evidence: `external/vllm/vllm/entrypoints/openai/apxm/api_router.py`, `external/vllm/vllm/v1/core/sched/scheduler.py`, `external/vllm/vllm/v1/request.py`.  
    The plan described Python-side work abstractly; reality is an actual forked implementation that now needs lifecycle management.

12. **No exact symbol named `vendor health sync` was found in `crates/`.**  
    Closest concrete implementations:
    - backend health checks and circuit breakers: `crates/apxm-backends/src/llm/backends/traits.rs`, `crates/apxm-runtime/src/model_router/health.rs`
    - AAM-aware ACP context propagation: `crates/apxm-acp/src/aam_bridge.rs`, `crates/apxm-runtime/src/executor/handlers/spawn_agent.rs`

## 3. REVISED PRIORITIES

Given the current state, the next work should not follow the March 31 phase ordering literally.

1. **Unify routing into one source of truth.**  
   Right now `LLMRegistry` already owns backend defaults, operation defaults, aliases, fallbacks, and health, while `ModelRouter` adds its own routing, health, and rate limiting on top. This duplication already caused real bugs. Pick one routing layer as canonical and collapse the other into it.

2. **Finish the Phase 3 activation path, not the Phase 3 data model.**  
   The Rust backend wrapper and the `external/vllm` fork both exist. The missing piece is runtime emission: register the graph before execution, attach `ApxmGraphHints` per request, and release the graph on completion/cancel.

3. **Ship a minimal target system instead of waiting for all five targets.**  
   Start with `parallel` and `latency`. Those map best onto the existing graph analysis, context stack, memoization, and vLLM hint infrastructure. Do not block on `cost`/`tokens`/`balanced` being perfect.

4. **Make `SPAWN_AGENT` + `COMMUNICATE` the explicit canonical agent architecture everywhere.**  
   The code has already moved there. The remaining work is cleanup: remove stale `INV(acp)` assumptions from docs, examples, and any residual code paths.

5. **Add modern end-to-end tests for the actual stack, not the planned stack.**  
   The right E2E path is now `.ais` -> compile -> runtime -> `SPAWN_AGENT` -> `COMMUNICATE` -> session output -> optional vLLM hinting. That path needs direct coverage.

6. **Productize only after coherence.**  
   Teams, Ollama polish, planner/meta-workflows, and richer session UX are already ahead of the substrate. They should be secondary until routing and graph-aware vLLM are internally coherent.

## 4. REALISTIC TIMELINE

### Observed delivery velocity

From **March 31, 2026** through **April 6, 2026**, the repo logged **203 commits**:

- March 31: 2
- April 1: 4
- April 2: 12
- April 3: 47
- April 4: 55
- April 5: 34
- April 6: 49

That is roughly **29 commits/day average** over the last 7 days, but this is not linear “remaining weeks = remaining commits / 29”. A lot of the recent velocity came from bursty architecture pivots, reversions, docs, and bug cleanup.

### Practical estimate from April 6, 2026

| Remaining scope | Estimate | Target window |
|---|---|---|
| Routing consolidation, missing model CLI surface, doc cleanup, and real E2E tests | **1 to 2 weeks** | **April 13 to April 20, 2026** |
| Runtime-to-vLLM graph registration/hint wiring and smoke validation | **+1 to 2 weeks** | **April 20 to May 4, 2026** |
| Minimal target system (`parallel`, `latency`) with compiler/runtime plumbing | **+1 to 2 weeks** | **April 27 to May 11, 2026** |
| Full original-plan closeout including all five targets, token pipelining, and convincing benchmark proof | **+2 to 4 more weeks** | **May 11 to June 8, 2026** |

### Bottom line

If scope is narrowed to “make the current architecture coherent and finish the already-started graph-aware path,” this looks like a **2 to 4 week** effort from **April 6, 2026**.

If the goal is to fully realize the original March 31 vision, including full target-directed compilation and token pipelining, the realistic remaining effort is closer to **5 to 8 weeks** from **April 6, 2026**.

## 5. ARCHITECTURE GAPS

These are the clearest places where implementation reality diverged from the plan.

| Gap | Evidence | Why it matters |
|---|---|---|
| `ModelRouter` and `LLMRegistry` both route | `crates/apxm-runtime/src/model_router/mod.rs`; `crates/apxm-backends/src/llm/registry/mod.rs`; `crates/apxm-driver/src/runtime/llm.rs`; `crates/apxm-driver/src/runtime/mod.rs` | This is the largest current architectural mismatch. Backends, aliases, operation routes, fallbacks, health, and rate limits are split across two layers. |
| The exact Phase 2 `AcpCapability` never landed | absence of `crates/apxm-runtime/src/capability/acp.rs`; current path is `crates/apxm-acp`, `spawn_agent.rs`, `communicate.rs`, `runtime/agents.rs` | The goal was achieved, but the capability abstraction in the plan was replaced by a process-oriented agent subsystem. |
| `INV(acp)` was replaced by `SPAWN_AGENT` + `COMMUNICATE` | `crates/apxm-driver/src/runtime/capabilities.rs` comment; current handlers in `crates/apxm-runtime/src/executor/handlers/` | This is a real architecture decision, not just naming drift. The docs and tests should reflect it consistently. |
| The planned `apxm models` CLI did not materialize as designed | `crates/apxm-cli/src/main.rs` | Model management is currently split across backend registration, config, and profiles. The UX is functional but architecturally uneven. |
| The vLLM implementation exists in three disconnected layers | Rust client: `crates/apxm-backends/src/llm/backends/vllm/*`; Python fork: `external/vllm/...`; missing runtime call sites for `with_apxm_hints` / `register_graph` / `release_graph` | The plan's value only appears when these layers are connected during real APXM execution. Right now they mostly coexist. |
| `operation_type` propagation was fragile enough to break routing | fixed in commit `3052b0a`; current code in `crates/apxm-runtime/src/executor/handlers/llm.rs` and `crates/apxm-driver/src/runtime/mod.rs` | This bug is a symptom of the dual-routing architecture. Routing decisions depend on metadata surviving multiple layers correctly. |
| ContextStack shipped as session prompt assembly, not full “spaghetti-stack” state architecture | `crates/apxm-runtime/src/context_stack/*`; no planned `assembly.rs`, `demand_paging.rs`, `manifest.rs`, `collapse.rs` layout | Useful and real, but narrower than the plan's conceptual scope. |
| Speculation exists inside cache infrastructure, not scheduler execution | `crates/apxm-runtime/src/executor/memoization.rs`; no scheduler call sites | The plan described predictive execution of nodes. The current code stops at speculative cache overlays. |
| Optimization targets were replaced by opportunistic passes and validation | `crates/apxm-graph/src/optimize.rs`; `crates/apxm-compiler/src/api/pipeline.rs`; no `passes/targets.rs` | The compiler improved, but not along the planned target-selection axis. |
| Route validation emerged as semantic checks, not a dedicated subsystem | `crates/apxm-graph/src/semantic.rs`, `validate.rs` | This is fine technically, but it means “route validator” is currently policy embedded in graph validation rather than a clearly named module/API. |

## Summary Judgment

The March 31 plan is no longer the best description of the system.

- **Phase 1 is effectively built**, although the CLI/user surface is incomplete.
- **Phase 2 is built in spirit but not in the planned shape**; the repo chose a better `SPAWN_AGENT`/`COMMUNICATE` architecture than `AcpCapability`.
- **Phase 3 is surprisingly far along**, including real `external/vllm` work, but the runtime does not yet activate it.
- **Phase 4 has substrate, not product**: context stack, memoization, and graph analysis exist, but target-directed compilation and token pipelining do not.

The highest-value next move is to stop adding surface area and make the current layers line up cleanly.
