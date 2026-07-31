# APXM agents architecture decisions

Accepted ADRs are binding for the target. Superseded ADRs preserve rationale
only and cannot be used as implementation authority.

| ADR | Status | Decision | Plan |
| --- | --- | --- | --- |
| [0001](0001-hooks-are-typed-callbacks-over-scoped-context.md) | Superseded by 0010 | Historical HookContext/HookResult model | — |
| [0002](0002-gao-specializes-the-conversational-agent-construct.md) | Superseded by 0014 | Historical package-level Conversational Agent/Gao specialization | — |
| [0003](0003-agent-program-contract-migrations-remove-old-semantics.md) | Accepted; amended by 0014 | Full replacement through generic frontend APIs, two closed AIS families, generic loop evidence, and examples without compatibility runtime | Agent Program portfolio |
| [0004](0004-agents-publishes-focused-surfaces-in-the-apxm-release-family.md) | Accepted | Focused release-family surfaces | Distribution/bridge/library plans |
| [0005](0005-conversational-agent-lowers-to-a-structured-ais-loop.md) | Superseded by 0010 | Historical runtime Turn lifecycle | — |
| [0006](0006-authoring-frontends-use-explicit-compiler-bridges.md) | Accepted | Explicit local/remote compiler bridges | Compiler bridge plan |
| [0007](0007-rust-embedding-uses-focused-libraries-and-injected-adapters.md) | Accepted; amended by 0009, 0010, 0013 | Focused Rust libraries and injected adapters | Rust embedding plan |
| [0008](0008-agent-programs-compose-through-new-and-invoke.md) | Accepted | `program.new`, `program.invoke`, `instance.invoke` | Composition/AIR plan |
| [0009](0009-air-has-five-public-semantic-operations.md) | Accepted | Five-operation AIR and 38-op disposition | Composition/AIR plan |
| [0010](0010-agent-program-source-owns-context-hooks-and-conversational-loops.md) | Accepted; amended by 0014 | Explicit context, Agent Facade Hooks, discovery-only Skills, and generic program-authored loops | Composition/AIR plan |
| [0011](0011-agent-program-execution-is-one-end-to-end-spine.md) | Accepted; amended by 0013 | One frontend/compiler/runtime/inference/evidence execution spine | APXM master and composition/AIR plans |
| [0012](0012-acp-uses-explicit-capabilities-selection-is-not-runtime-semantics.md) | Accepted; amended by 0013 | ACP is an external Capability; v1 selection is exact; APXM-owned routing is future work | ACP/exact-selection plan and future-routing plan |
| [0013](0013-core-semantics-are-closed-and-implementations-enter-through-exact-port-bindings.md) | Accepted | Closed semantic types, narrow Port Contracts, exact implementation bindings, and no first-party bypass | Portable core contract, Rust embedding plan, and APXM portability plan |
| [0014](0014-conversational-agent-and-gao-are-examples-over-generic-agent-program-apis.md) | Accepted; amended by 0015 and 0017 | Conversational Agent remains an example; Gao's examples-only ownership is replaced by Studio ownership; AIS owns separate closed effect/composition and structural operation families; completed-loop evidence is generic | Composition/AIR contract and plan |
| [0015](0015-source-first-agent-frontend-vocabulary.md) | Accepted | Source-first five-concept authoring surface, frozen declaration matrix, BoundAgentTree representation stack, FrontendGraph typed intents vs Rust-owned AIS selection, surface manifest | Source-first Agent frontend master plan |
| [0016](0016-tool-authoring-and-handler-execution-are-separate.md) | Accepted | Typed Tool references, private package-handler definitions, Rust-owned manifest, and Rust-admitted Capability execution are separate boundaries | Tool authoring and handler-execution plan |
| [0017](0017-gao-is-a-studio-owned-agent-program-over-host-capabilities.md) | Accepted | Gao is a Studio-owned ordinary Agent Program; Workflow authoring arrives through typed Host Capabilities with no named Agents semantics | Studio Gao Host plan and cross-repository frontend/runtime conformance |
| [0018](0018-event-readiness-and-local-scheduling-are-agents-semantics.md) | Accepted; requires accepted workspace ADR-0027 | Agents owns portable Program Event/reference/reducer/activation/readiness/local scheduling semantics while Server owns managed durable coordination | Event-driven runtime contract and full-replacement plan |

Canonical owner contract and plan:

- [Agent Program composition and AIR contract](../agents/agent-program-composition-and-air-contract.md)
- [Agent Program composition and AIR full-replacement plan](../agents/agent-program-composition-and-air-full-replacement-plan.md)
- [ACP interoperability and selection contract](../agents/acp-and-routing-contract.md)
- [ACP interoperability and exact-selection full-replacement plan](../agents/acp-and-routing-full-replacement-plan.md)
- [Future APXM-owned routing plan](../agents/future-apxm-routing-plan.md)
- [Portable core interface contract](../agents/portable-core-interface-contract.md)
- [Tool authoring and handler-execution plan](../agents/tool-authoring-and-handler-execution-plan.md)
- [Event-driven runtime and scheduler contract](../agents/event-driven-runtime-and-scheduler-contract.md)
- [Event-driven runtime full-replacement plan](../agents/event-driven-runtime-full-replacement-plan.md)
- [APXM Port Contracts and composition plan](../../../../docs/plans/apxm-port-contracts-and-composition-full-replacement-plan.md)
- [APXM master plan](../../../../docs/plans/apxm-master-plan.md)
