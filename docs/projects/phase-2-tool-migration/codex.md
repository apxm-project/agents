# Phase 2: Codex Changes -- Tool Migration

**Draft v6 -- Revised with investigation findings**

**Source:** [Plan 2: Codex Changes](../plan-2-codex-changes.md), Phases C5-C7
**Timeline:** ~6 days (Weeks 9-11)
**Dependencies:** Plan 1 Phase A5 (`CapabilitySystem` with interceptor pipeline, `CapabilityExecutor` trait, `~/.apxm/tools.toml` persistence)
**Does NOT depend on:** Plan 3 (Gemini-CLI) -- these are independent

---

## Overview

Codex has 26 total `ToolHandler` implementations: 21 standard handlers and 5 `multi_agents` handlers (`spawn`, `wait`, `send_input`, `resume_agent`, `close_agent`). Phase 2 wraps the 21 standard handlers as `CapabilityExecutor` adapters. The 5 `multi_agents` handlers are deferred to Phase 4 where they map to FLOW_CALL/WAIT_ALL graph-level operations.

Tool approval flows through APXM's interceptor pipeline instead of direct `GuardianReviewSessionManager` calls.

After this phase:
- Every standard Codex tool call goes through APXM's typed capability system
- Guardian approval is an interceptor, not a hardwired call
- Tool registrations persist in `~/.apxm/tools.toml`
- MCP-discovered tools are unified with built-in tools

---

## C5: ToolHandler to CapabilityExecutor Adapters (3 days)

Each of Codex's 21 `ToolHandler` implementations gets a thin adapter that wraps the existing handler as an APXM `CapabilityExecutor`:

```rust
use apxm_runtime::capability::{
    executor::{CapabilityExecutor, CapabilityResult},
    metadata::CapabilityMetadata,
};
use apxm_core::types::values::Value;
use async_trait::async_trait;

/// Thin adapter: wraps an existing Codex ToolHandler as an APXM capability.
pub struct ShellToolAdapter {
    handler: ShellHandler,
    metadata: CapabilityMetadata,
}

#[async_trait]
impl CapabilityExecutor for ShellToolAdapter {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        // Convert APXM Value args -> JSON string (ToolHandler expects &str arguments)
        let arguments = serde_json::to_string(&args)?;
        // Delegate to existing handler
        let result = self.handler.handle(/* invocation */).await;
        // Convert ToolOutput -> APXM Value
        Ok(tool_output_to_value(result))
    }

    fn metadata(&self) -> &CapabilityMetadata {
        &self.metadata
    }
}
```

