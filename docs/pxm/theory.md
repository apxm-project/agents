# Program Execution Model theory

- Status: normative APXM v1 theory
- Owner: APXM `agents`
- Binding decisions: [ADR-0008](../adr/0008-agent-programs-compose-through-new-and-invoke.md), [ADR-0009](../adr/0009-air-has-five-public-semantic-operations.md), [ADR-0010](../adr/0010-agent-program-source-owns-context-hooks-and-conversational-loops.md), [ADR-0011](../adr/0011-agent-program-execution-is-one-end-to-end-spine.md), [ADR-0013](../adr/0013-core-semantics-are-closed-and-implementations-enter-through-exact-port-bindings.md), and [ADR-0015](../adr/0015-source-first-agent-frontend-vocabulary.md)

## 1. Thesis

The APXM Program Execution Model (PXM) connects authored intent, compiled
meaning, admitted authority, runtime effects, durable execution, and evidence
without allowing any layer to invent behavior.

An Agent Program is ordinary Python or TypeScript source that defines the
behavior of an agent or workflow through an APXM authoring frontend. The source
records one equivalent FrontendGraph. Rust verifies and lowers that graph to
AIR containing the closed five-operation effect/composition family and the
closed structural family, then produces an immutable executable artifact.
The host admits the artifact under current identity, authority, budget, profile,
and event facts. The runtime executes the admitted artifact through exact
injected adapters. Evidence reports what actually happened.

The central rule is:

> Source owns behavior; the compiler owns meaning; the admission authority owns
> identity and spend; the host owns delivery; runtime owns faithful execution;
> evidence owns historical fact; presentation is a projection.

No projection, manifest, adapter, backend, registry, transport, or product UI
is permitted to become a competing behavior source.

## 2. Layered sources of truth

“Single source of truth” does not mean one database or one service owns every
fact. It means each kind of truth has exactly one canonical owner and every
consumer follows the same digest-linked chain.

| Truth | Sole authority | What it proves | What it cannot do |
| --- | --- | --- | --- |
| behavior | Agent Program source | authored control flow, loops, context movement, Hooks, composition, model and Capability calls | grant authority or claim execution |
| portable compiler input | FrontendGraph | deterministic language-neutral recording of the source-visible APXM constructs | redefine source behavior or execute |
| semantic legality | Rust compiler/verifier over AIS-owned operation families | types, effects, closed structural operations, source maps, artifact requirements | choose runtime providers or change source control flow |
| publication | immutable artifact digest | the exact admitted executable meaning and requirements | imply current authorization or successful execution |
| authority | Auth facts and signed decisions | who may act, as which Agent Identity, on which resource, within which ceiling | select program behavior, models, or retries |
| root admission and spend | admission authority | whether an Invocation may begin/resume; reservation and budget facts | rewrite AIR or perform hidden effects |
| durable delivery | OS journal | accepted commands/events, ordering, deduplication, retry, DLQ and replay lineage | grant authority or claim Program success |
| execution | runtime lifecycle | faithful node/region/Invocation state transitions over one admitted artifact | invent a conversation, loop, model, Tool cycle, fallback, or provider |
| external effects | exact admitted adapter | protocol-specific request, response, reconciliation and outcome facts | widen grants, choose an undeclared target, or become the ledger |
| history | runtime/owner evidence | what was attempted, committed, denied, cancelled, failed, or became unknown | retroactively change behavior or authority |
| presentation | host projection | explain authoritative facts | become an execution engine or private owner store |

The chain is content-addressed:

```text
source revision
  -> FrontendGraph digest
  -> compiler + contract digest
  -> AIR/artifact digest
  -> Compatibility Set + Runtime Profile
  -> admission/authority/budget decisions
  -> Program Invocation and NodeExecution evidence
  -> host projection
```

Breaking any link fails closed. Display names, mutable aliases, local config,
ambient environment, UI state, or inferred defaults never repair a missing
link.

## 3. Semantic object model

### 3.1 Agent Program and Program Instance

An **Agent Program** is the authored definition. It may be one-shot or
stateful. A **Program Instance** is a stateful realization created explicitly
with `program.new`. It has an exact artifact, Agent Identity, authority ceiling,
state, continuation, and single-flight lifecycle.

`program.invoke` invokes a program directly. `instance.invoke` invokes an
existing stateful instance. Both produce one admitted **Program Invocation**.
An Invocation may complete, yield, wait, fail, cancel, or become outcome
unknown according to its declared effects. Queueing is external to the
single-flight instance.

