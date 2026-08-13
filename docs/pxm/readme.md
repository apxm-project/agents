# PXM documentation

The [Program Execution Model theory](theory.md) is the current conceptual
authority. It explains how authored behavior becomes an admitted execution and
authoritative evidence.

The remaining pages preserve the earlier abstract-machine theory because it is
useful background. They are intentionally historical: they explain the
problems and vocabulary that led to the current design, but they are not
alternate APIs, schemas, operation sets, or runtime contracts.

| Historical page | Subject |
| --- | --- |
| [AAM](aam.md) | beliefs, goals, and capability-oriented abstract-machine state |
| [AIS](ais.md) | the earlier instruction-set and operation taxonomy |
| [Memory](memory.md) | memory tiers and durable context analysis |
| [Processes](processes.md) | process, spawn, communication, and handoff models |
| [Foundations](foundations.md) | early PXM terminology |
| [Compute](compute.md) | early compute and effect framing |
| [Scheduling](scheduling.md) | graph scheduling and readiness theory |

The current target replaces those prototypes with explicit Program Context,
local values, admitted Capabilities, model calls, program composition,
frontend-authored loops, exact Port bindings, and generic evidence. See the
[Agent Program composition and AIR contract](../agents/agent-program-composition-and-air-contract.md)
for the implementation boundary.
