---
status: accepted
date: 2026-07-16
decision: D-001E
owner: APXM agents
---

# Agent Programs compose through `new` and `invoke`

## Context

Every APXM program must be reusable from another APXM program without exposing
runtime placement, process, transport, filesystem, or deployment choices to
authoring code. The current implementation offers overlapping mechanisms such
as `FLOW_CALL`, `WORKFLOW_SPAWN`, `SPAWN_AGENT`, `COMMUNICATE`, `DELEGATE`, and
`HANDOFF`. Those mechanisms disagree about isolation, state, authority,
failure, output shape, and child lifecycle.

The target also needs one explicit distinction that the current vocabulary
lacks:

- a one-shot invocation whose local state ends with that invocation; and
- a stateful Program Instance that can be invoked repeatedly and retain only
  the state that its program explicitly commits.

Python and TypeScript are equivalent authoring frontends. They must present the
same concepts and lower them to the same typed `FrontendGraph`, AIR, artifact,
and runtime semantics.

## Decision

APXM exposes exactly three source-level composition forms:

```text
program.new(initial_context?) -> ProgramInstance
program.invoke(input)         -> T
instance.invoke(input)        -> T
```

Both `invoke` forms are suspending calls: Python uses `await` and TypeScript
uses `await`/`Promise<T>`. `new` is a source-level constructor expression in
the compiled frontend; its returned handle has a data dependency on replay-safe
runtime creation and is not a network/client future. Invocation returns the
program's declared output `T`, not a runtime metadata wrapper. Runtime metadata
is available separately through typed execution evidence.

### Canonical nouns

- **Agent Program** is frontend source defining agent or workflow behavior.
- **Program Instance** is the stateful logical realization created by
  `program.new`.
- **Program Invocation** is one admitted execution or resumption of an Agent
  Program.
- **Runtime Instance** remains the embedded Rust runtime kernel configured by
  an application. It is never a Program Instance.
- **ACP Agent Process** is an external process or protocol peer. It is not an
  Agent Program composition primitive.

These nouns are not interchangeable. In particular, `new` does not compile,
publish, deploy, clone an Agent Definition, start a process, or resolve a
mutable “latest” target.

### `program.invoke`

`program.invoke(input)` performs one isolated, attached child Program
Invocation against the exact imported Agent Program artifact:

1. the compiler records a typed, digest-bound program reference and target
   Agent Identity requirement;
2. the runtime admits a child invocation with a distinct invocation identity,
   lineage, context, state, evidence, and output tree;
3. admission binds the child to its independently authenticated Agent Identity
   and attenuated authority; artifact metadata is never identity proof;
4. the child receives only its typed input `I`; any caller context selected for
   the child is ordinary data explicitly placed inside that input;
5. the parent awaits a plain typed result or receives a typed failure; and
6. the ephemeral child state is discarded after the invocation completes.

Plain `invoke` is call-and-await. Concurrency is expressed only through the
frontend's language-neutral structured task scope, which lowers to structural
parallel regions. Dropping a structured task handle never detaches its child;
the scope joins every child, and an uncaught child failure fails the scope. Raw
Python coroutine and TypeScript Promise start/drop behavior is not part of the
contract and the compiler rejects composition outside direct `await` or that
structured scope. The parent invocation cannot commit successfully while an
attached child is unresolved. Parent cancellation or timeout requests
cancellation of every active attached child. Runtime ownership epochs,
heartbeats/leases, and bounded cancellation deadlines prevent a lost child
executor from blocking its owner forever: expiry records cancellation as
unconfirmed, fences late commits, preserves unknown-effect evidence, and never
allows the owner to report success.

### `program.new`

`program.new(initial_context?)` creates a stateful, attached Program
Instance of the exact imported Agent Program artifact.

- `initial_context` is typed program data. It is not a grant, credential,
  Skill injection, runtime path, or placement request.
- omitting it uses the program's deterministic declared default; a program
  without a default is stateful-construction-only and cannot use one-shot
  `program.invoke`;
- Per-instance configuration is ordinary typed data inside `initial_context`;
  `new` does not introduce a second configuration/state model.
- the returned `ProgramInstance` is an opaque typed handle; authoring code
  cannot inspect runtime internals or use the handle as authority;
- creation is replay-safe and idempotent at a logical call occurrence; a
  runtime replay resolves the same instance rather than creating a duplicate;
  and
- a Program Instance owns its explicitly committed Program Context across
  successful `instance.invoke` calls. Invocation-local variables remain
  ephemeral unless source explicitly places their data in that context.

A Program Instance created by a parent belongs to that parent Program
Instance. It may survive multiple invocations of the parent, but it cannot
outlive the owning parent instance or become detached. APXM Studio and the APXM
root invocation API create top-level published instances; authored
`program.new` never creates an independently managed deployment.

If `new` executes inside one-shot `program.invoke`, its owner is that call's
ephemeral instance. The child may be used during the call but closes when the
ephemeral owner returns or yields; its handle cannot escape in output. Closing
or completing an owner closes all owned descendants; cancelling it cancels
them. A stateful owner yield retains ready children. Recovery reconstructs
these rules from durable ownership edges rather than leaving an orphan.

Each authored execution of `new` is a new logical creation occurrence. Replay
of that same occurrence resolves the same child; a later loop visit or parent
invocation is a different occurrence. To reuse a child across parent
invocations, source must retain its opaque reference in the owner's explicit
Program Context. The reference cannot be exported, forged, or transferred to a
different owner.

### `instance.invoke`

`instance.invoke(input)` resumes the named Program Instance and executes one
typed invocation step. On the first invocation, `input: I` is the entrypoint
input. After a program `yield`, the compiler persists the typed continuation
(program counter and live loop values) and the next `input: I` becomes the
resume argument of that yield's successor region.

