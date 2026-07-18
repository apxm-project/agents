# APXM Agent Program guides

- Status: canonical target guides
- Normative theory: [Program Execution Model](../pxm/theory.md)
- Normative contract: [Agent Program composition and AIR](../agents/agent-program-composition-and-air-contract.md)

These guides describe the accepted v1 programming model. The examples are
contract-shaped target source used to create Python/TypeScript golden fixtures;
they do not claim the prototype packages already expose the final spelling.
Generated reference documentation becomes the syntax authority when the
frontend contract is implemented.

Reading order:

1. [Create an Agent Program](creating-an-agent-program.md)
2. [Create a Conversational Agent](creating-a-conversational-agent.md)
3. [Compose Agent Programs](composing-agent-programs.md)
4. [Use external coding agents through ACP](using-external-coding-agents-over-acp.md)
5. [Choose models and understand future routing](model-selection-and-future-routing.md)

Business-provider Integration and webhook authoring is documented at the
workspace level because it composes `adapters`, Auth, OS, Server, runtime and
Studio rather than belonging to the Agent Program frontend alone.
