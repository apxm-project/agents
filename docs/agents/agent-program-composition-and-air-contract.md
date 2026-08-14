# Agent Program composition and AIR contract

- Status: canonical APXM v1 contract
- Owner: APXM `agents`
- Decisions: [ADR-0008](../adr/0008-agent-programs-compose-through-new-and-invoke.md), [ADR-0009](../adr/0009-air-has-five-public-semantic-operations.md), [ADR-0010](../adr/0010-agent-program-source-owns-context-hooks-and-conversational-loops.md), and [ADR-0011](../adr/0011-agent-program-execution-is-one-end-to-end-spine.md)
- Canonical contracts: `apxm.frontend-graph`, `apxm.air`, `apxm.executable-artifact`, `apxm.runtime-evidence`

This document is the normative APXM v1 contract for composing, compiling,
and executing Agent Programs. It is intentionally target-only. The key words
**MUST**, **MUST NOT**, **SHOULD**, and **MAY** describe the accepted target,
not the current implementation.

## 1. Ownership and non-goals

The `agents` repository owns:

- Python and TypeScript Agent Program authoring semantics;
- the canonical `FrontendGraph` and the closed AIS effect/composition and
  structural operation families;
- the compiler verifier, lowering, source map, and printer;
- executable artifact admission;
- Program Instance state, invocation, structured children, Hooks, context,
  generic checkpoint/recovery, and execution evidence;
- the typed model-adapter/inference-backend boundary required by `model.call`;
  and
- cross-language conformance vectors.

Other APXM planes may expose root instance/run APIs, create and publish Agent
Programs, or visualize their evidence. They may not redefine `.new`,
`.invoke`, context, Hooks, loops, AIS/AIR, runtime, or model-backend semantics.

Installable frontends expose generic Agent Program APIs only.
`ConversationalAgent` is a repository-example construct. Example names are not
frontend exports or core contract names.

This contract does not define user administration, Agent Definition lifecycle,
company hierarchy, Skill catalogue policy, product billing, provider-specific
protocols, deployment placement, UI layout, or package coordinates.

## 2. Type model

```text
AgentProgram<I, O, C>
ProgramRef<I, O, C>
ProgramInstanceRef<I, O, C>
ProgramInvocation<I, O>
AgentIdentityBinding
EventRef<T>
AgentFacade<I, O, C>
NodeExecution
```

`I`, `O`, and `C` are compiler-known serializable types:

- `I` is invocation input;
- `O` is the plain output returned to program source; and
- `C` is explicit Program Context/state owned by a Program Instance.

An Agent Program either declares a deterministic default `C` or is
stateful-construction-only. Omitting `initial_context` uses that default;
one-shot `Program.invoke` is rejected when no default exists.

`ProgramRef` pins the imported artifact and entrypoint by digest and declares a
target Agent Identity requirement. The artifact declaration is not identity
proof: admission MUST bind it to an authenticated, company-scoped Agent
Identity and record that binding. `agent.identity` exposes only the admitted
binding. A reference cannot resolve mutable “latest,” a filesystem path, a
runtime profile, an endpoint, or a runtime-selected arbitrary target.

`ProgramInstanceRef` additionally identifies a stateful logical instance and
its owning parent/top-level scope. It is opaque and carries no authority.
It is an affine owner-scoped compiler value, not portable user data: it may be
retained in its owner's `C`, but MUST NOT appear in cross-program/root `I` or
`O`, be forged from bytes, or transfer to another owner. Runtime persistence
encodes it only inside that owner state.

## 3. Source API

Under ADR-0015 the author-facing surface is source-first: a definition is
declared with `Agent`, its state schema with `Context`, exact model targets with
`Model`, and model-callable actions with `Tool` (advanced programs add
`Capability`, `Event`, `Hook`, `TaskGroup`). The callback parameter `agent` is
inferred and exposes `.context` and `.yield_(...)`. No ordinary source imports
`AgentProgram`, `AgentFacade`, node/region ids, or operation constants.

### 3.1 Python

```python
from apxm_program import Agent, Context, Model, Tool


@Context
class Conversation:
    messages: tuple[Message, ...] = ()


SearchWeb = Tool[SearchRequest, SearchResult]("capability.search-web")
SupportModel = Model[ModelRequest, ModelResponse]("model.support")


@Agent(input=ConversationInput, output=ConversationOutput, context=Conversation)
async def Support(agent, incoming):
    ...


specialist = Specialist.new(
    context=SpecialistContext(domain="security"),
)
answer = await specialist.invoke(SpecialistInput(question=question))
summary = await Summarizer.invoke(SummaryInput(answer=answer))
```

