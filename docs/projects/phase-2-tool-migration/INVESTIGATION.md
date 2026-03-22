# Phase 2 Investigation: Tool Migration -- Source Code Cross-Reference

**Date:** 2026-03-20
**Scope:** APXM (A5), Codex (C5-C7), Gemini-CLI (G6-G8) plan claims vs source code

---

## 1. Verified Claims

### 1.1 APXM Capability System (Current State)

The Phase 2 plan describes the capability system as "more mature than one might expect." This is confirmed.

| Claim | Status | Evidence |
|-------|--------|----------|
| `CapabilityExecutor` trait with `execute(args) -> Value` + `metadata()` | VERIFIED | `apxm-runtime/src/capability/executor.rs:17-38` |
| `CapabilityMetadata` has 10 fields | VERIFIED | `metadata.rs:10-47` -- `name`, `description`, `parameters_schema`, `returns`, `cost_estimate`, `latency_estimate_ms`, `requires_auth`, `tags`, `read_only`, `metadata` |
| `CapabilityRegistry` is `DashMap`-backed | VERIFIED | `registry.rs:16-22` -- `DashMap<String, Arc<dyn CapabilityExecutor>>` |
| Registry has `register`, `get`, `list_names`, `list_metadata`, `unregister` | VERIFIED | `registry.rs:46,83,110,118,134` |
| `CapabilitySystem` has JSON Schema validation | VERIFIED | `mod.rs:304-332` -- `validate_args()` uses compiled `JsonSchema` |
| `CapabilitySystem` has timeout enforcement | VERIFIED | `mod.rs:286-288` -- `tokio::time::timeout` |
| `CapabilitySystem` has interceptor pipeline | VERIFIED | `mod.rs:239-278` -- iterates `interceptors`, calls `pre_invoke` |
| `CapabilitySystem` has post-invoke hooks | VERIFIED | `mod.rs:295-297` -- calls `post_invoke` on interceptors |
| `CapabilitySystem` has approval channel | VERIFIED | `mod.rs:65-66,99-101,246-273` -- `ApprovalChannel` with user override of deny decisions |
| `CapabilitySystem` has AAM integration | VERIFIED | `mod.rs:62,89-93,160-163` -- registers capabilities in AAM on `register()` |
| `InterceptDecision` has `Allow`, `Deny`, `EditArgs` | VERIFIED | `interceptor.rs:9-16` |
| `CapabilityInterceptor` trait has `name()`, `pre_invoke()`, `post_invoke()` | VERIFIED | `interceptor.rs:20-33` |
| `ApprovalStore` with `Once`, `Session`, `Always` scopes | VERIFIED | `approval.rs:14-21,39-103` |
| `ApprovalChannel` async trait | VERIFIED | `approval.rs:116-129` |
| 4 built-in tools: `BashCapability`, `ReadCapability`, `WriteCapability`, `SearchWebCapability` | VERIFIED | `apxm-tools/src/bash.rs:49`, `read.rs:54`, `write.rs:207`, `web_search.rs:91` |

### 1.2 What Is Missing (Plan Correctly Identifies)

| Missing Item | Plan Section | Current State |
|-------------|-------------|---------------|
| `ToolAdapter` trait + blanket impl | A5.1 | Does NOT exist -- `CapabilityExecutor` is the only trait; no simplified adapter |
| `ToolStore` persistence (`~/.apxm/tools.toml`) | A5.2 | Does NOT exist -- no persistence layer for tool registrations |
| HTTP capability bridge (`POST /v1/capabilities/`) | A5.3 | Does NOT exist -- `apxm-llm-service` does not exist yet either (Phase 1 deliverable) |
| `SandboxInterceptor`, `AuditInterceptor`, `CostInterceptor` | A5.4 | Do NOT exist -- only `DenyAll` test interceptor exists; no production interceptors |
| CLI `apxm tools` commands | A5.5 | Partially scaffolded per MEMORY.md (tools-cli-design.md exists) |

### 1.3 Codex Tool System

