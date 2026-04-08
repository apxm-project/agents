# Graph-Aware vLLM: A Practical Integration Plan for APXM

**Date**: April 1, 2026  
**Status**: Revised architecture proposal after repo investigation  
**Scope**: APXM -> vLLM request hints, priority propagation, shared-prefix reuse, KV-cache pinning, and research-track token pipelining

---

## Table of Contents

1. [Executive Assessment](#1-executive-assessment)
2. [Current Repo Reality](#2-current-repo-reality)
3. [What vLLM Already Gives Us](#3-what-vllm-already-gives-us)
4. [Recommended APXM to vLLM Contract](#4-recommended-apxm-to-vllm-contract)
5. [Workstream A: No-Fork Wins](#5-workstream-a-no-fork-wins)
6. [Workstream B: Graph-Aware KV Reuse](#6-workstream-b-graph-aware-kv-reuse)
7. [Workstream C: Research Track](#7-workstream-c-research-track)
8. [Phased Implementation Plan](#8-phased-implementation-plan)
9. [Benchmarks and Exit Criteria](#9-benchmarks-and-exit-criteria)
10. [Risks and Upstream Strategy](#10-risks-and-upstream-strategy)
11. [Key Numbers](#11-key-numbers)
12. [Summary](#12-summary)

---

## 1. Executive Assessment

The original idea is correct: APXM knows the execution graph, while vLLM only sees isolated inference requests. That gap leaves reuse, scheduling, and overlap opportunities on the table.

The original draft, however, mixed together three very different categories of work:

1. **Shippable now with the current APXM codebase**
   - Pass graph hints through the existing OpenAI-compatible backend path
   - Export node priority into vLLM's existing `priority` field
   - Reshape prompts so shared context becomes a real shared prefix
   - Add warmup requests and measurements

2. **Reasonable first fork of vLLM**
   - Pin prefix-reusable KV blocks for a bounded TTL
   - Add explicit graph release / cancellation cleanup
   - Measure pin memory cost against hit-rate gains

3. **Research, not committed product work**
   - Token-by-token pipelining between adjacent nodes
   - Deferred-free "result holding" as a stronger form of pinning
   - Full graph registration before the first request
   - Speculative prefill of likely branches

The revised plan therefore changes the strategy in four ways:

- It starts from the code that already exists in APXM, instead of inventing a parallel backend architecture first.
- It separates "transport and observability" from "scheduler semantics" and from "research prototypes".
- It replaces a few optimistic assumptions with explicit prerequisites and go/no-go checks.
- It treats vLLM changes as a thin, isolated patch set, not a rewrite.

### Recommendation

Ship the work in this order:

1. **Metadata + priority export**
2. **Prompt canonicalization + warmup for shared prefixes**
3. **KV pinning with TTL and explicit release**
4. **Only then evaluate token pipelining**

If step 3 does not materially move end-to-end latency on real APXM graphs, stop there. Do not pay the complexity cost of pipelining without proof that prefix reuse and priority are insufficient.

---

## 2. Current Repo Reality

This section is the main reason to revise the plan. The repo already has several useful building blocks, but they do not line up with the original document's assumptions.

### 2.1 What Already Exists

| Area | What exists now | Why it matters |
|---|---|---|
| LLM request transport | `LLMRequest` already has `metadata: HashMap<String, serde_json::Value>` | APXM already has a place to carry graph hints |
| Runtime LLM handler | `apxm-runtime/src/executor/handlers/llm.rs` builds `LLMRequest` objects | This is the natural insertion point for APXM graph metadata |
| OpenAI-compatible backend | `apxm-backends/src/llm/backends/openai/backend.rs` already targets OpenAI-style JSON APIs | vLLM should be reached through this path first |
| Compile-time priority field | `node.metadata.priority` already exists in the core execution model | APXM can already score nodes before runtime |
| Runtime scheduler priority | APXM runtime already schedules by priority tiers | Priority is a first-class concept across the stack |
| Graph analysis | `apxm-cli analyze` already computes critical path and estimated span | We do not need to invent critical-path analysis from scratch |
| BuildPrompt pass | The pass exists today | But its scope is narrower than the original document claimed |

### 2.2 What Does *Not* Exist Yet

| Area | Current reality | Consequence |
|---|---|---|
| Graph-aware backend | No dedicated `vllm_graph_aware.rs` backend exists | The lowest-risk path is extending the current OpenAI-compatible backend |
| Metadata serialization to provider body | The OpenAI backend does not currently serialize `LLMRequest.metadata` into the request JSON | APXM graph hints do not leave the process today |
| Prompt-prefix shaping pass | No pass exists that rewrites prompts to maximize shared-prefix reuse | Build-time reuse optimization still needs real compiler work |
| vLLM-specific priority export pass | No pass today emits per-node vLLM scheduling hints | Priority must be wired from APXM graph metadata into backend requests |
| Graph-aware pinning | No vLLM patch set exists in this repo | KV reuse beyond opportunistic prefix cache hits is still planned work |
| Token pipelining transport | No APXM or vLLM code here supports append-to-prefill streaming between nodes | This remains a research item |

### 2.3 Important Correction: `BuildPrompt` Is Not a Prefix-Shaping Pass

The current `BuildPrompt` pass does one specific thing: when an LLM op has an empty `template_str` but non-empty context, it generates a `{0}` placeholder so runtime prompt assembly works.

It does **not**:

- reorder prompt fragments,
- factor out shared context,
- compute reuse groups,
- or shape prompts for vLLM prefix caching.

That means the original plan overloaded `BuildPrompt` with responsibilities it does not have. The revised plan introduces a **new pass** for this work:

`PromptCanonicalization` or `SharedPrefixHints`

That new pass should produce explicit artifacts such as:

- `apxm.shared_prefix_group`
- `apxm.shared_prefix_est_tokens`
- `apxm.warmup_candidate`
- `apxm.pipeline_candidate`

### 2.4 Important Correction: Use the Existing Backend Path First

The original draft proposed a new `GraphAwareVllmBackend`. That is defensible later, but it is not the correct first move.

Right now the codebase already has:

`LLM handler -> LLMRequest -> OpenAI-compatible backend -> HTTP request body`

That should become:

`LLM handler -> LLMRequest + APXM hints -> OpenAI-compatible backend -> vLLM request body`

Only after that path is proven insufficient should APXM grow a dedicated vLLM-specific backend.

### 2.5 Important Correction: Token Pipelining Is Not Yet a Stable Product Surface

The original draft treated "append tokens to an in-progress downstream prefill" as if it were already a clean API decision. It is not.

The right framing is:

- vLLM has enough lower-level machinery that this idea is plausible,
- but APXM does not yet have a stable transport contract for it,
- and there is no proven end-to-end product path in this repo today.

So token pipelining stays in the document, but it moves to a research track with clear prerequisites.

---

## 3. What vLLM Already Gives Us

vLLM already provides several extension points that materially reduce the amount of new work APXM needs.

### 3.1 Existing Foundations

| vLLM capability | Why APXM cares |
|---|---|
| OpenAI-compatible request surface | APXM can integrate without inventing a new wire protocol |
| Request `priority` | Critical-path and speculative work can already be ranked |
| Prefix caching | Shared-prefix reuse already has a native execution model |
| Block-based KV cache management | Pinning can build on existing block lifecycle concepts |
| Chunked prefill / related prefill optimizations | APXM can benefit from smarter request shaping even before pinning exists |
| Resumable / streaming-oriented internals | Token pipelining may be possible later, but should not be assumed product-ready |

### 3.2 What APXM Still Has To Add

vLLM still does **not** know:

- which requests belong to the same APXM graph,
- which downstream node is expected next,
- whether a completed request should be kept warm,
- whether a branch is speculative,
- or when a graph has been canceled and any held state should be released.

That is the real integration boundary:

**APXM contributes graph semantics. vLLM contributes fast execution and cache management.**

### 3.3 Design Principle

Use vLLM's existing knobs before adding new ones:

- Use `priority` before inventing a graph scheduler
- Use prefix caching before inventing deferred-free
- Use prompt shaping before inventing token append transport
- Use TTL-based pinning before inventing full graph registration

This keeps the patch surface small and makes benchmark results interpretable.

---

## 4. Recommended APXM to vLLM Contract

The contract should be deliberately minimal in the first iteration.

### 4.1 Do Not Start With Custom Headers

The first draft proposed extra HTTP headers and a new graph registration endpoint. That is more moving parts than we need.

The recommended first contract is:

- top-level `priority` for actual scheduler behavior,
- `vllm_xargs.apxm` for graph metadata and observability.

This matches the current APXM backend architecture much better.

### 4.2 Proposed Request Shape

```json
{
  "model": "meta-llama/Llama-4-Maverick-17B-128E",
  "messages": [...],
  "priority": 2,
  "vllm_xargs": {
    "apxm": {
      "schema_version": 1,
      "graph_id": "g-a1b2c3d4",
      "execution_id": "exec-2026-04-01-001",
      "node_id": 12,
      "node_name": "review_code",
      "priority_class": "critical_path",
      "target": "latency",
      "downstream_nodes": [14, 15],
      "reuse_group": "pr-diff:shared-prefix",
      "pin_policy": {
        "mode": "prefix",
        "ttl_ms": 30000
      },
      "compiler_hints": {
        "shared_prefix_est_tokens": 2048,
        "warmup_candidate": true,
        "pipeline_candidate": false
      }
    }
  }
}
```

### 4.3 Contract Rules

| Field | Rule |
|---|---|
| `priority` | Always use the native vLLM field for actual scheduling |
| `vllm_xargs.apxm.schema_version` | Required; allows non-breaking evolution |
| `graph_id` | Stable across the full APXM execution graph |
| `execution_id` | Stable per runtime execution attempt |
| `node_id` | Must match APXM graph node id |
| `priority_class` | Human-readable observability only; do not rely on it for scheduling |
| `reuse_group` | Stable semantic identifier for shared-prefix cohorts |
| `pin_policy.mode` | Start with only `none` and `prefix` |
| `pin_policy.ttl_ms` | Hint, not guarantee |
| `compiler_hints` | Optional and forward-compatible |

### 4.4 Why `reuse_group` Matters

Downstream reuse is not always "request B reuses request A".

Often the structure is:

```
shared prefix: diff + repo summary + task setup
node 1 unique suffix: security review
node 2 unique suffix: style review
node 3 unique suffix: perf review
```

Those nodes belong to the same semantic reuse cohort even if they are not strict parent/child operations.

`reuse_group` lets APXM express that relationship without overloading `downstream_nodes`.

### 4.5 When To Add Graph Registration

Do **not** add a graph registration endpoint in the first slice.

Add it only if one of these becomes true:

1. per-request metadata is too large,
2. scheduler decisions need full-graph visibility before the first request,
3. pin cleanup needs server-side graph lifecycle state,
4. or speculation requires branch probabilities ahead of time.

Until then, registration is extra API surface without clear benchmark value.

---

## 5. Workstream A: No-Fork Wins

This workstream requires no vLLM fork. It should land first.

### 5.1 Metadata Pass-Through

**Goal**: APXM graph hints survive the entire path from compiled graph node to vLLM HTTP body.

#### APXM touch points

| File / layer | Change |
|---|---|
| `apxm-runtime/src/executor/handlers/llm.rs` | Build APXM hint payload from node metadata and execution context |
| `apxm-backends/src/llm/backends/request.rs` | Add an explicit helper for provider-specific extra request fields |
| `apxm-backends/src/llm/backends/openai/backend.rs` | Serialize `priority` and `vllm_xargs` into the JSON body when configured for vLLM-compatible targets |

#### Key rule

Do not overload generic `metadata` forever. Use it as the bridge, then promote the stable fields into a typed structure once the contract settles.

Suggested typed addition:

```rust
pub struct ProviderHints {
    pub openai_extra_body: serde_json::Value,
}
```

or more specifically:

```rust
pub struct VllmHints {
    pub priority: Option<i32>,
    pub vllm_xargs: serde_json::Value,
}
```

#### Acceptance criteria

- When backend target is vLLM, requests contain the expected `priority` and `vllm_xargs`.
- When backend target is non-vLLM OpenAI-compatible infrastructure, APXM can disable these fields cleanly.
- Trace logs include `graph_id`, `node_id`, and `priority_class`.

### 5.2 Priority Export

**Goal**: APXM uses the native vLLM priority knob before doing any scheduler surgery.

#### What already exists

- APXM nodes already carry `metadata.priority`.
- APXM CLI can already compute critical path.
- APXM runtime already schedules operations using priority tiers.

#### What needs to be added

- A deterministic mapping from APXM graph analysis to vLLM request priority
- A consistent scale and polarity
- Clear rules for speculative and background work

#### Recommended policy

Use a small integer scale where lower numbers mean more urgent work:

| APXM class | vLLM priority |
|---|---|
| Critical-path, user-visible | 0 |
| Critical-path, non-interactive | 2 |
| Normal | 5 |
| Speculative | 10 |
| Best-effort background | 15 |

#### Important note

The current compiler pass named `CapabilityScheduling` does **not** already solve this end-to-end. It annotates latency, tier, cost, and parallel-safe hints. That is useful, but it is not the same as generating concrete vLLM scheduling priorities.

So this plan requires a small new APXM feature:

`VllmPriorityHints`

That can live either:

- in the compiler as a new pass that emits node attributes, or
- in the runtime as a graph-analysis step before dispatch.

Compiler-first is better if we want stable artifacts and inspectable plans. Runtime-first is faster if we want to prove value quickly.

### 5.3 Shared-Prefix Shaping and Warmup

**Goal**: Improve prefix-cache reuse without touching vLLM internals.

This is the highest-leverage "APXM-only" optimization after priority export.

#### Why it matters

If APXM emits prompts like:

```text
As a security reviewer, review this code:
<large diff>
```

and

```text
As a style reviewer, review this code:
<large diff>
```

then the unique part comes first, which hurts prefix reuse.

If APXM emits:

```text
<large diff>

---
Review focus: security
```

and

```text
<large diff>

---
Review focus: style
```

then the expensive shared context becomes a true shared prefix.

#### Required new APXM compiler work

Create a new pass, for example:

`PromptCanonicalization`

Responsibilities:

- reorder prompt templates so shared context is placed first,
- emit `shared_prefix_group` hints,
- estimate shared-prefix token counts,
- mark warmup-worthy nodes.

This pass should be independent from `BuildPrompt`.

#### Warmup strategy

For large shared contexts, APXM may send a warmup request that:

- prefills the shared prefix,
- generates zero or one cheap token,
- and marks the request as a warmup candidate for reuse.

Warmup should be gated by thresholds:

- estimated shared prefix >= X tokens,
- fan-out >= Y downstream consumers,
- and request target is `latency`, not `cost`.

#### Acceptance criteria

- Shared-prefix fan-out graphs show materially higher prefix-cache hit rates.
- Warmup is disabled automatically when expected savings are too small.
- Warmup never becomes mandatory for correctness.

### 5.4 Observability Before Semantics

Before APXM starts pinning or pipelining anything, the team needs observability.

Add counters and spans for:

- request priority chosen,
- shared-prefix group selected,
- warmup requests issued,
- warmup requests later reused,
- prefix-cache hit ratio by graph and by node,
- repeated prefill tokens avoided.

If those numbers are not visible, later phases will be impossible to judge.

---

## 6. Workstream B: Graph-Aware KV Reuse

This is the first place where a vLLM fork or targeted patch set is justified.

### 6.1 Objective

Turn shared-prefix reuse from "opportunistic if blocks have not been evicted yet" into "likely or guaranteed within a bounded TTL for chosen graph edges".

### 6.2 Use Pinning, Not Deferred-Free, As The First Real Mechanism

The original draft described two stronger variants:

- **pinning**
- **result holding / deferred free**

The revised plan says:

- start with **pinning**,
- measure it,
- and only escalate to deferred-free if pinning proves insufficient.

Reason:

- pinning is conceptually closer to prefix caching,
- it should fit better with vLLM's existing cache lifecycle,
- and its memory cost is easier to bound.

### 6.3 Required vLLM Concepts

Do not implement pinning as an ad-hoc scheduler `touch()` hack. That is too brittle.

Add explicit concepts:

```python
@dataclass
class GraphPinPolicy:
    graph_id: str
    node_id: int
    reuse_group: str | None
    downstream_nodes: set[int]
    ttl_ms: int

@dataclass
class GraphPinHandle:
    request_id: str
    graph_id: str
    node_id: int
    reuse_group: str | None
    expiry_ts: float
    block_ids: list[int]
```

And registry state:

```python
self.apxm_pins_by_graph: dict[str, dict[str, GraphPinHandle]]
```

where the second key is either:

- request id,
- or `reuse_group`,
- depending on which lookup path proves simpler.

### 6.4 Pin Lifecycle

1. Request completes.
2. APXM hint says `pin_policy.mode == "prefix"`.
3. vLLM records a pin handle for the reusable prefix portion.
4. Downstream request arrives with matching `graph_id` and either:
   - matching upstream relationship, or
   - matching `reuse_group`.
5. Reusable blocks are protected from eviction until:
   - downstream reuse happens,
   - TTL expires,
   - graph cancellation arrives,
   - or memory pressure forces release.

### 6.5 Release Semantics

Pin release reasons should be explicit:

- `consumed`
- `ttl_expired`
- `graph_canceled`
- `memory_pressure`
- `scheduler_reset`

If these reasons are not logged, pinning failures will be opaque.

### 6.6 Do We Need A Graph Registry?

Not immediately.

The minimum viable version can operate from per-request hints plus an optional cleanup endpoint:

```
DELETE /v1/apxm/graphs/{graph_id}
```

That is enough to clean up pins when APXM execution is canceled or fails.

A full server-side graph registry should come only if per-request hints prove too weak.

### 6.7 Memory Pressure Policy

The original draft's "PIN vs HOLD vs RELEASE" decision tree is directionally right but needs to become operational policy.

Recommended first policy:

| KV usage | Action |
|---|---|
| < 85% | Allow eligible pins |
| 85%-92% | Allow only critical-path pins |
| 92%-97% | Downgrade new pins to normal cache behavior |
| > 97% | Release non-critical pins immediately |

This keeps the optimization subordinate to server health.

### 6.8 Required Metrics

Add vLLM-side metrics for:

- active pins
- pinned blocks
- pinned bytes
- pin lifetime ms
- pin hit ratio
- releases by reason
- requests helped by pinning
- requests stalled or penalized by pinning

### 6.9 Acceptance Criteria

Pinning is worth shipping only if it meets all three:

1. **Reuse value**: downstream shared-prefix hit rate approaches deterministic behavior on target graphs
2. **Health**: throughput and tail latency stay within acceptable regression bounds under mixed load
3. **Cleanup**: no leaked pins after graph cancellation, timeout, or worker failure

---

## 7. Workstream C: Research Track

This workstream is valuable, but it should be treated as explicit research.

### 7.1 Token Pipelining

The core idea is still attractive:

`ASK(node_1) -> ASK(node_2)`

should allow downstream work to begin before upstream fully finishes.

But that requires three things the repo does not have yet:

1. a stable APXM-side transport for partial downstream construction,
2. a stable vLLM-side API for append/resume behavior,
3. benchmark proof that the complexity beats simple warmup + pinning.

#### Research milestone

Build a narrow prototype for exactly one shape:

`ASK -> ASK`

with:

- one upstream producer,
- one downstream consumer,
- a known static prefix,
- and no tools or structured output in the loop.

Do not generalize beyond that until it is measured.

### 7.2 Result Holding / Deferred Free

This is stronger than pinning:

- pinning protects reusable cache state,
- deferred-free keeps results effectively "live" as if still actively owned.

That may be useful later, but it increases memory residency and scheduler coupling.

Treat it as a fallback when:

- pinning still allows too many misses under pressure,
- and the benchmark shows that those misses dominate latency.

### 7.3 Full Graph Registration

Pre-registering the graph can help with:

- speculative prefill,
- branch probabilities,
- bulk cleanup,
- and scheduler-wide graph policy.

But it also creates more API and state management.

Do not add it until per-request hints hit a hard limit.

### 7.4 Go / No-Go Conditions

Proceed with research-track features only if:

- Workstream A already produces reliable measurements
- Workstream B shows real wins but still leaves meaningful latency on the table
- The target workload is dominated by chained LLM nodes rather than parallel fan-out

Otherwise the complexity is not justified.

---

## 8. Phased Implementation Plan

The original week-based plan is replaced here with milestone-based slices that line up better with current repo reality.

### Phase 0: Baseline and Instrumentation

**Goal**: know what the current system does before changing behavior.

Deliverables:

- benchmark graphs for shared-prefix fan-out and chained-node latency
- request logging for graph id, node id, and chosen priority
- prefix-cache hit and miss reporting

No vLLM fork required.

### Phase 1: Metadata and Priority Export

**Goal**: APXM can tell vLLM what each request means.

Deliverables:

- typed APXM -> vLLM hints in APXM runtime/backend
- JSON body support for `priority` and `vllm_xargs`
- critical-path/speculative mapping to native vLLM priority
- feature flag to disable hints when target backend is not vLLM

Exit criteria:

- requests visibly carry the new fields
- critical-path requests outrank speculative requests in mixed tests

### Phase 2: Prefix-Shaping and Warmup

**Goal**: improve reuse without touching vLLM internals.

Deliverables:

- new compiler pass for prompt canonicalization
- shared-prefix group hints
- warmup request heuristic and runtime dispatch
- benchmark showing reduced repeated prefill work

Exit criteria:

- fan-out graphs show materially higher reuse
- warmup produces net benefit on selected workloads

### Phase 3: Pinning in vLLM

**Goal**: convert probabilistic reuse into bounded, policy-driven reuse.

Deliverables:

- thin vLLM patch for TTL pinning
- cleanup endpoint or equivalent graph-release mechanism
- pinning metrics
- memory pressure downgrade policy

Exit criteria:

- high reuse on target downstream graphs
- no pin leaks
- acceptable throughput / P99 impact under stress

### Phase 4: Research Spike for Pipelining

**Goal**: decide whether token pipelining is worth productizing.

Deliverables:

- prototype only for `ASK -> ASK`
- measured comparison against Phase 2 + Phase 3 baseline
- explicit recommendation: continue or stop

Exit criteria:

- continue only if measured end-to-end gain is clear and stable

### Decision Table

| Phase | APXM changes | vLLM changes | Ship confidence |
|---|---|---|---|
| 0 | Low | None | High |
| 1 | Low-Medium | None | High |
| 2 | Medium | None | High |
| 3 | Low-Medium | Medium | Medium |
| 4 | Medium | High | Low until proven |

---

## 9. Benchmarks and Exit Criteria

The plan is only useful if each slice has a measurable success condition.

### 9.1 Required Benchmark Shapes

| Benchmark | Purpose |
|---|---|
| Shared-prefix fan-out | Measure reuse across multiple review nodes sharing the same large context |
| Critical vs speculative mix | Measure whether priority improves graph completion time for user-visible path |
| Chained LLM nodes | Measure whether warmup or pipelining helps sequential latency |
| Cancellation under load | Verify pins are released correctly |
| Memory-pressure stress | Verify pinning degrades safely |

### 9.2 Metrics To Collect

| Metric | Why it matters |
|---|---|
| Prefix-cache hit rate | First-order signal for reuse success |
| Shared-prefix tokens recomputed | Direct measure of wasted work |
| End-to-end graph latency | What the user actually feels |
| Node latency on critical path | Validates priority work |
| Warmup reuse ratio | Ensures warmup is not wasted |
| Active pins / pinned bytes | Required for memory safety |
| Pin release reason counts | Required for debugging |
| P50 / P95 / P99 latency | Needed to catch server regressions |
| Throughput under mixed load | Prevents latency wins that destroy capacity |

### 9.3 Exit Criteria By Workstream

#### Workstream A succeeds if:

- APXM reliably emits the new metadata
- vLLM receives and logs it
- priority changes improve mixed-path scheduling behavior

#### Workstream B succeeds if:

- downstream reuse is near-deterministic for eligible graphs
- pinning does not destabilize the server under pressure
- graph cancellation fully cleans up state

#### Workstream C succeeds if:

- pipelining beats the simpler baseline in real graphs
- the transport API can be made stable without deep coupling

If Workstream C does not clear that bar, it should not graduate into the mainline roadmap.

---

## 10. Risks and Upstream Strategy

### 10.1 Main Risks

| Risk | Impact | Mitigation |
|---|---|---|
| APXM hints never leave the backend process | No feature actually reaches vLLM | Land transport first and verify request bodies |
| Prompt shaping harms output quality | Reuse gains offset by worse answers | Gate by benchmark and allow per-node opt-out |
| Pinning increases memory pressure | Throughput or latency regression | Hard thresholds, downgrade policy, explicit metrics |
| Cancellation leaks pins | Long-lived memory waste | Add graph release semantics before broad rollout |
| vLLM internals change upstream | Patch set becomes expensive to carry | Keep fork small and isolate APXM logic |
| Pipelining proves too invasive | Large engineering cost for uncertain gain | Keep it in research track until benchmarked |

### 10.2 Upstream Strategy

Treat the vLLM work as a sequence of small, separable patches:

1. request observability for APXM xargs
2. optional graph cleanup endpoint
3. TTL pinning primitives
4. scheduler policy for pin release under pressure
5. only then any experimental resumable-input work

This is better than a single "graph-aware scheduler" mega-patch.

### 10.3 Architectural Rule

APXM remains the source of truth for graph semantics.

vLLM should not learn how to execute APXM graphs. It should learn only enough graph metadata to:

- prioritize,
- preserve reusable cache state,
- and optionally overlap adjacent inference work.

That keeps responsibility boundaries clean.

---

## 11. Key Numbers

These numbers are planning targets, not promises.

### 11.1 Code Surface Estimates

| Slice | APXM | vLLM |
|---|---|---|
| Metadata + priority export | ~150-250 LOC | 0 |
| Prompt canonicalization + warmup | ~200-350 LOC | 0 |
| KV pinning + cleanup | ~50-150 LOC | ~250-500 LOC |
| Token pipelining prototype | ~300+ LOC | ~500-1000+ LOC |

### 11.2 Expected Benefit Ranges

| Feature | Expected effect |
|---|---|
| Priority export | Better critical-path completion under contention |
| Prompt shaping | 20-60% less repeated prefill work on shared-prefix fan-out |
| Warmup | Helpful only when shared prefix is large and reused quickly |
| KV pinning | Near-deterministic downstream prefix reuse within TTL |
| Token pipelining | Potentially 15-40% gain on chained-node latency, but unproven in this architecture |

### 11.3 Recommended Success Thresholds

| Slice | Threshold |
|---|---|
| Metadata + priority | No correctness regressions, clear request observability |
| Prompt shaping + warmup | Meaningful reduction in repeated prefill on target graphs |
| KV pinning | Strong reuse improvement with less than 10% throughput regression under mixed load |
| Pipelining | Clear gain over Phase 2 + Phase 3 baseline before any product commitment |

---

## 12. Summary

The original plan had the right destination but the wrong order of attack.

The practical version is:

1. **Use the backend path APXM already has**
2. **Exploit native vLLM priority before changing scheduler semantics**
3. **Improve prompt structure before changing cache internals**
4. **Add TTL pinning as the first real vLLM fork feature**
5. **Keep token pipelining explicitly in the research bucket until it proves itself**

That turns "graph-aware vLLM" from a broad vision into a staged execution plan:

- APXM owns graph analysis and request intent
- vLLM owns fast execution and bounded cache retention
- research features are gated by measured wins, not by architectural enthusiasm

This is the plan that should drive implementation.