### 3.2 TypeScript

```typescript
import { Agent, Context, Model, Tool } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

const ConversationContext = Context<Conversation>({ messages: [] });
const SearchWeb = Tool<SearchRequest, SearchResult>("capability.search-web");
const SupportModel = Model<ModelRequest, ModelResponse>("model.support");

export const Support = Agent<ConversationInput, ConversationOutput, Conversation>({
  name: "Support",
  context: ConversationContext,
  async run(agent, incoming) {
    const specialist = Specialist.new({ context: { domain: "security" } });
    const answer = await specialist.invoke({ question: incoming.question });
    return await Summarizer.invoke({ answer });
  },
});
```

The TypeScript module states its own source once with `source(import.meta.url)`,
as shown in [Create Your First APXM Agent](first-agent.md). Every module-scope
binding the Agent body reads is that Agent's declaration; neither language asks
an author to list them a second time.

Both examples MUST record semantically equivalent `FrontendGraph` values. The
exact Python decorator/TypeScript factory matrix is frozen by ADR-0015 §4.

### 3.3 Rules

- `Program.new(...)` MUST return `ProgramInstanceRef<I, O, C>`.
- `Program.new(...)` is a compiled constructor expression, not an awaited
  client/network call; graph data dependency prevents use before logical
  creation completes at runtime.
- `Program.invoke(input)` MUST lower with a `ProgramRef` receiver and execute
  one-shot in an isolated ephemeral instance initialized from the program's
  declared default `C`.
- `instance.invoke(input)` MUST lower to the same `program.invoke` AIR op with
  a `ProgramInstanceRef` receiver.
- Both invocation forms MUST return `O`, never an operational envelope.
- Plain invocation is direct call-and-await. Concurrency MUST use one
  language-neutral frontend structured task scope, lowered to structural
  parallel regions; raw Python-coroutine and TypeScript-Promise start/drop
  behavior is not semantic input.
- Same-program functions MUST remain internal calls and MUST NOT create a
  Program Invocation or Session Output subtree.
- An authored Program Instance MUST be attached to its owning Program Instance.
  A root instance is created by the host admission API, not by an unscoped
  form of `.new`.
- No authored API accepts protocol, endpoint, runtime profile, provider,
  process, executable, path, current directory, worker, or placement options.

## 4. Program Instance and Invocation state machines

```text
Instance:
admission_pending -> ready -> invoking --committed_yield------> ready
                              |       --committed_return-----> completed
                              |       --failed/cancelled-----> ready
ready | invoking -----------> closing -----------------------> closed
ready | invoking -----------> cancelling --------------------> cancelled

Invocation:
admission_pending -> running <-> waiting_event
running ----------> committed_yield | committed_return | failed
running | waiting_event ----> cancelling
cancelling -----------------> cancelled | cancellation_unconfirmed
```

An invocation failure or invocation-level cancellation preserves the owning
instance's last successful commit and returns a non-closing instance to
`ready`. An owner close/cancel changes the Instance state and propagates to its
descendants. These are the same authoritative states exposed by section 14;
the diagram does not introduce a second lifecycle vocabulary.

### 4.1 Creation

`program.new` MUST:

1. verify the exact Program reference and typed initial context;
2. derive a deterministic logical instance identity from its owner, callsite,
   and occurrence;
3. admit the program, required handlers, Capabilities, contracts, and limits;
4. bind the callee's independently authenticated Agent Identity and attenuated
   authority;
5. atomically create isolated initial Program Context plus the durable owner
   edge; and
6. return only after the logical instance is durable enough for runtime replay
   to resolve the same instance.

Replay MUST NOT create a duplicate. A conflicting identity with different
content MUST fail as a typed collision.

Every authored visit to `program.new` is a distinct logical occurrence. Runtime
retry/replay of that occurrence resolves the same instance; a later authored
loop visit or parent invocation creates another instance. Reuse across owner
invocations requires source to store the opaque `ProgramInstanceRef` in the
owner's explicit Program Context. The reference is serializable only inside
that owner state and MUST NOT be exported or transferred to another owner.

### 4.2 Invocation

One Program Instance is **single-flight, fail-busy**. The runtime MUST NOT
silently queue concurrent calls. A recursive call to the currently executing
same `ProgramInstanceRef` is also `ProgramInstanceBusy`; queueing belongs
outside the instance. An invocation:

