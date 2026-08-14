// Typed Agent authoring in ordinary TypeScript.
//
// An everyday author needs five names — Agent, Context, Tool, Model, and
// ordinary control flow — plus the inferred agent callback parameter. Advanced
// programs add Capability, Event, Hook, and TaskGroup. The frontend reads the
// authored source statically and traverses it into the language-neutral
// FrontendGraph; it never executes the authored body and exposes no operation
// constant, node or region identity, or raw graph builder.

export { Agent } from "./agent.js";
export { Capability, Context, Event, Model, Skill, Tool } from "./markers.js";
export { Hook, TaskGroup } from "./advanced.js";
