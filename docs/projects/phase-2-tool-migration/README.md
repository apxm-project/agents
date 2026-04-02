# Phase 2: Tool Migration

**Draft v6 -- Revised with investigation findings**

**Timeline:** Weeks 9-14
**Dependencies:** Phase 1 (LLM Backend) complete across all three plans
**Status:** Not started

---

## Goal

Register all consumer tools -- Codex's 21 standard `ToolHandler` implementations (26 total; 5 `multi_agents` handlers deferred to Phase 4) and Gemini-CLI's `DeclarativeTool` instances -- as APXM `CapabilityExecutor` instances. Route tool dispatch through APXM's capability system with the interceptor pipeline (approval, sandbox, audit) instead of consumer-specific ad-hoc dispatch.

After Phase 2, every tool call in every consumer flows through a single typed pipeline: schema validation, interceptor checks (approval, sandbox, cost), execution, and audit logging. Consumers retain their tool implementation code but delegate dispatch and policy enforcement to APXM.

**Universal tool registration.** The `ToolAdapter` trait and `CapabilitySystem` are framework-agnostic. Any agent framework can register its tools via the Rust trait (for Rust consumers) or HTTP bridge (for non-Rust consumers). This makes APXM a universal tool dispatch substrate.

---

## What Each Consumer Delivers

### APXM (Plan 1, Phase A5)

Capability system extensions that enable consumer tool registration:

| Deliverable | Location |
|-------------|----------|
| `ToolAdapter` trait + blanket `CapabilityExecutor` impl | `apxm-runtime/src/capability/adapter.rs` |
| `ToolStore` persistence (`~/.apxm/tools.toml`) | `apxm-runtime/src/capability/store.rs` |
| HTTP capability bridge (`POST /v1/capabilities/`) | `apxm-server/src/routes/capabilities.rs` |
| Sandbox/Audit/Cost interceptors | `apxm-runtime/src/capability/interceptor/` |
| CLI `apxm tool` commands (register, list, show, remove, test, doctor) | `apxm-cli/src/commands/tools.rs` |

See [apxm.md](apxm.md) for full details.

### Codex (Plan 2, Phases C5-C7)

21 standard tool adapters and guardian-based approval flow (5 `multi_agents` handlers deferred to Phase 4):

| Deliverable | Location |
|-------------|----------|
| 21 `CapabilityExecutor` adapter structs | `core/src/apxm_adapter/tool_adapters.rs` |
| `register_codex_tools()` registration function | `core/src/apxm_adapter/tool_registration.rs` |
| `GuardianInterceptor` (wraps `GuardianReviewSessionManager`) | `core/src/apxm_adapter/guardian_interceptor.rs` |
| Tool persistence to `~/.apxm/tools.toml` | Startup registration code |
| MCP tool registration via `CapabilitySystem` | MCP handler integration |

See [codex.md](codex.md) for full details.

### Gemini-CLI (Plan 3, Phases G6-G8)

Capability registration, VERIFY mapping, and policy bridge:

| Deliverable | Key Change |
|-------------|-----------|
| HTTP capability registration for all `DeclarativeTool` instances | Tools registered at startup via APXM service |
| MCP tools routed through APXM capability system | Unified auditing and interceptor support |
| Tool state machine formalized (7 states mapped to AIS operations) | `CoreToolCallStatus` mapped to INV/VERIFY lifecycle |
| Parallel tool calls as INV x N + WAIT_ALL | Fan-out/fan-in formalized as DAG parallelism |
| Tool confirmation as VERIFY operation | `shouldConfirmExecute()` mapped to VERIFY semantics |
| `PolicyEngine` rules as APXM interceptors | `PolicyDecision` mapped to interceptor decisions |

See [gemini-cli.md](gemini-cli.md) for full details.

---

## Timeline

| Week | APXM (A5) | Codex (C5-C7) | Gemini-CLI (G6-G8) |
|------|-----------|---------------|---------------------|
| 9-10 | ToolAdapter trait, blanket impl, ToolStore | C5: 26 ToolHandler adapters (21 standard + 5 multi_agents; multi_agents deferred to Phase 4) | G6: Capability registration bridge |
| 11-12 | HTTP capability bridge, interceptor pipeline | C6: Guardian interceptor | G7: Parallel calls + VERIFY mapping |
| 13 | CLI `apxm tool` commands | C7: Tool persistence + MCP | G8: PolicyEngine as interceptors |
| 14 | Integration testing | Integration testing | Integration testing |

---

## Validation Criteria

Combined acceptance criteria from all three plans:

### APXM (A5)