1. validates the complete current authority and target compatibility;
2. opens a new invocation and Session Output occurrence;
3. supplies typed input and last committed explicit Program Context;
4. on first invocation, passes `I` to the entrypoint; after yield, restores the
   compiler-owned continuation and passes `I` as its typed resume argument;
5. executes until typed return, program yield, failure, or cancellation;
6. atomically commits a successful output, next Program Context, continuation
   or completion state, commit sequence, resolved-child set, and evidence
   outbox record; and
7. exposes output `O` to the caller while evidence remains separate.

Program `yield` leaves the instance ready with a compiler-owned continuation
(program counter and live typed loop values). Program `return` marks it
completed; later invocation fails with `ProgramInstanceCompleted`. For a
one-shot `ProgramRef`, the first return/yield returns `O`, then the ephemeral
instance and any continuation close. `await.event` instead parks the same
invocation and is not an invocation-output boundary.

Failure before step 6 preserves the last committed Program Context and
continuation. External effects retain their own effect truth; rolling back
local state does not claim an already committed effect was undone.

### 4.3 Lifetime and structured children

- A child Program Invocation is attached to the current parent invocation.
- A child Program Instance is attached to the owning parent Program Instance.
- A child created inside one-shot invocation belongs to its ephemeral instance
  and MUST close when that owner returns/yields; its reference cannot escape.
- Parent success cannot commit with an unresolved active child invocation.
- Parent yield retains its owned ready child instances. Parent return/
  completion or close propagates close; parent cancellation propagates
  cancellation. No owner becomes terminal until descendant states are durably
  reconciled.
- Cancellation is fenced and bounded. Each active child carries an ownership
  epoch, heartbeat/lease, and admitted cancellation deadline. A child either
  acknowledges a terminal state or, after lease/deadline expiry, becomes
  `cancellation_unconfirmed`; the owner may then terminate as cancelled/closed
  with explicit unknown external-effect refs, never as successful. The fencing
  epoch is required at every node checkpoint, model/Capability admission,
  Capability invocation, and effect prepare/commit; a stale epoch rejects new
  work, effects, state, and output commits. Only an effect already initiated
  under the valid epoch may become outcome-unknown. Source cannot disable or
  widen these liveness bounds.
- Ownership and child identity are committed atomically; crash recovery walks
  the durable owner edges, so no child becomes an orphan.
- Dropping a structured task handle does not detach its child. Every task scope
  joins all children, and an uncaught child failure fails the scope.
- Canonical v1 provides no detach, abandon, transfer-owner, or fire-and-
  forget form.
- Fan-out/depth limits are admitted runtime policy and cannot be widened by
  source or context.

## 5. Context and callback contract

### 5.1 Program Context

Program Context is `agent.context: C`, an explicit typed local/state value.
Source assignments produce ordinary value flow and loop-carried state. For a
stateful instance, only the value at a successful invocation boundary becomes
the next committed context.

The runtime MAY record before/after snapshots or content-addressed diffs for
evidence. Those records do not become source mutation primitives.

The following MUST be explicit program code:

- adding model, Capability, child, Skill, Hook, or input data to context;
- removing, replacing, summarizing, or ordering context data;
- merging branch results;
- projecting model or Capability context; and
- deciding what survives to the next instance invocation.

### 5.2 Agent Facade

Every Hook receives one portable `AgentFacade<I, O, C>` named `agent`. The
facade MAY expose only typed fields valid at its static binding point:

```text
agent.identity
agent.context
agent.input
agent.output
agent.current_node
agent.current_model_call
agent.current_capability_call
agent.capabilities
agent.skill_discovery
agent.invocation
agent.budget
agent.deadline
agent.cancelled
```

`agent.identity` is the authenticated admission binding for the current
program, never an authored artifact string or inherited parent identity.

The facade MUST NOT expose runtime object identity, Execution Context,
credentials, bearer grants, scheduler, broker, database, storage backend,
mutable graph topology, or ambient filesystem/network access.

### 5.3 Hook bindings

Hook bindings are static artifact metadata containing:

```text
hook_id
scope: agent | loop | node | model | capability
phase: before | after
target_selector
declaration_order
handler_ref + digest
input/output types
return_mode: observe | replace_result
body_region_id
assigned_context_value_id (replace_result only)
```

