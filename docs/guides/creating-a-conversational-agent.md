# Build a conversational Agent

- Architectural status: canonical generic Agent Program behavior
- Frontend syntax status: implemented source-first authoring surface aligned
  with [Author an Agent](creating-an-agent-program.md)
- Implementation plan:
  [Source-first Agent frontend master plan](../agents/simple-agent-authoring-frontend-plan.md)
- Decisions:
  [ADR-0010](../adr/0010-agent-program-source-owns-context-hooks-and-conversational-loops.md)
  and
  [ADR-0014](../adr/0014-conversational-agent-and-gao-are-examples-over-generic-agent-program-apis.md)

## 1. Mental model

A conversational Agent is an ordinary `Agent` with a typed `Context`, a source
loop, explicit Model calls, explicit Tool calls, and a yield point. There is no
package-level `ConversationalAgent`, conversation runtime, hidden model/Tool
loop, or core `Turn` type.

The repository may organize its example with a local helper, but that helper
has no compiler, runtime, Server, contract, or Studio meaning.

Python's `@Agent`, `@Context`, local `@Tool`, and `@Hook` markers and
TypeScript's equivalent `Agent(...)`, `Context(...)`, `Tool(...)`, and
`Hook.before(...)` declarations all bind statically into the same semantic
tree. None runs the conversational loop while compiling.

The loop is only the familiar center. The same generic frontend can add typed
Events, before/after Hooks, specialist Agents, structured task groups, Skills
discovery, try/catch, cancellation checks, and exact Model/Capability calls.
That is why the conversational Agent is an example of APXM's power rather than
a separate product abstraction.

```text
readable Agent source
  -> native AST + immutable bound/typed source tree
  -> deterministic FrontendGraph emission
  -> Rust CFG/SSA + AIR/AIS lowering and legal optimization
  -> exact admitted runtime ports
  -> authoritative evidence joined back to source
```

## 2. Python

The executable Python reference is
[`examples/agents/conversational/python/agent.py`](../../examples/agents/conversational/python/agent.py).
Its request and response types define a closed union: the Model returns either
a final reply or a `search_web` request. The Agent owns dispatch and Model
re-entry:

```python
SearchWeb = Tool[SearchWebRequest, SearchWebResult]("cap.search")
SupportModel = Model[ModelRequest, ModelResponse]("model.target.v1")


@Context
class ConversationContext:
    messages: tuple[ConversationMessage, ...] = ()
    tool_calls: int = 0
    last_reply: str = ""


@Agent(
    input="ConversationInput",
    output="ConversationOutput",
    context=ConversationContext,
)
async def ConversationalExample(agent, incoming):
    while incoming["message"] != "":
        response = await SupportModel(
            {"messages": agent.context.messages, "incoming": incoming}
        )

        while response["kind"] == "tool_request":
            if response["tool_request"]["kind"] == "search_web":
                tool_result = await SearchWeb(response["tool_request"]["arguments"])
                response = await SupportModel(
                    {
                        "messages": agent.context.messages,
                        "incoming": incoming,
                        "tool_result": tool_result,
                    }
                )
            else:
                raise ValueError("undeclared tool request")

        if response["kind"] != "final":
            raise ValueError("undeclared model response")

        agent.context = ConversationContext(
            messages=agent.context.messages,
            tool_calls=agent.context.tool_calls,
            last_reply=response["reply"]["message"],
        )
        incoming = await agent.yield_(response["reply"])
```

If the first response is final, the inner loop performs zero iterations and
`SearchWeb` is not invoked. A Tool response is bound directly into the next
authored Model request together with the current input and persistent Context.

## 3. TypeScript

The equivalent executable source is
[`examples/agents/conversational/src/conversational-agent.ts`](../../examples/agents/conversational/src/conversational-agent.ts).

