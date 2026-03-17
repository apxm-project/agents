# AgentMate Integration Status

Current state of the AgentMate ↔ A-PXM integration.

## Done

| Component | Status |
|-----------|--------|
| `ApxmGraph` IR construction and validation | Done |
| APXM pipeline `compile_graph()` (graph → MLIR → passes → artifact) | Done |
| APXM linker graph support (`compile_graph`, `run_graph`) | Done |
| AgentMate APXM helper extraction (`apxm_helpers.rs`) | Done |
| `AMAgent` APXM execution path | Done |
| `AgentBuilder` → APXM graph path | Done |
| `WorkflowBuilder` + `CompiledWorkflow` | Done |
| `WorkflowBuilder` full op surface (ask/think/reason/.../const_) | Done |
| Rust per-node multi-agent config (`AgentNodeConfig`, `ask_with`) | Done |
| Rust graph introspection helpers (`peek_graph`, `then`, `parallel`) | Done |
| Rust AOT helpers (`save_artifact`, `load_artifact`) | Done |
| `ToolCapabilityAdapter` custom-tool bridge | Done |
| `with_standard_tools()` APXM config path | Done |
| Python graph layer (`agentmate.graph` IR/config/proxy/module/decorators) | Done |
| PyO3 graph bridge (`ApxmGraph`, `WorkflowBuilder::from_graph_json`) | Done |
| PyO3 expanded workflow methods + type stubs | Done |
| `agentmate-macros` crate with `apxm_flow!` | Done |
| Python CLI-based examples | Done |
| Rust examples | Done |

## Remaining Gaps

| Gap | Priority | Notes |
|-----|----------|-------|
| Cross-language AOT round-trip (Python save → Rust load, reverse) | High | Verify artifact portability end-to-end |
| Workflow integration tests | High | Expand e2e graph/tool execution coverage |
| Streaming integration for APXM-backed path | Medium | No token streaming to am-tui yet |
| `am-cli` APXM-backed command audit | Medium | Validate chat/exec flows |
| Optional typed flow builder | Low | Type-state API not implemented yet |

## What AgentMate Provides That A-PXM Lacks

These features live in AgentMate and complement A-PXM's backend:

| Feature | AgentMate | A-PXM |
|---------|-----------|-------|
| OS-level sandbox | am-sandbox (Seatbelt/Landlock) | No sandbox implementation |
| Terminal UI | am-tui (streaming, spinners) | No UI layer |
| RAG pipeline | am-rag (embedding + retrieval) | No RAG support |
| Document parsing | am-documents (PDF, HTML, etc.) | No document handling |
| MCP integration | am-mcp (client + server) | No MCP support |
| Prompt templates | am-config (layered config) | No template system |
| CLI framework | am-cli (chat, exec, models) | apxm-cli (compile, validate, analyze) |
| Python API | am-py (PyO3) | No Python bindings |
