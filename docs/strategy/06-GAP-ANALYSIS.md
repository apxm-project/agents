# Gap Analysis: Current State vs Target State

**Date**: March 31, 2026
**Scope**: What exists, what's missing, and what needs to change

---

## APXM Gaps

### Gap A1: Static Model Assignment

**Current**: Model is set at graph authoring time via `model` attribute on ASK/THINK/REASON nodes. If the model is unavailable, execution fails.

**Evidence**: `apxm-runtime/src/executor/handlers/llm.rs` extracts `model` from `node.attributes` and passes it directly to `LLMRegistry.get()`.

**Required**:
- ModelRouter module with health-aware resolution
- New `model_policy` attribute (replaces or supplements `model`)
- Circuit breaker pattern for backend failures
- `~/.apxm/models.toml` configuration file

**Effort**: 2-3 weeks

---

### Gap A2: No Context Management Beyond Memory System

**Current**: APXM has a 3-tier memory system (STM/LTM/Episodic) but no notion of context assembly per node. Each handler receives raw inputs from the token dependency map -- there's no mechanism to assemble, filter, or scope context per operation.

**Evidence**: Handler signature is `execute(ctx, node, inputs: Vec<Value>)`. The `inputs` are whatever upstream nodes produced -- no filtering, no manifests, no context budget.

**Required**:
- ContextStack data structure with spaghetti-stack organization
- Per-node context manifests (new graph attribute)
- Context assembly function (bottom-up/top-down traversal)
- Demand-paged loading from LTM

**Effort**: 4-5 weeks

---

### Gap A3: No Memoization

**Current**: Every node is executed fresh, even with identical inputs. Repeated pipeline runs re-invoke the same LLM calls.

**Evidence**: `DataflowScheduler::execute()` dispatches every ready node without checking for cached results.

**Required**:
- MemoCache (two-tier: session DashMap + persistent SQLite)
- CacheKey: hash of (inputs + operation + model + temperature)
- Integration in dispatcher before handler invocation
- `memoizable: true` attribute on nodes

**Effort**: 2 weeks

---

### Gap A4: No Speculative Execution

**Current**: DataflowScheduler waits for ALL input tokens before dispatching a node. No mechanism to start a node early with predicted inputs.

**Evidence**: `DataflowScheduler` checks `state.all_inputs_ready(node_id)` before adding to ready set.

**Required**:
- SpeculativeExecutor with commit/rollback semantics
- Integration with MemoCache for predictions
- Confidence thresholds per node
- Metrics for speculation hit rate

**Effort**: 3 weeks (depends on memoization)

---

### Gap A5: No Token Pipelining

**Current**: LLM handlers wait for complete response before returning. No streaming between adjacent LLM nodes.

**Evidence**: `llm::execute()` calls `backend.generate(request).await` which returns complete response.

**Required**:
- Streaming generate support in LLMBackend trait
- Pipeline coordination between adjacent LLM nodes
- Pre-materializable context identification
- vLLM incremental prefill support

**Effort**: 4 weeks (depends on vLLM extension)

---

### Gap A6: No Graph Import from External Formats

**Current**: APXM only accepts its own ApxmGraph JSON format. No import from ACPX flows or other workflow systems.

**Evidence**: `apxm compile` takes `.apxm` files only. No `apxm import` command.

**Required**:
- TypeScript tool to parse ACPX .flow.ts
- Translation logic for all 4 ACPX node types
- Parallelism detection (upgrade sequential -> parallel)
- `apxm import` CLI command

**Effort**: 4 weeks

---

## vLLM Gaps

### Gap V1: No Graph Awareness

**Current**: vLLM processes each request independently. No concept of request relationships, pipelines, or downstream dependencies.

**Evidence**: vLLM's `Scheduler` class queues requests by arrival order with no graph metadata.

**Required**:
- Graph registration API (`POST /v1/graphs/register`)
- Extended request headers (graph_id, node_id, downstream, priority)
- Priority scheduling based on graph critical path
- KV-cache pinning for downstream-feeding requests

**Effort**: 4-5 weeks

---

### Gap V2: No Incremental Prefill

**Current**: vLLM requires the complete input before starting prefill. No support for streaming input construction.

**Evidence**: `LLMEngine.generate()` takes a complete prompt string/token list.