```typescript
import { Agent, Context, Hook, Model, Tool } from "@apxm/frontend";

const SearchWeb = Tool<SearchWebRequest, SearchWebResult>("cap.search");
const SupportModel = Model<ModelRequest, ModelResponse>("model.target.v1");

export const ConversationalExample = Agent<
  ConversationInput,
  ConversationOutput,
  ConversationState
>({
  name: "ConversationalExample",
  context: ConversationContext,
  use: { SearchWeb, SupportModel },
  async run(agent, incoming) {
    while (incoming.message !== "") {
      let response = await SupportModel({
        messages: agent.context.messages,
        incoming,
      });

      while (response.kind === "tool_request") {
        if (response.tool_request.kind === "search_web") {
          const toolResult = await SearchWeb(response.tool_request.arguments);
          response = await SupportModel({
            messages: agent.context.messages,
            incoming,
            tool_result: toolResult,
          });
        } else {
          throw new Error("undeclared tool request");
        }
      }

      if (response.kind !== "final") {
        throw new Error("undeclared model response");
      }

      agent.context = {
        messages: agent.context.messages,
        tool_calls: agent.context.tool_calls,
        last_reply: response.reply.message,
      };
      incoming = await agent.yield_(response.reply);
    }
  },
});
```

Python and TypeScript goldens must capture equivalent typed loop intent, Tool
dispatch, lossless Model-request value expressions, context flow, and
yield/resume bindings. The re-entry request directly projects and embeds the
Tool-result SSA value;
Hooks cannot supply or mask that authored data edge. Rust must then select the
same structural `ais.loop` and closed effect/composition operations for both.
The paired executable sources also bind deterministic before/after Hooks around
`SearchWeb`: the before Hook applies a persisted context-window policy and the
after Hook records Tool-call accounting without moving dispatch or message
transitions out of the Agent body.

## 4. Context is explicit

Each iteration explicitly decides:

- what prior messages remain in `agent.context`;
- which Skills are searched and read;
- which selected Skill content reaches the Model;
- which Tool schemas are offered;
- which Tool requests are executed;
- how Model and Tool results update Context; and
- whether to yield, return, or continue.

Context carries information, never grants. Associating Skills with an Agent,
Company, Area/Department, or Group makes them discoverable; it does not inject
their bodies. Tool calls still require complete Auth-owned Capability Grants.

At execution, every Model request carries the sealed Context-envelope reference
and the digest of the exact persistent Context used for that call, plus a
lossless materialization of the exact authored request expression. Both
are part of request identity. Context does not substitute for request data: a
Tool result reaches Model re-entry through its authored SSA expression even
when before/after Hooks leave Context unchanged.

## 5. Why there is no `AgentLoop`

The Python and TypeScript `while` statements are the Agent loops. The frontend
captures typed loop/CFG intent and Rust emits `ais.loop`. A separate
`AgentLoop` import would expose lowering vocabulary and make ordinary control
flow harder to read.

`agent.yield_(output)` is the one explicit stateful boundary. It commits the
already assigned `agent.context`, returns the plain output, and binds the next
typed invocation input to the yield's exact resume SSA value when the Program
Instance resumes. Resumption preserves that committed Context; the delivered
input does not replace it. It is compiler-
recognized structural syntax, not a sixth effect operation.

## 6. Events

A Tool or Capability can return a typed `Event[T]`. Waiting on it parks the
same Program Invocation; it does not yield a reply, start another loop, or ask
the frontend runtime to poll.

The Python spelling is:

```python
from apxm_program import Event

RequestApproval = Tool[ApprovalRequest, Event[Approval]](
    "capability.request-approval.v1"
)

pending = await RequestApproval(ApprovalRequest(change=change))
approval = await pending.wait(timeout=ApprovalTimeout)
if approval.decision is Decision.DENIED:
    return AgentReply("The requested change was not approved.")
```

The equivalent TypeScript spelling is:

```typescript
import { Event } from "@apxm/frontend";

const RequestApproval = Tool<ApprovalRequest, Event<Approval>>(
  "capability.request-approval.v1",
);

const pending = await RequestApproval({ change });
const approval = await pending.wait({ timeout: ApprovalTimeout });
if (approval.decision === Decision.Denied) {
  return { content: "The requested change was not approved." };
}
```

The Tool call lowers through `capability.invoke`; `.wait(...)` lowers through
`await.event`. Event creation, fulfillment, timeout, cancellation, and replay
remain typed durable contracts rather than conversation-specific behavior.

## 7. Hooks

Hooks are statically bound typed callbacks. The callback parameter is still
the inferred `agent`; ordinary source does not import `AgentFacade`. Context is
updated by assigning `agent.context`, and error recovery remains authored
try/catch rather than a special error Hook.

The focused Hook spelling binds a static source target. Python names the target
and closed scope; TypeScript names both the Agent and target in the declaration:

```python
from apxm_program import Hook

@Hook.before(target="SupportModel", scope="model")
async def AddCurrentPolicy(agent):
    agent.context = agent.context.with_policy(current_policy(agent.identity))
```

```typescript
import { Hook } from "@apxm/frontend";

export const AddCurrentPolicy = Hook.before({
  agent: Support,
  target: SupportModel,
  async run(agent) {
    agent.context = withPolicy(
      agent.context,
      currentPolicy(agent.identity),
    );
  },
});
```

The binding is static, order is deterministic, and Context changes are explicit
values. Runtime invokes each selected handler before or after its exact target;
the handler cannot redirect dispatch to a different Model or Capability.

Hooks cannot grant authority, hide evidence, mutate undeclared global state,
select a fallback model, bulk-load Skills, or perform an unrecorded effect.

## 8. Specialists and structured work

Conversation does not change Agent composition. A loop may invoke one exact
specialist or run independent attached specialists in a `TaskGroup`; every
child is typed, joined, independently admitted, and visible in evidence.

```python
review = await SecurityReviewer.invoke(ReviewRequest(change=change))
```

```typescript
const review = await SecurityReviewer.invoke({ change });
```

There is no implicit handoff of Context, Skills, grants, identity, or model
history. The parent supplies typed input and explicitly uses the plain typed
result.

## 9. Exact model execution, including APXM-vLLM

Agent source declares an exact portable `ModelTargetRef`; it does not import a
provider SDK or name an endpoint. Admission binds that target to one exact
deployment and inference-port implementation. If the admitted implementation
is APXM-vLLM, one source Model call follows this path:

```text
SupportModel(request)
  -> model.call / ais.model_call
  -> Model NodeExecution and stable effect/request identity
  -> exact admitted APXM-vLLM adapter
  -> response or stream + provider-native usage
  -> authoritative execution commit and evidence
```

The adapter may translate backend-neutral compiler/runtime analysis into
supported vLLM graph, prefix-cache, or priority hints. It cannot change the
request meaning, execute Tool requests, call the Model again, substitute a
different deployment, or mutate Context. Tool requests return as typed data to
the authored loop shown above.

This separation gives APXM both portability and performance: the Agent remains
provider-neutral, while an exact backend can exploit proven optimization hints
without creating another programming model.

## 10. Full observability

Authors should not add logging calls to reconstruct program semantics. The
compiler and runtime retain the correlation needed to inspect the execution:

| Author concept | Inspectable execution evidence |
| --- | --- |
| Agent source and loop | source span, static region, dynamic loop occurrence, committed iteration |
| Model call | NodeExecution, attempt, exact admitted binding, request/output refs, usage, latency, outcome |
| Tool/Capability call | distinct NodeExecution, grant/effect identity, request/result refs, approval and failure |
| Hook | binding, scope, phase, order, execution, Context before/after, replacement result |
| Event wait | event ref, park, fulfillment/expiry/cancellation, wake and resume |
| Context assignment | typed before/after refs and committed transition |
| specialist invocation | parent/child instance and invocation lineage |
| compiler optimization | input/output artifact identity, pass provenance, preserved source/static-node correlation |

Authoritative Runtime Evidence owns execution history. Traces, streams, vLLM
metrics, logs, and backend cache/scheduler telemetry may enrich inspection but
cannot manufacture a completed iteration or override a committed outcome.
Studio projects the allowed evidence back onto the same generated Python or
TypeScript source; it does not invent conversational semantics.

## 11. Generic iteration evidence

Compiler source maps identify the static source loop without a conversational
annotation. Runtime emits `LoopIterationCompleted` only when the body and
back-edge commit atomically. The fact identifies the static loop, dynamic
occurrence, zero-based iteration, Program Invocation, and causal node
executions. Failed, cancelled, or rolled-back bodies emit no completion fact.

Studio may describe a completed example iteration as a “turn” in example copy,
but APXM core and Studio have no `Turn` product model.

## 12. Failure rules

- A failed node follows authored try/catch or terminates with a typed failure.
- A Model send with uncertain outcome becomes `ModelOutcomeUnknown`; no second
  Model is selected.
- A cancelled or rolled-back loop body emits no `LoopIterationCompleted`.
- Yield preserves the compiler-owned continuation and explicit next Context.
- Return completes the Program Instance and returns a plain typed value.
