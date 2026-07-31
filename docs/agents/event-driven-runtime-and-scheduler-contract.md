# Event-driven runtime and scheduler contract

Status: accepted target design under workspace ADR-0027 and Agents ADR-0018;
owner schemas, vectors, implementation, and production activation remain
incomplete.

Date: 2026-07-30

## 1. Purpose

This document defines the target event, readiness, local scheduling, and
durable-activation seam for the APXM Agent Program runtime. It answers four
questions precisely:

1. Which things are Program Events, and which are only runtime or transport
   signals?
2. When is a dynamic node occurrence semantically runnable?
3. How is runnable local computation placed and stolen efficiently?
4. Where do durable queueing, leasing, retry, replay, and crash recovery live?

This is a replacement design, not a description of shipped behavior. At Agents
revision `3e5a78ef3ae8f645fd24390244b039f8bacaa64c`,
`crates/runtime/execution/src/structural.rs` builds a flat
`Vec<ScheduleStep>` and `crates/runtime/execution/src/driver.rs::drive_from`
awaits that sequence in one loop. `crates/runtime/execution/src/ports.rs`
represents `EventRef` as a string wrapper with transport-owned wording, and
`Cargo.toml` declares `crossbeam-deque = "0.8"` without a source consumer. The
pre-canonical engine scheduler remains Git history only and is not a
compatibility base.

## 2. Binding principles

The target obeys these laws:

1. **Meaning before placement.** AIR dependencies, control, typed operands,
   Hooks, context transitions, structured joins, cancellation, and fences
   decide readiness. A queue never does.
2. **One authority per fact.** Agents owns event/reference reducers,
   activation meaning, readiness, and local execution. Server owns accepted
   occurrences, durable delivery/activation/effect work, leases, retry/DLQ,
   schedules, recovery, and managed observability. Adapters and Host SDK own
   provider/device protocol and correlation meaning; Auth owns authority. The
   Composition Root owns concrete executor and resource construction.
3. **Durability is not work stealing.** Server leases an activation across a
   process boundary. Agents workers steal only already-runnable computation
   inside that activation.
4. **Publication before notification.** An operand, outcome, activation, or
   EventRef transition becomes authoritative before a wakeup hint is sent.
5. **No effect replay from queue mechanics.** An expired lease or stolen task
   never proves that an external effect did not happen.
6. **Accepted I/O has provenance.** Origin and transport facts specialize an
   accepted Program occurrence. Raw samples, occurrence candidates, local I/O
   completion, outbound effects, delivery acknowledgements, and telemetry stay
   separate closed types and do not become Program Events by being I/O.
7. **Performance hints are non-semantic.** NUMA, priority, batching, victim
   history, queue length estimates, and spatial hints cannot change results.
8. **One canonical product path.** The target has no compatibility reader,
   alias, translator, fallback, dual write, or mixed scheduler path. One
   product generation may still retain several exact immutable digests while
   live durable objects reference them.

## 3. The three schedulers that must not be conflated

| Mechanism | Owner | Unit | Persistent? | Decides |
| --- | --- | --- | --- | --- |
| Program readiness | Agents `ReadinessKernel` | dynamic node occurrence | checkpointed at durable boundaries | whether all semantic prerequisites are satisfied |
| Local placement | Agents `LocalExecutor` | immutable `RunnableNode` | no | which local compute worker runs ready work |
| Managed activation scheduling | Server | `RunnableActivation` | yes | eligibility, lease, retry time, ordering lane, tenant fairness, DLQ/redrive |

The local executor may disappear and be replaced by a deterministic
single-threaded test executor without changing Program results. The Server
activation scheduler may change its database or notification implementation
without changing graph readiness. Neither substitution may change the exact
contracts and fences at their seam.

## 4. Closed vocabulary

### 4.1 Program-facing and durable semantic values

| Type | Meaning | Semantic owner | Managed implementation authority |
| --- | --- | --- | --- |
| `Event<T>` | public friendly typed durable reference whose `.wait(...)` lowers to `await.event` | Agents | none; compile-time/frontend projection |
| `EventTypeRef<T>` | internal immutable event-type reference plus payload-schema digest | Agents | exact value embedded by managed Server or a conformant standalone durability adapter |
| `EventRef<T>` | unforgeable, generation-scoped, revisioned reservation with optional exact wait binding and fulfillment association | Agents | managed Server or a conformant standalone durability adapter persists transitions |
| `EventOccurrence<T>` | portable accepted typed occurrence value and provenance | Agents | embedded by exact digest in a managed Server or conformant standalone record |
| `EventProvenance` | closed portable occurrence-provenance variants and semantic fields | Agents | Server embeds the exact value; Contracts may provide only common identifier/digest/version envelope mechanics |
| `SourceContract` | immutable source classification, disposition, acknowledgement, frontier, loss, and admission policy | Server | Server registers and persists it from exact owner contributions |
| `SourceReducerDescriptor` | content-addressed pre-admission stream-reduction declaration; never a PXM transition reducer | Server | an admitted Adapter or Host implementation executes the protocol-specific reducer |
| `OccurrenceCandidate<T>` | generic admission command proposed for verification/admission; never Program truth | Server | Server ingress constructs it from exact Adapter/Host/API contributions |
| `CommittedSourceDisposition` | exact durable disposition/response-obligation result that alone authorizes protocol response | Server | exact Server route, ingress Adapter, or Host gateway emits the corresponding protocol response |
| `AcceptedOccurrenceRecord` | durable idempotency, source-frontier, response, retention, and target-delivery record embedding one exact occurrence | Server | Server |
| `EventDelivery` / `DeliveryAttempt` | durable lineage and fenced attempts applying one occurrence to one exact target | Server | Server |
| `TargetApplicationRef` | portable stable application identity preserved across delivery redrive | Agents | Server or a conformant standalone durability adapter persists it |
| `EventTarget` | root Program binding or one exact `EventRef` generation | Agents | managed Server or a conformant standalone durability adapter stores the exact binding |
| `ActivationId` | stable deterministic identity of one invocation/resumption segment across reclaim | Agents | managed Server or a conformant standalone durability adapter persists it |
| `ActivationClaim` | storage-neutral owner/epoch/fence/deadline authorizing one runner attempt | Agents | Server lease or standalone adapter supplies it |
| `RunnableActivation` | immutable admitted activation envelope eligible for a runner | Agents | managed Server or a conformant standalone durability adapter persists it |
| `ActivationLease` | Server claim/attempt identity implementing one `ActivationClaim` | Server | Server |
| `PreparedEffect` / `EffectState` | stable effect identity and closed prepare/dispatch/uncertainty/reconciliation meaning | Agents | Server or standalone adapter persists work |
| `NodeOccurrenceId` | one dynamic visit to a static AIR node | Agents | local/reconstructible inside an activation |
| `RunnableNode` | immutable, already-ready local execution value | Agents | local executor only |
| `ExecutionCommitFence` | expected Program revision, activation claim, Instance owner, and consumed EventRef revision where applicable | Agents | exact durability adapter enforces it |

### 4.2 Values that are deliberately not Program Events

| Value | Why it is separate |
| --- | --- |
| `DependencySignal<T>` | in-memory publication to an exact operand slot; it is not durable external truth |
| `CompletionSignal<T>` | local result publication from compute or an explicitly non-external local resource operation; never an external effect outcome |
| `WakeToken` | executor liveness hint; duplicate or lost hints cannot change correctness |
| `DeliveryAttempt` | transport activity; delivery success is not fulfillment or Program success |
| `TransportAck` | acknowledgement at an external protocol boundary |
| `RuntimeObservation` | non-authoritative metrics/trace/log projection |
| `LeaseNotification` | optimization telling a runner durable work may exist |
| `QuiescenceSnapshot` | local drain observation; never durable no-work truth |