**Recent development**: vLLM (2026) has introduced streaming input with incremental KV cache construction at the engine level (per the patent's existing solutions section). This may partially close this gap.

**Required**:
- Incremental prefill manager
- API for starting prefill with partial input
- API for appending chunks to in-progress prefill
- Backpressure signaling between producer and consumer

**Effort**: 3-4 weeks (if built on vLLM's new streaming support)

---

### Gap V3: No Result Pinning

**Current**: Once a request completes, its KV-cache is eligible for eviction. No mechanism to keep it warm for downstream consumers.

**Required**:
- `result_disposition` metadata on requests
- KV-cache pinning with configurable TTL
- Cache reference tokens returned to APXM runtime
- Cleanup on graph completion

**Effort**: 2-3 weeks

---

## ACPX Gaps

### Gap C1: Sequential Execution Only

**Current**: ACPX executes nodes one at a time, strictly following the edge order. No parallel execution.

**Evidence**: `FlowRunner` processes nodes sequentially per the ARCHITECTURE.md: "Sequential execution (one node at a time)".

**Impact**: Even independent nodes (e.g., `ask_codex` and `ask_claude` in echo.flow.ts) run sequentially.

**Resolution**: The ACPX-to-APXM compiler fixes this by detecting independent nodes and marking them for parallel execution in the APXM graph.

---

### Gap C2: Static Agent/Model Assignment

**Current**: The `profile` field on ACP nodes is fixed at flow definition time. No dynamic routing.

**Evidence**: `review.flow.ts` line `profile: "claude"` -- hardcoded.

**Impact**: Can't fail over to another model if the selected one is down. Can't route based on load or cost.

**Resolution**: The `model_policy` attribute in the APXM translation maps profiles to dynamic routing policies.

---

### Gap C3: No Optimization Passes

**Current**: ACPX flows run as defined. No dead code elimination, fusion, or restructuring.

**Resolution**: After translation to APXM graph, the MLIR compiler applies standard optimization passes.

---

### Gap C4: No Context Scoping

**Current**: Each ACP node receives whatever prompt the flow author writes. No automatic context management -- the author must manually call `embedSkills()` and construct the prompt.

**Evidence**: `review.flow.ts` manually constructs prompts with `embedSkills(pr.projectRoot, ["review"])`.

**Resolution**: APXM's ContextStack with per-node manifests automates context assembly.

---

## Cross-System Gaps

### Gap X1: No Unified Observability

**Current**: Three separate trace formats with no correlation.

| System | Format | Location |
|--------|--------|----------|
| ACPX | NDJSON trace bundles | `~/.acpx/flows/runs/<runId>/` |
| APXM | JSON metrics file | `--emit-metrics metrics.json` |
| vLLM | Prometheus metrics | `:9090/metrics` |

**Required**: Unified trace format with distributed span IDs across all three systems.

**Effort**: 3 weeks

---

### Gap X2: No Automatic Graph Generation

**Current**: All graphs/flows are manually authored. No mechanism to generate a workflow from natural language.

**Required**:
- Graph generation agent (uses AIS operation catalog + model registry)
- Validation of generated graphs via `apxm validate`
- Iterative refinement based on execution results

**Effort**: 4-6 weeks (depends on graph quality requirements)

---

### Gap X3: No Feedback Loop

**Current**: Execution results don't feed back into compilation or routing decisions.

**Required**:
- Execution profile data stored in persistent cache
- Compiler passes that use profile data (PGO for graphs)
- Model routing adjustments based on observed latencies
- Speculation confidence updates from actual hit rates

**Effort**: 3-4 weeks

---

## Priority Matrix

| Gap | Impact | Effort | Dependencies | Priority |
|-----|--------|--------|-------------|----------|
| A1 (Model Router) | High | 2-3w | None | **P0** |
| A6 (Graph Import) | High | 4w | None | **P0** |
| A3 (Memoization) | High | 2w | None | **P1** |
| V1 (Graph Awareness) | Very High | 4-5w | A1 | **P1** |
| A2 (Context Stack) | Very High | 4-5w | None | **P1** |
| V3 (Result Pinning) | High | 2-3w | V1 | **P1** |
| A4 (Speculation) | High | 3w | A3 | **P2** |
| V2 (Incremental Prefill) | High | 3-4w | V1 | **P2** |
| A5 (Pipelining) | High | 4w | V2 | **P2** |
| X1 (Observability) | Medium | 3w | None | **P2** |
| X2 (Graph Generation) | Medium | 4-6w | A6 | **P3** |
| X3 (Feedback Loop) | Medium | 3-4w | X1 | **P3** |

---

## What Already Works (Strengths to Build On)

| Existing Capability | System | Why It Matters |
|---------------------|--------|---------------|
| 32 AIS operations with rich semantics | APXM | Comprehensive operation vocabulary for graph translation |
| Parallel dataflow scheduler with work-stealing | APXM | Foundation for parallel execution of imported flows |
| LLMBackend trait with pluggable backends | APXM | Clean extension point for graph-aware vLLM backend |
| 3-tier memory (STM/LTM/Episodic) | APXM | Natural mapping to memo cache tiers and context storage |
| CapabilitySystem with interceptors | APXM | Extension point for context stack integration |
| MLIR optimization pipeline | APXM | Foundation for new optimization passes |
| REST API + MCP + A2A protocols | APXM | Graph registration and metadata exchange endpoints |
| defineFlow() with typed nodes and edges | ACPX | Clean source format for graph import |
| Trace bundles with replay | ACPX | Foundation for unified observability |
| Session persistence across ACP nodes | ACPX | Maps to APXM FlowCall + shared context |
| High-throughput model serving | vLLM | Foundation for graph-aware scheduling |
| KV-cache management | vLLM | Foundation for result pinning and incremental prefill |
| OpenAI-compatible API | vLLM | Standard interface for APXM backend |