A Hook's body is captured as a region of the same AIR the Agent body lowers to,
reached through `body_region_id`. Its Model and Capability calls are ordinary
`model.call` and `capability.invoke` nodes, so context assembly, token budgets,
and compaction are measurable workflow structure rather than behavior behind a
digest-referenced handler the artifact only names. A handler run by a host port
could perform effects the artifact never declared; a captured body cannot.

Every callback has one argument shape: `async (agent) -> ...`. Context is
updated through `agent.context = ...`, and that assignment is what the return
mode is derived from: a body that assigns Context is `replace_result` and names
the assigned value, a body that mutates nothing is `observe`. The mode is not an
author assertion the compiler takes on trust — execution refuses an observing
Hook whose handler returns any replacement.

Frontend context assignment and optional result replacement lower to the
compiler-internal tagged ABI
`HookEffect<C,T> { next_context: C, result: Keep | Replace(T) }`. It is
structural IR, not a public `HookResult`; the tag remains unambiguous when `T`
is nullable. `before` callbacks run outer-to-inner and declaration order.
Successful `after` callbacks unwind inner-to-outer and declaration order, each
observing the current result. A failure skips remaining success-only `after`
callbacks and follows ordinary authored `try`/`catch`; v1 has no error Hook.

Model and Capability calls from a Hook pass through normal operation handlers,
budgets, cancellation, evidence, and Capability authority. They create their
own NodeExecutions. There is no Hook bypass or implicit recursion. Source must
author any desired recursion, whose admitted depth limits still apply.

## 6. Skills and Capabilities

- Skills are context available through discovery, not executable authority.
- No Skill body is automatically inserted into Program Context, a loop
  occurrence, or a model request.
- `search_skills`/`read_skill` are ordinary admitted Capabilities.
- An agent associated with Skills MAY include a small static instruction that
  explains how to use those discovery Capabilities.
- The program explicitly decides which discovered content reaches a model.
- Parent Skills do not copy to children. A child/specialist resolves its own
  discovery scope.
- Capabilities remain executable actions and require a complete admitted
  Capability Grant. A Program Instance handle, Skill, prompt, or context item
  is never a grant.
- Agent, loop, Node, Model, and Capability scopes MAY all have Hooks — the same
  five the FrontendGraph schema and the runtime evidence `HookScope` carry; a
  Capability Hook cannot bypass Capability admission.

## 7. FrontendGraph

`apxm.frontend-graph` is a language-neutral typed value. At minimum it
contains:

```text
contract_id
source_language
program definitions and typed entrypoints
exact imported ProgramRefs
typed values, blocks, regions, functions, and data edges
five semantic operation records
closed structural operation records
explicit context/state flow
static Hook bindings
Capability and model requirements
source map and non-executable semantic annotations
source and handler digests
```

The graph MUST NOT contain:

- raw arbitrary op names or raw AIR text;
- behavior duplicated from a manifest;
- runtime endpoint/path/profile/process/worker choices;
- credentials, bearer grants, or provider secrets;
- dynamic Hook/Capability registration; or
- a conversation-specific runtime type, memory tier, or autonomous-loop mode.

Unknown semantic fields fail closed. Descriptive source-map extensions may be
ignored only when the contract marks them non-semantic.

Under ADR-0015, the five semantic operation records and the closed structural
operation records are **typed source intents**, not AIR/AIS operation spellings.
FrontendGraph carries discriminated intents for Model invocation,
Tool/Capability invocation, Agent creation/invocation, and Event wait, plus
language-neutral conditional, loop, task-scope, try/catch, yield, and return
intents. A Tool invocation and an advanced Capability invocation are distinct
typed intents here; Rust alone converges both to `capability.invoke` and selects
every other AIS operation. FrontendGraph MUST contain no `ais.*` kind and no raw
operation string, and public authoring packages MUST export no operation
constant or structural AIS kind.

## 8. AIR effect/composition operations

### 8.1 `model.call`

```text
model.call(model_ref, request, options) -> ModelOutput<T>
```

`model_ref`, request schema, structured output type, Tool schemas, budgets, and
options are explicit typed operands/properties. The operation does not mutate
context, beliefs, goals, or graph topology and does not execute a tool loop.

The selected Deployment Composition Manifest predeclares exactly one
`model_ref -> ModelDeploymentRef -> ExactPortBindingRef` mapping. The host or
the identical standalone composition verifier validates and
materializes it without candidate search, ranking, or fallback. The adapter
sends only the admitted Model Context, messages, Tool schemas,
structured-output schema, options, budget, deadline, cancellation, and trace
identity. It returns typed text/structured output, refusal, provider-exposed
reasoning summary, usage, finish state, or Tool-call requests. A Tool request
is data for authored program control flow; the backend never executes the
Tool, invokes a Capability, calls the model again, mutates context, or creates
a Program Invocation.