- [ ] All 26 Codex `ToolHandler` implementations (21 standard + 5 multi_agents) registerable via `ToolAdapter` (~20 LOC each). The 5 multi_agents handlers (spawn, wait, send_input, resume_agent, close_agent) are deferred to Phase 4 as `FLOW_CALL`/`WAIT_ALL` graph operations.
- [ ] Gemini-CLI tools registerable via HTTP bridge
- [ ] Registrations persist in `~/.apxm/tools.toml`
- [ ] Interceptor pipeline (approval + sandbox + audit) works end-to-end
- [ ] `apxm tool list --json` returns all registered capabilities

### Codex (C5-C7)

- [ ] All 21 Codex `ToolHandler` types wrapped as `CapabilityExecutor` adapters
- [ ] `GuardianInterceptor` delegates to existing `GuardianReviewSessionManager`
- [ ] VERIFY semantics map to guardian risk-score verdicts
- [ ] Codex tool registrations persist to `~/.apxm/tools.toml`
- [ ] MCP-discovered tools registered via `CapabilitySystem`
- [ ] `apxm tool list --json` includes all Codex-registered tools
- [ ] Round-trip conversion tests pass for each handler

### Gemini-CLI (G6-G8)

- [ ] All `DeclarativeTool` implementations registered as APXM capabilities
- [ ] MCP-discovered tools registered through APXM capability system
- [ ] Tool confirmation flow works through VERIFY semantics
- [ ] PolicyEngine decisions recorded in APXM audit trail
- [ ] `~/.apxm/tools.toml` persists Gemini-CLI tool registrations
- [ ] Tool execution latency not measurably worse than direct dispatch

---

## Effort Summary

| Consumer | Steps | Days | New Files | Modified Files | New Lines (est.) |
|----------|-------|------|-----------|----------------|-----------------|
| APXM (A5) | Capability extensions | 15-20 | 5+ | 3+ | ~600+ |
| Codex (C5-C7) | Tool adapters + guardian + persistence | 6 | 3 | 4 | ~1020 |
| Gemini-CLI (G6-G8) | Registration + VERIFY + policy | 6 | 2+ | 3+ | ~400+ |
| **Total** | | **~28-32** | **~10** | **~10** | **~2020+** |

---

## Cross-Phase Dependency Chain

```
Phase 1 (A1: apxm-events)
    |
Phase 1 (A3: apxm-server)        <-- Phase 2 G6-G8 depends on this
    |
Phase 2 (A5.1: ToolAdapter)      <-- Can start in parallel with Phase 1
    |
Phase 2 (A5.2: ToolStore)        <-- Needs A5.1
    |
Phase 2 (A5.3: HTTP bridge)      <-- Needs A3 + A5.1
    |
    +-- C5 (Codex adapters)       <-- Needs A5.1 only
    |
    +-- G6 (Gemini registration)  <-- Needs A5.3
    |
Phase 2 (A5.4: Interceptors)     <-- Can start any time
    |
    +-- C6 (Guardian interceptor) <-- Needs A5.4
    |
    +-- G8 (Policy as interceptors) <-- Needs A5.4
```

**Key insight:** C5 (Codex tool adapters) only depends on A5.1 (`ToolAdapter` trait), not on the HTTP bridge or service. Codex can start Phase 2 as soon as A5.1 is ready, independent of Phase 1 completion. Gemini-CLI (G6) depends on the full `apxm-server` because it uses the HTTP bridge.

---

## Test Strategy

### Contract Tests (Phase Boundary)

These tests verify the contract between APXM capability system and consumers:

| Test | What It Verifies |
|------|-----------------|
| **CT-P2-1:** `ToolAdapter` blanket impl | Any struct implementing `ToolAdapter` automatically satisfies `CapabilityExecutor` |
| **CT-P2-2:** `ToolStore` round-trip | Register tool -> write `tools.toml` -> restart -> read `tools.toml` -> tool available |
| **CT-P2-3:** Interceptor pipeline ordering | `pre_invoke` interceptors run in registration order; `Deny` short-circuits |
| **CT-P2-4:** Approval escalation | `Deny` -> `ApprovalChannel.request_approval()` -> user `Allow` overrides |
| **CT-P2-5:** HTTP capability registration | `POST /v1/capabilities/register` creates a capability visible via `list_capabilities()` |
| **CT-P2-6:** HTTP capability invocation | `POST /v1/capabilities/{name}/invoke` executes and returns result |
| **CT-P2-7:** Codex ToolHandler -> CapabilityExecutor | Each of 21 Codex adapters: args -> execute -> result conversion preserves semantics |
| **CT-P2-8:** GuardianInterceptor verdict mapping | `Verdict::Allow` -> `Allow`, `Verdict::Deny` -> `Deny`, `RequireUserApproval` -> `Deny` + `ApprovalChannel` override |
| **CT-P2-9:** Gemini DeclarativeTool -> HTTP registration | Each Gemini tool registers via HTTP bridge with correct schema |
| **CT-P2-10:** CoreToolCallStatus -> AIS lifecycle | Each of 7 status values maps to the correct AIS operation state |

