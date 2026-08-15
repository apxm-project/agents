# Backend-neutral APXM graph hints

Superseded by
[`docs/adr/0021-backend-neutral-graph-hints.md`](../adr/0021-backend-neutral-graph-hints.md).
Keep this file as the long-form design notes. Ownership: Agents owns facts and
intents; `apxm-project/vllm` and `apxm-project/llama.cpp` `apxm` branches
receive the envelope. The vLLM pin/join catalog is retired.


- Scope: APXM inference adapters and runtime evidence
- Decision sought: replace backend-shaped graph metadata with one APXM-owned
  semantic contract and adapter-owned lowerings
- Primary integration proof: the same hints project honestly into both vLLM
  and llama.cpp

## 1. Decision

APXM should own one closed `ApxmGraphHints` contract. It describes:

1. facts APXM knows about a node's position and estimated work in an admitted
   graph; and
2. advisory optimization intents whose omission may affect performance but
   never correctness, authority, target identity, or output typing.

It must not describe a backend's scheduler, cache representation, queue,
worker, slot, block, handle, endpoint, or extension envelope.

Every inference adapter implements the same projection interface. An adapter
may support all, some, or none of the fields. It must report, field by field,
what it projected, approximated, or omitted. Provider acknowledgement and
outcome measurement require separate producer contracts; no current adapter
claims either. Supporting zero fields is a conforming implementation; silently
claiming support is not.

The common contract and provider-specific Port Contracts have different jobs:

```text
compiler/runtime facts + intents
            |
            v
      ApxmGraphHints                 APXM-owned semantics
            |
            v
   exact admitted adapter
      |             |
      v             v
 vLLM lowering   llama.cpp lowering  provider-owned mechanisms
      |             |
      v             v
 provider request + adapter projection evidence
```

The provider-specific contract remains exact and independently conformant. A
common source contract does not turn provider transports into one protocol.

## 2. Why the current surface needs to change

The common type is `ApxmGraphHints` and is genuinely portable:
graph/execution/node identity, successors, compiler estimates, remaining path
length, and stage information. It previously also owned `PinMode`, `PinPolicy`,
`PriorityClass`, a default pin TTL, and graph status in units of pinned handles
and blocks. Those are realizations of graph intent, not portable graph
semantics, and they are gone from
`crates/machine/contracts/src/types/graph_hints.rs`. Provider resource
observations now travel in the adapter-keyed
`GraphStatusSnapshot::adapter_observations` map.

The vLLM adapter previously serialized the common object directly into
`vllm_xargs.apxm` and separately derived a numerical request priority from it,
which made the common shape double as a vLLM wire shape. Both adapters now
render only what their own plan authorized
(`GraphHintProjector::render_graph_hint_fields` in
`crates/runtime/backends/src/llm/backends/vllm/backend.rs` and
`.../llama_cpp/mod.rs`), and `ApxmGraphHints::project_envelope` is the single
place the common envelope becomes a provider-bound document.

Graph lifecycle was broadcast to every registered backend on a best-effort
basis, with default implementations returning success for registration and
release even when they did nothing, and status discovery classifying backends
as `GraphAware` or `Generic`. That contradicted the exact inference-binding
direction already implemented elsewhere: one model effect resolves to one
admitted driver, with no ranking, substitution, or fallback
(`crates/runtime/inference/src/driver.rs:1-5`).

The broadcast chain is gone. It is replaced at the adapter boundary by a
digest-fenced exact-binding lifecycle seam, not by another registry fan-out.
`find_graph_aware_backends`, `pre_release_status_all`, the
adapter-observation polling, and `GraphLifecycleOutcome` were deleted because
nothing called them; `supports_graph_extensions` followed once its only
remaining caller was a forwarding impl. `register_graph`, `release_graph`, and
`get_graph_status` remain as `LLMBackend` methods with the vLLM adapter's own
HTTP implementations behind them, and nothing in the runtime calls any of the
three. What survives as a live surface is the declaration:
`GraphLifecycleCapability` on `GraphHintCapabilities`
(`crates/machine/contracts/src/types/graph_hints.rs`), which enters the
capability digest and so is held against the admitted binding before send.
`prepare_graph` and `release_graph_preparation` consume the lifecycle
declaration on one selected adapter; the general graph executor does not yet
call that seam.

The target design therefore preserves the portable facts, removes backend
mechanisms from the common type, and makes projection and evidence explicit.

## 3. Non-negotiable invariants

### 3.1 Hints are never authority

Hints must not:

- select or change a model, deployment, adapter, replica class, or endpoint;
- add a Tool, Capability, credential, permission, or provider option;
- change messages, sealed model context, sampling, structured-output schema,
  token budgets, or cancellation semantics;
- authorize batching requests that were not already proven compatible;
- relax ordering, correlation, idempotency, retry, or commit rules; or
- turn a performance preference into a correctness guarantee.

If a behavior is required for correctness, privacy, cost, or an admitted
service guarantee, it belongs in the Model Target, Runtime Profile, policy, or
Port Contract. It is not a hint. Admission may require a graph-hint capability
from the exact binding, but the request envelope still remains advisory.

### 3.2 Unsupported means explicit omission

A known but unsupported hint is omitted and recorded as
`omitted_unsupported`. An unknown field or invalid value is rejected before
send. No adapter may pass an unknown common field through as provider metadata.

### 3.3 Projection is exact-binding local

