# Phase 2: Gemini-CLI Changes -- Tool Migration

**Draft v6 -- Revised with investigation findings**

**Source:** [Plan 3: Gemini-CLI Changes](../plan-3-gemini-changes.md), Phases G6-G8
**Timeline:** ~6 days (Weeks 9-11)
**Dependencies:** Plan 1 Phase A5 (capability system extensions, interceptor pipeline), Phase 1 A3 (`apxm-server`)
**Does NOT depend on:** Plan 2 (Codex) -- these are independent

---

## Overview

Register Gemini-CLI's tool implementations as APXM capabilities, route tool calls through APXM's `CapabilitySystem`, and gain the interceptor pipeline (approval, sandbox, audit).

### What Changes

- Tool calls go through APXM `CapabilitySystem` instead of direct `DeclarativeTool.execute()`
- Tool confirmation flow maps to VERIFY operation semantics
- `PolicyEngine` rules become APXM interceptors
- MCP tools register through APXM's MCP integration
- Tool state machine is formalized in APXM terms

### What Is NOT Changed

- `DeclarativeTool` implementations themselves (the code that runs tools stays)
- TUI rendering of tool results
- Hook system
- Chat compression
- `GeminiChat` conversation management

### Actual File Sizes (Investigation Findings)

The following files are larger than originally estimated and should be budgeted accordingly:

| File | Original Estimate | Actual Lines |
|------|-------------------|-------------|
| `scheduler/scheduler.ts` | ~400 | 785 |
| `scheduler/policy.ts` | ~100 | 260 |
| `scheduler/confirmation.ts` | ~100 | 339 |
| `tools/tools.ts` | ~400 | 1001 |
| `tools/tool-registry.ts` | ~200 | 762 |
| `hooks/hookSystem.ts` | ~200 | 444 |

These files are 2-4x larger than the original estimates. The tool migration touches many of them, and their actual complexity exceeds what was originally budgeted. Consider adding 2-3 buffer days for G6-G8.

---

## G6: Register Gemini-CLI Tools as APXM Capabilities (3 days)

### G6.1 Capability Bridge

Each `DeclarativeTool` is wrapped as an APXM `CapabilityExecutor` via HTTP registration. Gemini-CLI registers each tool at startup via `apxm-server`:

```typescript
// Pseudocode -- registers built-in + MCP tools with APXM
async function registerToolsWithApxm(
  toolRegistry: ToolRegistry,
  apxmClient: ApxmServiceClient,
): Promise<void> {
  for (const tool of toolRegistry.getAllTools()) {
    await apxmClient.registerCapability({
      name: tool.name,
      description: tool.schema.description,
      parameters: tool.schema.parameters,
      metadata: {
        read_only: isReadOnlyTool(tool),
        latency_tier: estimateToolLatency(tool),
      },
    });
  }
}
```

This uses the HTTP capability bridge from [APXM A5.3](apxm.md#a53-http-capability-bridge) (`POST /v1/capabilities/register`).

### G6.2 MCP Tools

MCP tools discovered by `McpClientManager` register through APXM's MCP integration. Instead of Gemini-CLI calling MCP servers directly, APXM's capability system handles MCP dispatch, gaining unified tool auditing and interceptor support.

### G6.3 Tool State Machine Formalization

Gemini-CLI's current 7-state tool state machine maps cleanly to APXM's capability lifecycle:

| Gemini-CLI State (`CoreToolCallStatus`) | APXM Operation | AIS Equivalent |
|----------------------------------------|----------------|----------------|
| `Validating` | Capability lookup + schema validation | Pre-INV validation |
| `AwaitingApproval` | Interceptor pipeline (approval) | **VERIFY** |
| `Scheduled` | Capability ready, awaiting executor | INV queued |
| `Executing` | `CapabilityExecutor.invoke()` running | **INV** (active) |
| `Success` | Result recorded | INV completed |
| `Error` | Error captured | INV failed (TRY_CATCH) |
| `Cancelled` | Abort signal propagated | INV cancelled |

---

## G7: Parallel Tool Calls as INV x N + WAIT_ALL (2 days)

### G7.1 Fan-out/Fan-in

Gemini-CLI's scheduler already dispatches concurrent tool calls. The APXM mapping formalizes this as:

```
Model returns N function calls
  --> N x INV operations (no edges between them --> parallel)
  --> WAIT_ALL (join barrier)
  --> results collected as tool responses
```

The scheduler's `Promise.all()` concurrency pattern becomes automatic DAG parallelism. In Phase 2, this is a conceptual mapping only -- actual graph execution happens in Phase 4.

### G7.2 Tool Confirmation as VERIFY

The `shouldConfirmExecute()` callback on `ToolInvocation` maps to a **VERIFY** operation:

```
INV(tool_a) --> VERIFY(approval_required?) --> INV(tool_a, execute)
```

The existing confirmation flow via `MessageBus` (publish request, wait for response via `correlationId`) becomes the VERIFY operation's implementation. The `ToolConfirmationOutcome` (approve/deny) maps to VERIFY's verdict.

Compare with Codex's approach: Codex wraps its `GuardianReviewSessionManager` as a `GuardianInterceptor` (see [codex.md](codex.md), C6). Both consumers converge on the same VERIFY semantics through different interceptor implementations.

---

## G8: PolicyEngine Rules as Interceptors (1 day)

### G8.1 Policy Bridge

Gemini-CLI's `checkPolicy()` function (in `scheduler/policy.ts`) evaluates `PolicyRule` instances against tool calls. These map to APXM interceptor pipeline entries:

| PolicyEngine Concept | APXM Interceptor |
|---------------------|-------------------|
| `PolicyDecision.ALLOW` | Interceptor passes |
| `PolicyDecision.DENY` | Interceptor rejects (typed error) |
| `PolicyDecision.ASK_USER` | Interceptor triggers VERIFY |
| `ApprovalMode` per tool | Interceptor configuration |
| `PolicyRule.denyMessage` | Interceptor error payload |

The interceptor pipeline provides audit logging for free -- every policy decision is recorded in the execution trace. This uses the interceptor infrastructure from [APXM A5.4](apxm.md#a54-interceptor-pipeline-refinement).

---

## Phase 2 Summary

| Task | Days | Key Deliverable |
|------|------|----------------|
| G6: Capability registration | 3 | All tools registered as APXM capabilities |
| G7: Parallel calls + VERIFY | 2 | Fan-out/fan-in formalized, confirmation as VERIFY |
| G8: PolicyEngine as interceptors | 1 | Policy rules mapped to interceptor pipeline |
| **Total** | **6** | -- |

---

## Phase 2 Validation

- [ ] All `DeclarativeTool` implementations registered as APXM capabilities
- [ ] MCP-discovered tools registered through APXM capability system
- [ ] Tool confirmation flow works through VERIFY semantics
- [ ] PolicyEngine decisions recorded in APXM audit trail
- [ ] `~/.apxm/tools.toml` persists Gemini-CLI tool registrations
- [ ] Tool execution latency not measurably worse than direct dispatch

---

## Cross-References

- **APXM capability extensions:** [apxm.md](apxm.md) -- HTTP bridge (A5.3), interceptor pipeline (A5.4), ToolStore (A5.2)
- **Codex tool migration:** [codex.md](codex.md) -- independent but parallel effort with same VERIFY semantics
- **Phase overview:** [README.md](README.md)
- **Full plan:** [Plan 3: Gemini-CLI Changes](../plan-3-gemini-changes.md)