The adapter and backend MUST NOT substitute another model, provider,
deployment, prompt, Tool schema, sampling policy, or execution path after
failure. Load balancing is allowed only among replicas identical under the
same admitted model deployment digest and configuration. Streaming chunks are
provisional; final output and usage become authoritative only through the
runtime NodeExecution commit. Each call has a stable `model_effect_id` and
request digest across its attempts. Automatic retry is allowed only before the
adapter may have sent the request, or when the exact admitted backend proves an
idempotency/reconciliation contract that prevents duplicate billable or
stateful work. If acceptance is possible and reconciliation is unavailable,
the node terminates as typed `ModelOutcomeUnknown`; it records known or
uncertain usage/cost provenance and MUST NOT silently reissue the request.

### 8.2 `capability.invoke`

```text
capability.invoke(capability_ref, arguments, context_projection?) -> T
```

It is the only Capability execution chokepoint. Admission, grant validation,
effect identity, retry policy, idempotency, cancellation, redaction, and audit
apply here. Sandbox/code execution and external I/O are Capabilities, not
privileged AIR escape hatches.

### 8.3 `program.new`

```text
program.new(program_ref, initial_context?)
  -> ProgramInstanceRef<I, O, C>
```

It performs the stateful logical creation contract in section 4. It cannot
select placement or create an external process. Source syntax is synchronous
because Python/TypeScript are declarative compiler frontends: they record a
typed result edge, while runtime execution of that edge completes durable
creation before any dependent node may use the reference.

### 8.4 `program.invoke`

```text
program.invoke(ProgramRef<I, O, C> | ProgramInstanceRef<I, O, C>, input: I)
  -> O
```

The receiver type selects one-shot versus stateful semantics. The operation is
attached and awaits completion. Compiler-internal parallel/task lowering MAY
schedule several calls concurrently while preserving structured ownership and
typed joins. `input: I` is the only caller-data operand; any selected caller
context is explicitly encoded inside `I`. Dynamic choice is limited to a
finite set of statically imported typed receivers selected by ordinary control
flow.

### 8.5 `await.event`

```text
await.event(EventRef<T>, timeout?, cancellation?) -> T
```

It parks the current Program Invocation without producing `O` or committing an
invocation boundary. APXM contracts/runtime own the generic event lifecycle;
the graph only consumes an `EventRef<T>` supplied as typed input or returned by
an admitted Capability.

An Event reference is unforgeable and binds event id, schema digest,
company/authority scope, expiry policy, and creating identity. At first await,
APXM atomically binds it to one program instance/invocation, static callsite,
and logical occurrence. The lifecycle is:

```text
pending -> fulfilled | expired | cancelled
```

- fulfillment uses the APXM root event API, a complete grant/approval proof,
  and an idempotency key;
- the first admitted terminal value wins; an identical retry is idempotent and
  a conflicting value fails;
- registration is durable before the wait becomes externally visible;
  fulfillment that races or precedes execution is retained and consumed once;
- replay of the same await occurrence observes the same terminal event and
  never consumes it twice;
- attempting to await the same ref from a different owner, callsite, or logical
  occurrence fails with `EventAlreadyBound`;
- timeout and owner cancellation produce typed outcomes and cannot be confused
  with fulfillment; and
- the runtime checkpoints its compiler-owned continuation while waiting, but
  the invocation remains active and its parent cannot commit.

The event-terminal transaction also writes one idempotent `wake_requested`
work/outbox item keyed by the bound await occurrence. Delivery may be
at-least-once, but consuming it uses a compare-and-set from `waiting_event` to
`running`, records `invocation.resumed`, and marks the wake consumed in one
transaction. A reconciler re-enqueues every terminal event whose wake remains
unconsumed after a crash. If cancellation already won, the wake records that
terminal truth and cannot resume execution.

For pre-fulfillment, no await occurrence exists when the event first becomes
terminal. The first-await registration transaction therefore binds the
occurrence and creates the same deterministic `wake_requested` item (or
atomically performs its equivalent waiting-to-running CAS). The reconciler
also detects every terminal-and-bound event with a missing or unfinished wake
and repairs it idempotently.

AIR contains no polling URL, notification target, store, backend TTL, or
`resume` operation. Approval products fulfill this APXM contract rather than
defining another event state machine.

