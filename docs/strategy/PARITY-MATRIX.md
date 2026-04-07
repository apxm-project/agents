# APXM Frontend Parity Matrix

This inventory is the Phase 0 scope boundary for the `apxm-frontend` plan.
It is based on the 51 first-party `.ais` examples currently under `examples/`
plus the embedded Python frontend surface in `external/agentmate/python/`.

## Tier Mapping

- Tier 1: `core graph`
- Tier 2: `multi-agent`, `ACP`
- Tier 3: `composition`, `advanced`

## External Python Surface Inventory

The embedded `external/agentmate/python/agentmate/` package exposes more than
the frontend parity work needs. For `.ais` replacement, the graph-authoring
surface is the critical path; the standalone agent/tooling APIs are secondary.

| Module | Public surface | Relevance to `.ais` parity |
| --- | --- | --- |
| `agentmate.graph` | `compile`, `validate_graph`, `CompiledFlow`, `ExecutionMode`, `WorkflowCheckpoint`, `ApxmGraph`, `GraphNode`, `GraphEdge`, `Parameter`, `GraphRecorder`, `NodeRef`, `FlowModule`, `AgentConfig`, `BashConfig`, `ReadConfig`, `WriteConfig`, `SearchWebConfig`, `ToolsConfig`, `constants` | Primary migration target. This is the surface needed to author, validate, compose, and run Python graphs. |
| `agentmate.graph.proxy` | `GraphRecorder.ask`, `think`, `reason`, `query_memory`, `update_memory`, `invoke`, `wait_all`, `merge`, `plan`, `reflect`, `verify`, `print_`, `return_`, `flow_call`, `communicate`, `spawn_agent`, `register_capability`, `agent`, `const_`, `yield_`, `delegate`, `negotiate`, `nop`, `identity`, plus `NodeRef` | Most load-bearing surface for example parity. The first-party `.ais` corpus directly maps to this layer. |
| `agentmate.graph.decorators` | `compile` | Required. The plan explicitly keeps Python as the authoring frontend and uses function signatures for workflow params. |
| `agentmate.graph.ir` | `ApxmGraph`, `GraphNode`, `GraphEdge`, `Parameter`, `ValidationResult`, `validate_against_apxm` | Required for serialization, merge/composition, testing, and CLI handoff. |
| `agentmate.graph.module` | `FlowModule`, `FlowModule.embed`, `FlowModule.to_graph` | Required for Tier 3 composition examples. |
| `agentmate.graph.execution` | `CompiledFlow.save/load/run_streaming/register_tool`, `WorkflowCheckpoint.save/load`, `validate_graph` | Needed for a complete Python authoring story, but not the first blocker for static parity docs. |
| `agentmate.graph.config` | `AgentConfig`, `ToolsConfig`, `BashConfig`, `ReadConfig`, `WriteConfig`, `SearchWebConfig` | Needed where examples rely on ACP agents, tool/capability registration, or shell-style side effects. |
| `agentmate.providers` | `ProviderSpec`, `resolve_provider`, `list_providers` | Nice-to-have parity for package completeness; not required by any first-party `.ais` example. |
| Top-level non-graph surface | `Agent`, `AgentResult`, `ModelSettings`, `Codelet`, `CodeletGraph`, `Step`, `RunContext`, event types/emitters, guardrails, HITL types, `PromptTemplate`, `FewShotExample`, `Supervisor`, `tool`, `FunctionTool`, `ToolRegistry`, `ToolExecutor`, `ToolResult`, `MCPServerConfig`, `mcp_server` | Not required for `.ais` example parity. Defer unless the Python frontend expands beyond graph authoring. |

## Python API Legend

The matrix below uses these shorthand labels for the required Python API:

- `compile`: `agentmate.graph.compile` / `apxm.graph.compile`
- `core`: `GraphRecorder` + `NodeRef` authoring for `ask`, `think`, `reason`, `print_`, `return_`
- `params`: function-signature params or equivalent `GraphRecorder.param` support
- `tools`: capability/tool registration plus `GraphRecorder.invoke`
- `acp`: `GraphRecorder.spawn_agent`, `GraphRecorder.communicate`, `AgentConfig`, generated agent refs
- `compose`: multiple flows/modules via `FlowModule` and `GraphRecorder.flow_call`
- `memory`: `GraphRecorder.query_memory`, `GraphRecorder.update_memory`
- `shell`: external command/tool execution, likely via `GraphRecorder.invoke` plus `BashConfig` or a repo-owned helper

## Core Graph

| Example | Dominant features | Tier | Needed Python API |
| --- | --- | --- | --- |
| `examples/basics/hello.ais` | Single `ask` entry flow | Tier 1 | `compile`, `core` |
| `examples/basics/tool_use.ais` | Capability declaration plus tool invocation | Tier 1 | `compile`, `core`, `tools` |
| `examples/patterns/iterative-refine/iterative-refine.ais` | Sequential self-critique loop unrolled as a DAG | Tier 1 | `compile`, `core` |
| `examples/patterns/plan-fan-out/plan-fan-out.ais` | Plan once, fan out parallel writes, synthesize | Tier 1 | `compile`, `core` |
| `examples/workflows/sub/adversary.ais` | Parameterized single-node `reason` sub-workflow | Tier 1 | `compile`, `core`, `params` |
| `examples/workflows/sub/architect.ais` | Parameterized single-node `think` sub-workflow | Tier 1 | `compile`, `core`, `params` |
| `examples/workflows/sub/impl-expert.ais` | Parameterized single-node implementation advisor | Tier 1 | `compile`, `core`, `params` |
| `examples/workflows/sub/synthesize.ais` | Multi-parameter synthesis sub-workflow | Tier 1 | `compile`, `core`, `params` |