Each adapter is ~20 lines of boilerplate (thanks to the `ToolAdapter` blanket impl from [APXM A5.1](apxm.md#a51-adapter-trait-for-consumer-tools)). A `register_codex_tools()` function registers all 21:

```rust
pub fn register_codex_tools(
    system: &CapabilitySystem,
    session: &Session,
) -> Result<(), RuntimeError> {
    system.register(Arc::new(ShellToolAdapter::new(&session)))?;
    system.register(Arc::new(ReadFileAdapter::new(&session)))?;
    system.register(Arc::new(ApplyPatchAdapter::new(&session)))?;
    system.register(Arc::new(McpAdapter::new(&session)))?;
    // ... remaining 17 handlers
    Ok(())
}
```

### C5 Deliverables

| Deliverable | Location | Lines (est.) |
|-------------|----------|-------------|
| 21 standard adapter structs (5 multi_agents deferred to Phase 4) | `core/src/apxm_adapter/tool_adapters.rs` | ~420 |
| Registration function | `core/src/apxm_adapter/tool_registration.rs` | ~80 |
| Round-trip conversion unit tests | `core/src/apxm_adapter/tests/` | ~300 |

---

## C6: Approval Flow through Interceptor Pipeline (2 days)

Replace direct `GuardianReviewSessionManager` calls with an APXM `CapabilityInterceptor` that delegates to the existing guardian.

**InterceptDecision enum.** The actual `InterceptDecision` enum has three variants: `Allow`, `Deny { reason }`, and `EditArgs { args }`. There is no `Escalate` variant. Escalation is handled by the existing `ApprovalChannel` mechanism: when an interceptor returns `Deny`, `CapabilitySystem.invoke_with_timeout()` checks whether an `ApprovalChannel` is configured. If so, the denial is routed to the user via `ApprovalChannel.request_approval()`, and the user can override with `Allow` or confirm the `Deny`. This flow is already implemented and tested in `mod.rs:246-273`.

```rust
pub struct GuardianInterceptor {
    guardian: Arc<GuardianReviewSessionManager>,
}

#[async_trait]
impl CapabilityInterceptor for GuardianInterceptor {
    fn name(&self) -> &str { "codex-guardian" }

    async fn pre_invoke(
        &self, capability: &str, args: &HashMap<String, Value>,
    ) -> InterceptDecision {
        let verdict = self.guardian.review(capability, args).await;
        match verdict {
            Verdict::Allow => InterceptDecision::Allow,
            Verdict::Deny(reason) => InterceptDecision::Deny { reason },
            // RequireUserApproval maps to Deny; the ApprovalChannel
            // in CapabilitySystem handles user override automatically.
            // Flow: Deny -> ApprovalChannel.request_approval() ->
            //   user Allow (overrides) or user Deny (confirms).
            Verdict::RequireUserApproval => InterceptDecision::Deny {
                reason: "Guardian requires user approval".into(),
            },
        }
    }
}
```

The VERIFY AIS operation maps to this interceptor: the guardian's risk score IS the verification evidence, and its verdict IS the VERIFY output. When the guardian returns `RequireUserApproval`, the `Deny` + `ApprovalChannel` pattern routes the decision to the user -- achieving escalation without a dedicated enum variant.

This integrates with the interceptor pipeline from [APXM A5.4](apxm.md#a54-interceptor-pipeline-refinement). The `GuardianInterceptor` is registered via `CapabilitySystem::register_interceptor()` during session initialization.

### C6 Deliverables

| Deliverable | Location | Lines (est.) |
|-------------|----------|-------------|
| Interceptor implementation | `core/src/apxm_adapter/guardian_interceptor.rs` | ~60 |
| Integration with `CapabilitySystem::register_interceptor()` | Session init code | ~60 |

---

## C7: Tool Persistence and MCP Registration (1 day)

- Codex's 26 tool registrations (21 standard + 5 multi_agents) persist to `~/.apxm/tools.toml` so they are visible to `apxm tool list`. The 5 multi_agents handlers (spawn, wait, send_input, resume_agent, close_agent) are deferred to Phase 4 as `FLOW_CALL`/`WAIT_ALL` graph operations.
- MCP tools discovered by `McpHandler` and `McpResourceHandler` are registered as APXM MCP capabilities
- `apxm tool list --json` includes Codex-registered tools

The persistence layer is provided by [APXM A5.2](apxm.md#a52-tool-persistence-layer) (`ToolStore`).

### C7 Deliverables

| Deliverable | Description |
|-------------|------------|
| Startup registration | Writes Codex tool metadata to `~/.apxm/tools.toml` |
| MCP tool registration | MCP-discovered tools registered via `CapabilitySystem::register()` |
| Integration test | `apxm tool list` shows all Codex tools |

---

## Phase 2 Summary

| Step | Days | New Files | Modified Files | New Lines (est.) |
|------|------|-----------|----------------|-----------------|
| C5: Tool adapters | 3 | 2 | 1 | ~800 |
| C6: Guardian interceptor | 2 | 1 | 1 | ~120 |
| C7: Tool persistence | 1 | 0 | 2 | ~100 |
| **Total** | **6** | **3** | **4** | **~1020** |

---

## Cross-References

- **APXM capability extensions:** [apxm.md](apxm.md) -- ToolAdapter trait (A5.1), ToolStore (A5.2), interceptors (A5.4)
- **Gemini-CLI tool migration:** [gemini-cli.md](gemini-cli.md) -- independent but parallel effort
- **Phase overview:** [README.md](README.md)
- **Full plan:** [Plan 2: Codex Changes](../plan-2-codex-changes.md)