### Integration Tests (End-to-End within Phase)

| Test | What It Verifies |
|------|-----------------|
| **IT-P2-1:** Full Codex tool round-trip | LLM returns tool call -> APXM dispatches -> ToolHandler executes -> result returned |
| **IT-P2-2:** Guardian interceptor flow | Tool call -> GuardianInterceptor -> risk score -> approval/deny -> executes or rejects |
| **IT-P2-3:** `apxm tool list` shows Codex tools | Codex registers tools at startup -> `apxm tool list --json` returns all 26 tools (21 standard + 5 multi_agents) |
| **IT-P2-4:** Gemini HTTP tool registration | Gemini-CLI registers tools via HTTP -> `apxm tool list` shows them -> invoke works |
| **IT-P2-5:** MCP tool unified registration | MCP tools discovered by Codex/Gemini -> registered in APXM -> visible alongside built-ins |
| **IT-P2-6:** Parallel tool invocation | N concurrent tool calls -> APXM dispatches in parallel -> all results collected |
| **IT-P2-7:** Sandbox interceptor | Tool with blocked args -> SandboxInterceptor denies -> tool does not execute |
| **IT-P2-8:** Audit trail | Tool invocation -> AuditInterceptor logs -> audit record retrievable |
| **IT-P2-9:** Codex tool persistence | Register tools -> restart -> tools still registered |
| **IT-P2-10:** Gemini VERIFY flow | Tool needing confirmation -> VERIFY interceptor -> user approves -> tool executes |

### Test Location Map

```
apxm/crates/apxm-runtime/tests/capability/
  tool_adapter.rs         -- CT-P2-1 (blanket impl)
  tool_store.rs           -- CT-P2-2 (persistence round-trip)
  interceptor_pipeline.rs -- CT-P2-3, CT-P2-4 (ordering, escalation)

apxm/crates/apxm-server/tests/
  capability_registration.rs -- CT-P2-5
  capability_invocation.rs   -- CT-P2-6

openai/codex/codex-rs/core/tests/apxm_adapter/
  tool_adapters.rs        -- CT-P2-7 (21 standard adapter round-trips; 5 multi_agents deferred to Phase 4)
  guardian_interceptor.rs -- CT-P2-8 (verdict mapping)
  tool_registration.rs   -- IT-P2-3 (apxm tool list integration)
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

## Risk Matrix

| Severity | Risk | Mitigation |
|----------|------|------------|
| **HIGH** | Phase 1 delays block Phase 2 entirely (`apxm-events`, `apxm-server`, `ToolAdapter` must exist) | Decouple: A5.1 (`ToolAdapter`) only depends on `apxm-runtime` (no Phase 1 dependency). Start in parallel. C5 only needs A5.1. |
| **HIGH** | Tool adapter count underestimated (26 total handlers, not 21) | Budget for 26. Explicitly defer 5 `multi_agents` handlers to Phase 4 (FLOW_CALL/WAIT_ALL). |
| **MEDIUM** | Gemini-CLI files 2-4x larger than estimated (scheduler.ts 785, policy.ts 260, confirmation.ts 339, tools.ts 1001, tool-registry.ts 762, hookSystem.ts 444) | Budget 2-3 extra days for G6-G8. |
| **MEDIUM** | `InterceptDecision` lacks `Escalate` variant | Use existing `Deny` + `ApprovalChannel` pattern (already implemented and tested). |
| **LOW** | `tools.toml` schema stability between phases | Version the schema (`version = 1`). Freeze before consumers write. |

---

## Consumer Plan Files

- [apxm.md](apxm.md) -- APXM capability system extensions (Plan 1, Phase A5)
- [codex.md](codex.md) -- Codex tool migration (Plan 2, Phases C5-C7)
- [gemini-cli.md](gemini-cli.md) -- Gemini-CLI tool migration (Plan 3, Phases G6-G8)

## Source Plans

- [Plan 1: APXM Changes](../plan-1-apxm-changes.md)
- [Plan 2: Codex Changes](../plan-2-codex-changes.md)
- [Plan 3: Gemini-CLI Changes](../plan-3-gemini-changes.md)
