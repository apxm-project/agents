# APXM Frontend Parity Matrix

This inventory is the Phase 0 scope boundary for the `apxm-frontend` plan.
It is based on the 51 first-party `.py` examples currently under `examples/`
plus the embedded Python frontend surface in `crates/apxm-frontend/python/`.

## Tier Mapping

- Tier 1: `core graph`
- Tier 2: `multi-agent`, `ACP`
- Tier 3: `composition`, `advanced`

## External Python Surface Inventory

The embedded `crates/apxm-frontend/python/apxm/` package exposes more than
the frontend parity work needs. For `.py` replacement, the graph-authoring
surface is the critical path; the standalone agent/tooling APIs are secondary.

| Module | Public surface | Relevance to `.py` parity |
| --- | --- | --- |
| `apxm.graph` | `compile`, `validate_graph`, `CompiledFlow`, `ExecutionMode`, `WorkflowCheckpoint`, `ApxmGraph`, `GraphNode`, `GraphEdge`, `Parameter`, `GraphRecorder`, `NodeRef`, `FlowModule`, `AgentConfig`, `BashConfig`, `ReadConfig`, `WriteConfig`, `SearchWebConfig`, `ToolsConfig`, `constants` | Primary migration target. This is the surface needed to author, validate, compose, and run Python graphs. |
| `apxm.graph.proxy` | `GraphRecorder.ask`, `think`, `reason`, `query_memory`, `update_memory`, `invoke`, `wait_all`, `merge`, `plan`, `reflect`, `verify`, `print_`, `return_`, `flow_call`, `communicate`, `spawn_agent`, `register_capability`, `agent`, `const_`, `yield_`, `delegate`, `negotiate`, `nop`, `identity`, plus `NodeRef` | Most load-bearing surface for example parity. The first-party `.py` corpus directly maps to this layer. |
| `apxm.graph.decorators` | `compile` | Required. The plan explicitly keeps Python as the authoring frontend and uses function signatures for workflow params. |
| `apxm.graph.ir` | `ApxmGraph`, `GraphNode`, `GraphEdge`, `Parameter`, `ValidationResult`, `validate_against_apxm` | Required for serialization, merge/composition, testing, and CLI handoff. |
| `apxm.graph.module` | `FlowModule`, `FlowModule.embed`, `FlowModule.to_graph` | Required for Tier 3 composition examples. |
| `apxm.graph.execution` | `CompiledFlow.save/load/run_streaming/register_tool`, `WorkflowCheckpoint.save/load`, `validate_graph` | Needed for a complete Python authoring story, but not the first blocker for static parity docs. |
| `apxm.graph.config` | `AgentConfig`, `ToolsConfig`, `BashConfig`, `ReadConfig`, `WriteConfig`, `SearchWebConfig` | Needed where examples rely on ACP agents, tool/capability registration, or shell-style side effects. |
| `apxm.providers` | `ProviderSpec`, `resolve_provider`, `list_providers` | Nice-to-have parity for package completeness; not required by any first-party `.py` example. |
| Top-level non-graph surface | `Agent`, `AgentResult`, `ModelSettings`, `Codelet`, `CodeletGraph`, `Step`, `RunContext`, event types/emitters, guardrails, HITL types, `PromptTemplate`, `FewShotExample`, `Supervisor`, `tool`, `FunctionTool`, `ToolRegistry`, `ToolExecutor`, `ToolResult`, `MCPServerConfig`, `mcp_server` | Not required for `.py` example parity. Defer unless the Python frontend expands beyond graph authoring. |

## Python API Legend

The matrix below uses these shorthand labels for the required Python API:

- `compile`: `apxm.graph.compile` / `apxm.graph.compile`
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
| `examples/basics/hello.py` | Single `ask` entry flow | Tier 1 | `compile`, `core` |
| `examples/basics/tool_use.py` | Capability declaration plus tool invocation | Tier 1 | `compile`, `core`, `tools` |
| `examples/patterns/iterative-refine/iterative-refine.py` | Sequential self-critique loop unrolled as a DAG | Tier 1 | `compile`, `core` |
| `examples/patterns/plan-fan-out/plan-fan-out.py` | Plan once, fan out parallel writes, synthesize | Tier 1 | `compile`, `core` |
| `examples/workflows/sub/adversary.py` | Parameterized single-node `reason` sub-workflow | Tier 1 | `compile`, `core`, `params` |
| `examples/workflows/sub/architect.py` | Parameterized single-node `think` sub-workflow | Tier 1 | `compile`, `core`, `params` |
| `examples/workflows/sub/impl-expert.py` | Parameterized single-node implementation advisor | Tier 1 | `compile`, `core`, `params` |
| `examples/workflows/sub/synthesize.py` | Multi-parameter synthesis sub-workflow | Tier 1 | `compile`, `core`, `params` |

## Multi-Agent

| Example | Dominant features | Tier | Needed Python API |
| --- | --- | --- | --- |
| `examples/multi-agent/apxm_council.py` | Parallel expert branches with final synthesis | Tier 2 | `compile`, `core` |
| `examples/multi-agent/code_review_council.py` | Parallel specialist reviews over a workflow parameter | Tier 2 | `compile`, `core`, `params` |

## ACP