- The instance begins from its last committed context/state.
- The program explicitly updates `agent.context`; invocation-local variables
  are not persistent state.
- A successful program `yield` atomically commits output, next Program Context,
  and the compiler-owned continuation, then leaves the instance ready.
- A successful program `return` atomically commits its final output and marks
  the instance completed; later invocation fails with
  `ProgramInstanceCompleted`.
- Failure or cancellation before that boundary preserves the last committed
  state and records the failed occurrence separately.
- A Program Instance is **single-flight, fail-busy**. Concurrent calls and
  recursive `instance.invoke` against the currently executing same instance
  fail with `ProgramInstanceBusy`; queueing belongs outside the instance.
- A missing, completed, closed, cancelled, wrong-company, wrong-artifact, or
  incompatible instance fails explicitly. `instance.invoke` never creates an
  instance.

An instance may yield many times over its lifetime. A Conversational Agent is
the frontend pattern that uses those repeated invocations as authored loop
iterations. The runtime still executes only generic program state, regions,
and invocation boundaries.

For one-shot `program.invoke`, either program `return` or the first program
`yield` produces the plain output and then closes the ephemeral instance; a
yield continuation is deliberately discarded. `await.event` is different: it
parks the current invocation and produces no program output or commit boundary.

### Isolation and explicit projection

The caller and callee do not share local variables, Program Context, mutable
objects, Skills, or output folders.

- Input and output cross the declared typed program signature.
- Any context projection is authored explicitly as part of typed input `I`;
  AIR has no hidden context-projection operand.
- The callee's output is ordinary data until the caller explicitly adds it to
  its own context.
- A callee discovers its own Skills through its admitted Skill Discovery
  Capabilities. Parent Skills are not injected or inherited.
- A specialist may have Skills the parent does not have because Skills are
  knowledge context, not authority.
- A Program Instance handle grants no action. Every invocation still requires
  an admitted complete Capability Grant; child authority can only preserve or
  attenuate the caller's effective authority.

### Placement and external agents

The same source and artifact run in-process, in a worker, or in any canonical
v1 admitted execution profile without changing the authoring API. Placement is
an application/admission concern and is recorded
in evidence, not in AIR attributes supplied by the program.

An external non-APXM agent is integrated as a Capability, Integration, Host,
or ACP process adapter. It does not become a `ProgramInstance`, and APXM does
not preserve `SPAWN_AGENT` or `COMMUNICATE` as a second composition model.

Canonical v1 permits dynamic choice only among a finite set of statically imported
typed `ProgramRef`s. A model or Capability may return an enum/key, and ordinary
source control flow selects one of those exact references. Runtime creation of
an arbitrary `ProgramRef` is rejected. There is no `HANDOFF` opcode,
string-target dispatch, or model-granted authority.

## Frontend shape

The following examples specify the target semantics; they are not a claim that
the current packages already implement the API.

```python
researcher = Researcher.new(
    initial_context=ResearchContext(topic="AIR"),
)

first = await researcher.invoke(ResearchRequest(question="What exists?"))
second = await researcher.invoke(ResearchRequest(question="What changed?"))
summary = await Summarizer.invoke(SummaryRequest(items=[first, second]))
```

```typescript
const researcher = Researcher.new({
  initialContext: { topic: "AIR" },
});

const first = await researcher.invoke({ question: "What exists?" });
const second = await researcher.invoke({ question: "What changed?" });
const summary = await Summarizer.invoke({ items: [first, second] });
```

Ordinary functions remain ordinary functions. Same-program code reuse lowers
to internal function/subgraph calls and never creates a Program Invocation.

## Failure, retry, and evidence

- Runtime retry of the same invocation occurrence retains one
  `node_execution_id` and records nested attempts.
- A retry written by the Agent Program creates a new NodeExecution occurrence.
- Replay derives child and instance identity from the parent identity, static
  callsite, and logical occurrence; replay cannot duplicate children.
- No retry is implicit for a Capability effect, program invocation, or Hook.
- Each Program Invocation has its own Session Output subtree and generic
  NodeExecution evidence.
- The public return remains `T`; status, attempts, cost, timestamps,
  provenance, source mapping, model/capability evidence, and placement remain
  in the execution evidence contract.

## Alternatives considered

### Keep separate flow, workflow, spawn, communicate, delegate, and handoff APIs

Rejected. They encode placement and coordination policy in the graph, produce
incompatible output/state shapes, and force authors to choose runtime
mechanisms instead of program semantics.

### Make `program.new` a pure immutable binding

Rejected. It would not satisfy the confirmed need for repeated invocation with
private state and would leave stateful composition to another API.

### Let authored instances detach or outlive their parent

Rejected for the target contract. It breaks structured concurrency and turns
source composition into deployment management. Top-level durable instance
ownership belongs to the root execution/control API.

### Return a runtime result envelope from every invocation

Rejected. It pollutes program types with operational metadata. Programs return
their declared `T`; tools and Studio query execution evidence separately.

## Consequences

- Python and TypeScript gain one equivalent program composition surface.
- The compiler and AIR must preserve typed `ProgramRef` versus
  `ProgramInstanceRef`; untyped target strings are invalid.
- Runtime placement and protocols become invisible to Agent Program source.
- `FLOW_CALL`, `WORKFLOW_SPAWN`, `SPAWN_AGENT`, `COMMUNICATE`, `HANDOFF`, and
  `DELEGATE` cease to be supported Agent Program composition semantics.
- The exact target AIR constitution is fixed by
  [ADR-0009](0009-air-has-five-public-semantic-operations.md).
- Delivery is governed by the
  [Agent Program composition and AIR full-replacement plan](../agents/agent-program-composition-and-air-full-replacement-plan.md).