An API that accepts one of these types cannot accept another through a string,
JSON discriminator, `dyn Any`, or catch-all variant.

### 4.3 Open event catalogue, closed runtime mechanics

APXM does not enumerate business event names in the runtime. `UserMessage`,
`WebhookOrderPaid`, `BrokerRecord`, `ApprovalGranted`, `FileArrived`,
`ObstacleDetected`, and future event definitions are immutable typed data
identified by `EventTypeRef<T>` plus an exact payload-schema digest. Adding one
does not add an AIR operation, runtime state, scheduler branch, or provider
enum.

The catalogue of event definitions and exact source descriptors is therefore
open through content-addressed typed refs. The mechanics applied to every event
remain closed: candidate disposition, occurrence acceptance, delivery, target
application, EventRef fulfillment/consumption, activation cause, failure,
retention, and evidence. Unknown lifecycle variants fail closed even when the
payload type itself is new.

Open globally does not mean open at execution time. Each Program artifact,
source binding, target binding, `await.event` site, and Invocation Admission
pins a closed exact set of event-type/schema digests. An
`ExternalEventBinding` pins source/generation, authenticated Adapter/source
descriptor, external envelope/profile and schema constraints, normalizer
digest, internal `EventTypeRef<T>`, and one exact target binding. Ambiguous
schema matches, catch-all handlers, dynamic fallback, and “send or start”
selection fail admission.

A webhook, WebSocket frame, broker record, Widget message, Host frame, file
notification, device observation, or future transport is a source mechanism,
not a new Program-event lifecycle. Its exact Adapter/Host/source descriptor
normalizes it into a typed candidate. Server alone accepts that candidate as an
occurrence. Transport headers, credentials, topics, and callback URLs never
become runtime dispatch strings.

Ownership is explicit even though these values compose. Agents owns portable
`EventOccurrence<T>`/`EventProvenance` meaning and PXM transition reducers.
Server owns `SourceContract`/`SourceReducerDescriptor` admission semantics,
registration, and durable records. The Adapter or Host SDK owner publishes the
protocol-specific source descriptor and deterministic classifier, normalizer,
or reducer implementation. Contracts indexes and generates those publications
without becoming their semantic owner. The term **transition reducer** refers
to an Agents PXM state transition; **source reducer** refers only to declared
pre-admission stream processing.

### 4.4 The word event does not collapse distinct facts

For Agent Program semantics, an event is external to the activation segment it
enters. It need not originate outside APXM: a schedule fire or an explicitly
exported Program-lifecycle fact can become a later occurrence. It must still
cross an exact source binding and durable admission boundary before it may
start a root Invocation or fulfill an EventRef. An internal transition never
becomes a Program Event merely because an observer can report it.

APXM surfaces several things that external systems may call an event, but their
types and consequences remain separate:

- a Program `EventOccurrence<T>` is an admitted coordination input;
- a runtime evidence/lifecycle fact may report that an Invocation or
  NodeExecution started, returned, failed, or was cancelled;
- a lifecycle/authority command requests cancel, terminate, close, revoke, or
  deadline action;
- an effect outcome reports external work; and
- a dependency/completion/wake signal coordinates local execution.

An execution-finished evidence fact can be exported as CloudEvents/AsyncAPI or
explicitly admitted through a new source binding as another Program occurrence,
but it never automatically triggers a Program or fulfills an EventRef. Child
Program completion normally returns through `program.invoke`; an explicit
cross-Program event bridge creates a new occurrence identity and provenance.
Likewise, an execution-started projection is observation, not proof that a
worker may run.

Idempotency is never one cross-plane key. Auth owns verification/replay
coordinates. Server owns source-attempt/occurrence, delivery,
target-application, schedule-fire, activation-claim, and managed-work
deduplication. Host SDK owns frame/result identity. Adapters own external
provider/effect idempotency behavior. Agents owns stable EventRef, activation,
effect, and Execution Commit identities plus their transition consequences.
Every cross-plane record carries the exact adjacent identities needed for
correlation without allowing one scope to substitute for another.

## 5. Event model

### 5.1 Type, occurrence, delivery, application, and activation are separate

An internal event type says what a value means. An occurrence says that a value
was accepted as Program input. A delivery is the durable lineage for applying
it to an exact target. A delivery attempt is one fenced claim. Target
application makes root admission or EventRef fulfillment true. An activation
authorizes one Program segment. These records and identities never collapse.

The external ingress ladder is explicit. Classification uses only the exact
immutable source-contract revision; current rate, queue depth, credits, worker
saturation, and deployment topology are forbidden classifier inputs.

```text
External SourceItem
  -- classify(SourceContractRef) --> OccurrenceCandidate<T>
                                  | StreamElement<T>
                                  | StreamGap | StreamWatermark
                                  | ProtocolControl
                                  | InvalidItem

StreamElement<T> + declared stream control
  -- exact SourceReducerDescriptor --> OccurrenceCandidate<U> | control/gap record

OccurrenceCandidate<T>
                              |
                  reserve complete acceptance capacity
                              |
             +----------------+----------------+
             v                v                v
   EventOccurrence<T>  response/source   InitialDelivery
             |         disposition/frontier    |
             |                                  v
             |                         DeliveryAttempt
             |                                  |
             +--------------------------> TargetApplication
                                                |
                           +--------------------+------------------+
                           v                                       v
                 RootAdmission/Invocation                EventRef fulfillment
                           |                              (+ activation iff bound)
                           v                                       |
                  RunnableActivation <-----------------------------+
```

Only Server's acceptance transaction creates the managed occurrence and
exactly one initial delivery, together with the response obligation and the
source-position disposition/frontier consequence allowed by the source
contract. Candidate verification, staging, notification, or raw evidence is
not enough. A standalone composition uses an explicit conformant occurrence-
admission adapter and cannot bypass the same Agents occurrence/EventRef
semantics by constructing a ref or occurrence string. Delivery settlement
remains separate from target application, Program start, effect completion,
and Program completion.

Two boundaries prevent overload from creating a durability feedback loop.
`AttemptPresented` means bytes or a frame reached ingress; Server may refuse it
without a durable attempt record. `AttemptAdmittedToDispositionJournal` means
Server atomically reserved capacity for the disposition record, payload and
provenance bytes, referenced evidence retention, idempotency/frontier state,
response/outbox obligation, and exactly one initial delivery if acceptance
wins. If any reservation fails, no occurrence or delivery exists.

Presentation has a closed protocol result:

```text
AdmittedToDispositionJournal(reservation)
Backpressured(scope, source_position, credit_epoch, optional_retry_after)
PayloadTooLarge(admitted_byte_ceiling)
ProtocolViolation(reason)
InfrastructureUnavailable(reason)
```

`Backpressured` creates no occurrence, delivery, acknowledgement, frontier
advance, or DLQ item. Retry retains the exact source position/idempotency
identity. Infrastructure failure remains distinct from deliberate capacity
backpressure.

Every attempt admitted to the journal commits one closed Server-owned
disposition:

```text
AttemptAdmittedToDispositionJournal
  -> Accepted(occurrence_ref, initial_delivery_ref)
   | Duplicate(original_occurrence_ref)
   | Handshake
   | NoEvent
   | RejectedPermanent(reason, candidate_digest)
   | AuthorizedSkip(policy_revision, source_interval, reason)
   | GapCommitted(gap_ref)
   | ResetCommitted(new_source_generation, prior_frontier_disposition)
   | TransientFailure(reason)
```

Only a disposition that the exact source contract declares terminal may
acknowledge the attempt or advance/replace its frontier. `TransientFailure`
leaves the source position unresolved. Gap, reset, epoch, and checkpoint
behavior is typed per source family; neither silence nor a transport timeout
means `NoEvent`. Provider acknowledgement follows a durable terminal
disposition, not occurrence acceptance alone.

