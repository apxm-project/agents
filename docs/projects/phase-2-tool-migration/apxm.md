# Phase 2: APXM Changes -- Capability System Extensions

**Draft v6 -- Revised with investigation findings**

**Source:** [Plan 1: APXM Changes](../plan-1-apxm-changes.md), Phase A5
**Timeline:** ~3-4 weeks (Weeks 9-12)
**Dependencies:** Phase 1 (A0-A4) complete
**Blocks:** Codex tool migration (C5-C7), Gemini-CLI tool migration (G6-G8)

---

## Overview

Phase A5 extends the APXM capability system to support registration of **consumer tools** -- Codex's 21 `ToolHandler` implementations and Gemini-CLI's `DeclarativeTool` instances -- and persists those registrations across sessions.

The capability system already exists and is more mature than one might expect:

- **`CapabilityExecutor` trait** (`apxm-runtime/src/capability/executor.rs`): `async fn execute(args) -> Value` + `fn metadata() -> &CapabilityMetadata`
- **`CapabilityMetadata`** (`apxm-runtime/src/capability/metadata.rs`): Already has `name`, `description`, `parameters_schema`, `returns`, `cost_estimate`, `latency_estimate_ms`, `requires_auth`, `tags`, `read_only`, `metadata` fields
- **`CapabilityRegistry`** (`apxm-runtime/src/capability/registry.rs`): `DashMap`-backed concurrent registry with `register`, `get`, `list_names`, `list_metadata`, `unregister`
- **`CapabilitySystem`** (`apxm-runtime/src/capability/mod.rs`): Full coordinator with JSON Schema validation, timeout enforcement, interceptor pipeline (`pre_invoke`/`post_invoke`), approval channel, AAM integration
- **Four built-in tools** (`apxm-tools/src/`): `BashCapability`, `ReadCapability`, `WriteCapability`, `SearchWebCapability`

What is missing is the infrastructure to register consumer tools and persist those registrations.

---

## A5.1 Adapter Trait for Consumer Tools

**Status: Entirely new work.** No `ToolAdapter` trait exists today. The only abstraction is `CapabilityExecutor`, which requires ~60 LOC per tool. The blanket impl introduced here reduces that to ~20 LOC per tool. Without it, each consumer writes 3x more boilerplate for every adapter.

Consumers need a lightweight way to wrap existing tool implementations as APXM capabilities:

```rust
/// Simplifies wrapping foreign tool handlers as CapabilityExecutors.
/// Consumer implements this; APXM provides the CapabilityExecutor wrapper.
pub trait ToolAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    fn execute(&self, args: serde_json::Value) -> BoxFuture<'_, Result<serde_json::Value, String>>;
}
```

A blanket `impl CapabilityExecutor for T where T: ToolAdapter` bridges the gap -- consumers write ~20 LOC per tool, not ~60.

**Why this matters for consumers:**
- Codex wraps each of its 21 `ToolHandler` implementations in ~20 lines (see [codex.md](codex.md), C5)
- Gemini-CLI registers tools via the HTTP bridge (see [gemini-cli.md](gemini-cli.md), G6) which uses `ToolAdapter` internally

---

## A5.2 Tool Persistence Layer

**Status: Entirely new work.** No persistence layer for tool registrations exists today. Tools are in-memory only via `CapabilityRegistry` (`DashMap`-backed). The `ToolStore` and `~/.apxm/tools.toml` file format must be built from scratch.

**New file:** `~/.apxm/tools.toml`

Registered tools persist across sessions:

```toml
version = 1  # Schema version -- freeze before consumers start writing

[tools.shell]
type = "binary"
command = "bash"
description = "Execute shell commands"
schema = '{"type":"object","properties":{"command":{"type":"string"}}}'
read_only = false

[tools.filesystem_read]
type = "binary"
command = "cat"
description = "Read file contents"
read_only = true
```

**Schema versioning:** The `version = 1` field must be present in every `tools.toml` file. The schema must be frozen before consumers (Codex, Gemini-CLI) start writing to it. Schema changes require a version bump and migration logic in `ToolStore`.

**`ToolStore`** -- load from / save to `~/.apxm/tools.toml`, produces `Vec<CapabilityMetadata>` for registration at startup.

Both Codex and Gemini-CLI write their tool registrations to this file so that `apxm tool list` shows a unified view of all registered capabilities regardless of which consumer registered them.

---

## A5.3 HTTP Capability Bridge

For non-Rust tools (Gemini-CLI's TypeScript tools), `apxm-server` exposes a capability invocation endpoint:

```
POST /v1/capabilities/{name}/invoke
```

And a registration endpoint:

```
POST /v1/capabilities/register
```

This allows Gemini-CLI to register its tools at startup and invoke them through the APXM capability system. See [gemini-cli.md](gemini-cli.md), G6.1 for how Gemini-CLI uses this bridge.

---

## A5.4 Interceptor Pipeline Refinement

The interceptor pipeline infrastructure exists (`approval.rs`, `interceptor.rs`) and the `CapabilityInterceptor` trait works (verified by tests). However, **no production interceptors exist today** -- only test interceptors (`DenyAll`, `ObservingPost`) have been implemented. The following are all new work:

- **SandboxInterceptor**: Validate tool args against sandbox policy before execution
- **AuditInterceptor**: Log every capability invocation with args and result to episodic memory
- **CostInterceptor**: Track cumulative cost estimates and enforce budgets

These interceptors are used by both consumers:
- Codex's `GuardianReviewSessionManager` is wrapped as a `GuardianInterceptor` (see [codex.md](codex.md), C6)
- Gemini-CLI's `PolicyEngine` rules map to interceptor pipeline entries (see [gemini-cli.md](gemini-cli.md), G8)

---

## A5.5 CLI Commands

Extend the existing `apxm tool` CLI:

```bash
apxm tool add my-tool --type binary --command ./my-tool --description "..."
apxm tool list [--json]
apxm tool show my-tool [--json]
apxm tool remove my-tool
apxm tool test my-tool
apxm tool doctor           # validate all registrations
```

---

## A5.6 Deliverables

| Deliverable | Location |
|-------------|----------|
| `ToolAdapter` trait + blanket impl | `apxm-runtime/src/capability/adapter.rs` |
| `ToolStore` persistence | `apxm-runtime/src/capability/store.rs` |
| HTTP capability bridge routes | `apxm-server/src/routes/capabilities.rs` |
| Sandbox/Audit/Cost interceptors | `apxm-runtime/src/capability/interceptor/` |
| CLI `tools` commands | `apxm-cli/src/commands/tools.rs` (already scaffolded) |

---

## A5.7 Acceptance Criteria

- [ ] All 21 Codex ToolHandlers registerable via `ToolAdapter` (~20 LOC each)
- [ ] Gemini-CLI tools registerable via HTTP bridge
- [ ] Registrations persist in `~/.apxm/tools.toml`
- [ ] Interceptor pipeline (approval + sandbox + audit) works end-to-end
- [ ] `apxm tool list --json` returns all registered capabilities

---

## Cross-References

- **Codex tool adapters:** [codex.md](codex.md) -- C5 uses `ToolAdapter` trait, C6 uses interceptor pipeline
- **Gemini-CLI registration:** [gemini-cli.md](gemini-cli.md) -- G6 uses HTTP bridge, G8 uses interceptor pipeline
- **Phase overview:** [README.md](README.md)
- **Full plan:** [Plan 1: APXM Changes](../plan-1-apxm-changes.md)
- **Tool CLI design spec:** [tools-cli-design.md](../../cli/tools-cli-design.md)
