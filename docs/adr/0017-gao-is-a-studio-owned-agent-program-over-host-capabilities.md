---
status: accepted
date: 2026-07-28
owner: APXM agents
amends: ADR-0014
adopts: APXM workspace ADR-0014
---

# Gao is a Studio-owned Agent Program over Host Capabilities

## Context

ADR-0014 correctly removed Gao from frontend packages, compiler/runtime
semantics, evidence discriminants, and privileged product identities. Its
examples-only ownership did not deliver Gao as the APXM expert inside Studio or
provide an admitted boundary for interacting with Studio authoring state.

Workspace ADR-0014 assigns Gao's product source and authoring Capability
semantics to Studio while preserving the generic Agent Program execution spine.

## Decision

Gao is an ordinary source-first Agent Program owned by Studio. It imports only
the public generic frontend surface and lowers through the same FrontendGraph,
AIR, artifact, runtime, and evidence contracts as another program.

Agents owns no Gao constructor, package export, prompt, knowledge bundle,
Capability definition, runtime mode, admission identity, or product route. The
existing Gao repository example is removed after the Studio package supplies
equivalent generic-program conformance input.

Agents conformance may compile the exact Studio-owned Gao source bundle or an
immutable fixture derived from it. Such a fixture proves generic frontend and
runtime behavior; it does not transfer source ownership to Agents. No test may
branch on Gao's name beyond negative reachability checks proving the absence of
named core semantics.

A Studio Workflow is an Agent Program draft. Agents receives only the resulting
Python or TypeScript source/FrontendGraph through the normal compiler boundary.
Studio Host operations for listing, creating, or editing Workflows are ordinary
`capability.invoke` effects. They do not add an AIR operation, frontend marker,
runtime callback, raw graph builder, or direct draft mutation path in Agents.

Conversational remains the repository teaching reference. The two closed AIS
families, generic `LoopIterationCompleted` evidence, source-first vocabulary,
and Tool/handler separation from ADR-0014 through ADR-0016 are unchanged.

## Consequences

- Gao source, knowledge and product tests move to Studio.
- Agents example commands and docs stop presenting Gao as an owned example.
- Cross-repository conformance compiles Studio Gao with packed generic
  frontends and rejects any named compiler/runtime behavior.
- Host protocol and Studio authoring command semantics remain outside Agents;
  runtime observes ordinary Capability calls and typed effects.
- The Agents composition and frontend plans use Conversational for the primary
  teaching reference and Studio Gao only as an external integration fixture.