Server alone commits that disposition and its response obligation. The exact
`CommittedSourceDisposition` authorizes the exact ingress protocol handler—a
Server route, admitted Adapter, or Host gateway—to physically emit the protocol
acknowledgement, refusal, or retry response. A response timeout resolves the
same source-attempt/disposition identity; it never permits the transport
implementation to infer or choose a disposition.

A committed gap records source ref and generation/boot epoch, exact half-open
missing-position interval when known, source-time interval when known,
expected/observed/lost counts, cause, detector, source-contract/policy revision,
evidence refs, and detection time. Unknown bounds use a distinct unknown-gap/
reset-required result; they never fabricate an interval. A small isolated
control/gap journal is reserved so saturated data queues cannot suppress loss
evidence. The gap becomes a Program occurrence only when the exact source
contract explicitly maps it to an event type.

Protocol adapters map refusal without changing these meanings:

| Source protocol | Bounded refusal behavior |
| --- | --- |
| Host/stream credits | advertise zero new item/byte credits; exceeding granted credit is a protocol violation |
| broker or pull source | stop pulling or withhold acknowledgement; position remains unresolved |
| HTTP/API per-client quota | `429 Too Many Requests`, normally with `Retry-After` |
| HTTP shared storage/service saturation | `503 Service Unavailable`, normally with `Retry-After` |
| HTTP payload above admitted byte ceiling | `413 Content Too Large` without first buffering the complete body |
| non-pausable declared-lossy source | commit an exact gap through isolated reserved capacity before frontier advance |
| non-pausable source without gap semantics | disconnect/fail closed; never drop silently or synthesize a gap |

Durable acceptance is independent of local execution. It never waits for an
Agents worker, local runnable/dispatch permit, activation claim, or available
compute core. The source acknowledgement may complete once the occurrence,
response/frontier consequence, and initial delivery are durable. Downstream
saturation may reduce future ingress credits, but cannot revoke, evict,
coalesce, or reclassify accepted work. Delivery/target application remains
durable until a terminal result or managed DLQ state. External occurrences and
delivery records never enter `RunnableNode` or a local work-stealing queue.

Target application also has closed outcomes:

```text
Applied(original_commit_ref)
AlreadyApplied(original_commit_ref)
TargetClosed
ProgramInstanceBusy
IdentityConflict
AuthorityUnavailable
AuthorityDenied
StaleBindingGeneration
```

`ProgramInstanceBusy` is the required fail-busy result for an existing
single-flight Instance. It is terminal for that application attempt: delivery
does not invent a mailbox or retry it without bound. A later authorized
business attempt requires the source/target contract to create a distinct
application identity.

### 5.2 Occurrence provenance

`EventOccurrence<T>` has a closed provenance envelope with two structural
variants: `ExternalSource`, which carries exact content-addressed source and
source-observation descriptors, and `InternalSchedule`, which carries one
exact schedule/fire generation. API, webhook, broker, Widget/user message,
Host, file, physical edge, and future transports specialize the external
descriptor rather than adding a runtime enum branch. Common fields are:

- occurrence id, event-type ref, and payload-schema digest;
- exact Company/identity scope and admitted source reference;
- canonical payload digest and, when allowed, an opaque content reference;
- APXM observed time and monotonic receive sequence;
- source time, source sequence, and clock-quality classification when the
  origin contract supplies them;
- normalization/admission revision and accepted idempotency identity; and
- integrity, confidentiality, retention, and redaction classification.

Duplicate source attempts resolve the original accepted occurrence; they do not
mint an occurrence with `duplicate_of`. Duplicate lineage belongs to the Server
attempt/admission resolution. The origin kind does not select a handler, and a
new provider does not add a new core event lifecycle.

### 5.3 Root versus awaited targets

`RootProgramTarget` binds an admitted occurrence to one exact Program revision,
root input schema, identity binding, invocation admission, and deterministic
activation id. Target application proves occurrence payload `T` exactly
satisfies Program input `I`; any semantic conversion is an explicit typed
Adapter or authored Program, never a hidden deployment mapping or implicit
fan-out.

`AwaitedEventRef` binds one `EventRef<T>` to one Program Instance, Program
Invocation, continuation, static callsite, and dynamic wait generation.
Fulfillment records the exact occurrence and target-application refs, retaining
the accepted payload and provenance rather than copying a bare value.

Pre-fulfillment and a racing first await converge on the same committed binding
and activation identity. A wake is a repairable notification derived from that
truth, never the truth itself.

Fulfillment, expiry, and cancellation compete by authoritative durability-
adapter commit order under the Agents-owned state machine; Server supplies that
order in managed APXM. Source timestamps never decide the winner. A late
physical or provider observation may remain admissible evidence under its
retention contract while failing to fulfill a ref that already expired or was
cancelled.

Every EventRef carries at least:

```text
Company
event_ref + event_ref_generation
event_type_ref + payload_schema_digest
reservation_owner
correlation_binding_digest where applicable
state_revision
optional WaitBinding(
  ProgramInstanceRef,
  ProgramInvocationRef,
  expected_program_revision,
  continuation_ref,
  wait_site_ref,
  wait_generation,
  optional deadline
)
optional Fulfillment(occurrence_ref, target_application_ref)
```

The closed lifecycle is:

```text
ReservedUnboundEmpty
  -- Bind(expected ref + Program revisions) --> ReservedBoundEmpty
  -- Fulfill(expected ref revision) ----------> FulfilledUnbound
  -- Expire | Cancel --------------------------> Expired | Cancelled

ReservedBoundEmpty
  -- Fulfill --> FulfilledBoundReady + EnsureActivation
  -- Expire | Cancel --> Expired | Cancelled

FulfilledUnbound
  -- Bind --> FulfilledBoundReady + EnsureActivation
  -- Abandon(exact owner terminal) --> Abandoned

FulfilledBoundReady
  -- Consume in winning Execution Commit --> Consumed
  -- Abandon(exact owner terminal before consume) --> Abandoned

Consumed | Abandoned | Expired | Cancelled
  -- any non-identical transition --> Conflict
```

Fulfillment is an immutable association, not the final EventRef lifecycle
state. Identical bind/fulfill retries resolve the original result; different
bindings or occurrences conflict. `EnsureActivation` is unique over Company,
EventRef generation, wait generation, occurrence, Program Instance,
Invocation, and expected Program revision. Only the winning Execution Commit
consumes fulfillment. Temporary unavailability cannot abandon it.

`ReserveEventRef` is available during effect preparation. An asynchronous
Capability that arranges external completion co-commits the stable effect
identity, typed reserved ref, and provider correlation binding before dispatch.
The Capability returns that exact `Event<T>`/`EventRef<T>` value; authors never
construct it from a string.

### 5.4 I/O specialization

HTTP, message broker, filesystem, Host, and sensor inputs are I/O mechanisms.
Origin/transport facts become provenance only on an accepted
`EventOccurrence<T>`. A schedule fire is an internally materialized occurrence
with schedule provenance. A Host result or model/Capability completion is an
effect outcome unless an explicit source contract independently admits a later
occurrence. Raw stream samples, candidate inputs, local readiness/completion,
outbound effect dispatch/outcomes, delivery/acks, and telemetry keep their own
closed types. They do not introduce `IoEvent<T>` or a second `EventRef` state
machine.

Source items are classified only by the immutable source contract, never by
frequency. A high-rate discrete alarm may be an occurrence candidate; a
low-rate raw sample may remain a stream element. Only a declared stream element
follows a reducer, whose exact filter, threshold, window, inference, or safety-
state transition may produce a typed candidate.

Frequency is not an event classifier. If an exact source contract declares
every sample or record to be a Program occurrence, APXM applies the same
acceptance and delivery semantics at any rate it can admit. When capacity is
exhausted it returns or records the source-specific typed backpressure,
unavailable, gap, or rejection disposition; it never silently changes event
meaning, drops an accepted occurrence, or reclassifies the source as a stream.
Reduction, sampling, conflation, and windowing are allowed only before
occurrence acceptance under an explicit source contract that declares their
loss/ordering semantics.

