# Gao Conversational Agent implementation plan — superseded

- Status: superseded on 2026-07-22
- Superseded decision: [ADR-0002](../adr/0002-gao-specializes-the-conversational-agent-construct.md)
- First replacement decision: [ADR-0014](../adr/0014-conversational-agent-and-gao-are-examples-over-generic-agent-program-apis.md)
- Current ownership decision: [ADR-0017](../adr/0017-gao-is-a-studio-owned-agent-program-over-host-capabilities.md)
- Replacement plan: [Agent Program composition and AIR full-replacement plan](agent-program-composition-and-air-full-replacement-plan.md), phases P2/P3/P6/P8/P9

This tombstone preserves the former named-specialization plan as historical
rationale. ADR-0014 first reduced Gao to a repository example; ADR-0017 then
replaced that examples-only ownership. Gao now belongs to Studio as an ordinary
Agent Program. Agents owns only generic compiler/runtime conformance and may
consume the Studio bundle as an external fixture.

The superseded plan treated Gao as the release-blocking TypeScript
specialization of a standard Conversational Agent. The replacement plan keeps
only the conformance goal: Gao cannot preserve a direct
GraphBuilder/AUTONOMOUS loop, HookContext, automatic Skill injection, named
package API, or privileged compiler/runtime path.

The remaining Agents proof is P6/P9 evidence that the external Studio-owned Gao
source uses the same Agent Facade, explicit context, `program.new`,
`program.invoke`, FrontendGraph v1, AIR v1, artifact v1, and runtime as every
other program, without an Agents-owned Gao package or named branch.