| Claim | Status | Evidence |
|-------|--------|----------|
| Plan says 21 `ToolHandler` implementations | **PARTIAL** | 26 total `impl ToolHandler` found; 21 named handlers + 5 `multi_agents` internal handlers |
| `ToolRouter` exists | VERIFIED | `tools/registry.rs` contains tool routing logic |
| `GuardianReviewSessionManager` exists | VERIFIED | `guardian/review_session.rs`, `guardian/review.rs`, `guardian/mod.rs` |
| `ShellHandler`, `ShellCommandHandler` exist | VERIFIED | `tools/handlers/shell.rs:152,236` |
| `ReadFileHandler` exists | VERIFIED | `tools/handlers/read_file.rs:95` |
| `ApplyPatchHandler` exists | VERIFIED | `tools/handlers/apply_patch.rs:128` |
| `McpHandler`, `McpResourceHandler` exist | VERIFIED | `tools/handlers/mcp.rs:14`, `mcp_resource.rs:182` |
| `BatchJobHandler` exists | VERIFIED | `tools/handlers/agent_jobs.rs:182` |
| All 21 handlers listed in plan match actual files | VERIFIED | Every handler name and module path in the plan table (codex.md lines 41-61) maps to a real file |

### 1.4 Gemini-CLI Tool System

| Claim | Status | Evidence |
|-------|--------|----------|
| `DeclarativeTool` abstract class exists | VERIFIED | `tools/tools.ts:420` -- `export abstract class DeclarativeTool<TParams, TResult>` |
| `BaseDeclarativeTool` extends `DeclarativeTool` | VERIFIED | `tools/tools.ts:623-626` |
| `CoreToolCallStatus` has 7 states | VERIFIED | `scheduler/types.ts:25-33` -- `Validating`, `Scheduled`, `Error`, `Success`, `Executing`, `Cancelled`, `AwaitingApproval` |
| `McpClientManager` exists | VERIFIED | `tools/mcp-client-manager.ts` (18 files reference it) |
| `PolicyDecision`, `PolicyRule` exist in `policy.ts` | VERIFIED | `scheduler/policy.ts:10,12` |
| `HookEventName` has 11 events | VERIFIED | `hooks/types.ts:34-46` |
| `processFunctionCalls()` exists | VERIFIED | `agents/local-executor.ts:993` |
| `shouldConfirmExecute()` callback exists | Need to verify -- plan references this as tool confirmation |

#### File Size Discrepancies (from Phase 1 but relevant to Phase 2 scope)

| File | Plan Claims | Actual |
|------|------------|--------|
| `scheduler/scheduler.ts` | ~400 | 785 (+96%) |
| `scheduler/policy.ts` | ~100 | 260 (+160%) |
| `scheduler/confirmation.ts` | ~100 | 339 (+239%) |
| `tools/tools.ts` | ~400 | 1001 (+150%) |
| `tools/tool-registry.ts` | ~200 | 762 (+281%) |
| `hooks/hookSystem.ts` | ~200 | 444 (+122%) |

These files are significantly larger than the plan estimates. The tool migration touches many of these files, and their actual complexity exceeds what the plan budgets for.

---

## 2. Discrepancies Summary

### 2.1 Critical Discrepancies

| # | Claim | Actual | Impact on Phase 2 |
|---|-------|--------|-------------------|
| D1 | Plan says 21 ToolHandlers need adapters | 26 total `impl ToolHandler` blocks | 5 `multi_agents` handlers need consideration -- are they wrapped as capabilities or deferred to Phase 4? |
| D2 | Plan says C5 needs ~420 lines for 21 adapters | With 26 handlers at ~20 LOC each = ~520 lines minimum | 24% line count underestimate |
| D3 | Gemini-CLI scheduler files are 2-3.8x larger than claimed | `policy.ts` is 260 not 100, `confirmation.ts` is 339 not 100 | Phase G8 (PolicyEngine as interceptors) integration is more complex than budgeted |
| D4 | `tools/tool-registry.ts` is 762 lines not ~200 | G6 registration bridge touches a larger codebase | Registration integration effort underestimated |

### 2.2 Structural Observations

1. **No `ToolAdapter` trait exists yet** -- The plan depends on this blanket impl (A5.1) to make consumer wrapping easy (~20 LOC per tool). Without it, each consumer writes ~60 LOC per tool. This is the critical APXM deliverable for Phase 2.

2. **No `ToolStore` persistence exists** -- Tools are currently in-memory only. The `~/.apxm/tools.toml` persistence layer (A5.2) is entirely new work.

3. **No HTTP capability bridge** -- This depends on Phase 1's `apxm-llm-service` existing first. If Phase 1 is delayed, Phase 2's Gemini-CLI integration is blocked.

4. **Interceptor infrastructure is minimal** -- The `CapabilityInterceptor` trait exists and works (verified with tests in `mod.rs:477-504`), but only test interceptors (`DenyAll`, `ObservingPost`) exist. No production-ready `SandboxInterceptor`, `AuditInterceptor`, or `CostInterceptor` has been built.