### 5.5 Stream reduction before occurrence acceptance

A `SourceReducerDescriptor` is content-addressed before traffic and declares
input identity/sequence domain, window type and boundaries, watermark source,
late/duplicate/reorder policy, sampling/conflation/hysteresis/debounce,
stable output occurrence-key derivation, input range/count/digest retained in
each output, and exact loss/gap behavior. Overload never switches a per-item
source into sample, latest-only, drop-oldest, drop-newest, or coalescing mode.
Transport batching retains each candidate identity; it does not merge events.
Gaps carry exact sequence/time intervals when known; overflow uses the declared
gap/reset result rather than blocking device-local safety work or fabricating a
precise interval.

Raw evidence is staged under an exact retention/integrity contract. Server
accepts a derived occurrence only after every referenced evidence object is
durably addressable. Backpressure propagates as bounded Server credits to Host
and reducer. Live commands, occurrence metadata, and bulk evidence/backfill use
separate credit/queue classes so reconnect catch-up cannot starve live work or
execute stale commands.

### 5.6 Start, resume, steer, and finish

One generic occurrence can have only one exact target application:

- a root target applies payload `T` as the exact Program entry input `I` and
  creates one Invocation plus initial activation;
- an await target fulfills one exact `EventRef<T>` generation and creates a
  resume activation only when its exact wait binding exists; or
- an explicitly authorized new-business reprocessing operation creates a new
  occurrence and target identity.

A user message is ordinary typed data under this model. It may start a new
Invocation, resume a conversation Program that explicitly awaits its
`EventRef<UserMessage>`, or target a ready stateful Instance through the root
lifecycle. It cannot interrupt arbitrary running code or mutate Context through
an implicit mailbox. If the single-flight Instance is executing or parked,
generic root application returns `ProgramInstanceBusy`; a parked wait resumes
only through its exact EventRef fulfillment. Programs that need continuous
steering explicitly author a loop that obtains/reserves the next typed EventRef
and waits for it.

A message carrying an exact previously published `EventRef<UserMessage>` may
pre-fulfill that ref while the Instance is still executing. It records the
immutable fulfillment but creates no concurrent activation and interrupts
nothing. When execution later binds/reaches that wait, the fulfilled ref creates
or supplies the unique continuation activation. A generic message without that
exact ref cannot use pre-fulfillment to bypass fail-busy.

An occurrence never marks a Program completed by itself. It may cause a resume
activation whose authored control flow commits `Program Return`, or it may lead
to another wait/yield. External cancellation, termination, revocation, and
deadline expiry are closed lifecycle/authority commands, not generic business
events and not payload conventions such as `{ "type": "cancel" }`.

### 5.7 Authoring, compiler, artifact, and runtime contract

The end-to-end representation is generic over `T` and contains no source-kind
branch:

```text
source descriptor + typed payload T
  -> Server OccurrenceCandidate<T> and durable EventOccurrence<T>
  -> exact TargetApplicationRef
     -> root: artifact entry input I with proof schema(T) == schema(I)
     -> await: EventRef<T> generation and exact WaitBinding
  -> RunnableActivation(cause = RootOccurrence | AwaitFulfillment)
  -> Agents ExecutionPlan / ReadinessKernel / LocalExecutor
  -> Program Return | Program Yield | next committed wait/effect/checkpoint
```

Authoring frontends expose `Event<T>` only as an unforgeable typed value and
`.wait(...)`; authors never spell source transports or construct refs from
strings. For root input, ordinary `Agent<I,O,C>` input typing is sufficient:
an artifact entrypoint embeds the exact `I` schema/digest. Multiple business
event types use an authored tagged union or distinct exact Program bindings,
not a runtime catch-all object.

`BoundAgentTree` and `FrontendGraph` retain the `Event<T>` value type and exact
schema/ref flow. Rust lowering emits only the existing `await.event` operation
for an authored wait. A root occurrence starts at the artifact entrypoint and
adds no sixth AIR operation. The artifact binds entry input/output/context
schemas, declared event-type/ref requirements, wait sites, source maps, and the
ExecutionPlan digest. Runtime admission rejects schema/ref/generation mismatch
before creating local work.

Runtime evidence preserves occurrence, target application, activation cause,
EventRef/wait generation when applicable, Invocation, NodeExecution, Program
Return/Yield/wait/effect outcome, and commit identities. A webhook receipt or
user-message acknowledgement remains distinct from all of those facts.

## 6. Immutable execution plan

### 6.1 Why the flat schedule is replaced

The current `ScheduleStep` vector records a deterministic traversal but erases
which independent operations could run concurrently. Parallel joins, typed
data dependencies, Hook ordering, context transitions, and effect fences need
an executable dependency plan, not a promise that a worker may inspect inputs
and requeue a node when they are missing.

### 6.2 Plan contents

Admission derives one immutable `ExecutionPlan` from validated AIR. It contains:

- static step ids for semantic operations, Hook calls, context transitions,
  control gates, joins, loop boundaries, yield, return, and throw;
- typed predecessor edges, operand slots, and result slots;
- static structured-scope ownership and cancellation propagation;
- exact effect class and whether dispatch may cross an external boundary;
- non-stealable/pinned classification where correctness requires it;
- admitted priority class and resource-cost ceiling;
- source/AIR evidence mappings; and
- a digest bound into the executable artifact/admission proof.

The plan contains no worker id, CPU id, URL, broker subject, database key,
credential, lease, or mutable queue state.

### 6.3 Dynamic occurrence frames

Loops, branches, calls, and structured tasks instantiate dynamic occurrence
frames. A frame binds:

- Program revision, activation id, invocation id, and commit fence;
- static step id and dynamic `NodeOccurrenceId`;
- structured parent and loop-iteration occurrence ids;
- typed operand/result cells;
- remaining predecessor count;
- cancellation epoch and deadline view; and
- closed node lifecycle state.

Dynamic ids are deterministic from admitted parent identity, static structured
position, branch path, loop-iteration index, and authored child ordinal.
Atomic spawn/completion race order, worker order, randomness, and wall-clock
time are not identity sources.

## 7. Readiness kernel

### 7.1 Required transition

For a node with `N` predecessors, admission initializes `remaining = N` and
every typed slot as `Empty`. Each exact predecessor may publish its exact edge
once:

1. validate activation, occurrence, edge, type, and claim;
2. atomically claim the exact slot `Empty -> Writing`; only the winner may
   initialize its storage;
3. write the typed operand/outcome, then publish `Writing -> Published` with
   release ordering;
4. execute exactly one `remaining.fetch_sub(1, AcqRel)`;
5. if and only if the observed old value is `1`, acquire the predecessor
   release sequence, reserve the successor `ActivityCredit`, and only then
   compare-and-transition `blocked -> runnable`; the winner publishes one
   `RunnableNode` through a queue operation whose synchronization completes the
   happens-before chain, while a cancellation/failure loser releases the
   reserved credit exactly once.

Zero-predecessor nodes use an explicit admission-time `blocked -> runnable`
transition because no decrement observes old value one. Activation admission
acquires a bootstrap credit before it publishes any zero-predecessor node, then
transfers that credit through the same reserve-before-CAS protocol.
Cancellation racing the last predecessor is legal: exactly one lifecycle
transition wins. A cancelled queued item is discarded lazily before dispatch;
running-node and effect-safe cancellation follow their own closed transitions.

An underflow, duplicate slot claim, missing slot, type mismatch, stale
activation, or duplicate successful runnable transition is a closed invariant
failure. Saturating decrement and “requeue if inputs are missing” are forbidden
because they hide readiness bugs.

### 7.2 Happens-before requirement

The implementation may use a different safe representation, but it must prove:

```text
producer claims exact slot
  happens-before
producer writes and release-publishes typed operand
  happens-before
producer performs the last prerequisite RMW
  happens-before
winner publishes runnable through the queue
  happens-before
consumer reads every operand
```

Every predecessor sequences its slot publication before one `AcqRel` RMW on
the same remaining counter. That counter is never reset or modified by a plain
store while the frame is live. Each RMW reads the preceding counter value, so
the old-value-one RMW transitively acquires the complete producer RMW chain.
The consumer either relies on that counter-to-queue synchronization chain or
acquire-checks every slot as `Published` before reading it; mixing weaker
assumptions is forbidden.

The proof belongs in module documentation and a model test. Relaxed ordering is
allowed only for diagnostic counters that cannot affect the transition.
Model cases and injected failpoints cover after slot claim, after operand
publication, after decrement, and before queue publication for every producer.

### 7.3 Completion publication

A node result is published through exact outgoing edges. A successful result,
typed failure, cancellation, external outcome-unknown, and structural control
outcome are distinct values. Failure propagation follows the compiled
structured scope; it does not manufacture default operands to unblock work.

### 7.4 Effect nodes end the local activation before external dispatch

The local node prepares an exact effect but never sends external bytes. Its
winning Execution Commit atomically persists the continuation/checkpoint,
stable effect identity, request digest, exact adapter/binding, authority
decision, and provider idempotency/reconciliation contract; that commit ends or
parks the activation and makes durable effect work `Dispatchable`.

```text
Prepared
  -- winning Execution Commit --> Dispatchable

Dispatchable
  -- claim while authority remains valid --> ClaimedBeforeSend(epoch, attempt)
  -- authority revoked/cancelled ----------> RevokedBeforeSend | CancelledBeforeSend

ClaimedBeforeSend
  -- durable pre-send transition --> DispatchStarted(attempt)
  -- proven no bytes could leave --> Dispatchable | TerminalNotSent

DispatchStarted
  --> Committed
  --> ProvenNotApplied
  --> OutcomeUnknown -> Reconciling

Reconciling
  --> Committed
  --> ProvenNotApplied -- explicit new attempt, same effect id --> Dispatchable
  --> OutcomeUnknown
  --> TerminalManualDecision
```

Server owns managed durable effect work, claims, attempts, `DispatchStarted`,
outcome storage, reconciliation scheduling, and publication of the exact next
activation. A standalone durability adapter must implement the same
Agents-owned transition contract and histories. Adapters own the exact provider
send/status/idempotency/cancellation protocol.

This is a three-layer reconciliation boundary, not shared ownership. Agents
owns `PreparedEffect`/`EffectState` transitions, `OutcomeUnknown` meaning, and
the condition under which a result may authorize a later activation. Server
owns durable managed work, attempt/result records, reconciliation scheduling,
and exact consequence publication. The Adapter owns provider/device send,
status, idempotency, cancellation, and reconciliation protocol execution and
interpretation.

The dispatch boundary enforces the current claim/fence where the provider or
device supports it. Where it cannot enforce fencing or idempotency, loss after
preparation/dispatch becomes `OutcomeUnknown`; replacement work reconciles
rather than resends. Reclaim observes prepared/dispatched state before allowing
any new attempt. Commit uncertainty and effect uncertainty remain distinct.

## 8. Local executor

### 8.1 Queue topology

V1 selects an owner/thief queue only after pinning its exact source/checksum and
reviewing the Rust memory model, weak-memory behavior, reclamation, cache-line
layout, capacity/overflow, panic behavior, and model-test evidence. The current
`crossbeam-deque` 0.8.6 dependency is only a candidate: its own source describes
volatile concurrent slot access as technically a data race, and its owner/thief
indices are adjacent. It is not accepted by presence or popularity.

The chosen topology has one stealable owner-local deque per worker per service
class, per-class/domain injectors, and a separate owner-only queue for pinned
and runtime-integrity control work. Each compute worker is the sole owner of its
local ends. Cross-thread cancellation, fence, shutdown, and newly-ready pinned
work enters through a bounded per-worker MPSC control inbox; the owner drains
that inbox into its private queue. Inbox publication precedes wakeup under the
same no-lost-wakeup protocol, overload fences admission rather than dropping
control work, and a bounded control-service budget prevents repeated control
traffic from starving ordinary drain/cleanup. When the selected sound
implementation is Chase-Lev-style, owner push/pop and opposite-end stealing
are used.

Each NUMA domain has an external-injection path for newly-started activations
and local completions. Global cross-domain injection is reserved for work
without a domain affinity or for explicit imbalance repair. Workers are pinned
inside the effective cpuset; topology comes from cpuset/cgroup plus NUMA
discovery, with explicit hotplug/change handling and first-touch/allocation
policy. A closed uniform-topology deployment profile is used when NUMA facts
are unavailable; it is ordinary supported composition, not a compatibility
fallback.

Only `RunnableNode` enters these queues. Parked waits, durable activations,
leases, secrets, and raw futures do not.

An arbitrary worker panic/loss may strand nodes from several activations in its
private queues or leave a `Writing` slot. V1 therefore fences the whole runtime
instance and every active activation claim unless a later accepted design
proves complete panic-surviving queue membership. All workers reject further
slot publication, starts, commits, and effect preparation under those claims;
frame memory and permits stay live until stale items drain and reclamation is
safe. Only the durability owner may revoke/reclaim the old claims and issue new
epochs from the last committed checkpoints. A local epoch increment is never
sufficient, and execution never guesses which private items were lost.

### 8.2 Fast path and steal policy

The conceptual worker loop is:

```text
while not shutting_down:
    if bounded_owner_control_service() yields work:
        run(work)
        continue

    eligible_classes = nonempty_class_mask & admitted_class_mask
    for service_class in weighted_classes_with_age_promotion(eligible_classes):

        if local[service_class].pop_lifo() yields work:
            run(work)
            continue outer

        if domain[service_class].injector.steal_batch_and_pop(local[service_class]) yields work:
            run(work)
            continue outer

        for victim in bounded_local_victims(last_success, persistent_permutation_cursor):
            if victim[service_class].steal_capped_batch(local[service_class]) yields work:
                remember(victim)
                run(work)
                continue outer

        for domain in bounded_remote_domains(rotated_random_order):
            if domain[service_class].steal_eligible_capped_batch(local[service_class]) yields work:
                run(work)
                continue outer

    idle_with_spin_yield_then_park()
```

The implementation never scans every worker on every miss. It first retries a
recently successful local victim, then advances a persistent randomized victim
permutation/cursor through a bounded subset. The cursor supplies bounded
coverage across repeated misses; independent repeated samples are
insufficient for a starvation bound. A worker probes every currently eligible
service class before parking.

Remote-NUMA steals begin only after bounded local misses or an explicit domain
imbalance signal. Batch size is capped so a thief cannot drain a victim or
violate priority service.

### 8.3 Locality and hints

Local LIFO execution tends to reuse hot activation and graph state. A stolen
node carries an optional preferred NUMA domain derived from already-resident
immutable data, but the hint may be ignored. Pinned/non-stealable work never
enters a public stealer.

The scheduler records steal distance (same worker/domain/socket/remote domain)
so policy changes are evidence-based. It does not promise cache locality from a
hint alone.

### 8.4 Priority and starvation

Priority is a closed admitted class, not an arbitrary integer supplied by a
caller. Locality is preferred within a class. Per-class deques and injectors
use deficit/weighted dispatch budgets plus bounded age promotion that does not
scan whole deques. Same-class batch steals have a fixed cap. An explicit
per-activation dispatch-token cap prevents one wide activation from occupying
every worker. Only nodes holding one of those tokens may enter worker deques;
additional semantically ready nodes remain in a bounded activation-local ready
backlog while retaining activity credit. Completion or lazy discard returns a
token and promotes the next eligible ready node, so an ineligible activation
can never block eligible work behind it in a mixed LIFO deque. Cancellation,
fencing, and shutdown control work uses the owner-private control path rather
than an infinitely high user priority.