## Multi-Agent

| Example | Dominant features | Tier | Needed Python API |
| --- | --- | --- | --- |
| `examples/multi-agent/apxm_council.ais` | Parallel expert branches with final synthesis | Tier 2 | `compile`, `core` |
| `examples/multi-agent/code_review_council.ais` | Parallel specialist reviews over a workflow parameter | Tier 2 | `compile`, `core`, `params` |

## ACP

| Example | Dominant features | Tier | Needed Python API |
| --- | --- | --- | --- |
| `examples/acp-agents/architect-implement-review.ais` | ACP architect -> coder -> reviewer chain | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/code-review.ais` | ACP analysis followed by ACP review | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/cross-critique-pipeline.ais` | Parallel ACP proposals plus cross-critiques | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/cross-critique.ais` | Two ACP agents critique each other’s outputs | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/dev-workflow.ais` | ACP architect/coder/reviewer loop | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/full-sdlc.ais` | ACP design -> implementation -> review handoff | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/multi-turn-communicate.ais` | Repeated ACP communication with one agent | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/parallel-agents.ais` | Parallel ACP analyses merged in-graph | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/spawn-communicate-basic.ais` | Smallest spawn + communicate example | Tier 2 | `compile`, `core`, `acp` |
| `examples/multi-agent/multi_agent_communicate.ais` | Two ACP agents in a pipeline | Tier 2 | `compile`, `core`, `acp` |
| `examples/patterns/multi-agent-negotiate/negotiate-consensus.ais` | ACP negotiation with graph-side summary | Tier 2 | `compile`, `core`, `acp` |
| `examples/patterns/resilient-acp/resilient-acp-pipeline.ais` | Spawn, format, communicate, summarize | Tier 2 | `compile`, `core`, `acp` |
| `examples/workflows/codex-claude-fix.ais` | ACP bug-fix workflow with graph-side synthesis | Tier 2 | `compile`, `core`, `acp` |
| `examples/workflows/designer/brief_analyzer.ais` | ACP brief analysis helper | Tier 2 | `compile`, `core`, `acp` |
| `examples/workflows/planner/apxm_planner.ais` | Multi-role ACP planning council | Tier 2 | `compile`, `core`, `acp` |
| `examples/workflows/quad-agent-compiler.ais` | Four spawned ACP workers plus summary | Tier 2 | `compile`, `core`, `acp` |

## Composition

| Example | Dominant features | Tier | Needed Python API |
| --- | --- | --- | --- |
| `examples/multi-agent/multi_flow.ais` | Multiple flows, cross-flow calls, memory block | Tier 3 | `compile`, `core`, `compose`, `memory` |
| `examples/workflows/ultrathink-coder.ais` | Reusable planner flows plus ACP implementation agent | Tier 3 | `compile`, `core`, `params`, `compose`, `acp` |

## Advanced

| Example | Dominant features | Tier | Needed Python API |
| --- | --- | --- | --- |
| `examples/patterns/memory-rag/memory-rag-pipeline.ais` | Recall + answer + verification + memory write-back | Tier 3 | `compile`, `core`, `memory` |
| `examples/patterns/worker-pool/worker-pool.ais` | Parallel worker branches plus memory persistence | Tier 3 | `compile`, `core`, `memory` |
| `examples/workflows/ais-writer/ais-writer.ais` | Meta-workflow that designs and writes `.ais` files | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/add-feature.ais` | ACP orchestration over app-builder artifacts | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/add-part.ais` | ACP orchestration over app-builder artifacts | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/create.ais` | End-to-end session bootstrap, file system writes, ACP phases | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/edit-screen.ais` | ACP-driven screen-edit workflow over saved artifacts | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/phases/common-parts.ais` | Generates component sets, writes files, runs ACP verification | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/phases/common-screens.ais` | Generates shared screens with artifact reads/writes | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/phases/discover.ais` | Multi-agent spec generation with artifact persistence | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/phases/feature-screens.ais` | Parallel feature-screen generation plus verification gates | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/phases/implement.ais` | Framework/backend implementation over generated artifacts | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/phases/test.ais` | Test/audit phase over generated project state | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/sub/component-widget.ais` | Per-component code generation and artifact writing | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/sub/convert.ais` | HTML -> framework conversion, file IO, judging | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/sub/export-to-figma.ais` | Artifact export helper for downstream tooling | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/sub/screen.ais` | Screen generation helper with verification-oriented outputs | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/sub/sync-from-figma.ais` | External sync plus artifact updates | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/sub/verify-structural.ais` | Whole-app structural audit workflow | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/sub/verify-visual.ais` | Whole-app visual review workflow | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/designer/phase1/discover.ais` | Parallel ACP specialists producing JSON artifacts | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/designer/phase2/generate.ais` | Component/screen generation with verification stages | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/graph-builder/graph-builder.ais` | Meta-workflow that designs other workflows | Tier 3 | `compile`, `core`, `acp` |

## Notes

- No first-party `.ais` example currently requires the standalone top-level
  `Agent`, `Supervisor`, guardrail, HITL, MCP, or prompt-template APIs.
- The highest-value parity cut is:
  1. `compile` + `GraphRecorder` core ops
  2. function-signature parameters
  3. ACP spawn/communicate
  4. composition helpers
  5. memory and shell/tool bridges
- `GraphRecorder.verify` exists in `agentmate.graph.proxy`, but the current
  `.ais` corpus does not rely on a native VERIFY node. Verification is modeled
  today through regular graph ops and ACP prompts.
