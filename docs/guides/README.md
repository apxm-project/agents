# APXM Agent Program guides

- Architectural status: canonical target semantics
- Frontend syntax status: design proposal pending the D0 owner decision in the
  frontend plan
- Normative theory: [Program Execution Model](../pxm/theory.md)
- Normative contract: [Agent Program composition and AIR](../agents/agent-program-composition-and-air-contract.md)
- Frontend master plan: [Source-first Agent frontend](../agents/simple-agent-authoring-frontend-plan.md)

These guides distinguish the accepted v1 programming model from the proposed
short frontend spelling. The examples are contract-shaped target source used
to create Python/TypeScript golden fixtures; they do not claim the current
packages already expose the proposed API.
Generated reference documentation becomes the syntax authority when the
frontend contract is implemented.

The frontend is the coding face of APXM: readable typed source becomes one
statically analyzable and optimizable program, exact admitted execution, and
source-correlated evidence. A conversational Agent is simply one example—an
ordinary Agent with an authored loop, explicit Context, Model/Tool calls, and
yield/resume. Events, Hooks, composition, and structured work use the same
generic vocabulary. Exact inference implementations such as APXM-vLLM remain
behind admitted ports and never create a second frontend or runtime model.

Python decorators and TypeScript typed declaration factories are
language-native projections of the same reviewed surface matrix. They bind to
equivalent semantic-tree nodes and FrontendGraph intents; individual guides do
not define independent decorator names or behavior.

Installable frontends expose generic Agent Program APIs only. Conversational
Agent remains a repository example. Gao is a Studio-owned ordinary Agent
Program and an external conformance input; neither introduces a package export
or core contract.

Reading order:

1. [Start authoring an Agent](authoring-workflow.md) — the short, practical
   path: begin with Conversational, then take focused Coder or Gao patterns
   only when they fit.
2. [Author an Agent](creating-an-agent-program.md)
3. [Build a conversational Agent](creating-a-conversational-agent.md)
4. [Compose Agent Programs](composing-agent-programs.md)
5. [Use external coding agents through ACP](using-external-coding-agents-over-acp.md)
6. [Choose models and understand future routing](model-selection-and-future-routing.md)

Business-provider Integration and webhook authoring is documented at the
workspace level because it composes `adapters`, Auth, OS, Server, runtime and
Studio rather than belonging to the Agent Program frontend alone.
