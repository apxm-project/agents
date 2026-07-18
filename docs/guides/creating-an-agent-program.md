# Create an Agent Program

- Status: canonical target guide
- Audience: Python and TypeScript authors

## 1. What you are creating

An Agent Program is source that defines the behavior of an agent or workflow
using one APXM authoring frontend. It is not a “program package,” process,
runtime object, model prompt, Studio canvas, or deployment manifest.

Choose Python or TypeScript based on the authoring team. Both record the same
FrontendGraph and must pass the same compiler and runtime conformance vectors.
Neither language executes the Agent Program in production.

## 2. Minimal one-shot program

The intended Python shape is:

```python
from apxm import agent_program, model
from app.models import ExactSupportModel


@agent_program
async def summarize(request: SummaryRequest) -> Summary:
    response = await model.call(
        model=ExactSupportModel,
        messages=[{"role": "user", "content": request.text}],
        response_type=Summary,
    )
    return response
```

The equivalent TypeScript shape is:

```typescript
import { agentProgram, model } from "@apxm/agents";
import { ExactSupportModel } from "./models";

export const summarize = agentProgram<SummaryRequest, Summary>(
  async (request) => {
    return model.call({
      model: ExactSupportModel,
      messages: [{ role: "user", content: request.text }],
      responseType: Summary,
    });
  },
);
```

Exact generated identifiers and decorators/builders are frozen by the frontend
contract lane. The semantic golden is already fixed: one typed input, one exact
model reference, one `model.call`, and one plain typed return.

## 3. Capability effects

Use a typed Capability for executable work:

```python
record = await crm.get_customer(customer_id=request.customer_id)
```

```typescript
const record = await crm.getCustomer({ customerId: request.customerId });
```

The wrapper binds a Capability Definition and lowers to `capability.invoke`.
Source never embeds credentials, provider endpoints, grants, shell commands or
raw Tool JSON. Hooks on the Capability are static callbacks compiled with the
Program; Auth still decides whether the occurrence is allowed.

## 4. Compile and inspect

The authoring flow is:

```text
source
  -> language frontend records FrontendGraph
  -> Rust bridge verifies and compiles
  -> diagnostics + deterministic AIR/artifact
  -> publish draft
  -> Server admission under Runtime Profile and Auth
```

Compilation must prove:

- input/output and local value types;
- structured control flow;
- exact Program, model and Capability references;
- Hook/context effect legality;
- artifact requirements and source maps; and
- equivalence with the other frontend's golden vector.

Inspect the generated FrontendGraph/AIR to learn and debug; do not edit it as a
second behavior source.

## 5. Test before publishing

Every program should include:

- pure source tests for branches and transformations;
- compiler golden for FrontendGraph, AIR and diagnostics;
- contract fakes for model/Capability adapters injected explicitly;
- negative authority, cancellation, budget and size cases;
- evidence assertions for source mapping and terminal outcome; and
- Python/TypeScript parity when the guide or platform ships both examples.

Test fakes cannot become production fallbacks. A missing admitted adapter is a
typed failure.

## 6. Publish and run

An Agent record is either draft or published. Publishing compiles and pins an
immutable artifact digest plus requirements. It does not create a user-visible
version hierarchy. Editing starts another draft and publication atomically
advances the active artifact.

At invocation, Server checks the exact artifact, Compatibility Set, Runtime
Profile, Agent Identity, Acting Principal, grants and budget. Runtime returns
the plain typed result. Query evidence separately for metadata, usage, context,
events, files and node details.

## 7. Rules to remember

- Source owns every loop, Tool cycle, retry and context transition.
- Model selection is exact in v1.
- Skills are discovered and loaded explicitly; none are auto-inserted.
- Program composition uses `program.new`/`program.invoke`/`instance.invoke`.
- Return a typed result, not a home-grown result/metadata envelope.
- Never depend on runtime defaults, aliases, fallback, ambient configuration or
  a Studio-only construct.