Return values are plain typed program results. Runtime metadata, cost, trace,
and source maps are queried through evidence APIs; they are not smuggled into a
generic result envelope.

### 3.2 Two closed AIS operation families

The effect/composition family exposes exactly:

1. `model.call`
2. `capability.invoke`
3. `program.new`
4. `program.invoke`
5. `await.event`

The separate structural family contains compiler-emitted functions, regions,
typed values/blocks, branches, loops including `ais.loop`, task scopes,
try/catch, yield, return, and region exits. Both families are AIS-owned,
generated, and closed for an exact contract digest. Structural operations are
not a public frontend operation API, and `ais.loop` is not a sixth
effect/composition operation. A frontend cannot publish raw AIR or create an ad
hoc operation. External agents, integrations, storage, Tools, and provider
actions are typed Capabilities, not new operation families.

### 3.3 Composition

Agent Programs compose through typed program references. Source imports a
finite `ProgramRef`, creates or invokes it, and handles its typed result.
Organizational hierarchy does not imply invocation. A parent Agent may invoke
a specialist whose Skill associations differ, but the child runs as its own
Agent Identity under an explicitly attenuated grant. A parent cannot transfer
knowledge, authority, context, or Skills merely by spawning a child.

There is no generic runtime Agent Router. Source may choose among statically
known Program references using ordinary control flow. A Capability may help
source make that decision, but it returns a closed typed value rather than an
arbitrary executable name.

## 4. Context, Hooks, Skills, and Capabilities

### 4.1 Context is a local accumulated value

Program Context is an explicit typed local value. Agent Program source reads
and replaces `agent.context` as work progresses. The compiler threads the value
through structured regions and makes every model-visible projection explicit.
The runtime does not append messages, inject memory, trim history, load Skills,
or interpret a Hook result as hidden context.

Context may contain ordinary portable data and typed opaque references whose
contracts permit retention. It may not contain plaintext secrets, authority,
live process snapshots, ambient handles, or unverifiable mutable registries.

### 4.2 Hooks are callbacks

A Hook is a typed callback function attached statically to a Program, loop,
node, or Capability boundary. The callback receives a scoped Agent Facade from
which it can inspect permitted context and execution facts. It changes context
only through an explicit assignment such as `agent.context = next_context`.
Before/after bindings compile into structured regions and a private
`HookEffect<C,T>` ABI; no mutable runtime Hook registry or name lookup exists.

Hooks can prepare or reduce model context, validate data, redact, measure, or
transform an allowed result. They cannot widen authority, suppress immutable
evidence, silently retry an effect, or choose a fallback target.

### 4.3 Skills teach; Capabilities act

A Skill is model context that teaches how to perform work. A Capability is an
authorized executable operation. Association with a Skill means the Skill is
eligible for discovery; it does not insert the Skill body and does not grant a
Capability.

The initial model context contains only the Skill discovery Capability contract
and the minimal instruction that discovery is available. The model searches
metadata, explicitly loads selected Skill content, and the Agent Program
chooses whether that content enters a model-visible context. Company,
Area/Department, Group, and Agent associations form the discoverable catalogue;
Auth and policy filter the effective set for the current principal and Agent.

Capabilities also have static Hooks, typed requests/results, effect classes,
exact adapter requirements, grants, approvals, budgets, and evidence. Tool is a
model-facing projection of selected Capabilities, not a separate authority.

## 5. Generic loops and conversational examples

Loops are ordinary authored structured control flow. The compiler emits
AIS-owned structural operations, including `ais.loop`, and source maps identify
the static loop without a conversation-specific annotation.

When a loop body and its back-edge commit atomically, runtime emits one
`LoopIterationCompleted` fact containing the static loop id, dynamic occurrence
id, iteration index, and causal execution identities. Failed, cancelled, or
rolled-back bodies emit no completion fact. Hosts may project this generic
evidence without defining a core `Turn` model.

The repository's Python and TypeScript conversational examples may define an
example-local `ConversationalAgent` and describe a completed iteration as a
“turn.” Gao is a Studio-owned ordinary Agent Program that may use the same
generic loop pattern and admitted Host Capabilities. Neither example name is
an installable frontend API, contract, compiler/runtime branch, route, or
admission identity.

The model's returned content may include a provider-supported reasoning or
thinking field. A host may display that attributed output when the provider
made it available and policy permits. APXM never requests, reconstructs, or
fabricates private chain-of-thought.