Priority cannot reorder dependency edges, bypass a structured join, or run a
node before its operands are published. The separate runtime-integrity control
path covers cancellation, fencing, and shutdown only; it is never a physical
safety-control lane.

### 8.5 Granularity

The compiler/runtime may inline or fuse scheduler-internal pure steps when the
transformation is deterministic and preserves evidence boundaries. A task is
not enqueued merely to copy a scalar or decrement one local counter when the
producer can complete the transition inline. Effect, Hook, cancellation,
source-lineage, and NodeExecution evidence boundaries cannot be fused away.

Benchmarks establish the minimum profitable task granularity; no hard-coded
duration guess becomes semantics.

## 9. Idle and wakeup protocol

### 9.1 Required phases

After bounded failed steals, a worker:

1. spins for a small bounded iteration count while observing the work epoch;
2. yields a bounded number of times;
3. registers itself as a sleeper;
4. rechecks local/domain/global work and the epoch with acquire ordering;
5. parks only if both remain unchanged; and
6. clears sleeper registration after wake or timeout.

A producer publishes work first, increments the work epoch, and unparks one
eligible sleeper. The primitive must retain a wake token when unpark races
ahead of park. A burst may wake more workers proportional to published
parallelism, but never all workers for one node.

### 9.2 No-lost-wakeup proof obligation

The design must cover these races with model tests:

- enqueue before sleeper registration;
- enqueue between registration and recheck;
- enqueue after recheck but before park;
- unpark before the thread actually parks;
- multiple producers and one sleeper;
- one producer and multiple sleepers;
- shutdown/cancellation racing with park; and
- a spurious wake with no work.

Periodic polling is a recovery aid for broken external notifications, not the
normal local scheduling loop.

## 10. Boundedness and backpressure

Local deques are not the admission boundary. Memory is bounded by admitted
resource ceilings:

- maximum active leased activations per runtime instance;
- maximum static and dynamic node occurrences per activation;
- maximum structured-task fan-out and loop occurrences between checkpoints;
- maximum in-flight effect, async-port, and blocking-port operations;
- maximum activation/frame-state permits and runnable queue-slot permits per
  NUMA domain; and
- maximum result/output bytes retained before commit.

Managed ingress additionally enforces count-and-byte ceilings at global,
Company, source, and pipeline-stage scopes. Separate service pools and capacity
reservations cover protocol control (handshake/credit/gap/reset/rejection), live
candidate validation/admission, bulk evidence/reconnect backfill, durable
delivery/application, activation eligibility/claims, and effect dispatch/
reconciliation. Control/gap reserve cannot be consumed by data, bulk/backfill
cannot starve live admission, and one Company/source cannot exhaust the global
budget.

Admission atomically reserves an activation-width/state permit bundle; a
check-then-acquire sequence is forbidden. The reservation guarantees that a
last predecessor can always publish its winning runnable node without blocking
or dropping it. Activation/frame-state permits stay charged to the frame's
home domain until every stale queue entry is discarded and the frame and
reclamation buffers are safe to free; cancellation, claim loss, or panic does
not return them early.

Each queued node also holds a runnable queue-slot permit for its current queue
domain. A remote steal atomically transfers that permit before publishing into
the destination deque; when the destination has no permit, the steal is not
eligible. The activation-local ready backlog and per-activation dispatch-token
cap ensure that excess ready nodes do not enter a worker deque and cannot hide
eligible work behind a capped activation.

If dynamic fan-out reaches its admitted ceiling, the parent releases/transfers
the permits required by children before suspending on an internal capacity
edge, preventing fork/join deadlock. If a tighter dynamic scheme is selected,
it must use a bounded lossless ready-pending-capacity structure with the same
guarantee. Server sees explicit runner saturation and applies durable admission
backpressure rather than flooding local queues.

Memory ceilings include queue buffers, any retired/reclamation buffers,
injector allocations, result retention, and repeated burst/recovery cycles—not
only live item counts.

Blocking calls run on a separately bounded executor. A blocking adapter cannot
consume compute-worker or async-I/O capacity indefinitely.

Agents compute/work-stealing, blocking-I/O, accelerator, Host streaming, and
device-controller real-time lanes use distinct bounded pools. Effective
topology is derived from the process cpuset/cgroup allocation, not the host's
total CPUs. Agents never borrows controller/safety cores, real-time priorities,
callback threads, locked memory, or reserved memory-bandwidth budgets. A
firm-deadline APXM activation/node has closed `ExpiredBeforeStart` and
`DeadlineMissed` outcomes; it does not borrow the hard-real-time controller to
catch up.

## 11. Activation runner and durability seam

### 11.1 Input envelope

The Agents `ActivationRunner` accepts one immutable envelope containing:

- exact Program artifact and execution-plan digest;
- Program Instance and Invocation identities and expected state revision;
- activation id, cause, and parent occurrence identities;
- continuation/checkpoint for a resume, or typed root input for a new invoke;
- exact Invocation Admission, model bindings, Capability authority references,
  limits, cancellation epoch, and deadlines;
- storage-neutral `ActivationClaim` id, attempt epoch, fence, owner, and expiry
  view; and
- exact runtime-profile and port-binding proof.

Missing, mismatched, expired, unsupported, or stale values fail before local
work publication. There is no resolution, discovery, or fallback inside the
runner.

### 11.2 Output envelope

The runner returns exactly one of:

- `Committed` with commit id, committed Program revision, evidence position,
  and terminal/yield result;
- `Parked` with committed continuation, wait binding, and Program revision;
- `Cancelled` with committed cancellation proof;
- `InfrastructureRetryable` only when no ambiguous external effect or commit
  exists and the same activation identity may be retried;
- `FailedCommitted` for a semantic/runtime failure already made authoritative;
- `CommitOutcomeUnknown` requiring idempotent Execution Commit resolution; or
- `ClaimLost` when the local runner cannot lawfully commit.

`EffectOutcomeUnknown` is not an `ActivationRunner` result. The activation has
already ended at the effect-preparation commit; the durable effect worker
persists uncertainty and reconciliation creates a later activation only after
it reaches a committed, proven-not-applied, or authorized manual-terminal
outcome.

The durability adapter changes activation state only when stable activation id,
claim attempt/epoch, expected Program revision, current Instance execution
owner, consumed EventRef revision where applicable, and commit proof match. A
response timeout is not proof that the runner did not commit and is not
automatically an external-effect unknown.

### 11.3 Lease discipline

The durability implementation chooses eligible activations. In managed APXM,
Server atomically transitions `Activation Ready + Program Instance Ready` to
`Activation Leased(epoch) + Program Instance Executing(owner)`, then adapts
that lease to the Agents `ActivationClaim`. A standalone implementation must
perform the equivalent atomic transition. Heartbeat may extend the same epoch;
reclaim increments it so an old runner cannot commit. `ActivationId` is stable
across reclaim while claim/lease attempt identity changes.

A long-running checkpoint ends the current activation and atomically creates
the next exact activation under a new Program revision. The old runner cannot
continue in place. External effects leave only through the durable effect
state machine in section 7.4; a local claim check never authorizes direct
external dispatch.

### 11.4 Durable notification

A broker, database notification, or in-process signal may tell a runner to poll
for activations. The managed Server record or standalone durable adapter remains
authoritative. Missing a notification delays work but does not lose it;
duplicate notifications do not create duplicate activation identities.

## 12. Failure, retry, DLQ, and replay

### 12.1 Retry categories

- Local contention (`Steal::Retry`, CAS collision) retries immediately within
  a bounded loop and is not a Program attempt.
- A durable effect worker's declared pre-send or proven-not-applied transient
  failure follows the exact effect policy under one stable effect identity.
