# Gao Conversational Agent implementation plan — consolidated

- Status: consolidated on 2026-07-16
- Decision: [ADR-0002](../adr/0002-gao-specializes-the-conversational-agent-construct.md)
- Owner plan: [Agent Program composition and AIR full-replacement plan](agent-program-composition-and-air-full-replacement-plan.md), phase P6

Gao remains the release-blocking TypeScript reference specialization of the
standard Conversational Agent. Its migration now belongs to the common plan so
Gao cannot preserve a direct GraphBuilder/AUTONOMOUS loop, HookContext,
automatic Skill injection, or a privileged compiler/runtime path.

Gao completion is the P6/P9 evidence proving that one TypeScript Agent Program
uses the same Agent Facade, explicit context, `program.new`, `program.invoke`,
FrontendGraph v1, AIR v1, artifact v1, and runtime as every other program.
