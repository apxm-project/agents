# APXM Agent Program guides

These guides describe the shipped source-first authoring surface. Python and
TypeScript express the same generic Agent Program concepts; Rust owns graph
verification, AIR/AIS lowering, artifacts, and execution.

Reading order:

1. [Start authoring](authoring-workflow.md)
2. [Create an Agent Program](creating-an-agent-program.md)
3. [Compose Agent Programs](composing-agent-programs.md)
4. [Create a conversational example](creating-a-conversational-agent.md)

Conversational behavior is an ordinary authored loop. It is not a runtime
special case, a product lifecycle, or a named frontend export. External
products provide their own integrations and admission roots around these
generic programs.