- A runner/process failure may reclaim the same activation only under a new
  durability-claim epoch and the same committed checkpoint.
- An ambiguous external send becomes `OutcomeUnknown` and reconciles; it does
  not enter ordinary retry.
- A semantic Program failure commits once and is not converted into a delivery
  DLQ item.

### 12.2 Dead-letter meaning

DLQ applies to managed occurrence delivery or an operational activation that
cannot be processed after its closed retry policy. A DLQ record retains the
same occurrence/activation identity, reason, attempts, ordering lane, and
payload reference. Redrive creates another delivery/lease attempt while
preserving occurrence, target-application identity, and strict-lane position;
it never creates a new root Program Invocation.

Strict-order lanes cannot release later work merely because their head moved
to DLQ. Redrive atomically replaces the dead-lettered head at the same position;
authorized discard records the decision and advances the frontier. New
business reprocessing is a separate authorized operation that creates a new
occurrence and application identity.

### 12.3 Replay operations

The system names different replay modes:

- evidence restoration semantics owned by Agents: deterministic EventRef and
  Program state reconstruction with no managed delivery operation;
- Server projection rebuild: no Program execution;
- Server delivery redrive: same occurrence and target-application identity;
- Server activation recovery: same activation from committed checkpoint;
- deterministic diagnostic re-execution: isolated, no external effects, not
  production truth; and
- new business reprocessing: new explicitly admitted root occurrence.

No generic `replay` flag chooses among them.

### 12.4 Schedule and timer semantics

A schedule definition is Server-owned managed state with stable `schedule_ref`,
monotonic generation, exact target binding, canonical UTC representation,
timezone-data digest when civil time is authored, DST ambiguity/nonexistence
policy, leap-second policy, database-clock authority, lateness/catch-up limit,
and fixed-rate or fixed-delay anchor semantics.

One fire identity is `(schedule_ref, generation, nominal_time)`. Materializing
a fire atomically creates one internal-schedule `EventOccurrence<T>` and its
initial delivery; retry resolves the same fire. Cancellation increments the
generation so an old timer cannot publish after cancellation. Source/physical
time never substitutes for the authoritative schedule clock, and catch-up
never emits an unbounded burst.

The schedule contract remains Server-owned in every APXM deployment. A
standalone Agents Composition Root has no implicit scheduler. If it exposes
APXM schedule definitions or fire identities, it consumes the exact
Server-owned `ScheduleEngine` contract through an admitted durable
implementation. Otherwise a timer is an external source under a
`SourceContract`, not an APXM schedule.

### 12.5 Managed Host gateway

Host SDK owns Link frames, enrollment/session/resume, credit, acting-principal,
and effect protocol meaning. Server owns the managed attachment record,
attachment epoch, per-direction sequence/digest, durable outbox, request/result
correlation, result application, retry/DLQ, and operational query state.

Epoch takeover fences the old attachment. Reuse of one sequence with different
bytes is a conflict. Ack loss replays the same frame/result identity; a resume
gap is explicit and cannot be skipped. Detach or authority revocation blocks
new dispatch while still allowing a late result for an already-dispatched
effect to be recorded and reconciled. Separate credit classes keep live
commands, occurrence metadata, and bulk evidence/backfill from starving one
another.

### 12.6 Authority decision leases

An admitted authority decision identifies issuer/key epoch, authority epoch,
audience, Company/principal, exact operation/resource generation, decision id,
and validity bound. Effect claim revalidates the exact decision at the pre-send
boundary. Revocation or stale generation prevents new execution/dispatch;
results for an effect whose `DispatchStarted` already committed remain
recordable for evidence and reconciliation without granting new authority.

## 13. Quiescence, drain, and shutdown

Every runnable/running producer and internal completion source holds an
`ActivityCredit` with the closed ownership lifecycle
`Unowned -> Reserved -> Owned -> Transferred | Released`. Reservation happens
before a runnable lifecycle CAS; a losing CAS releases it. Before a node
releases its own credit, it reserves/transfers credits to every successor and
increments the activity generation before publication. Exactly one terminal
credit action is required for normal completion, lazy-cancelled or stale queue
entries, invariant failure, queue-publication failure, and panic fencing. A
leak prevents quiescence; a premature release is an invariant failure that can
produce false quiescence.

A ready node waiting for its per-activation dispatch token retains its activity
credit. A lower-level capacity wait that intentionally releases its credit
must first register a bounded capacity waker and increment the activity
generation; that waker reserves a new credit before republishing the node.
Activation bootstrap acquires a credit before any zero-predecessor publication.

Local quiescence is linearized only when activity reaches zero, all local/
injection queues are empty, and the generation remains unchanged across an
acquire recheck. Every blocked node must then classify as a committed external
wait, a capacity wait with a live credit/permit path, or an invariant failure.
A local wake count or one empty queue is insufficient. Model tests cover the
last-completion-versus-successor-publication and last-completion-versus-park
races plus every credit terminal path.

Runtime drain means no new activation is admitted locally, existing activation
claims are completed/parked or relinquished safely, blocking and async lanes
are drained, and workers park/exit.

Only Server can answer whether durable eligible, scheduled, leased, retrying,
or dead-letter work exists. ARTS/OCR-style epoch termination is therefore not
used as managed durable no-work truth.

## 14. Evidence and observation

Authoritative evidence records semantic transitions and fences, not every
queue probe. Required facts include:

- activation admitted/leased/started and matching fence epoch;
- node occurrence became runnable and started;
- NodeExecution outcome, attempt identity, and effect phase;
- invocation parked/resumed/yielded/returned/failed/cancelled;
- EventRef reservation/binding/fulfillment/consume/abandon/expire/cancel
  transitions; and
- Execution Commit id and committed sequence.

High-volume scheduler observations are sampled metrics/traces:

- ready-to-start delay by priority and operation class;
- local pop, injector pop, and steal counts;
- steal success/retry/empty rates and distance;
- runnable depth and age distributions;
- spin/yield/park/unpark counts and idle duration;
- per-domain imbalance and remote-memory indicators;
- activation admission saturation;
- async/blocking lane utilization; and
- stale-fence, duplicate-publication, and invariant-failure counts.

Telemetry loss cannot alter a queue, wake a Program, satisfy a dependency, or
drive recovery.

## 15. Performance verification

### 15.1 Microbenchmarks

The Agents benchmark suite measures:

| Benchmark | Variables | Required output |
| --- | --- | --- |
| dependency release | fan-in, fan-out, producer count | ops/s, p50/p95/p99 publish-to-runnable |
| local deque | task size, worker count | local ops/s, cache misses, scheduler ns/task |
| stealing | victim count, batch size, skew | steal success, retries, distance, fairness |
| park/wake | burst size, idle duration | wake latency, lost-wakeup count, unnecessary wakes |
| structured fork/join | width, depth, cancellation point | throughput, join latency, retained state |
| effect completion | async latency and completion burst | ready-to-start latency, compute-worker blockage |
| activation start | checkpoint size, graph size | lease-to-first-node latency and memory |
| commit fence | conflict and lease-loss rates | success/unknown/conflict latency |
| physical ingress | sample rate, window, evidence size | candidate-to-accepted latency, reducer CPU/memory, gap rate |
| reconnect/backfill | live/backlog ratio, disconnect duration | live-vs-backlog fairness, catch-up throughput, stale-command rejection |
| controller co-location | APXM load, evidence upload, accelerator load | controller jitter/deadline maxima and isolation violations |
| overload recovery | burst width, repeated cycles, reclamation lag | queue-age tails, peak/RSS recovery, retained buffers |
| effect fence | claim loss, partition, provider idempotency | prepare/dispatch/reconcile latency and duplicate-send count |

Results live under `.apxm/benchmarks/results/`, never under `docs/`.
Every result records machine/topology fingerprint, effective cpuset and
affinity, NUMA map, SMT, frequency governor, allocator, dependency revision,
warmup, sample count, confidence interval, and workload-corpus digest.

