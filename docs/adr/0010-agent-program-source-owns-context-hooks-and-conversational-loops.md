---
status: accepted
date: 2026-07-16
decision: D-001A, D-001B, D-001C
owner: APXM agents
supersedes: ADR-0001, ADR-0005
amends: ADR-0007
amended_by: ADR-0014
---

# Agent Program source owns context, Hooks, and conversational loops

ADR-0014 preserves source-owned behavior, explicit Context, Agent Facade Hooks,
discovery-only Skills, and generic authored loops. It replaces this decision's
package-level `ConversationalAgent`, Gao, conversational source-map, and
Studio `Turn` provisions with repository examples and generic loop-iteration
evidence.

## Context

ADR-0001 modeled a Hook as a callback over a specialized `HookContext` and
event-specific `HookResult`. ADR-0005 made Turn admission, rearm, and terminal
outcome semantics a versioned conversational runtime contract. Subsequent
architecture decisions clarified a simpler target:

- a Hook is an ordinary callback over the current agent facade;
- Program Context is an explicit local variable accumulated by source code;
- Conversational Agent is a Python/TypeScript authoring construct whose loop is
  authored and compiler-lowered; and
- runtime executes generic program operations, regions, invocations, yields,
  and evidence without knowing what a conversation or Turn is.

The earlier decisions are therefore superseded, not amended in place.

## Decision

Frontend Agent Program source is the sole behavioral authority for context
flow, callbacks, loops, model/Capability sequencing, retries, and composition.
The Source Bundle manifest identifies inputs and requirements but contains no
second behavior language.

### Program Context is explicit local state

`agent.context` is the program's typed accumulated local context value.
Authors update it explicitly:

```python
agent.context = agent.context.with_item(customer)
result = await lookup.invoke(customer.id)
agent.context = agent.context.with_item(result)
```

The compiler lowers these assignments to ordinary SSA/region-carried values
and, for a Program Instance, explicit committed instance state. Runtime may
retain snapshots and diffs as evidence, but `ContextSnapshot`, `ContextDelta`,
and `ContextMerge` are not public authoring or AIR operations.

No model result, Capability result, Skill content, child output, Hook return,
or previous invocation state enters Program Context implicitly. Parallel
branches must produce explicit typed values and the program must merge them
with ordinary code before assigning the next context value.

Model Context and Capability Context are explicit least-privilege projections
created for a specific call. Runtime-private Execution Context remains
inaccessible to source code.

### Hooks are callbacks over the agent facade

All Agent, loop, Node, Model, and Capability Hooks use one argument shape:

```text
HookCallback = async (agent: AgentFacade) -> statically_declared_return
```

The callback receives the same portable `agent` facade used by the Agent
Program. It can access the fields valid at that callback point, including
typed identity, `agent.context`, current input/output, current node/model/
Capability view, Skills discovery handles, admitted Capability handles,
budget/deadline/cancellation views, the current invocation reference, and safe
helpers.

The facade is the whole program-visible agent, not a specialized Hook Context.
It is also not the live Python/TypeScript authoring object and not runtime
Execution Context. It contains no plaintext credential, bearer grant,
scheduler, broker, database, mutable graph, ambient filesystem/network, or
method that bypasses model/Capability admission.

Hooks are registered at typed `before` and `after` points for the agent, loop
region, Node, Model, or Capability scope. Error recovery remains ordinary
authored `try`/`catch`; canonical v1 has no separate error-Hook protocol. The callback argument
does not change by scope. A callback updates context through explicit
`agent.context` assignment. Return behavior is fixed by its static binding:
an observer returns `None`; a result-transforming `after` Hook returns exactly
the plain result type `T`. There is no runtime `T | None` ambiguity, public
`HookResult`, allow/deny/defer union, or arbitrary mutation envelope. Denial,
approval, or deferral are ordinary typed program/capability results and control
flow.

The simple mutation-like frontend syntax lowers to a compiler-internal tagged
ABI:

```text
HookEffect<C, T> = { next_context: C, result: Keep | Replace(T) }
```

`Keep` and `Replace` are distinct even when `T` is nullable. This ABI is
structural compiler IR, not a public Hook result object. It makes the next
context an explicit SSA value while preserving the source experience of
assigning `agent.context`.

Execution order is deterministic middleware order: `before` callbacks run
outer scope to inner scope and declaration order; successful `after`
callbacks unwind inner scope to outer scope and declaration order, each seeing
the current possibly replaced result. A failure skips remaining success-only
`after` callbacks and unwinds through ordinary authored error control flow.

Hook bindings are compiled static artifact records. The compiler inserts calls
around their target regions; the runtime does not provide dynamic Hook
registration or a parallel callback engine. Callback failures are ordinary
typed failures visible to authored `try`/`catch` control flow, subject to the
same cancellation, budget, authority, and evidence rules as other code.