Only the adapter selected by the admitted inference binding sees the hints.
There is no broadcast, provider discovery, graph-aware backend search, ranking,
or fallback. A missing required projection capability fails admission for that
binding; it never causes selection of another backend.

### 3.4 Attempts do not renegotiate semantics

The materialized hint digest, capability snapshot, and field-level projection
outcomes are stable across all attempts of one model effect. A retry cannot
opportunistically widen support or change cache, priority, or lifecycle policy.
An attempt that failed before send may rematerialize an attempt-local physical
lease on an identical admitted replica; for example, it may receive a different
slot. That lease is not common semantics, and its new attempt projection digest
must be recorded. After a possible send, rematerialization follows the existing
idempotency/reconciliation rules and is never justified by hints alone. Each
projected request digest commits to all provider fields emitted for that
attempt.

### 3.5 Evidence never outruns knowledge

“The adapter sent a control,” “the backend acknowledged a control,” and “a
benefit was measured” are three different claims. Evidence records only the
first, and never restates it as one of the others (§6.3). Lack of
acknowledgement does not prove rejection; a cache hit does not prove an
affinity or retention intent was honored.

## 4. The common semantic model

The contract deliberately separates facts from intents:

```rust
pub struct ApxmGraphHints {
    pub schema: ContractId, // exactly "apxm.inference-graph-hints"
    pub scope: GraphHintScope,
    pub facts: NodeGraphFacts,
    pub intents: GraphExecutionIntents,
}

pub struct GraphHintScope {
    pub graph_ref: GraphRef,
    pub graph_execution_ref: GraphExecutionRef,
    pub node_ref: NodeRef,
    pub node_execution_ref: NodeExecutionRef,
}

pub struct NodeGraphFacts {
    pub critical_path: Option<bool>,
    pub successor_refs: Vec<NodeRef>,
    pub remaining_path_len: Option<u32>,
    pub stage_index: Option<u32>,
    pub work_class: Option<WorkClass>,
    pub estimated_input_tokens: Option<u32>,
    pub estimated_output_tokens: Option<u32>,
    pub expected_shared_prefix_tokens: Option<u32>,
    pub prefix_warmup_eligible: Option<bool>,
    pub pipeline_eligible: Option<bool>,
    pub coexecution_group_ref: Option<OpaqueGroupRef>,
}

pub enum WorkClass {
    Short,
    Medium,
    Long,
}

pub struct GraphExecutionIntents {
    pub objective: Option<OptimizationObjective>,
    pub reusable_context: Option<ReusableContextIntent>,
}

pub enum OptimizationObjective {
    MinimizeGraphCompletionTime,
    Balanced,
    MaximizeThroughput,
}

pub struct ReusableContextIntent {
    pub preference: ReusePreference, // initially only PreferWhenBeneficial
    pub affinity_ref: Option<OpaqueAffinityRef>,
    pub benefit_horizon_ms: Option<u32>,
    pub expected_uses: Option<u32>,
}
```

This is a target shape, not a commitment to the illustrated Rust module
layout. The important contract is the field semantics below.

### 4.1 Scope

All four scope values are opaque, non-empty, and runtime supplied. They carry
correlation, not authority or content.

- `graph_ref` identifies the admitted static graph.
- `graph_execution_ref` identifies one execution of that graph.
- `node_ref` identifies the static node.
- `node_execution_ref` identifies this runtime occurrence.

Node names are intentionally absent. Human-readable names often contain source
or product information and do not improve backend optimization.

### 4.2 Facts

| Field | Meaning | Must not be interpreted as |
|---|---|---|
| `critical_path` | Whether compiler/runtime analysis places this node on a current critical path | A provider queue-priority number |
| `successor_refs` | Direct graph successors known at materialization | Permission to execute or reorder them |
| `remaining_path_len` | Estimated count of downstream stages on the longest path | A deadline |
| `stage_index` | Stable compiler stage coordinate when one exists | A pipeline command |
| `work_class` | Coarse estimated inference work | A latency SLO |
| `estimated_input_tokens` | Planner estimate of model-visible input size | Provider token accounting |
| `estimated_output_tokens` | Planner estimate of likely output work | A replacement for the admitted output budget |
| `expected_shared_prefix_tokens` | Estimated reusable prefix shared with related calls | Proof that any cache contains it |
| `prefix_warmup_eligible` | Static proof that preparation would preserve semantics | An instruction to issue an extra model call |
| `pipeline_eligible` | Static proof that pipelining is semantically allowed | Permission to violate dependencies |
| `coexecution_group_ref` | Opaque group already proven compatible by APXM | Permission for the adapter to invent batching or correlation |

Facts may be absent when analysis cannot prove them. The producer must not
substitute zero, `false`, or a guessed group for unknown information.

### 4.3 Intents

`objective` tells an adapter which performance trade-off is useful for this
graph execution. It is not a deadline or SLO. `MinimizeGraphCompletionTime`
allows an adapter to favor critical-path progress; `MaximizeThroughput` allows
it to favor efficient coexecution; `Balanced` makes neither preference
dominant. Graph-execution-scoped intents must be identical on every node hint
for the same `graph_execution_ref`; conflicting values are rejected rather
than resolved by last-writer-wins behavior.

`reusable_context.preference = PreferWhenBeneficial` says APXM expects reuse to
be beneficial. Omission means no preference, not “disable all backend caches.”
Disabling reuse for privacy or deterministic-execution policy is a binding or
runtime-profile requirement outside this contract.