5. **`CapabilitySystem` already has approval flow** -- The `ApprovalChannel` + `ApprovalStore` + scope system is mature and well-tested. This is a strong foundation for the `GuardianInterceptor` (C6) and VERIFY mapping (G7).

---

## 3. Integration Test Recommendations

### 3.1 Phase 2 Contract Tests (Phase Boundary)

These tests verify the contract between APXM capability system and consumers:

#### APXM -> Consumer Contract

| Test | What It Verifies | Location |
|------|-----------------|----------|
| **CT-P2-1: `ToolAdapter` blanket impl** | Any struct implementing `ToolAdapter` automatically satisfies `CapabilityExecutor` | `apxm-runtime/tests/capability/` |
| **CT-P2-2: `ToolStore` round-trip** | Register tool -> write `tools.toml` -> restart -> read `tools.toml` -> tool available | `apxm-runtime/tests/capability/` |
| **CT-P2-3: Interceptor pipeline ordering** | `pre_invoke` interceptors run in registration order; `Deny` short-circuits | `apxm-runtime/tests/capability/` (partially exists) |
| **CT-P2-4: Approval escalation** | `Deny` -> `ApprovalChannel.request_approval()` -> user `Allow` overrides | `apxm-runtime/tests/capability/` (partially exists) |
| **CT-P2-5: HTTP capability registration** | `POST /v1/capabilities/register` creates a capability visible via `list_capabilities()` | `apxm-llm-service/tests/` |
| **CT-P2-6: HTTP capability invocation** | `POST /v1/capabilities/{name}/invoke` executes and returns result | `apxm-llm-service/tests/` |

#### Consumer -> APXM Contract

| Test | What It Verifies | Location |
|------|-----------------|----------|
| **CT-P2-7: Codex ToolHandler -> CapabilityExecutor** | Each of 21+ Codex adapters: `ToolHandler.handle()` args -> `CapabilityExecutor.execute()` args -> result conversion preserves semantics | `codex-rs/core/tests/apxm_adapter/` |
| **CT-P2-8: GuardianInterceptor verdict mapping** | `Verdict::Allow` -> `InterceptDecision::Allow`, `Verdict::Deny` -> `InterceptDecision::Deny`, `Verdict::RequireUserApproval` -> `InterceptDecision::Escalate`(?) | `codex-rs/core/tests/apxm_adapter/` |
| **CT-P2-9: Gemini DeclarativeTool -> HTTP registration** | Each Gemini tool registers via HTTP bridge with correct schema | `google/gemini-cli/packages/core/tests/` |
| **CT-P2-10: CoreToolCallStatus -> AIS lifecycle** | Each of 7 `CoreToolCallStatus` values maps to the correct AIS operation state | `google/gemini-cli/packages/core/tests/` |

### 3.2 Phase 2 Integration Tests (End-to-End within Phase)

| Test | What It Verifies | Mock Boundaries |
|------|-----------------|-----------------|
| **IT-P2-1: Full Codex tool round-trip** | LLM returns tool call -> APXM capability system dispatches -> ToolHandler executes -> result returned to LLM | MockBackend, real ToolHandler |
| **IT-P2-2: Guardian interceptor flow** | Tool call -> GuardianInterceptor -> risk score -> approval/deny -> tool executes or rejects | Mock GuardianReviewSessionManager |
| **IT-P2-3: `apxm tools list` shows Codex tools** | Codex registers tools at startup -> `apxm tools list --json` returns all 21+ tools with correct metadata | Real `ToolStore` in temp dir |
| **IT-P2-4: Gemini HTTP tool registration** | Gemini-CLI registers tools via HTTP -> `apxm tools list` shows them -> invoke via HTTP works | In-process service |
| **IT-P2-5: MCP tool unified registration** | MCP tools discovered by Codex/Gemini -> registered in APXM -> visible alongside built-in tools | Mock MCP server |
| **IT-P2-6: Parallel tool invocation** | N concurrent tool calls -> APXM dispatches in parallel -> all results collected | Mock tools with artificial delay |
| **IT-P2-7: Sandbox interceptor** | Tool with blocked args -> SandboxInterceptor denies -> tool does not execute | Real SandboxInterceptor |
| **IT-P2-8: Audit trail** | Tool invocation -> AuditInterceptor logs -> audit record retrievable | Real AuditInterceptor in test mode |
| **IT-P2-9: Codex tool persistence** | Register tools -> restart -> tools still registered | Temp `~/.apxm/` directory |
| **IT-P2-10: Gemini VERIFY flow** | Tool needing confirmation -> VERIFY interceptor -> user approves -> tool executes | Mock confirmation channel |