Nodes and Capabilities may each declare their own Hooks. Capability execution
still passes through the single authority chokepoint, including calls made by
a Hook.

### Skills are discovered, never injected

A Skill is model context, not authority. No Skill body is inserted
automatically into a Program Invocation, Program Context, loop iteration, or
model request.

An agent associated with Skills receives instructions explaining the admitted
Skill Discovery Capabilities, such as `search_skills` and `read_skill`. The
program/model invokes those Capabilities when needed and explicitly chooses
whether returned Skill data enters the next model context.

Company, area/department, group, and specialist associations constrain what
the discovery Capability can find; they do not bulk-inject content. A
specialist can discover its own Skills even when its parent lacks them. Skills
never create Tools, Capabilities, grants, credentials, or descendant
authority.

### Conversational Agent is authored program structure

`ConversationalAgent` is a Python/TypeScript frontend construct and standard
library pattern. It authors a generic structured loop, context updates, Hooks,
model calls, Capability calls, `program.new`/`program.invoke`, and yield points.
It creates no runtime type, service, mode string, hidden host loop, or
agent-id-specific scheduler.

One successful `instance.invoke(input)` resumes one Program Instance through
the next authored iteration/yield boundary and returns the plain program
output. The instance's explicit context/state then becomes the starting value
for its next invocation. The same instance accepts one active invocation at a
time.

A **Turn** is the Studio projection of one dynamic occurrence of the
Conversational Agent's annotated loop region. It begins when that region
consumes the invocation input and ends when it completes/yields. Input queueing
and client delivery are outside the Turn. Compiler source maps preserve the
static region and semantic annotation; runtime records only generic region and
NodeExecution occurrences.

There is no `TurnOutcome`, `TurnResult`, `apxm.conversational-loop.v1`, or
runtime Turn entity in the target contract. Success/failure/cancellation are
generic Program Invocation states. Business outcomes such as denied or
deferred exist only when the program's declared result type defines them.

### Model and Capability flow

A model can produce tool requests, structured output, text, usage, refusals,
and provider-exposed reasoning summaries. The authored loop decides whether to
invoke a Capability, how to use its result, whether to update context, and
whether to call the model again. Runtime performs no hidden reasoning loop and
never auto-merges tool output into context.

Studio shows one NodeExecution for each actual model call and another for each
actual Capability call. A later model call is a new NodeExecution. Double-click
inspection may show exact admitted request, returned output, provider-exposed
reasoning summary, usage, latency, attempts, Hooks, context before/after, and
output files according to permissions and data policy. Hidden chain of thought
is never claimed or reconstructed.

## Gao

Gao remains an ordinary TypeScript specialization of the standard
Conversational Agent. Gao may add workflow-authoring prompts, discovery
Capabilities, Skills associations, Hooks, and program logic. It adds no direct
loop builder, custom context engine, privileged Studio callback, compiler
branch, or runtime path.

## Replacement

The target Compatibility Set replaces and rejects:

- `apxm.program-context.v1`, `apxm.agent-hook.v1`, and
  `apxm.conversational-loop.v1` as executable target semantics;
- public `HookContext`, `HookResult`, Context Delta/Merge, and automatic Skill
  injection APIs;
- runtime-controlled conversation/rearm loops, graph splicing, string modes,
  and `AUTONOMOUS` cognition loops; and
- manifest-defined Hooks, loop phases, or context behavior.

There are no aliases, adapters, compatibility interpreters, dual callback
engines, or product-side fallbacks.

## Alternatives considered

### Keep the specialized Hook Context and Hook Result contract

Rejected. It creates a second programming model inside callbacks and forces
authors to learn event envelopes instead of using the agent they already
defined.

### Pass the live frontend object or runtime Execution Context

Rejected. The portable Agent Facade preserves the source-level agent API
without exposing language object identity, credentials, scheduler internals,
or ambient authority.

### Let the runtime own a durable conversational lifecycle

Rejected. It makes Turn, rearm, context reconstruction, and model/tool behavior
runtime assumptions instead of program-authored behavior.

### Inject associated Skills at every Turn

Rejected. It bloats model context, obscures provenance, and treats availability
as use. Discovery Capabilities keep context bounded and explicit.

## Consequences

- ADR-0001 and ADR-0005 become historical and superseded.
- The old Hook/Context and Conversational Loop contracts/plans are replaced by
  the canonical composition/AIR contract and full-replacement plan.
- Python and TypeScript must expose the same Agent Facade and callback
  behavior.
- The compiler, artifact, and runtime remain generic; Studio owns Turn and
  inspection projections.
- Implementation follows the
  [Agent Program composition and AIR contract](../agents/agent-program-composition-and-air-contract.md).