### 15.2 Scaling and topology tests

Run at 1, 2, 4, 8, and available physical-core counts; repeat with SMT on/off
where the harness can control it. NUMA tests compare pinned single-domain,
balanced multi-domain, and adversarial remote-data placement. Oversubscription
tests prove that blocking adapters and excessive async completions do not stall
compute progress. Hardware counters cover LLC/cache-line contention and remote
DRAM; idle tests measure CPU consumption and wake amplification.

The production policy is enabled only when it improves representative graphs
without unacceptable p99 regression. A single throughput number is not an
acceptance criterion. Tests also bound per-activation fairness so one wide
activation cannot monopolize every local worker.

Before measurement, Agents checks in a digest-bound performance-policy
manifest naming the fixed workload corpus and Runtime Profile digests,
supported topology classes, deterministic single-thread and non-stealing
baselines, normalized minimum throughput/scaling gains, maximum p99 and
fairness regression, maximum victim probes and batch size, class-dispatch and
activation-cap bounds, idle-CPU/wake-amplification budgets, and memory/RSS
reclamation ceilings. Results are compared within a topology class rather than
to one cross-machine absolute number. A topology/profile without a passing
policy result is not admitted for production parallel execution; it does not
silently select another scheduler.

### 15.3 Correctness and race tests

- model/loom tests for last-predecessor, duplicate publication, runnable
  transition, cancellation, and park/wakeup races;
- property tests for deterministic dynamic occurrence identities;
- deterministic single-thread executor parity against the parallel executor
  over identities, results, committed semantic facts, and required
  happens-before edges—not scheduler timing or total observation order;
- structured-scope and Hook/context ordering tests;
- failpoints before/after effect preparation, dispatch, result publication,
  checkpoint commit, activation reply, and lease expiry;
- stale-claim and split-brain runner tests;
- worker-panic tests proving the affected activation/runtime is fenced and
  recovered from the last durable checkpoint rather than continued with a
  lost private deque;
- crash recovery from every durable activation transition; and
- negative tests proving no transport ack, observer event, or wake token can
  fulfill an `EventRef`.

## 16. Physical AI profile

The runtime may support physical AI only under these additional constraints:

- physical provenance is a closed `Observation`, `Derivation`, or `Effect`
  family with exact references to immutable frame-tree/transform snapshots,
  calibration artifacts and effective intervals, units/conventions,
  covariance/confidence, algorithm/config/model digests, and execution domain;
- every physical observation/derivation has mandatory clock id, device boot or
  reset generation, clock epoch/discontinuity counter, acquisition interval,
  source sequence, synchronization method, mapping interval/rate/drift, and
  bounded uncertainty;
- source sequence defines the ordered chain within
  `(source, boot_generation, clock_epoch)`. Temporal order across sources, and
  between observations whose acquisition/uncertainty intervals overlap, is
  only partial. Server acceptance order is never physical chronology;
- occurrence/acquisition time, Server acceptance time, activation
  eligibility/deadline, and effect validity interval are distinct types;
- freshness is revalidated at every use and effect dispatch. A cross-machine
  absolute expiry is converted to a device-local monotonic deadline through an
  attested clock mapping; uncertainty beyond the safety margin fails closed;
- every physical source item is classified only by its immutable source
  contract; declared stream elements remain below the durable-event boundary
  until an admitted reduction produces a Program occurrence, while declared
  per-item candidates use ordinary bounded occurrence admission at any rate;
- an actuation Capability carries exact device/resource authority, device boot
  generation, persisted command sequence/idempotency scope, precondition and
  provenance snapshot, validity interval, sequence/fence, safe-state policy,
  acknowledgement class, and reconciliation behavior;
- after the generic durable `Prepared -> Dispatchable -> ClaimedBeforeSend ->
  DispatchStarted` boundary, the Adapter/device substate is closed:

  ```text
  DispatchStarted -> device_received -> accepted | rejected
                           |             |
                           |             +-> started -> completed
                           |                         \-> locally_stopped
                           |                         \-> outcome_unknown
                           +-> outcome_unknown
  ```

  Stop is separately `stop_requested -> stop_confirmed | cannot_stop |
  outcome_unknown`; device reboot invalidates the old command scope;
- a later sensor observation is a new occurrence and may corroborate an
  outcome, but an acknowledgement never proves physical completion;
- disconnect and device safe-state/failsafe transitions are expected closed
  states, not instructions to repeat motion and not product fallback paths;
- simulation, software-in-the-loop, hardware-in-the-loop, and physical
  evidence are distinct; and
- safety-critical bounded-deadline control remains on the device/edge controller.

The local executor is optimized for throughput and latency but makes no
hard-real-time claim.

## 17. Contract acceptance gates

The new runtime is not canonical until all gates pass:

1. Workspace, Agents, and Server ownership ADRs plus required Auth and Host SDK
   amendments are accepted.
2. Agents and Server owner schemas/descriptors identify one owner for every
   event type/occurrence/provenance, source contract/reducer/position/
   disposition, EventRef/fulfillment, delivery/target application,
   activation/claim, effect/attempt/reconciliation layer, schedule/fire, Host
   attachment/frame/result, response obligation/emitter, authority decision
   lease, idempotency scope, evidence type, and projection cursor.
3. Generated consumers are byte-identical products of those exact owner
   publications.
4. The readiness kernel and deterministic executor pass model/parity tests.
5. The work-stealing executor passes race, scaling, NUMA, starvation, and
   backpressure gates.
6. Server occurrence/source, delivery/strict-lane, activation claim,
   effect-dispatch/reconciliation, schedule/time, and Host gateway state
   machines pass executable vectors and failpoint tests.
7. Every producer and consumer in the canonical Compatibility Set uses those
   exact owner contracts; mixed schema generations are inadmissible.
8. Managed Server and standalone durability adapters pass identical
    Agents-owned transition histories and failpoints.
9. Every executable durable object references an available exact contract,
    artifact, and schema digest; an unavailable digest is inadmissible and
    cannot execute.
10. Root binding proves occurrence payload `T` exactly satisfies Program input
    `I`, and EventRef reservation co-commits with effect/correlation identity
    before dispatch.

State transformation, authority barriers, deletion, rollback, and product
activation are full-replacement-plan gates, not part of this target semantic
contract.

## 18. Explicit non-goals

- Supporting dynamic DAG splicing or a second execution engine.
- Adding an AIR operation for queueing, scheduling, resume, or work stealing.
- Treating Tokio, a broker, or a database queue as the PXM semantic scheduler.
- Moving Server admission, leases, DLQ, or multi-tenant policy into Agents.
- Claiming exactly-once external effects.
- Executing high-frequency safety control in APXM.
- Exposing an alternate route, reader, or scheduler path.

## 19. Evidence sources

The accompanying coordinator research record pins and audits DARTS, ARTS,
SWARM, OCR, durable workflow/actor/queue systems, event theory, and physical-AI
sources. Its conclusions are inputs to this design, not runtime dependencies.
The code-level source note is
`docs/research/agents-runtime-darts-arts-swarm-ocr-2026-07-30.md` in the APXM
coordinator checkout. Before this contract is accepted, the source-note claims
and exact permalinks must be copied into the governing ADR review packet or
linked through an immutable review artifact; the Agents product repository
must not depend on a mutable coordinator worktree path at runtime.

The current frontend/compiler/artifact/runtime gap and end-to-end conformance
matrix are audited separately in the coordinator note
`docs/research/agents-typed-event-flow-audit-2026-07-30.md`; it is review
evidence and does not become a runtime dependency.

The immutable source-classification, pre-admission backpressure, atomic capacity
reservation, and gap constraints are grounded in the coordinator note
`docs/research/event-runtime-source-classification-and-backpressure-2026-07-30.md`.