Those programs use only the public generic Agent Program, Hook, Context,
composition, structured-control-flow, source-map, and compiler-bridge APIs.

## 6. Models and future routing

V1 source selects one immutable content-addressed `ModelTargetRef` for each
`model.call`. The portable target fixes exact model/checkpoint/configuration and
deployment-eligibility requirements. The host binds it once to one admitted
deployment before dispatch; runtime invokes only that immutable binding. An unavailable or ineligible model is a
typed failure; there is no alias, default, first-available selection, or
post-send fallback.

APXM-owned model routing is future work. If added, it must be an explicit source opt-in
to an immutable policy whose finite candidate set, hard constraints, objective,
budget facts, evaluation claim, selected binding, and rejected candidates are
evidenced. The admission authority owns the pre-dispatch decision; runtime still receives
one immutable exact binding. Research systems such as Lemonade and RouteLLM are
precedents only, never dependencies or semantic owners.

## 7. External agents and ACP

An External Agent is a non-APXM agent loop. APXM v1 acts as an outbound ACP
Client through an exact admitted adapter. Agent Program source invokes typed
external-agent session Capabilities, each of which lowers to
`capability.invoke`. ACP adds no AIR operation and the ACP process is not a
Program Instance, APXM structural loop, model backend, or runtime.

The external agent's prompt turn may contain its own model calls, Tool cycles,
plans, diffs, and subprocess work. APXM records those as nested attributed ACP
events under the outer Capability NodeExecution, not as fabricated APXM nodes
or loop iterations. Peer-reported usage remains external evidence and cannot overwrite
Server's spend ledger.

An `ExternalAgentSessionRef` is an opaque scoped reference bound to one Program
Instance/root lineage (or one one-shot invocation), Acting Principal, invoking
Agent Identity, exact profile and grant lease. The same instance may keep it in
typed Program Context while valid; it cannot cross owners and carries no
authority. ACP client-directed filesystem, terminal and permission requests
are separately authorized APXM Capability effects. MCP uses a distinct
attenuated APXM MCP/Capability gateway, and agent-native tools remain inside
the confined outer effect. ACP feature negotiation is protocol negotiation,
not APXM Capability authorization.

Claude, Codex, or another agent is supported only through an exact adapter and
profile that passed APXM's pinned ACP conformance, supply-chain, sandbox,
authority, lifecycle, and evidence matrix. “Any ACP agent” never means an
arbitrary command.

## 8. Integrations and durable events

A business-provider Integration is an admitted composition of an Integration
Adapter, Auth connection and verification methods, OS ingress/delivery facts,
Server admission, Capability bindings, and Studio projections. It is not a
Plugin, Skill, ACP transport, or runtime extension.

Inbound events become accepted only after exact-byte verification,
normalization into a closed event type, and durable OS commit. HTTP acceptance
does not mean Program success. Outbound actions are ordinary
`capability.invoke` effects with stable identities, exact connections, and
typed committed/failed/outcome-unknown evidence.

## 9. Evidence is part of semantics

For every occurrence APXM preserves identifiers and causal links across:

```text
Program Instance
  -> Program Invocation
    -> structural loop occurrence / committed iteration
      -> NodeExecution
        -> attempt
          -> authority and budget decision
          -> exact adapter/model/profile binding
          -> request/effect id
          -> output, usage, event and file references
          -> terminal outcome
```

Structural evidence is monotonic. Content is separately classified, encrypted,
retained, redacted, and purpose-gated. Session Output uses one folder per actual
NodeExecution occurrence; administrators may inspect content only through an
authorized, audited request. A transport receipt, model response, external
agent update, webhook acknowledgment, or broker acknowledgment never implies a
different layer's success.

## 10. Design test

A proposal is compatible with PXM only if all answers are explicit:

1. Which source construct authors the behavior?
2. Which compiler rule gives it meaning?
3. Which of the five effect/composition operations represents each effect, and
   which closed structural operations represent authored control flow?
4. Which exact owner admits identity, authority, budget, delivery, and target?
5. Which adapter executes the effect, with what stable identity?
6. What happens on cancellation, crash, retry, duplicate, and uncertain send?
7. Which evidence proves the result without trusting a projection?
8. Can Studio explain it without owning private runtime truth?
9. Do Python and TypeScript compile equivalent golden vectors?
10. Does removal of every prototype alias/fallback leave one executable path?

If any answer depends on runtime inference, a mutable name, ambient state, a
second source of truth, or an unevidenced fallback, the proposal is not APXM
PXM.
