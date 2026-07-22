# Build the conversational repository example

- Status: canonical target guide
- Decisions: [ADR-0010](../adr/0010-agent-program-source-owns-context-hooks-and-conversational-loops.md), [ADR-0014](../adr/0014-conversational-agent-and-gao-are-examples-over-generic-agent-program-apis.md)
- Contract: [composition and AIR §10](../agents/agent-program-composition-and-air-contract.md#10-generic-loop-iteration-contract)

## 1. Mental model

The repository's conversational Agent is an example built with generic Python
or TypeScript `AgentProgram` APIs. The example may define a local
`ConversationalAgent` helper for its own source organization, but that name is
not exported by an APXM package and has no compiler, runtime, Server, contract,
or Studio meaning.

Context is a typed local accumulated value. Each iteration explicitly decides
what the model sees, which Skills are discovered/loaded, which Capabilities are
offered, how outputs update context, and whether to continue, yield or return.
Rust compiles the ordinary source loop to structural `ais.loop`; the five
effect/composition operations remain unchanged.

## 2. Contract-shaped source

```python
from apxm_program import AgentProgram


@AgentProgram(context=SupportContext)
async def support(agent, incoming: UserMessage):
    while not agent.context.done:
        available = await skills.search(
            query=agent.context.current_need,
            limit=5,
        )
        selected = choose_relevant_skills(available)
        skill_context = await skills.load(selected)

        response = await model.call(
            model=ExactSupportModel,
            messages=build_messages(agent.context, incoming, skill_context),
            tools=[crm.lookup, tickets.create],
        )

        agent.context = reduce_context(agent.context, response)
        if agent.context.needs_user:
            incoming = await agent.yield_(AgentReply(response.content))

    return FinalAnswer.from_context(agent.context)
```

`agent.yield_(output)` is a compiler-recognized structural frontend primitive,
not a sixth AIR operation. It commits the already assigned `agent.context`,
returns the plain output to the caller, and binds the next typed invocation
input when the same Program Instance resumes.

```typescript
import { agentProgram } from "@apxm/frontend";

const support = agentProgram(
  { context: SupportContext },
  async (
    agent: Agent<SupportContext>,
    incoming: UserMessage,
  ): Promise<FinalAnswer> => {
    while (!agent.context.done) {
      const available = await skills.search({
        query: agent.context.currentNeed,
        limit: 5,
      });
      const skillContext = await skills.load(chooseRelevantSkills(available));
      const response = await model.call({
        model: ExactSupportModel,
        messages: buildMessages(agent.context, incoming, skillContext),
        tools: [crm.lookup, tickets.create],
      });
      agent.context = reduceContext(agent.context, response);
      if (agent.context.needsUser) {
        incoming = await agent.yield_(new AgentReply(response.content));
      }
    }
    return FinalAnswer.fromContext(agent.context);
  },
);
```

Python and TypeScript goldens must compile to equivalent structured regions,
resume-input bindings, `ais.loop`, and the same five-operation
effect/composition family.

The snippets fix the semantic shape, not final decorator or generic spelling.
Packed-package declarations and clean-consumer tests become syntax authority
when the frontend implementation lane lands.

## 3. Hooks as callbacks

Hooks are normal typed callbacks attached statically:

```python
async def before_model(agent: Agent[SupportContext], request: ModelRequest):
    agent.context = redact_context(agent.context)


async def after_model(
    agent: Agent[SupportContext],
    response: ModelResponse,
) -> ModelResponse:
    agent.context = record_summary(agent.context, response)
    return response
```

A Program, loop, node or Capability can bind its own before/after callbacks.
The Agent Facade exposes only facts allowed at that boundary. Error recovery is
ordinary source try/catch, not a magical error Hook or runtime retry.

Hooks cannot grant authority, hide evidence, mutate an undeclared global,
select a fallback model, bulk-load Skills or perform an unrecorded effect.

## 4. Skills without context bloat

Associating Skills with the Agent, Company, Area/Department or Group only makes
them discoverable. The initial prompt does not contain their bodies. It contains
the minimal Skill-discovery Capability contract. The model or source searches,
loads a small relevant set, and source explicitly projects selected content into
the next model call.

Discovery is filtered by both association and the current principal/Agent
policy. Knowing how to do something does not authorize the Capability that can
do it.

## 5. Generic iteration evidence

Compiler source maps identify the static loop without a conversational
annotation. Runtime emits `LoopIterationCompleted` only when the body and
back-edge commit atomically. The fact carries the static loop id, dynamic
occurrence id, zero-based iteration index, and causal execution ids. A failed
or rolled-back body emits no completion.

Studio shows generic loop iterations and the nodes within them. Example-local
copy may call one completed conversational iteration a “turn,” but core Studio
has no `Turn` model.

If a model returns an attributed reasoning/thinking field, Studio may display
it under provider and policy rules. It never fabricates hidden reasoning.

## 6. Failure rules

- A failed node follows authored try/catch or terminates with a typed failure.
- A model send with uncertain result becomes `ModelOutcomeUnknown`; no second
  model is selected.
- A cancelled or rolled-back loop body emits no `LoopIterationCompleted`.
- Yield preserves the compiler-owned continuation and explicit next context.
- Return completes the Program Instance and returns a plain typed value.