`affinity_ref` is an opaque APXM-derived identity for requests expected to
share reusable context. It must not contain prompt text, a tenant name, a raw
context digest, or a provider slot identifier. It does not authorize state
sharing across security or model-deployment boundaries. Materialization binds
it to the exact model deployment and admitted security scope before projection.

`benefit_horizon_ms` estimates when reuse stops being useful. It is neither a
minimum-retention promise nor a deletion deadline. An adapter may use it to
choose a TTL only when its Port Contract defines that lowering. A real maximum
retention policy belongs outside hints.

`expected_uses` is an estimate used to decide whether preparation cost is
worthwhile. It is not a reservation.

### 4.4 Materialization

The compiler emits a graph-hint template containing static facts. The runtime
materializes the complete object only after it owns the graph execution, node
execution, sealed context, and exact inference binding. Materialization:

1. validates all opaque references and numeric bounds;
2. adds runtime-known facts and intents without changing compiler-proven facts;
3. computes a canonical `graph_hints_digest`;
4. asks only the exact adapter to project it; and
5. seals the projection with the model-effect evidence before send.

No frontend or authored Agent Program supplies backend graph hints directly.

## 5. What is deliberately not in the contract

The following remain adapter or profile vocabulary:

- `pin`, `pin_policy`, `pinned_blocks`, `pinned_handles`, and cache-block TTL;
- `priority` numbers or provider queue policy;
- `slot`, `id_slot`, slot filenames, slot save/restore/erase, and worker ids;
- `vllm_xargs`, request headers, response headers, and provider route names;
- backend cache-reset controls and benchmark isolation switches;
- arbitrary `metadata`, `extra_body`, or extension maps; and
- structured output, tool calling, reasoning, multimodality, token limits, and
  other model capabilities unrelated to graph optimization.

There is intentionally no common `backend_options` escape hatch. Adding one
would recreate a provider-specific contract inside the common object.

## 6. Adapter projection contract

Every inference adapter implements a required projection seam:

The shipped seam is `GraphHintProjector`
(`crates/machine/contracts/src/types/graph_hints.rs`): declare, plan, render,
and seal. `project_graph_hints` is the only path from hints to a provider
request, and it validates the plan against the declaration before rendering
anything.

```rust
pub trait GraphHintProjector {
    fn graph_hint_capabilities(&self) -> GraphHintCapabilities;

    fn plan_graph_hints(
        &self,
        hints: Option<&ApxmGraphHints>,
    ) -> Result<GraphHintPlan, String>;

    fn render_graph_hint_fields(
        &self,
        hints: &ApxmGraphHints,
        plan: &GraphHintPlan,
    ) -> Result<Map<String, Json>, String>;

    fn project_graph_hints(
        &self,
        hints: Option<&ApxmGraphHints>,
        attempt: u32,
    ) -> Result<GraphHintDispatchProjection, String>;
}
```

Planning is a pure, deterministic transformation of the binding's declared
capability surface and the common hints; the attempt ordinal is the only other
input, and it does not change field outcomes.
`crates/runtime/backends/tests/graph_hint_projection_conformance.rs` holds
every adapter to that: two attempts of one envelope carry one plan, one
rendered request, and one projected-request digest.

**Status: partial.** The common descriptor and exact-binding lifecycle seam are
implemented (§7), and an adapter-local preparation digest can fence lifecycle
release. Attempt-local physical resource leases — such as a fenced llama.cpp
slot lease and rebinding after a proven pre-send failure — are intentionally not
implemented. No adapter advertises one.

### 6.1 Static capabilities

Capabilities are declared per exact driver/profile/binding, not per provider
brand:

```rust
pub struct GraphHintCapabilities {
    pub fields: BTreeMap<GraphHintField, GraphHintFieldCapability>,
    pub lifecycle: GraphLifecycleCapability,
}

pub enum GraphHintFieldCapability {
    Direct,
    Derived,
    Unsupported,
}
```

`Direct` means the adapter has a provider control with the same useful
semantic effect. `Derived` means it uses a documented heuristic or combination
of controls. Neither is a universal performance guarantee. Capabilities are
content-addressed (`GraphHintCapabilities::digest`) and the digest is optional
binding evidence checked before send by
`InferenceDriverBinding::authorize_graph_hint_capabilities`
(`crates/runtime/inference/src/driver.rs`), so a health probe cannot silently
widen them mid-effect. Declarations are complete and closed: every
`GraphHintField` is classified, and the projection seam rejects a plan that
relabels an unsupported field or commits to a different envelope.

An earlier draft of this section gave each supported field a set of
`EvidenceKind` values — `AdapterProjection`, `BackendAcknowledgement`,
`OutcomeMeasurement` — on the reasoning that evidence kinds form a set rather
than a strength ladder. That is still the right model of the world, but a
declaration is a claim about what this binding will produce, and only the
adapter's own projection is ever produced (see §6.3). The set was removed
rather than left as a per-field claim no dispatch could honor.

The closed `GraphHintField` enum is the only field-name owner. Unknown field
names fail validation instead of entering a stringly typed support table.

### 6.2 Per-dispatch plan and projection