## 9. Structural AIS and durability

The second closed AIS family provides ordinary functions, regions, blocks,
values, branches, switches, loops including `ais.loop`, parallel joins,
try/throw/catch, return, and yield. It is versioned and generated as part of
AIR but is not exposed as an agent operation builder. The five operations in
section 8 remain the complete effect/composition family; `ais.loop` is not a
sixth member.

- The frontend standard library's structured task scope lowers to parallel
  regions whose exit joins all children; there is no detach or language-native
  coroutine/Promise scheduling semantic.
- Program yield returns `O`, commits next `C` plus the continuation, and its
  resume successor accepts the next `I`.
- Python and TypeScript expose the equivalent compiler-recognized structural
  primitive `incoming = await agent.yield_(output)`; it is not a public AIR
  operation and never embeds Program Context inside `output`.
- Program return returns `O` and completes the Program Instance.
- Loop-carried values represent accumulated context/state.
- Region/block arguments replace a generic `MERGE` operation.
- Runtime checkpointing is automatic at admitted durable boundaries.
- Storage location, backend, TTL, and recovery policy come from the injected
  Runtime Profile, not AIR.
- Runtime retry remains an attempt under one NodeExecution. Source-authored
  repetition creates a new NodeExecution occurrence.

## 10. Generic loop iteration contract

Source authors ordinary structured loops and yield points through generic
Agent Program APIs. Rust emits `ais.loop` and related structural AIS; source
maps identify static loop regions without a conversation-specific annotation.

When one loop body and its back-edge commit atomically, runtime MUST emit one
`LoopIterationCompleted` fact in the same Execution Commit. The fact MUST
contain:

```text
static_loop_id
loop_occurrence_id
iteration_index
program_invocation_id
causal_node_execution_ids
```

`iteration_index` is zero-based within the dynamic loop occurrence. Fact
identity and ordering MUST be replay-stable. A failed, cancelled, or rolled-back
body MUST NOT emit completion. A trace or region-start fact
cannot substitute for the committed fact.

Hosts may project generic loop iterations and the core has no `Turn` type.
Repository
examples may label a conversational iteration as a “turn” in example-local
documentation.

### 10.1 External agents are Capability effects

An ACP peer's prompt loop is not an APXM structural loop occurrence. Source
uses the typed External Agent Capability
surface; every operation lowers to `capability.invoke`. The outer occurrence is
one Capability NodeExecution whose ordered ACP messages, plans, diffs, tools,
terminal activity, reverse requests and usage are nested attributed evidence.
The runtime never turns those peer-private events into APXM nodes.

V1 source selects one exact admitted External Agent Profile and one exact model
deployment. Policy-based routing is external to this core contract and cannot
change v1 composition or add AIR operations.

## 11. Execution evidence

`apxm.runtime-evidence` records, at minimum:

```text
program/artifact/entrypoint identity
authenticated Agent Identity binding
program_instance_id when stateful
program_invocation_id and parent lineage
static callsite and logical occurrence
static program_node_id
node_execution_id per actual visit
attempt_id per runtime retry
region occurrence ids
static loop ids, dynamic loop occurrence ids, and iteration indexes
status, timestamps, cancellation, and typed errors
model/capability/program operation evidence, including model/effect ids
Hook executions and context before/after refs
usage, cost inputs, trace/provenance, and placement
Session Output tree and content refs
```

Lifecycle truth is an append-only sequence of monotonic facts, not a trace
span or client projection:

| Fact | Required authoritative content |
| --- | --- |
| `instance.state_changed` | monotonic sequence, prior/next state, ownership epoch, command/cause, exact instance scope |
| `invocation.state_changed` | monotonic sequence, prior/next state, ownership epoch, command/cause, exact invocation scope |
| `instance.created` | exact artifact/entrypoint, admitted Agent Identity, owner edge, creation occurrence, initial-context digest |
| `invocation.admitted` | instance/invocation/parent/callsite/occurrence, input digest, authority/admission refs |
| `child.attached` | parent and child identities, owner, target identity/artifact, structured-scope identity |
| `attempt.recorded` | NodeExecution, attempt number, retry cause, timestamps |
| `LoopIterationCompleted` | static loop id, dynamic occurrence id, zero-based iteration index, Program Invocation id, causal NodeExecution ids |
| `invocation.committed` | monotonic commit sequence, `yielded` or `returned`, output/context/continuation refs, resolved-child set |
| `invocation.failed` / `invocation.cancelled` | typed cause, last commit sequence, effect outcome refs |
| `event.created` | event id/schema/scope/creator, idempotency identity, expiry policy |
| `event.await_registered` | bound owner/instance/invocation/callsite/occurrence and parked continuation ref |
| `invocation.parked` | invocation/event binding, continuation/checkpoint ref, deadline |
| `event.terminal` | event/await occurrence, fulfilled/expired/cancelled state, value digest when fulfilled |
| `invocation.resumed` | event/wait occurrence, prior parked sequence, wake id, resumed running sequence |
| `instance.closed` / `instance.cancelled` | terminal owner state and reconciled descendant set |
| `delivery.recorded` | external consumer/delivery status, always separate from execution success |

