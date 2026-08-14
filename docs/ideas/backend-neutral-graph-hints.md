# Backend-neutral APXM graph hints

- Status: design
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
what it projected, approximated, omitted, and later observed. Supporting zero
fields is a conforming implementation; silently claiming support is not.

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
 provider request and response evidence
```

The provider-specific contract remains exact and independently conformant. A
common source contract does not turn provider transports into one protocol.

## 2. Why the current surface needs to change

The current type is already named `ApxmGraphHints`, and part of it is genuinely
portable: graph/execution/node identity, downstream nodes, compiler estimates,
fan-out, remaining path length, and stage information. The same type also owns
`PinMode`, `PinPolicy`, `PriorityClass`, a default pin TTL, and graph status in
units of pinned handles and blocks. Those are realizations of graph intent, not
portable graph semantics. See
`crates/machine/contracts/src/types/graph_hints.rs:14-147`,
`:258-454`, and `:456-535`.

The vLLM adapter currently serializes the common object directly into
`vllm_xargs.apxm` and separately derives a numerical request priority from it.
That makes the common shape double as a vLLM wire shape instead of making vLLM
an explicit lowering target. See
`crates/runtime/backends/src/llm/backends/vllm/backend.rs:476-532`.

Graph lifecycle is currently broadcast to every registered backend on a
best-effort basis. Default backend implementations return success for graph
registration and release even when they do nothing. Status discovery then
classifies backends as either `GraphAware` or `Generic`. See
`crates/runtime/backends/src/llm/backends/traits.rs:168-227` and
`crates/runtime/backends/src/llm/registry/mod.rs:889-967`.

That lifecycle contradicts the exact inference-binding direction already
implemented elsewhere: one model effect resolves to one admitted driver, with
no ranking, substitution, or fallback
(`crates/runtime/inference/src/driver.rs:1-5`). Graph hint projection should
follow the exact binding too.

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
benefit was measured” are three different claims. The evidence model keeps
them separate. Lack of acknowledgement does not prove rejection; a cache hit
does not prove an affinity or retention intent was honored.

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

```rust
pub trait GraphHintProjector {
    fn graph_hint_capabilities(&self) -> GraphHintCapabilities;

    fn plan_graph_hints(
        &self,
        binding: &InferenceDriverBinding,
        hints: &ApxmGraphHints,
    ) -> Result<GraphHintPlan, GraphHintProjectionError>;

    async fn materialize_graph_hint_plan(
        &self,
        plan: &GraphHintPlan,
        attempt: &InferenceAttempt,
    ) -> Result<GraphHintProjection, GraphHintProjectionError>;

    fn observe_graph_hint_result(
        &self,
        projection: &GraphHintProjection,
        response: &BackendResponse,
    ) -> GraphHintRealization;
}
```

Planning is a pure, deterministic transformation of the exact binding, its
admitted capability snapshot, and the common hints. Materialization binds that
stable plan to one attempt and may acquire an attempt-local provider resource,
such as a future fenced llama.cpp slot lease. Ordinary HTTP transport remains
elsewhere. A materializer may change physical coordinates after a proven
pre-send failure, but it cannot change the plan's field outcomes.

### 6.1 Static capabilities

Capabilities are declared per exact driver/profile/binding, not per provider
brand:

```rust
pub struct GraphHintCapabilities {
    pub contract_digest: Digest,
    pub fields: BTreeMap<GraphHintField, GraphHintFieldCapability>,
    pub lifecycle: GraphLifecycleCapability,
}

pub enum GraphHintFieldCapability {
    Direct { evidence: BTreeSet<EvidenceKind> },
    Derived { evidence: BTreeSet<EvidenceKind> },
    Unsupported,
}

pub enum EvidenceKind {
    AdapterProjection,
    BackendAcknowledgement,
    OutcomeMeasurement,
}
```

`Direct` means the adapter has a provider control with the same useful
semantic effect. `Derived` means it uses a documented heuristic or combination
of controls. Neither is a universal performance guarantee. Capabilities are
content-addressed and included in binding evidence so a health probe cannot
silently widen them mid-effect.

Evidence kinds form a set, not a strength ladder. A provider may expose a
measured cache hit without acknowledging the request control, or acknowledge a
control without exposing a comparable measurement.

The closed `GraphHintField` enum is the only field-name owner. Unknown field
names fail validation instead of entering a stringly typed support table.

### 6.2 Per-dispatch plan and projection

```rust
pub struct GraphHintPlan {
    pub graph_hints_digest: Digest,
    pub binding_digest: Digest,
    pub capability_digest: Digest,
    pub outcomes: BTreeMap<GraphHintField, ProjectionOutcome>,
}