```rust
pub struct GraphHintPlan {
    pub graph_hints_digest: Option<Digest>,
    pub capability_digest: Digest,
    pub outcomes: BTreeMap<GraphHintField, ProjectionOutcome>,
}

pub struct GraphHintProjection {
    pub plan_digest: Digest,
    pub attempt: u32,
    pub mechanism_bindings: Vec<BackendMechanismRef>,
    pub projected_request_digest: Digest,
}

pub enum ProjectionOutcome {
    Applied { mechanism_ref: BackendMechanismRef },
    Approximated { mechanism_ref: BackendMechanismRef, reason: ReasonCode },
    OmittedUnsupported,
    OmittedByProfile { reason: ReasonCode },
}
```

`mechanism_ref` is a closed, adapter-owned identifier such as
`llama.cache_prompt` or `vllm.request_priority`; it carries no secret values.
An attempt-local mechanism binding may later commit to a fenced resource by
digest, but never exposes its raw coordinate. The actual provider request stays
private to the adapter. Projection evidence must never include prompt content,
credentials, slot-save filenames, or raw provider bodies.

An invalid common value, capability-digest mismatch, or adapter attempt to
change an authority-bearing request field is a pre-send typed error. It is not
an omission.

### 6.3 Result evidence

An adapter records two claims, both its own: the field-by-field plan and the
projection it sent. `record_graph_hint_evidence`
(`crates/runtime/backends/src/llm/backends/graph_hint_dispatch.rs`) writes
exactly those two onto the response, by digest and closed vocabulary.

There is no third, response-side record. A `GraphHintRealization` carrying a
per-field `Acknowledgement` and a `reused_input_tokens` measurement was built
and then removed, because nothing could populate it:

- **Acknowledgement.** Only the provider can produce one, and no provider
  response any adapter here parses carries a field-level acknowledgement. The
  one mechanism that would have — vLLM's `fields_honored`, read through the
  conformance join — is retired by ADR 0021. llama.cpp's OpenAI-compatible
  response does not acknowledge `cache_prompt` individually (§9). Every
  constructed acknowledgement was therefore `NotReported`, on every response.
- **Measurement.** The only proposed vocabulary entry, `reused_input_tokens`,
  has one reachable source: `TokenUsage::cached_input_tokens`, parsed from
  `usage.prompt_tokens_details.cached_tokens` by the OpenAI adapter that both
  graph-aware adapters delegate to
  (`crates/runtime/backends/src/llm/backends/openai/backend.rs`). That field is
  a `usize` defaulted to zero, so it cannot distinguish a reported zero from an
  absent detail — precisely the distinction this section exists to preserve.
  Emitting it would manufacture a measured zero on every provider that reports
  nothing. The quantity itself is already recorded, as accounting, on the usage
  path (`LlmStepUsagePayload::cached_input_tokens` and the
  `apxm.usage.cached_input_tokens` span attribute).
- With those two gone, the remaining per-field `projected: bool` restated
  `GraphHintPlan::projects` and the `projection_digest` re-addressed a record
  already written out in full, so the whole type carried no information the
  other two did not.

**Status: intentionally incomplete.** The model of three separable claims is unchanged, and both
missing layers are worth having. Building the measurement layer needs a
reported-vs-zero signal the usage carrier deliberately does not have (it sums
`cached_input_tokens` across calls) and a provider that actually reports the
detail; building the acknowledgement layer needs a provider response contract
that states one per field. Until then the machine declares neither, and
`tools/tests/test_canonical_only_reachability.py`
(`test_no_graph_hint_evidence_layer_reports_only_nothing`) keeps the names from
returning without their producer.

Should a measurement layer return, its vocabulary stays small and only covers
quantities whose measurement point is genuinely comparable across providers.
Queue delay, service time, cache blocks, cache entries, and slots are not; they
remain namespaced adapter evidence.

## 7. Graph lifecycle

**Status: implemented at the adapter boundary.** `ApxmGraphDescriptor`,
`prepare_graph`, and digest-fenced `release_graph_preparation` are implemented
on `LLMBackend`. vLLM converts the fact-only descriptor to its adapter-owned
registration request and retains the provider graph id only in an adapter-local
preparation table. A transport or parse error after send returns
`OutcomeUnknown`; a stale or malformed preparation fails before send. llama.cpp
returns explicit `NotNeeded`, and the default backend path also returns
`NotNeeded` rather than a successful no-op. There is no registry broadcast. The
general graph executor does not yet call this seam because no live graph
lifecycle caller exists in this branch; direct adapter callers are exact-bound.

Some providers benefit from receiving static graph facts before the first
request. Others need no graph lifecycle at all. The common companion object
would therefore be a fact-only `ApxmGraphDescriptor`; it would contain graph
identity, node references, edges, and static node facts, but no cache or
scheduler commands.

Lifecycle would be exact-binding local:

```text
prepare_graph(exact_binding, descriptor)
  -> Prepared(projection evidence)
   | NotNeeded
   | Unsupported
   | FailedBeforeSend
   | OutcomeUnknown

release_graph_preparation(exact_binding, preparation_ref)
  -> Released
   | NotNeeded
   | FailedBeforeSend
   | OutcomeUnknown
```

There is no default successful no-op. `NotNeeded` and `Unsupported` are
explicit outcomes. Preparation and release are methods on one exact backend
binding, never registry operations.

Backend resource status is adapter evidence: `GraphStatusSnapshot` carries
adapter-keyed observations, and neither provider's resource units enter
`ApxmGraphHints`. Nothing calls `get_graph_status`, so no snapshot is produced.