Every authoritative state transition above—creation, admission, child attach,
attempt state, completed loop iteration, park/event registration, event
terminal, successful commit, failure/cancellation, instance terminal, and
delivery—MUST share one
transaction with its corresponding fact/outbox row. Evidence publication is
idempotent and reconstructable from those outboxes. A trace, socket close,
token stream, or delivery record cannot override lifecycle truth. Uncertain
external effects are recorded as `effect.outcome_unknown`; they do not silently
turn an invocation into success or erase the last committed state.

Every state in section 14 is reconstructed from the latest scoped
`instance.state_changed` or `invocation.state_changed` sequence. The specialized
facts add required payload and MUST share the same sequence/transaction; they
cannot contradict the state fact.

A Session Output folder belongs to each actual NodeExecution occurrence, not
only to the static node. Authorized host inspection can drill from a
loop-iteration projection to each NodeExecution and its evidence. Access and
content capture remain governed by host access, retention, and
observability policy.

## 12. Failure contract

Compilation/admission MUST fail on:

- retired/unknown operation ids or contract versions;
- untyped or mutable program targets;
- wrong input/output/context types;
- missing or digest-mismatched program/handler references;
- illegal Hook result type or target selector;
- dynamic grants/credentials/paths embedded in the graph;
- unsupported instance ownership or detach semantics; and
- unresolved structural children at a terminal boundary.

Runtime produces distinct typed failures for instance busy/missing/completed/
closed, admission or identity-binding denial, incompatible artifact,
capability denial, model failure, child failure, event timeout/conflict,
event already-bound, cancellation/cancellation-unconfirmed, resource limit,
handler failure, and internal runtime failure. None selects a fallback
execution path.

## 13. Full replacement

The target release removes all current operation builders, handlers, generated
types, tests, docs, and artifacts that encode retired semantics. It rejects old
artifacts and provides no runtime translator. Historical evidence may be kept
as a digest-bound export, never as executable legacy support.

The complete current-operation disposition is normative in
[ADR-0009](../adr/0009-air-has-five-public-semantic-operations.md). The
implementation and deletion sequence is the
The implementation follows the current source and compiler ownership rules.

## 14. Root lifecycle service contract

The owning APXM service schema MUST generate clients for these semantic
operations; HTTP/RPC route spelling is intentionally not frozen here:

```text
CreateRootInstance
InvokeRootProgram
InvokeInstance
GetInstance
CloseInstance
CancelInstance
GetInvocation
CancelInvocation
CreateEvent
GetEvent
FulfillEvent
CancelEvent
ListEvidence(after_cursor)
```

Every request, including reads, carries company/tenant scope, request id,
authenticated Acting Principal/service identity, authorization/grant refs,
trace/correlation ids, and access purpose where content may be returned. Reads
re-evaluate current authorization and audit permissioned content access; a
cursor or resource id is never authority. Mutating requests additionally carry
an idempotency key and, as applicable, authenticated target Agent Identity,
exact artifact/entrypoint/Compatibility Set, approval refs, and typed payload.
The server—not source code—chooses placement and runtime profile.

The authoritative states are:

```text
Instance:   admission_pending | ready | invoking | completed |
            closing | closed | cancelling | cancelled
Invocation: admission_pending | running | waiting_event | committed_yield |
            committed_return | failed | cancelling | cancelled |
            cancellation_unconfirmed
Event:      pending | fulfilled | expired | cancelled
```

A command receipt has `accepted`, `duplicate`, `rejected`, or `conflict`
disposition plus the stable resource id and current authoritative state when
one exists. `accepted` means the command/idempotency record is durable, not
that execution succeeded. Product/client **pending** means the resource is in
a nonterminal authoritative state. **Unknown** is only a client transport
projection when no response was observed; it is never written as success or a
runtime terminal state and MUST be reconciled by the same idempotency key or
stable resource id.