### 3.3 Mock/Stub Boundaries

| Boundary | What to Mock | Why |
|----------|-------------|-----|
| **Codex `ToolHandler` execution** | Some handlers (Shell, JsRepl) require real system state | Use safe, deterministic handlers (Echo, ReadFile with temp files) for integration tests |
| **`GuardianReviewSessionManager`** | Actual guardian calls an LLM for risk scoring | Mock with deterministic verdicts (`Allow`/`Deny`/`RequireUserApproval`) |
| **MCP servers** | MCP tools require running external processes | Mock MCP server responding to `tools/list` and `tools/call` |
| **File system (`~/.apxm/tools.toml`)** | Persist to temp directory | Avoid polluting real user config |
| **Gemini-CLI `MessageBus`** | Confirmation flow uses pub/sub with correlation IDs | Mock bus that auto-approves or auto-denies |
| **PolicyEngine** | Gemini-CLI policy rules evaluated at runtime | Mock with deterministic `PolicyDecision` responses |

### 3.4 Test Location Map

```
apxm/crates/apxm-runtime/tests/capability/
  tool_adapter.rs         -- CT-P2-1 (blanket impl)
  tool_store.rs           -- CT-P2-2 (persistence round-trip)
  interceptor_pipeline.rs -- CT-P2-3, CT-P2-4 (ordering, escalation)

apxm/crates/apxm-llm-service/tests/ (or apxm-service)
  capability_registration.rs -- CT-P2-5
  capability_invocation.rs   -- CT-P2-6

openai/codex/codex-rs/core/tests/apxm_adapter/
  tool_adapters.rs        -- CT-P2-7 (21+ adapter round-trips)
  guardian_interceptor.rs -- CT-P2-8 (verdict mapping)
  tool_registration.rs   -- IT-P2-3 (apxm tools list integration)
  mcp_registration.rs    -- IT-P2-5 (MCP unified registration)
  tool_persistence.rs    -- IT-P2-9

google/gemini-cli/packages/core/src/core/apxm/
  __tests__/
    tool-registration.test.ts  -- CT-P2-9, IT-P2-4
    tool-state-mapping.test.ts -- CT-P2-10
    verify-flow.test.ts        -- IT-P2-10
    policy-interceptor.test.ts -- G8 policy bridge
```

---

## 4. Risk Assessment

### 4.1 High Risk

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| **Phase 1 delays block Phase 2 entirely** -- `apxm-events` crate, `apxm-llm-service`, and `ToolAdapter` trait all must exist before consumers can start | HIGH | HIGH | Decouple: `ToolAdapter` trait (A5.1) only depends on `apxm-runtime` (no Phase 1 dependency). Start A5.1 in parallel with Phase 1. |
| **Tool adapter count is underestimated** -- 26 handlers vs 21 planned | HIGH | MEDIUM | Budget for 26 adapters. Decide now whether `multi_agents` handlers get Phase 2 adapters or are deferred. |
| **Gemini-CLI scheduler files are 2-4x larger than estimated** | MEDIUM | MEDIUM | The actual integration points in `scheduler.ts` (785 lines), `policy.ts` (260), `confirmation.ts` (339) are more complex. Budget extra time for G7-G8. |

### 4.2 Medium Risk

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| **`InterceptDecision` lacks `Escalate` variant** -- Plan C6 maps `GuardianReviewSessionManager` verdicts to `Allow`/`Deny`/`Escalate`, but actual enum is `Allow`/`Deny`/`EditArgs` | HIGH | MEDIUM | Either (a) use `Deny` + `ApprovalChannel` for escalation (which already works -- `mod.rs:246-273`), or (b) add an `Escalate` variant. Current code already handles the escalation path via `ApprovalChannel`. |
| **`tools/tool-registry.ts` is 762 lines (3.8x estimate)** -- Gemini-CLI tool registry is a substantial subsystem | MEDIUM | MEDIUM | Avoid modifying it in Phase 2 if possible. Instead, register APXM capabilities alongside existing registry (additive, not replacement). |
| **Read-only vs write capability locking** -- `CapabilityMetadata.read_only` exists but no locking mechanism for write capabilities during parallel dispatch | MEDIUM | LOW | Phase 2 focuses on registration, not parallel execution optimization. Defer locking to Phase 4. |

