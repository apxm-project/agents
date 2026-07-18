# Agent topology boundary

- Status: hard target runtime rule
- Owner decision: APXM Studio hierarchy/admission policy; APXM generic enforcement
- Program contract: [Agent Program composition and AIR contract](agents/agent-program-composition-and-air-contract.md)

Company, area/department, group, reporting, visibility, approval, and routing
relationships are product/control-plane policy. They are never AIR operations,
runtime scheduling hints, implicit context, or Program Instance ownership.

## Hard rule

APXM runtime MUST remain organization-topology agnostic. It MUST NOT parse,
store, schedule on, or branch on policy fields such as `reports_to`,
`directory_policy`, `delegates_to`, `consults`, `notifies`, `observes`,
`peers_with`, or `approves`.

APXM Studio/Auth may use those relationships before admission. The runtime receives
only exact, typed executable facts and validates/enforces the resulting
identity, authority, and scope. Mutable organization policy never becomes an
unstamped replay input.

## Ownership

| Layer | Owns | Does not own |
| --- | --- | --- |
| APXM Studio | Company/area/group/agent authoring and visualization | Runtime scheduling or execution authority |
| APXM Studio/Auth | Directory visibility, target selection policy, authenticated Agent Identity, complete grants, approvals, Skill Associations | AIR, Program Context, callbacks, or program composition semantics |
| APXM admission/runtime | Exact ProgramRef/instance admission, identity/grant validation, five AIR operations, generic regions/invocations/evidence | Company hierarchy, reporting policy, Skill injection, or implicit reach |

APXM receives concrete facts only:

- exact digest-bound `ProgramRef` or `ProgramInstanceRef`;
- authenticated target `AgentIdentityBinding` supplied by admission;
- typed input `I`, including any caller data explicitly projected by source;
- complete attenuated Capability Grants and approval/event references;
- admitted Skill-discovery scope for `search_skills`/`read_skill`, never Skill
  bodies injected into context; and
- generic invocation, callsite, occurrence, trace, and policy-evidence refs.

## Admission model

1. APXM Studio receives a task, event, question, approval, or target-selection request.
2. APXM Auth authenticates the caller and resolves company/topology policy.
3. Studio selects an exact allowed published Agent Program/identity and obtains
   complete attenuated authority and policy evidence.
4. APXM admission verifies that identity, artifact, scope, compatibility, and
   authority agree.
5. If admitted, authored source uses only `program.new`, `program.invoke`,
   `capability.invoke`, or `await.event` as appropriate. If denied, no Program
   Invocation begins.
6. APXM records exact policy/admission references but never reinterprets the
   organization relation that led to them.

Dynamic choice inside a program is limited to a finite set of statically
imported typed Program references. A model may select a key; ordinary program
control flow chooses the exact reference. There is no `HANDOFF`, `DELEGATE`,
`COMMUNICATE`, `FLOW_CALL`, `SPAWN_AGENT`, string target, or model-minted
authority operation. Provider/human messaging and non-APXM agent protocols are
admitted Capabilities or external adapters.

## Relationship projection

| Product relation | Permitted product/control-plane effect |
| --- | --- |
| reports to | Routing default, escalation, visualization, or approval policy; never an AIR edge |
| may delegate/invoke | Admission may allow one of the source program's exact typed ProgramRefs |
| consults/notifies | Admission may allow a messaging Capability with a concrete target/resource |
| observes | Product may expose permissioned evidence; no automatic Program Context sharing |
| peers with | Directory visibility or target-selection policy; never implicit authority |
| approves | Authorized fulfillment of an exact typed APXM event/approval reference |
| shares Skills | Skill-discovery association/filter only; never prompt injection or a grant |

Visibility is not authority. Knowing that an agent, Skill, Tool, or Capability
exists cannot authorize its use. A specialist may discover Skills unavailable
to its parent while still operating only under its own admitted identity and
attenuated complete grants.

## Runtime requirements

- Runtime events expose generic Program Instance/Invocation/NodeExecution
  lineage, not organization relationships.
- Parent/child ownership is structured program lifecycle, not company
  hierarchy.
- Policy changes affect later admissions only unless a signed revocation or
  cancellation contract explicitly applies to active work.
- Replay uses the recorded identity/grant/policy evidence for that occurrence;
  it never queries ambient hierarchy to invent different behavior.
- No adapter or middleware may add a sixth AIR operation, bypass
  `capability.invoke`, inject Skills/context, or widen child authority.

## Non-goals

APXM program/runtime semantics do not answer who can discover, invoke, message,
observe, supervise, or approve whom. Those are valid APXM Studio decisions;
APXM consumes only their exact admitted result.