## 8. Backend lowering matrix

The matrix is intentionally asymmetric. The common contract is successful
when each cell is honest, not when every backend manufactures the same control.
Both columns are the shipped `graph_hint_capabilities` / `plan_graph_hints` /
`render_graph_hint_fields` implementations in
`crates/runtime/backends/src/llm/backends/vllm/backend.rs` and
`.../llama_cpp/mod.rs`; the declared field sets are asserted exactly by
`crates/runtime/backends/tests/graph_hint_projection_conformance.rs`.

| Common semantic | vLLM projection | llama.cpp projection | Generic remote adapter |
|---|---|---|---|
| Scope refs | `Applied` into the provider extension envelope | `Applied` into the APXM envelope | `OmittedUnsupported` |
| `critical_path` | `Approximated` onto a queue value under the admitted scheduler policy; `OmittedByProfile(MechanismNotAdmitted)` without that policy, and `OmittedByProfile(ProfileWithholdsMechanism)` when the objective asks for throughput | `OmittedUnsupported` | `OmittedUnsupported` |
| Successors/stage/path facts | `Applied` into the extension envelope | `OmittedUnsupported` | `OmittedUnsupported` |
| `objective` | `Applied` into the extension envelope | `OmittedUnsupported` | `OmittedUnsupported` |
| Work/token estimates | `OmittedUnsupported` | `OmittedUnsupported` | `OmittedUnsupported` |
| Reuse preference | `Approximated` onto a prefix-reuse control | `Applied` as `cache_prompt: true` | `OmittedUnsupported` |
| `affinity_ref` | `Applied` into the extension envelope | `OmittedUnsupported` | `OmittedUnsupported` |
| `benefit_horizon_ms` | `Applied` into the extension envelope | `OmittedUnsupported` | `OmittedUnsupported` |
| Warmup eligibility | `OmittedUnsupported` | `OmittedUnsupported` | `OmittedUnsupported` |
| Pipeline eligibility | `OmittedUnsupported` | `OmittedUnsupported` | `OmittedUnsupported` |
| Coexecution group | `OmittedUnsupported` | `OmittedUnsupported`; server continuous batching remains server policy | `OmittedUnsupported` |
| Prepare/release graph | declares `PrepareRelease`; no caller | declares `NotNeeded` | declares `NotNeeded` |

Every `OmittedUnsupported` cell above is a fact the runtime still knows and
still records; it just does not reach the provider.

A provider-side response evidence row belonged here and no longer does: see
§6.3. The vLLM conformance join that would have carried the acknowledgement
half is retired by ADR 0021, along with `backend_join.rs` and the pinned vLLM
vector catalogue.

## 9. llama.cpp integration

llama.cpp is the primary portability test because its useful controls do not
look like the current vLLM-shaped graph surface.

### 9.1 The shipped integration

This section originally proposed pinning a stock `llama-server` contract. ADR
0021 decided otherwise: llama.cpp, like vLLM, lives on an APXM-org `apxm`
branch that receives the envelope. The shipped adapter
(`crates/runtime/backends/src/llm/backends/llama_cpp/mod.rs`) targets that
branch and uses the OpenAI-compatible Chat Completions endpoint for ordinary
inference, delegating transport, usage parsing, and streaming wholesale to the
OpenAI adapter.

It declares two supported fields and does two provider lowerings:

```text
scope                                        -> apxm envelope
reusable_context.preference = prefer_when_beneficial
                                             -> request field cache_prompt = true
```

