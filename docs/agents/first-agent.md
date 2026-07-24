# Create Your First APXM Agent

An Agent Program is ordinary Python or TypeScript source. The frontend reads
that source statically into `apxm.frontend-graph.v1`; Rust alone validates it,
constructs CFG/SSA and AIR, and produces the artifact admitted by Server.

The public authoring vocabulary is `Agent`, `Context`, `Tool`, `Model`, and
ordinary language control flow. `Capability`, `Event`, `Hook`, and `TaskGroup`
are focused advanced declarations. Graph builders, AIR text, node ids, and
`AgentFacade` are not author APIs.

## Python

```python
from apxm_program import Agent, Model

SummaryModel = Model[object, object]("model.summary.v1")


@Agent(input="SummaryRequest", output="Summary")
async def summarize(agent, request):
    return await SummaryModel(request)
```

## TypeScript

```typescript
import { Agent, Model } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type SummaryRequest = { readonly text: string };
type Summary = { readonly text: string };

const source = staticSource(import.meta.url);
const SummaryModel = Model<SummaryRequest, Summary>("model.summary.v1");

export const Summarizer = Agent<SummaryRequest, Summary>({
  name: "Summarizer",
  source,
  use: { SummaryModel },
  async run(agent, request) {
    return await SummaryModel(request);
  },
});
```

Both forms are semantically equivalent. The callback parameter is inferred;
authors use `agent.context` and `agent.yield_(...)` when their program carries
context across stateful invocations.

The TypeScript compiler needs a static source token. Define it once beside the
Agent source when compiling in Node:

```typescript
// static-source.ts
import { readFileSync } from "node:fs";
import { relative } from "node:path";
import { fileURLToPath } from "node:url";

export function staticSource(url: string) {
  const absoluteFileName = fileURLToPath(url);
  return {
    fileName: relative(process.cwd(), absoluteFileName),
    text: readFileSync(absoluteFileName, "utf8"),
  };
}
```

Keep bindings at module scope, include every binding used by an Agent in its
`use` object, and use exact references such as `model.summary.v1` or
`capability.search.v1`. The frontend rejects dynamic marker lookup and local
shadowing instead of guessing what an Agent means.

## Compile through the owner boundary

Place source and its manifest in an Agent Program Source Bundle, then submit it
through Server compile-admission. Studio follows the same source-first path.
The compiler bridge is explicit and has no local fallback, raw-AIR input, or
frontend runtime mode.

For the complete author journey, including Context, Tools, Hooks, Events,
composition, and yield/resume, read the [authoring guide](../guides/creating-an-agent-program.md)
and the [conversational example guide](../guides/creating-a-conversational-agent.md).
