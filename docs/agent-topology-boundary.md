# Agent Topology Boundary

Status: hard runtime architecture rule.

Agent hierarchy, reporting lines, directory visibility, delegation rights, and
approval relationships are fleet policy. They are authored and enforced outside
the APXM runtime.

## Hard Rule

APXM runtime MUST remain agent-topology agnostic.

The runtime MUST NOT parse, store, schedule on, or branch on organization-policy
fields such as `reports_to`, `directory_policy`, `handoff_policy`,
`delegates_to`, `consults`, `notifies`, `observes`, `peers_with`, or
`approves`.

Those fields may exist in higher layers such as `apxm-studio` and `apxm-os`, but
they must be enforced before execution is admitted or lowered into APXM runtime
primitives.

## Ownership

| Layer | Owns | Does not own |
|---|---|---|
| `apxm-studio` | Authoring and visualization of agent topology; export of desired-state policy. | Runtime authorization decisions. |
| `apxm-os` | Agent identity, inboxes, discovery, routing, topology policy, approval, and handoff admission. | APXM graph scheduling semantics. |
| APXM runtime | AIR/.apxmobj execution, DAG scheduling, operation handlers, memory tiers, capability execution, events. | Company/org topology, directory policy, reporting hierarchy. |

APXM receives concrete executable facts only:

- AIR graph nodes and edges.
- Operation attributes such as `recipient`, `target_agent`, `handoff_to`,
  `agent_name`, `skill_id`, and protocol.
- Scoped context that has already been admitted by the host.
- Capability and skill visibility sets chosen by the host.
- Generic trace metadata such as execution id, span id, agent code, and
  correlation id.

## Lowering Model

Topology policy affects execution at the admission boundary:

1. A host receives an event, question, task, handoff request, approval request,
   or tool/skill call.
2. The host identifies the caller and target.
3. The host evaluates topology policy.
4. If denied, the host rejects or records a policy-denied event.
5. If allowed, the host lowers the request into a normal APXM primitive:
   `COMMUNICATE`, `HANDOFF`, `DELEGATE`, `FLOW_CALL`, `CALL_SKILL`, an inbox
   event, or an A2A/MCP message.
6. APXM runtime executes the concrete graph without interpreting the topology
   relation that allowed it.

This keeps replay, optimization, and scheduler behavior tied to the artifact and
its admitted inputs, not to mutable fleet policy.

## Relationship Mapping

| Topology concept | Runtime effect |
|---|---|
| `reports_to` | Host routing, escalation, default supervisor, visualization. Never a DAG edge. |
| `delegates_to` | Host may admit `DELEGATE`, `HANDOFF`, local task send, or A2A `tasks/send` when reach allows it. |
| `consults` | Host may admit message/question exchange. No ownership transfer. |
| `notifies` | Host may emit one-way events or inbox deliveries. |
| `observes` | Host may deliver safe belief/event updates. |
| `peers_with` | Host may admit lateral discovery/message, according to policy. |
| `approves` | Host may satisfy approval gates, often by emitting pending/approved/rejected events before a continuation runs. |

Reach levels are cumulative only when the policy layer explicitly defines them
that way. Visibility is not authority: `know` does not imply `message`, and
directory visibility does not imply handoff.

## Runtime Requirements

APXM runtime code that touches multi-agent primitives must follow these rules:

- `COMMUNICATE`, `HANDOFF`, `DELEGATE`, `FLOW_CALL`, `SPAWN_AGENT`, and
  `CALL_SKILL` accept concrete targets, not relationship names.
- Broadcast-style fan-out is a concrete runtime mode over the registered runtime
  set. If a host wants topology-aware broadcast, it must filter the target set
  before lowering or attach host policy middleware before the handler runs.
- Runtime events may expose generic execution topology, such as
  `agent_spawned`, `communicate_dispatched`, and `graph_edge`. They must not
  encode organization-policy relation types as core runtime semantics.
- If an embedded host enforces topology through `OperationMiddleware`, the
  middleware is host policy. It must use canonical operation attributes and
  scoped metadata; it must not add topology fields to the core runtime model.
- Dynamic policy decisions that change what can execute must be made before
  execution or recorded as durable host policy-decision events. Do not make
  replay depend on unstamped ambient topology state.

## Constants And Strings

Topology schema tokens and relation names belong to the policy-owning layer.
When APXM runtime needs shared strings for executable operation attributes,
events, or metadata, they must live in the appropriate shared constants module
instead of being repeated inline.

Do not add topology relation string literals to runtime handlers. If a new
executable APXM concept is required, add a typed AIS operation, attribute, or
event contract; if it remains fleet policy, keep it in `apxm-os`/Studio.

## Non-Goals

APXM runtime does not answer:

- Who can discover which agent?
- Who can message which agent?
- Who may hand off work to whom?
- Who approves a risky action?
- Which agents are in the same company, team, domain, or reporting chain?

Those are valid APXM-family concerns, but they are not runtime scheduling or
artifact semantics.