pub struct GraphHintProjection {
    pub plan_digest: Digest,
    pub attempt: u32,
    pub mechanism_bindings: Vec<BackendMechanismBinding>,
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
An attempt-local `BackendMechanismBinding` may commit to a fenced resource by
digest, but never exposes its raw coordinate. The actual provider request stays
private to the adapter. Projection evidence must never include prompt content,
credentials, slot-save filenames, or raw provider bodies.

An invalid common value, capability-digest mismatch, or adapter attempt to
change an authority-bearing request field is a pre-send typed error. It is not
an omission.

### 6.3 Result evidence

```rust
pub struct GraphHintRealization {
    pub projection_digest: Digest,
    pub fields: BTreeMap<GraphHintField, FieldRealization>,
    pub measurements: Vec<GraphHintMeasurement>,
}

pub struct FieldRealization {
    pub projected: bool,
    pub acknowledgement: Acknowledgement,
}

pub enum Acknowledgement {
    BackendAcknowledged,
    BackendRejected,
    NotReported,
}
```

Measurements use a small common vocabulary only where the quantity and
measurement point are genuinely comparable. The initial common measurement is
`reused_input_tokens`. Queue delay, service time, cache blocks, cache entries,
and slots remain namespaced adapter evidence until APXM defines comparable
measurement points and units; those values are not interchangeable.

`NotReported` is not “not honored.” `BackendRejected` is allowed only when the
provider contract makes the rejection explicit. Only the provider can produce
an acknowledgement; only a returned or independently observed metric can
produce measurement evidence.

## 7. Graph lifecycle

Some providers benefit from receiving static graph facts before the first
request. Others need no graph lifecycle at all. The common companion object is
therefore a fact-only `ApxmGraphDescriptor`; it contains graph identity, node
references, edges, and static node facts, but no cache or scheduler commands.

Lifecycle is exact-binding local:

```text
prepare_graph(exact_binding, descriptor)
  -> Prepared(projection evidence)
   | NotNeeded
   | Unsupported
   | FailedBeforeSend
   | OutcomeUnknown

release_graph(exact_binding, preparation_ref)
  -> Released
   | NotNeeded
   | FailedBeforeSend
   | OutcomeUnknown
```

There is no default successful no-op. `NotNeeded` and `Unsupported` are
explicit outcomes. Preparation and release are invoked only on the exact
adapter bound to affected model calls, never across the registry.

Backend resource status is adapter evidence. The common runtime records
lifecycle state and portable measurements; vLLM may additionally record pinned
handles/blocks, while llama.cpp may record slot or prompt-cache observations.
Neither provider's resource units enter `ApxmGraphHints`.

## 8. Backend lowering matrix

The matrix is intentionally asymmetric. The common contract is successful
when each cell is honest, not when every backend manufactures the same control.
The vLLM column summarizes the current adapter injection and capability surface
in `crates/runtime/backends/src/llm/backends/vllm/backend.rs:476-532` and
`:732-810`. The llama.cpp column is a target projection against the official
server contract cited in section 9.

| Common semantic | vLLM projection | llama.cpp projection | Generic remote adapter |
|---|---|---|---|
| Scope refs | APXM extension correlation | Local adapter evidence only | Local adapter evidence only |
| `critical_path` + completion-time objective | Derived numerical request priority when the admitted scheduler contract supports it | `OmittedUnsupported` | `OmittedUnsupported` |
| Successors/stage/path facts | Graph registration and APXM scheduler metadata | `OmittedUnsupported` | `OmittedUnsupported` |
| Work/token estimates | Scheduler or observability input when supported | `OmittedUnsupported`; the common facts remain in runtime evidence | `OmittedUnsupported`; the common facts remain in runtime evidence |
| Reuse preference | Prefix-reuse control | `cache_prompt: true` | `OmittedUnsupported` unless the exact provider contract says otherwise |
| `affinity_ref` | Prefix cohort/reuse-group lowering | `OmittedUnsupported` initially | `OmittedUnsupported` |
| `benefit_horizon_ms` | Derived pin/retention TTL | `OmittedUnsupported` initially | `OmittedUnsupported` |
| Warmup eligibility | Backend preparation only when explicitly supported | `OmittedUnsupported` initially | `OmittedUnsupported` |
| Pipeline eligibility | Backend scheduling input when explicitly supported | `OmittedUnsupported` initially | `OmittedUnsupported` |
| Coexecution group | Correlated batch/scheduler input after APXM compatibility proof | Server continuous batching remains server policy; the group is not sent | Only through a separately proven correlated-batching contract |
| Reused-token measurement | Provider/fork cache evidence | `timings.cache_n` or `usage.prompt_tokens_details.cached_tokens` | When exposed by exact provider response contract |
| Prepare/release graph | APXM graph routes | `NotNeeded` in the first integration | `NotNeeded` or `Unsupported` |

The vLLM-specific conformance join remains provider-owned and exact; it is not
replaced by this common contract. The current join already treats vLLM vectors
as pinned external evidence rather than a second APXM implementation of vLLM
semantics (`crates/runtime/inference/src/backend_join.rs:1-35`).

## 9. llama.cpp integration

llama.cpp is the primary portability test because its useful controls do not
look like the current vLLM-shaped graph surface.

### 9.1 First integration: stock `llama-server`

The first adapter should target a pinned stock `llama-server` contract and use
its OpenAI-compatible Chat Completions endpoint for ordinary inference. The
server documents synchronous and streaming chat completions, server-specific
completion options on that endpoint, structured response formats, and tool
calling separately from graph optimization. See the official
[llama-server documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#post-v1chatcompletions-openai-compatible-chat-completions-api).

For graph hints, the first implementation does exactly one provider lowering:

```text
reusable_context.preference = prefer_when_beneficial
    -> request field cache_prompt = true
```

llama.cpp documents `cache_prompt` as reusing a common prompt prefix and
processing only the unseen suffix. It also warns that different batching can
make logits non-bit-identical. The adapter must therefore explicitly include
the projected field in `projected_request_digest` and keep it stable across
attempts. It may advertise this lowering only for a model deployment whose
admitted inference profile permits that numerical-determinism envelope;
otherwise the field is `OmittedByProfile`. See the official
[completion option documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#post-completion-given-a-prompt-it-returns-the-predicted-completion).

The adapter records `Applied(llama.cache_prompt)` at projection time. It does
not claim backend acknowledgement because the response does not acknowledge
that field individually. When returned, `timings.cache_n` or
`usage.prompt_tokens_details.cached_tokens` becomes the common measured value
`reused_input_tokens`; a zero value is a valid measurement, not proof the
control was ignored. llama.cpp documents both response forms in
[Timings and context usage](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md#timings-and-context-usage).

All topology, priority, affinity, horizon, warmup, and pipeline fields are
explicitly omitted as unsupported in the first integration. Structured output
and tool calling are admitted model capabilities and must be implemented by
the ordinary llama.cpp model adapter, not by graph hints.

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

### 9.3 No llama.cpp fork is required initially

The first integration needs no custom server routes and no APXM payload inside
llama.cpp. A future fork or upstream extension is justified only if a measured
use case cannot be expressed through stock controls and the extension has its
own pinned Port Contract and conformance vectors.

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
| Backend omits acknowledgement | Record `NotReported`; do not infer success or failure |

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
| `supports_graph_extensions()` | explicit lifecycle capability |
| default successful `register_graph` / `release_graph` | explicit `NotNeeded` / `Unsupported` outcomes |
| `register_graph_all` / `release_graph_all` | exact-bound adapter lifecycle |
| `GraphAware` / `Generic` backend kind | capability evidence for the exact binding |
| common pinned handle/block status | common lifecycle/evidence plus adapter-owned observations |
| direct common-envelope serialization into `vllm_xargs` | vLLM projection from common semantics into its pinned wire contract |
| response `fields_honored` strings | typed field realization joined to projection digest |

Existing backend request types already carry `apxm_hints` as a provider-neutral
field (`crates/runtime/backends/src/llm/backends/request.rs:122-176`). The
migration should preserve that single attachment point while replacing its
shape and moving all wire rendering behind projectors.

## 12. Implementation plan

### Phase A — freeze semantics and vectors

1. Turn this proposal into an ADR and canonical contract after review.
2. Define the closed field enum, validation limits, canonical serialization,
   digest rules, and positive/negative JSON vectors.
3. Define `ApxmGraphDescriptor`, lifecycle outcomes, projection outcomes,
   evidence grades, and reason codes.
4. Add a rule that graph hints cannot originate in frontend source.

Exit gate: two backend-independent vectors produce the same canonical hint
digest, and unknown fields/invalid references fail closed.

### Phase B — common projection seam

1. Replace mixed graph-hint types in the contracts crate.
2. Add `GraphHintProjector` at the exact inference-adapter boundary.
3. Bind the capability digest into `InferenceDriverBinding` evidence.
4. Add projection and realization to runtime evidence without embedding raw
   provider bodies.
5. Remove graph-wide registry broadcast and binary graph-aware discovery.

Exit gate: a backend with zero capabilities returns a complete explicit
projection report and receives an otherwise unchanged model request.

### Phase C — migrate vLLM without changing its provider contract

1. Implement the vLLM projector from common facts/intents.
2. Keep pin policy, numerical priority, APXM routes, `vllm_xargs`, and pinned
   resource units inside the vLLM adapter.
3. Preserve the exact external vLLM conformance join and re-attest any changed
   APXM-side vector digests.
4. Replace string `fields_honored` handling with typed realization evidence
   while parsing the provider-owned acknowledgement at the adapter boundary.

Exit gate: equivalent semantic inputs produce the intended existing vLLM wire
controls, and no vLLM mechanism remains in common graph types.

### Phase D — add the llama.cpp backend

1. Add an exact `llama.cpp` driver/profile and adapter, separate from the
   generic OpenAI provider identity even when transport structs are reused.
2. Pin health, model identity, chat, stream, usage, error, tool, and structured
   output behavior with conformance vectors.
3. Implement the initial graph projector: reuse preference to
   `cache_prompt: true`; every other field explicit unsupported/local-only.
4. Parse cached-token measurements and bind them to the projection digest.
5. Exercise buffered and streaming inference with and without reusable prefix.

Exit gate: the same canonical hints run through both the vLLM and llama.cpp
projectors, produce different expected projections, and preserve identical APXM
model-effect identity and output contracts.

### Phase E — lifecycle and optional llama.cpp slot leasing

1. Replace vLLM broadcast lifecycle with exact-binding preparation/release.
2. Measure whether stock llama.cpp cache selection is sufficient.
3. Only if measurements justify it, design and implement the fenced slot lease
   protocol from section 9.2 behind a separate profile capability.
4. Keep save/restore/erase disabled unless explicitly admitted and secured.

Exit gate: crash, busy-slot, cancellation, restart, collision, and stale-lease
vectors pass before llama.cpp advertises affinity or lifecycle support.

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

- every present common field has exactly one projection outcome;
- unsupported and profile-omitted fields cannot appear in provider requests;
- `Applied` cannot be upgraded to `BackendAcknowledged` without provider
  evidence;
- measurements join only to the matching projection and model effect;
- capability-digest drift fails before send; and
- no projector can change target, context, tools, output schema, budgets,
  sampling, or authority-bearing fields.

### vLLM

- critical-path and objective lower to priority only under the admitted
  scheduler capability;
- affinity and horizon lower to provider-owned prefix cohort/pin controls;
- graph preparation/release is exact-binding local;
- acknowledgement parsing produces typed realization evidence; and
- existing pinned external vectors remain independently joined.

### llama.cpp

- reuse preference emits explicit `cache_prompt: true` in buffered and streamed
  chat requests;
- no preference does not invent an APXM cache-disable policy;
- affinity, horizon, topology, priority, warmup, and pipeline report unsupported
  and never emit `id_slot`;
- `timings.cache_n` and `cached_tokens` normalize to `reused_input_tokens`;
- zero cached tokens remains a measured zero, not a failed-hint claim;
- provider request digest changes when projected cache behavior changes;
- structured output and tool calling work independently of graph hints; and
- an unavailable or post-send-uncertain llama.cpp request follows the existing
  typed inference outcome rules.

### Exact binding and lifecycle

- one graph execution using different admitted model bindings prepares only the
  adapters actually used by its nodes;
- no registry-wide broadcast or graph-aware backend discovery occurs;
- `NotNeeded`, `Unsupported`, pre-send failure, and uncertain post-send outcome
  are distinguishable; and
- release cannot target a different binding or stale preparation reference.

## 14. Acceptance criteria

The design is complete when all of the following are true:

1. One APXM-owned type expresses only graph facts and performance intents.
2. Every backend implements the same projection interface, including explicit
   zero support.
3. vLLM-specific scheduling, pinning, routes, status units, and wire fields
   live only in the vLLM adapter.
4. llama.cpp runs ordinary APXM model calls and can use reusable-prefix intent
   through stock `llama-server` without an APXM server fork.
5. llama.cpp does not claim affinity, TTL, priority, or graph lifecycle support
   until their exact ownership semantics exist.
6. Static capability, per-request projection, backend acknowledgement, and
   outcome measurement are four distinguishable evidence layers.
7. Graph lifecycle follows exact admitted bindings and never broadcasts.
8. Hints cannot change model semantics, authority, retry identity, or typed
   outcome behavior.
9. Provider-specific Port Contracts and conformance joins remain independent.
10. Unsupported optimizations are visible, testable, and safe.

## 15. Recommended first cut

The smallest implementation that proves the architecture is:

1. land the fact/intent type and projection-result types;
2. implement a zero-capability projector for the mock/generic adapter;
3. migrate vLLM through a projector without changing its provider wire;
4. add llama.cpp through stock Chat Completions with only
   `PreferWhenBeneficial -> cache_prompt: true`;
5. normalize cached-token measurements; and
6. remove broadcast graph lifecycle only after the exact-bound lifecycle seam
   is exercised by vLLM.

This first cut proves that APXM owns the semantics, adapters own mechanisms,
and evidence connects the two without pretending all backends are identical.
