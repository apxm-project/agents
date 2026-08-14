# Create Your First APXM Agent

An Agent Program is ordinary Python or TypeScript source. The frontend reads
that source statically into `apxm.frontend-graph`; Rust alone validates it,
constructs CFG/SSA and AIR, and produces the artifact admitted by the host.

The public authoring vocabulary is `Agent`, `Context`, `Tool`, `Model`, and
ordinary language control flow. `Capability`, `Event`, `Hook`, and `TaskGroup`
are focused advanced declarations. Graph builders, AIR text, node ids, and
`AgentFacade` are not author APIs.

## Python

```python
from typing import TypedDict

from apxm_program import Agent, Model


class SummaryRequest(TypedDict):
    text: str


class Summary(TypedDict):
    text: str


SummaryModel = Model[SummaryRequest, Summary]("model.summary")


@Agent(input=SummaryRequest, output=Summary)
async def summarize(agent, request):
    return await SummaryModel(request)
```

## TypeScript

```typescript
import { Agent, Model } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type SummaryRequest = { readonly text: string };
type Summary = { readonly text: string };

const SummaryModel = Model<SummaryRequest, Summary>("model.summary");

export const Summarizer = Agent<SummaryRequest, Summary>({
  name: "Summarizer",
  async run(agent, request) {
    return await SummaryModel(request);
  },
});
```

Both forms are semantically equivalent. The callback parameter is inferred;
authors use `agent.context` and `agent.yield_(...)` when their program carries
context across stateful invocations.

Both forms state the program's typed interface as the types themselves —
`@Agent(input=SummaryRequest, ...)` and `Agent<SummaryRequest, Summary>` — never
as strings naming them.

A TypeScript module states its own source once, above its Agent definitions,
with `source(import.meta.url)` from `@apxm/frontend/node`. Python recovers the
authored text through `inspect`; JavaScript has no equivalent, so the module
supplies it.

Keep bindings at module scope and use exact references. A Model reference is a
string naming an exact target, such as `model.summary`. A Capability reference
has to name something an implementation exists for — a builtin id, or an id the
package ships a handler for at `capabilities/<id>/handler.ts` — so import the
builtin from the generated catalogue instead of retyping it:

```python
from apxm_program.capabilities import SEARCH_WEB

SearchWeb = Tool[SearchWebRequest, SearchWebResult](SEARCH_WEB)
```

```typescript
import { SEARCH_WEB } from "@apxm/frontend/capabilities";

const SearchWeb = Tool<SearchWebRequest, SearchWebResult>(SEARCH_WEB);
```

The catalogue is generated from `crates/machine/ais/src/capabilities.rs`, so a
misspelled symbol fails at import, while a misspelled string survives until
`dekk agents compile-service-canonical` holds every reference against the
granted set. The marker itself refuses a mutable display name on sight:
`Tool("search-web")` raises "Tool accepts an exact typed reference, not a
display name 'search-web'" in both languages, because the id is `search_web`.
That is the `ToolDisplayNameRejected` case the surface manifest declares for the
Tool binding (`contracts/vectors/apxm.frontend-surface.json`).

The declarations a module makes are its Agents' declarations — an author does not
list again the names their own body already names. The frontend rejects dynamic
marker lookup and local shadowing instead of guessing what an Agent means.

`Tool` in an Agent Program is a typed reference to an admitted Capability; it
does not implement or execute a runtime tool. A package-local handler, when one is
needed, uses `Tool.define`/`Tool.answer` from `@apxm/agent-packaging` in
TypeScript or `capability(...)` from `apxm_program.handlers` in Python, in a
separate handler module. Either returns the Capability id it implements, so the
reference and the implementation are one object.

Declaring is not executing, and the split runs along the language. Only a
TypeScript handler becomes an executable one: `HandlerLanguage` admits only
`typescript` (`crates/machine/contracts/src/types/handler_manifest.rs`), and the
package build recognizes only `capabilities/<id>/handler.ts`
(`crates/tools/cli/src/commands/agent.rs`). A Python `capability(...)` states
what a package contains; no bundler or admitted worker adapter turns it into a
handler an artifact can run. Either way Rust owns the admitted Capability
execution boundary. See
[ADR-0016](../adr/0016-tool-authoring-and-handler-execution-are-separate.md)
and its amending record
[ADR-0022](../adr/0022-capability-references-resolve-against-a-catalogue-and-permissions-are-declared-requests.md).

## Compile through the owner boundary

Place source and its manifest in an Agent Program Source Bundle, then submit it
through host compile-admission. Other tools follow the same source-first path.
The compiler bridge is explicit and has no local fallback, raw-AIR input, or
frontend runtime mode.

For the complete author journey, including Context, Tools, Hooks, Events,
composition, and yield/resume, read the [authoring guide](../guides/creating-an-agent-program.md)
and the [conversational example guide](../guides/creating-a-conversational-agent.md).