### 4.3 Low Risk

| Risk | Probability | Impact | Mitigation |
|------|------------|--------|------------|
| **`ToolStore` format stability** -- `~/.apxm/tools.toml` schema may change between phases | LOW | MEDIUM | Version the schema in the TOML file. Include a `version = 1` field. |
| **JSON Schema validation rejects valid tools** -- Some Codex handlers may have schemas that don't compile cleanly | LOW | MEDIUM | Test schema compilation for all 21+ handlers. `CapabilityRegistry.register()` already validates schemas at registration time. |
| **Existing interceptor tests are minimal** -- Only `DenyAll` and `ObservingPost` test interceptors exist | LOW | MEDIUM | Write comprehensive interceptor tests as part of A5.4 before consumer integration. |

---

## 5. Specific Recommendations

### 5.1 Resolve the ToolHandler Count

The plan consistently says "21 ToolHandler implementations" but there are 26. The 5 `multi_agents` handlers (`spawn`, `wait`, `send_input`, `resume_agent`, `close_agent`) are coordinated by `BatchJobHandler` and represent multi-agent operations. The plan maps these to `FLOW_CALL`/`WAIT_ALL` in Phase 4.

**Recommendation:** Explicitly decide and document:
- Phase 2: Wrap 21 "standard" handlers as `CapabilityExecutor` adapters
- Phase 4: Wrap 5 `multi_agents` handlers as graph-level operations
- This makes the 21 count correct for Phase 2 scope, but the total is 26

### 5.2 Start `ToolAdapter` Immediately

The `ToolAdapter` trait (A5.1) only depends on `apxm-runtime`, not on any Phase 1 deliverable. It can and should be implemented now, in parallel with Phase 1 work. This derisks the Phase 2 critical path.

### 5.3 Clarify InterceptDecision Escalation

The plan's `GuardianInterceptor` (C6) returns `InterceptDecision::Escalate`, but the actual enum has `Allow`, `Deny { reason }`, and `EditArgs { args }`. No `Escalate` variant exists.

The current `CapabilitySystem.invoke_with_timeout()` already handles the escalation pattern:
1. Interceptor returns `Deny { reason }`
2. If `ApprovalChannel` exists, route to user
3. User can override with `Allow` or confirm `Deny`

**Recommendation:** Use `Deny` + `ApprovalChannel` for the guardian flow (no enum change needed). Update the plan pseudocode to match.

### 5.4 Budget for Larger Gemini-CLI Files

Six Gemini-CLI files critical to Phase 2 are 2-4x larger than plan estimates. The total Phase 2 LOC budget for Gemini-CLI (G6-G8: ~400+ lines) may be too tight given the integration complexity of these larger files.

**Recommendation:** Add 2-3 buffer days for G6-G8 beyond the 6 days budgeted.

### 5.5 Test `tools.toml` Schema Early

Before consumers start writing to `~/.apxm/tools.toml`, define and freeze the TOML schema. The plan shows a sample but does not define:
- How consumer-specific metadata is stored (Codex vs Gemini-CLI)
- How tool types beyond `binary` are represented
- How MCP tool registrations differ from direct tools
- Schema versioning

**Recommendation:** Define `tools.toml` schema as a contract test (CT-P2-2) before any consumer writes to it.

### 5.6 Cross-Phase Dependency Chain

```
Phase 1 (A1: apxm-events)
    |
Phase 1 (A3: apxm-llm-service)  <-- Phase 2 G6-G8 depends on this
    |
Phase 2 (A5.1: ToolAdapter)     <-- Can start in parallel with Phase 1
    |
Phase 2 (A5.2: ToolStore)       <-- Needs A5.1
    |
Phase 2 (A5.3: HTTP bridge)     <-- Needs A3 + A5.1
    |
    +-- C5 (Codex adapters)      <-- Needs A5.1 only
    |
    +-- G6 (Gemini registration) <-- Needs A5.3
    |
Phase 2 (A5.4: Interceptors)    <-- Can start any time
    |
    +-- C6 (Guardian interceptor) <-- Needs A5.4
    |
    +-- G8 (Policy as interceptors) <-- Needs A5.4
```

**Key insight:** C5 (Codex tool adapters) only depends on A5.1 (`ToolAdapter` trait), not on the HTTP bridge or service. Codex can start Phase 2 as soon as A5.1 is ready, independent of Phase 1 completion. Gemini-CLI (G6) depends on the full `apxm-llm-service`/`apxm-service` because it uses the HTTP bridge.