| Example | Dominant features | Tier | Needed Python API |
| --- | --- | --- | --- |
| `examples/acp-agents/architect-implement-review.py` | ACP architect -> coder -> reviewer chain | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/code-review.py` | ACP analysis followed by ACP review | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/cross-critique-pipeline.py` | Parallel ACP proposals plus cross-critiques | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/cross-critique.py` | Two ACP agents critique each other’s outputs | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/dev-workflow.py` | ACP architect/coder/reviewer loop | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/full-sdlc.py` | ACP design -> implementation -> review handoff | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/multi-turn-communicate.py` | Repeated ACP communication with one agent | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/parallel-agents.py` | Parallel ACP analyses merged in-graph | Tier 2 | `compile`, `core`, `acp` |
| `examples/acp-agents/spawn-communicate-basic.py` | Smallest spawn + communicate example | Tier 2 | `compile`, `core`, `acp` |
| `examples/multi-agent/multi_agent_communicate.py` | Two ACP agents in a pipeline | Tier 2 | `compile`, `core`, `acp` |
| `examples/patterns/multi-agent-negotiate/negotiate-consensus.py` | ACP negotiation with graph-side summary | Tier 2 | `compile`, `core`, `acp` |
| `examples/patterns/resilient-acp/resilient-acp-pipeline.py` | Spawn, format, communicate, summarize | Tier 2 | `compile`, `core`, `acp` |
| `examples/workflows/codex-claude-fix.py` | ACP bug-fix workflow with graph-side synthesis | Tier 2 | `compile`, `core`, `acp` |
| `examples/workflows/designer/brief_analyzer.py` | ACP brief analysis helper | Tier 2 | `compile`, `core`, `acp` |
| `examples/workflows/planner/apxm_planner.py` | Multi-role ACP planning council | Tier 2 | `compile`, `core`, `acp` |
| `examples/workflows/quad-agent-compiler.py` | Four spawned ACP workers plus summary | Tier 2 | `compile`, `core`, `acp` |

## Composition

| Example | Dominant features | Tier | Needed Python API |
| --- | --- | --- | --- |
| `examples/multi-agent/multi_flow.py` | Multiple flows, cross-flow calls, memory block | Tier 3 | `compile`, `core`, `compose`, `memory` |
| `examples/workflows/ultrathink-coder.py` | Reusable planner flows plus ACP implementation agent | Tier 3 | `compile`, `core`, `params`, `compose`, `acp` |

## Advanced

| Example | Dominant features | Tier | Needed Python API |
| --- | --- | --- | --- |
| `examples/patterns/memory-rag/memory-rag-pipeline.py` | Recall + answer + verification + memory write-back | Tier 3 | `compile`, `core`, `memory` |
| `examples/patterns/worker-pool/worker-pool.py` | Parallel worker branches plus memory persistence | Tier 3 | `compile`, `core`, `memory` |
| `examples/workflows/ais-writer/ais-writer.py` | Meta-workflow that designs and writes `.py` files | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/add-feature.py` | ACP orchestration over app-builder artifacts | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/add-part.py` | ACP orchestration over app-builder artifacts | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/create.py` | End-to-end session bootstrap, file system writes, ACP phases | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/edit-screen.py` | ACP-driven screen-edit workflow over saved artifacts | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/phases/common-parts.py` | Generates component sets, writes files, runs ACP verification | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/phases/common-screens.py` | Generates shared screens with artifact reads/writes | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/phases/discover.py` | Multi-agent spec generation with artifact persistence | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/phases/feature-screens.py` | Parallel feature-screen generation plus verification gates | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/phases/implement.py` | Framework/backend implementation over generated artifacts | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/phases/test.py` | Test/audit phase over generated project state | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/sub/component-widget.py` | Per-component code generation and artifact writing | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/sub/convert.py` | HTML -> framework conversion, file IO, judging | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/sub/export-to-figma.py` | Artifact export helper for downstream tooling | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/sub/screen.py` | Screen generation helper with verification-oriented outputs | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/sub/sync-from-figma.py` | External sync plus artifact updates | Tier 3 | `compile`, `core`, `acp`, `shell` |
| `examples/workflows/app-builder/sub/verify-structural.py` | Whole-app structural audit workflow | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/app-builder/sub/verify-visual.py` | Whole-app visual review workflow | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/designer/phase1/discover.py` | Parallel ACP specialists producing JSON artifacts | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/designer/phase2/generate.py` | Component/screen generation with verification stages | Tier 3 | `compile`, `core`, `acp` |
| `examples/workflows/graph-builder/graph-builder.py` | Meta-workflow that designs other workflows | Tier 3 | `compile`, `core`, `acp` |

## Notes

- No first-party `.py` example currently requires the standalone top-level
  `Agent`, `Supervisor`, guardrail, HITL, MCP, or prompt-template APIs.
- The highest-value parity cut is:
  1. `compile` + `GraphRecorder` core ops
  2. function-signature parameters
  3. ACP spawn/communicate
  4. composition helpers
  5. memory and shell/tool bridges
- `GraphRecorder.verify` exists in `apxm.graph.proxy`, but the current
  `.py` corpus does not rely on a native VERIFY node. Verification is modeled
  today through regular graph ops and ACP prompts.