llama.cpp documents `cache_prompt` as reusing a common prompt prefix and
processing only the unseen suffix. It also warns that different batching can
make logits non-bit-identical. The projected field is therefore part of
`projected_request_digest` and stable across attempts, which
`graph_hint_projection_conformance.rs` asserts. The adapter now gates this
lowering on the exact registration profile key
`graph_hint_cache_prompt_admitted`; a false value produces
`OmittedByProfile(ProfileWithholdsMechanism)` while still carrying the scope
envelope. See the official
[completion option documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#post-completion-given-a-prompt-it-returns-the-predicted-completion).

The adapter records `Applied(llama.cache_prompt)` at projection time and claims
nothing further. It does not claim backend acknowledgement, because the
response does not acknowledge that field individually. It also records no
measurement: the adapter does no response parsing of its own, and the
delegated OpenAI usage path flattens an absent
`usage.prompt_tokens_details.cached_tokens` to zero (§6.3). Nothing in this
repository reads llama.cpp's native `timings.cache_n` — the delegated response
parser does not look for it, and an adapter that wanted it would first need a
way to see raw provider JSON that delegation does not give it.

All topology, priority, affinity, horizon, warmup, and pipeline fields are
explicitly omitted as unsupported. Structured output and tool calling are
admitted model capabilities implemented by the ordinary model adapter, not by
graph hints.

### 9.2 Why `id_slot` is not the affinity lowering

The native completion API exposes `id_slot`, and the server exposes slot
monitoring plus save, restore, and erase operations. Those mechanisms are not
yet an APXM affinity contract. Directly hashing `affinity_ref` into a slot would
permit collisions, target a busy slot, conflate capacity with identity, and
leave cancellation/recovery and cross-execution cleanup undefined. The
official server documentation describes
[slot state](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#get-slots-returns-the-current-slots-processing-state)
and [slot persistence operations](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#post-slotsid_slotactionsave-save-the-prompt-cache-of-the-specified-slot-to-a-file),
but those endpoints alone do not provide the missing ownership semantics.

A future llama.cpp affinity implementation requires a separate, exact slot
lease protocol in the adapter:

1. atomically lease an available slot to the exact model deployment and
   `affinity_ref`;
2. fence concurrent or stale lease holders;
3. bind every `id_slot` projection to the lease digest;
4. define busy, unavailable, cancellation, and process-restart outcomes;
5. erase or release state according to an admitted retention policy, not a
   hint; and
6. emit lease and slot evidence without exposing filenames or prompt content.

Only after those vectors pass may `affinity_ref` become `Derived` or `Direct`
for the llama.cpp binding. Slot save/restore is optional even then; it is not a
prerequisite for useful prompt-prefix reuse.

### 9.3 The `apxm` branch is the receiver

This section originally argued that no llama.cpp fork was required, since
`cache_prompt` is a stock control. ADR 0021 settled the ownership question the
other way: llama.cpp is an APXM-org `apxm`-branch receiver of the envelope, on
the same footing as vLLM. The Agents side is unaffected either way — the two
projected fields are the two the adapter declares, and a receiver that ignores
the `apxm` envelope entirely still gets a valid request.

## 10. Failure and degradation rules

| Condition | Required behavior |
|---|---|
| Hints absent | Send the ordinary admitted request; record no projection |
| Known field unsupported | Omit it and record `OmittedUnsupported` |
| Profile disables a supported lowering | Omit it and record `OmittedByProfile` with a closed reason |
| Unknown field or enum value | Reject before send |
| Capability digest changed | Reject/re-admit before send; do not widen in place |
| Projection tries to alter model semantics or authority | Reject before send as an adapter contract violation |
| Optional lifecycle preparation fails before send | Apply admitted degradation policy and record it; do not broadcast |
| Required capability absent | Fail binding admission, not runtime provider selection |
| Lifecycle or inference outcome uncertain after send | Preserve the existing typed `OutcomeUnknown` discipline |
| Backend omits acknowledgement | Record nothing about acknowledgement; do not infer success or failure (§6.3) |

An optional hint may degrade only performance. If dropping it could change a
typed result, security boundary, billing commitment, or correctness property,
it was misclassified and must move out of this contract.

## 11. Migration from the current implementation

This is an in-place replacement, not a parallel legacy envelope.

| Current surface | Target |
|---|---|
| `ApxmGraphHints` with mixed facts and mechanisms | `ApxmGraphHints { scope, facts, intents }` |
| `schema_version: 1` | unversioned APXM-owned `schema` identity |
| numeric `node_id` plus optional name | opaque typed node and node-execution refs |
| `PriorityClass` | `critical_path` fact plus graph optimization objective |
| `reuse_group` | opaque `affinity_ref` intent |
| `PinMode` / `PinPolicy` | vLLM lowering; no common pin vocabulary |
| `CompilerHints` | normalized fact fields with precise eligibility semantics |
| boolean `BackendGraphCapabilities` | field-level `Direct` / `Derived` / `Unsupported` capability table |
| `supports_graph_extensions()` | deleted; `GraphLifecycleCapability` is the declaration, and it enters the capability digest |
| default successful `register_graph` / `release_graph` | legacy methods fail closed; exact `prepare_graph` / `release_graph_preparation` returns typed lifecycle outcomes |
| `register_graph_all` / `release_graph_all` | deleted |
| `GraphAware` / `Generic` backend kind | capability evidence for the exact binding |
| common pinned handle/block status | adapter-owned observations on `GraphStatusSnapshot` |
| direct common-envelope serialization into `vllm_xargs` | vLLM projection from common semantics into its own wire contract |
| response `fields_honored` strings | nothing; the acknowledgement layer is not built (§6.3) |

Backend request types carry `apxm_hints` as the single provider-neutral
attachment point (`LLMRequest::apxm_hints` in
`crates/runtime/backends/src/llm/backends/request.rs`), and all wire rendering
is behind the projectors.

## 12. Implementation plan

Phases A, B, D, and F have landed. Phase C's projector exists but its provider
wire has not been re-attested against an `apxm` vLLM branch. Phase E's exact
binding seam is now landed, but there is no live graph-executor caller and no
fenced llama.cpp slot lease. The cached-token measurement remains deliberately
unbuilt because the current usage carrier cannot distinguish reported zero from
absence (§6.3).

### Phase A — freeze semantics and vectors

1. Turn this proposal into an ADR and canonical contract after review.
2. Define the closed field enum, validation limits, canonical serialization,
   digest rules, and positive/negative JSON vectors.
3. Define `ApxmGraphDescriptor`, lifecycle outcomes, projection outcomes,
   evidence grades, and reason codes.
4. Add a rule that graph hints cannot originate in frontend source.

Exit gate: two backend-independent vectors produce the same canonical hint
digest, and unknown fields/invalid references fail closed.

Landed as `contracts/schemas/apxm.inference-graph-hints.json` and its vectors,
`ApxmGraphHints::{canonical_json, digest}`, the closed `ReasonCode` enum, the
fact-only `ApxmGraphDescriptor` and digest-fenced lifecycle identities, and the
numeric/reference limits published in both the schema and the type. The
frontend-origination rule is enforced by
`tools/tests/test_canonical_only_reachability.py`
(`test_graph_hints_cannot_originate_in_authored_source`).

### Phase B — common projection seam

1. Replace mixed graph-hint types in the contracts crate.
2. Add `GraphHintProjector` at the exact inference-adapter boundary.
3. Bind the capability digest into `InferenceDriverBinding` evidence.
4. Add projection evidence to the runtime record without embedding raw
   provider bodies.
5. Remove graph-wide registry broadcast and binary graph-aware discovery.

Exit gate: a backend with zero capabilities returns a complete explicit
projection report and receives an otherwise unchanged model request.

Landed. `GraphHintProjector::project_graph_hints` is the only path from hints
to a provider request; the capability digest is optional evidence on
`InferenceDriverBinding` with a pre-send drift check; plan and projection are
recorded in response metadata by digest. The registry broadcast and the
graph-aware backend search are deleted (§2). Step 4 originally also said
"realization" — that record was removed for the reasons in §6.3.

### Phase C — migrate vLLM without changing its provider contract

1. Implement the vLLM projector from common facts/intents.
2. Keep pin policy, numerical priority, APXM routes, `vllm_xargs`, and pinned
   resource units inside the vLLM adapter.
3. ~~Preserve the exact external vLLM conformance join~~ — retired by ADR 0021.
4. ~~Replace string `fields_honored` handling with typed realization evidence~~
   — dropped with the join; there is no acknowledgement to parse (§6.3).

Exit gate: equivalent semantic inputs produce the intended existing vLLM wire
controls, and no vLLM mechanism remains in common graph types.

Steps 1, 2, and the exit gate's second half have landed. The first half —
that the wire the projector produces is the wire the receiver expects — has not
been re-attested against an `apxm` vLLM branch since the projector replaced
verbatim forwarding.

### Phase D — add the llama.cpp backend

1. Add an exact `llama.cpp` driver/profile and adapter, separate from the
   generic OpenAI provider identity even when transport structs are reused.
2. Pin health, model identity, chat, stream, usage, error, tool, and structured
   output behavior with conformance vectors.
3. Implement the initial graph projector: reuse preference to
   `cache_prompt: true`; every other field explicit unsupported/local-only.
4. ~~Parse cached-token measurements and bind them to the projection digest.~~
   Not built, and not pending: see §6.3 for why the only reachable source
   cannot state the measurement honestly.
5. Exercise buffered and streaming inference with and without reusable prefix.

Exit gate: the same canonical hints run through both the vLLM and llama.cpp
projectors, produce different expected projections, and preserve identical APXM
model-effect identity and output contracts.

Landed. The adapter is a distinct provider identity that reuses the OpenAI
transport (steps 1, 3). Adapter conformance runs the published hint vectors
through every projector in
`crates/runtime/backends/tests/graph_hint_projection_conformance.rs`, which
also states each binding's declared field set exactly, so the exit gate's
"different expected projections" is asserted rather than assumed. Step 2's
health/model-identity/error/tool/structured-output pinning is not done: those
behaviours are the OpenAI adapter's, tested there generically and not pinned
for this provider. Streaming (step 5) carries the same evidence through
`stream_with_graph_hint_evidence`, asserted against the buffered form in
`graph_hint_dispatch.rs`. Neither form is exercised against a live server with
and without a reusable prefix, and doing so would prove nothing here: the
difference would show up as a measurement, and there is none (§6.3).

### Phase E — lifecycle and optional llama.cpp slot leasing

1. Replace vLLM broadcast lifecycle with exact-binding preparation/release.
2. Measure whether llama.cpp cache selection is sufficient.
3. Only if measurements justify it, design and implement the fenced slot lease
   protocol from section 9.2 behind a separate profile capability.
4. Keep save/restore/erase disabled unless explicitly admitted and secured.

Exit gate: crash, busy-slot, cancellation, restart, collision, and stale-lease
vectors pass before llama.cpp advertises affinity or lifecycle support.

Step 1 is implemented as an exact-binding seam rather than a broadcast: the
vLLM adapter prepares/releases a digest-fenced registration, llama.cpp returns
`NotNeeded`, and stale-release/invalid-descriptor/uncertain-transport outcomes
are typed. Step 2 has no honest instrument — measuring cache selection needs the
measurement layer §6.3 explains is not built. Steps 3 and 4 remain deliberately
unstarted: llama.cpp advertises neither affinity nor lifecycle support, which is
what the exit gate protects.

### Phase F — removal and consistency gates

1. Delete old pin/priority/backend-kind types from common contracts.
2. Delete string field lists, silent default lifecycle success, and registry
   broadcasts.
3. Update documentation and observability names to fact/intent/projection
   vocabulary.
4. Add reachability checks preventing backend mechanism names from returning
   to common graph-hint modules.

Exit gate: repository search finds `pin`, `slot`, `vllm_xargs`, and provider
priority fields only under provider adapters, tests, or provider-specific docs.

Landed. `vllm_xargs`, the provider priority key and its queue values, and the
pinned-resource observation names live in
`crates/runtime/backends/src/llm/backends/vllm/graph_meta.rs`; `cache_prompt`
lives in `.../llama_cpp/mod.rs`. `test_common_contracts_name_no_provider_mechanism`
holds the four common contract crates against a forbidden-name list.

## 13. Required conformance tests

### Common contract

- canonical field ordering and stable digest across construction paths;
- unknown fields, empty refs, invalid digests, and out-of-range estimates fail;
- absent facts remain absent rather than receiving guessed defaults;
- hints cannot contain messages, prompt data, credentials, endpoint, model, or
  arbitrary provider metadata;
- the materialized digest, capability snapshot, and semantic projection
  outcomes are stable across attempts; attempt-local leases are separately
  digested; and
- omission of all hints leaves the ordinary model request unchanged.

### Projection

Adapter conformance runs the published hint vectors through every projector in
`crates/runtime/backends/tests/graph_hint_projection_conformance.rs`. It lives
with the adapters, not in `contracts/vectors/`, because what an adapter
projects is a per-binding decision named in provider mechanism vocabulary that
Phase F moved out of the common crates and gates from returning; the published
vectors are the neutral *input*, and the assertions are this contract.

- every present common field has exactly one projection outcome;
- unsupported and profile-omitted fields cannot appear in provider requests;
- what a binding sends stays within what it declares;
- an unsupported field is reported `OmittedUnsupported`, never disguised as a
  profile choice;
- projection is deterministic, and a retry carries the same plan, the same
  rendered request, and the same projected-request digest;
- projection evidence carries no scope or successor reference from the envelope
  it commits to;
- a rejected envelope is rejected by every projector; and
- capability-digest drift fails before send
  (`crates/runtime/inference/tests/driver_binding_conformance.rs`).

The adapter unit suite now asserts that projection preserves target, context,
tools, output schema, budgets, sampling, and existing provider extensions, and
that conflicting or non-object extension bodies fail before send.

### vLLM

- critical-path and objective lower to priority only under the admitted
  scheduler policy (`graph_hint_dispatch.rs`, inline);
- affinity and horizon lower into the provider extension envelope; and
- the declared field set is exactly the nine in §8.

The exact-binding lifecycle seam is exercised in
`graph_hint_dispatch.rs` for `NotNeeded`, invalid descriptors, and stale
preparations. Acknowledgement parsing is gone with the conformance join, along
with the pinned external vectors.

### llama.cpp

- reuse preference emits explicit `cache_prompt: true`;
- the buffered and streamed forms carry identical evidence;
- no preference does not invent an APXM cache-disable policy;
- affinity, horizon, topology, priority, warmup, and pipeline report unsupported
  and never emit `id_slot`; and
- the declared field set is exactly `scope` and the reuse preference.

The provider request digest changing when the profile withholds cache behavior
is asserted directly. Structured output, tool calling, and unavailable or
post-send-uncertain outcomes remain the delegated OpenAI adapter's behavior and
are not separately pinned for this provider. The two cached-token bullets this
list used to carry are removed: see §6.3.

### Exact binding and lifecycle

- no registry-wide broadcast or graph-aware backend discovery occurs — asserted
  by absence: the functions are deleted, and
  `tools/tests/test_canonical_only_reachability.py` keeps them out.

**Status: implemented at the exact adapter boundary.** The seam distinguishes
`NotNeeded`, `Unsupported`, pre-send failure, and uncertain post-send outcome,
and refuses a release that targets a stale preparation. A graph-executor caller
that selects affected exact bindings is still outside this branch.

## 14. Acceptance criteria

The design is complete when all of the following are true. Each is marked with
where it currently stands.

1. **Met.** One APXM-owned type expresses only graph facts and performance
   intents.
2. **Met.** Every backend implements the same projection interface, including
   explicit zero support.
3. **Met.** vLLM-specific scheduling, pinning, routes, status units, and wire
   fields live only in the vLLM adapter.
4. **Met**, against an `apxm`-branch receiver rather than a stock server
   (§9.1, §9.3): llama.cpp runs ordinary APXM model calls and carries
   reusable-prefix intent.
5. **Met.** llama.cpp does not claim affinity, TTL, priority, or graph
   lifecycle support.
6. **Not met, and narrowed.** Static capability and per-request projection are
   two distinguishable evidence layers. Backend acknowledgement and outcome
   measurement are not layers the machine has; §6.3 records why, and what
   producing either would require.
7. **Met at the adapter boundary.** Nothing broadcasts; lifecycle preparation
   and release are exact-binding local and digest-fenced. The general graph
   executor has no caller for the seam yet.
8. **Met by construction**, not by assertion — see §13.
9. **Met** by retiring the vLLM join rather than keeping it independent
   (ADR 0021).
10. **Met.** Every unsupported optimization is an explicit `OmittedUnsupported`
    in the plan, and adapter conformance asserts it.

## 15. Recommended first cut

The smallest implementation that proves the architecture was:

1. land the fact/intent type and projection-result types;
2. implement a zero-capability projector for the mock/generic adapter;
3. migrate vLLM through a projector without changing its provider wire;
4. add llama.cpp through Chat Completions with `scope` and
   `PreferWhenBeneficial -> cache_prompt: true`;
5. ~~normalize cached-token measurements~~ (§6.3); and
6. ~~remove broadcast graph lifecycle only after the exact-bound lifecycle seam
   is exercised by vLLM~~ — the broadcast was removed first, because it had no
   caller to exercise and keeping dead code until a replacement arrives is how
   a surface that binds nothing survives.

Steps 1 through 4 are in. What the cut proved holds: APXM owns the semantics,
adapters own mechanisms, and evidence connects the two without pretending all
backends are identical. What it did not prove is any claim about what a
provider did with a hint — see §6.3.