Idempotency scope is `(company/tenant scope, operation, idempotency_key)`.
The canonical request fingerprint includes every semantic field. Repeating the
same key/fingerprint returns the same resource and current/terminal result;
reusing the key with another fingerprint is `conflict`. Retention of that
record MUST cover the full resource/evidence retention window.

`ListEvidence(after_cursor)` is an exclusive, stable ordered read over one
authorized scope/filter. Each fact has immutable `fact_id` and monotonically
increasing scoped `event_sequence`. The opaque signed cursor binds that scope,
filter, and last returned sequence but no authority. A page returns facts in
ascending order plus `next_cursor`, `high_watermark`, `retention_floor`, and
`has_more`. Retrying the same cursor/filter is stable while retained; transport
replay may duplicate a page, so consumers deduplicate by fact id/sequence, but
the service never silently omits an in-range fact. If the cursor precedes the
retention floor, the read fails with typed `EvidenceGap` containing the floor
and any authorized archive/snapshot reference. Permission changes may redact
content on a later read without changing lifecycle fact identity or order.

Race rules are deterministic:

- invoke against a non-ready instance returns the exact busy/completed/closed
  error and never queues silently;
- successful commit and cancellation compete in one authoritative transition;
  whichever transaction commits first determines lifecycle truth;
- cancelling an already terminal invocation returns its terminal truth;
- close is graceful: it enters `closing`, prevents new invokes, closes owned
  ready descendants, and waits/fences active descendants before `closed`;
- cancel enters `cancelling`, propagates cancellation, and reaches `cancelled`
  or explicit `cancellation_unconfirmed` evidence under the bounded lease rule;
  and
- delivery/stream disconnect never cancels work or implies success unless an
  explicit authorized cancellation command wins the state transition.

`CreateRootInstance` is the top-level analogue of authored `program.new` but
is not exposed inside Agent Program source. Root one-shot and stateful calls
use the same artifact, identity, state, commit, event, and evidence semantics
as nested calls.

## 15. Required conformance vectors

Positive vectors MUST cover:

- semantically equivalent Python and TypeScript graphs for all five ops;
- one-shot program invocation and stateful repeated instance invocation;
- authenticated root/child Agent Identity binding independent of artifact
  self-description;
- context preservation across successful invokes and preservation of last
  commit on failure;
- yield/resume input and continuation preservation, return/completed behavior,
  and one-shot first-yield disposal;
- instance-busy/self-invocation behavior and deterministic creation replay;
- attached child success/failure/cancellation, task-scope joins, and structured
  fan-out;
- explicit context projection and no implicit child/result merge;
- Agent Facade before/after Hooks at every supported scope, deterministic
  ordering, nullable result replacement, and explicit next-context ABI;
- event pre-fulfillment, fulfillment race/retry/conflict, timeout,
  pre-fulfilled first-bind/wake, cancellation, crash-before/after-wake,
  idempotent resume, replay, and pending/bound evidence;
- model tool request followed by explicit Capability and later model calls;
- model pre-send retry, backend-idempotent retry/reconciliation, and
  post-send `ModelOutcomeUnknown` without duplicate work;
- Skill discovery without automatic Skill injection;
- generic loop/yield, `LoopIterationCompleted`, replay-stable iteration
  projection, and NodeExecution evidence; and
- local embedded and host-admitted execution equivalence.

Negative vectors MUST cover:

- every retired operation id and pre-canonical contract identity;
- raw op builder, runtime path, protocol, endpoint, profile, or credential in
  source/graph;
- `instance.invoke` on missing/completed/closed/wrong-scope instance;
- concurrent instance invocation;
- raw coroutine/Promise scheduling or dynamic arbitrary ProgramRef creation;
- EventRef reuse from another owner/callsite/occurrence;
- cancellation while a child execution plane is permanently unavailable,
  including lease expiry, stale-epoch Capability/effect rejection, fenced late
  commit, and already-started unknown effect evidence;
- evidence cursor ordering, page replay dedupe, authorization re-check,
  watermark, and typed retention gap;
- parent completion with active child;
- parent-to-child context/Skill/authority inheritance not explicitly admitted;
- Capability result automatically entering context;
- Hook access to runtime internals or illegal result replacement;
- runtime-created model/tool loop or conversation-specific entity; and
- alias, fallback, translator, or mixed artifact acceptance.
