# Gao Conversational Agent implementation plan — superseded

- Status: superseded on 2026-07-22
- Superseded decision: [ADR-0002](../adr/0002-gao-specializes-the-conversational-agent-construct.md)
- Replacement decision: [ADR-0014](../adr/0014-conversational-agent-and-gao-are-examples-over-generic-agent-program-apis.md)
- Replacement plan: [Agent Program composition and AIR full-replacement plan](agent-program-composition-and-air-full-replacement-plan.md), phases P2/P3/P6/P8/P9

This tombstone preserves the former named-specialization plan as historical
rationale. Gao now belongs only in repository examples, where it may specialize
an example-local Conversational Agent using packed generic frontend APIs.

The superseded plan treated Gao as the release-blocking TypeScript
specialization of a standard Conversational Agent. The replacement plan keeps
only the conformance goal: Gao cannot preserve a direct
GraphBuilder/AUTONOMOUS loop, HookContext, automatic Skill injection, named
package API, or privileged compiler/runtime path.

Gao completion is the P6/P9 evidence proving that one TypeScript Agent Program
uses the same Agent Facade, explicit context, `program.new`, `program.invoke`,
FrontendGraph v1, AIR v1, artifact v1, and runtime as every other program.
