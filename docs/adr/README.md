# APXM agents architecture decisions

These decisions are the compact record of the current product-neutral
abstract-machine architecture. Historical prototype decisions were removed
from the active index; Git history retains their rationale.

| ADR | Decision |
| --- | --- |
| [0006](0006-authoring-frontends-use-explicit-compiler-bridges.md) | Frontends use explicit compiler bridges. |
| [0007](0007-rust-embedding-uses-focused-libraries-and-injected-adapters.md) | Rust roles stay focused and use injected adapters. |
| [0008](0008-agent-programs-compose-through-new-and-invoke.md) | Programs compose through `new` and `invoke`. |
| [0009](0009-air-has-five-public-semantic-operations.md) | AIR has five public semantic operations. |
| [0010](0010-agent-program-source-owns-context-hooks-and-conversational-loops.md) | Source owns Context, Hooks, and loops. |
| [0011](0011-agent-program-execution-is-one-end-to-end-spine.md) | Compilation and execution form one spine. |
| [0013](0013-core-semantics-are-closed-and-implementations-enter-through-exact-port-bindings.md) | Implementations enter through exact Port bindings. |
| [0015](0015-source-first-agent-frontend-vocabulary.md) | Source-first vocabulary and FrontendGraph are shared across languages. |
| [0016](0016-tool-authoring-and-handler-execution-are-separate.md) | Tool declaration and admitted execution are separate boundaries. |
| [0018](0018-event-readiness-and-local-scheduling-are-agents-semantics.md) | Event readiness and local scheduling are portable runtime semantics. |
| [0019](0019-builtin-capabilities-own-no-durable-scheduling-or-wake-bridge.md) | Builtins do not own durable scheduling or wake bridges. |
| [0020](0020-durable-coordination-enters-through-port-contracts-and-composition-root.md) | Managed durability is whatever the Composition Root binds behind the durable-event Port Contract, not a named product. |
| [0021](0021-backend-neutral-graph-hints.md) | Graph hints are one Agents contract projected onto vLLM and llama.cpp `apxm` branches; the vLLM pin/join catalog is retired. |
| [0022](0022-capability-references-resolve-against-a-catalogue-and-permissions-are-declared-requests.md) | Capability references resolve against a generated catalogue plus shipped handlers; a declared permission is a request the layer stack may only tighten. |
| [0024](0024-non-loopback-serving-requires-a-complete-security-profile.md) | Non-loopback Event/OpenAI HTTP serving requires a complete security profile. |
| [0025](0025-host-fulfilled-capabilities-are-requested-by-the-runtime-and-settled-by-the-host.md) | A package-declared `host:` Capability is requested by the runtime and settled by the embedding host; APXM never executes it. |

## Amending an accepted record

An accepted ADR is amended by a **later record**, never by editing its decision
text and never by appending an amendment section to it. The later record
declares the relationship in its front matter as `amends: ADR-NNNN`, restates
what changed and why, and leaves the amended record as it was written, so what
was decided when stays legible. ADR-0020 amends ADR-0018 and ADR-0019 this way;
ADR-0022 amends ADR-0015 and ADR-0016. Some earlier records also carry the
back-link `amended_by:`.

The PXM pages under `docs/pxm/` are historical theory, not ADRs or executable
compatibility promises.
