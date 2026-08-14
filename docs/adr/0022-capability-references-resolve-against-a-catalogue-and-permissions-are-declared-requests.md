---
status: accepted
date: 2026-08-14
owner: APXM agents
requires: accepted Agents ADR-0006, ADR-0009, ADR-0013
amends: ADR-0015, ADR-0016
---

# Capability references resolve against a catalogue, and permissions are declared requests

## Status and authority

Accepted for the Agents repository. It is subordinate to ADR-0013 (core
semantics are closed and implementations enter through exact Port Bindings)
and to ADR-0009 (AIR has five public semantic operations); it adds no
operation. It amends ADR-0015 and ADR-0016 by restating what a Capability
reference is, what an author may say about one, and where the metadata that
used to sit beside one now lives. It does not reopen ADR-0015's five-concept
everyday surface, its representation stack, or its FrontendGraph-owns-intent
boundary, and it does not reopen ADR-0016's separation of Tool declaration
from admitted execution. Following ADR-0020's precedent, the amended records
are left as they were written; this is the later record that resolves them.

## Context

ADR-0015 §4 froze the cross-language declaration matrix, and ADR-0016 §1
repeated one row of it: an imported Tool binding is spelled
`Tool[I, O](capability_ref)`. In both records the argument is a bare exact
reference and nothing else — ADR-0015 §4 admits "statically resolvable types,
exact references, closed options, or literals" as marker arguments, and
ADR-0015 §2 states that "Grants, credentials, Runtime Profiles, endpoints,
graph node ids, AIR text, and deployment bindings never appear in author
source." ADR-0016 §3 then placed "Capability identity, read-only
classification, approval, grants, and resource authority" in "the capability
definition, Auth admission, and exact bound adapter."

Three things on this branch left those statements no longer describing the
tree. The reference stopped being a string an author invents unaided: the
builtin allowlist is now projected into both frontends as importable symbols.
The marker grew a second argument, `permission`, so the program states what it
is asking for. And the capability definition ADR-0016 §3 named — a
per-capability `capability.toml` beside a per-capability `permission.toml` —
was deleted along with the aggregates that restated them, so the sentence
points at a file that no longer exists in any package.

None of that is a change of position about who holds authority. It is the
opposite: the reason a permission may now be written in source is that writing
one grants nothing. But ADR-0015 and ADR-0016 are accepted decisions carrying
`amends` provenance to ADRs 0006, 0007, 0010, and 0014, and this repository
amends such a record by a later one rather than editing its decision text.
This is that record.

## Decision

### A Capability reference is resolved against a catalogue, not merely spelled

The builtin half of the grantable surface is generated. `apxm codegen
capabilities` (`crates/tools/cli/src/frontend/codegen_capabilities.rs`) renders
`apxm_ais::capabilities::BUILTINS` and `BUILTIN_GROUPS`
(`crates/machine/ais/src/capabilities.rs`) into
`crates/compiler/frontend/python/apxm_program/_generated/capabilities.py` and
`crates/compiler/frontend/typescript/src/generated/capabilities.ts`, and
`apxm codegen permissions` does the same for the decision vocabulary. Both
projections are checked for drift: `dekk agents check` runs `codegen
capabilities --check` and `codegen permissions --check` among its steps
(`.dekk.toml`).

What is projected is set membership, not the constant list. `MANAGE_TASK` is
declared in the source of truth and has an `AGENT_MANAGEMENT_BUILTINS` slot,
but it is absent from `BUILTINS`, so no importable symbol is minted for it;
the codegen test
`generated_catalogue_projects_binding_admissibility_not_the_constant_list`
pins that, and a second test pins that the catalogue publishes none of the
operation vocabulary ADR-0015 §3 keeps off the authoring surface. Emitting the
constant list instead would have minted a symbol whose only effect on import is
to fail the grant check.

The catalogues are reachable at `apxm_program.capabilities` and
`@apxm/frontend/capabilities` and are deliberately not re-exported from the
package root, which still carries exactly the ADR-0015 §7 surface manifest
(`crates/compiler/frontend/python/apxm_program/capabilities.py`,
`crates/compiler/frontend/typescript/src/capabilities.ts`).

The marker parameter itself is still a string in both languages
(`crates/compiler/frontend/python/apxm_program/_markers.py`,
`crates/compiler/frontend/typescript/src/markers.ts`), because a package's own
handlers are not in any generated catalogue. The catalogue removes the need to
hand-type a builtin id and makes a misspelled one an import failure; it is not
what makes an unresolvable reference fail. That is the grant check below.

### A permission is declared in source, and a declaration is a request

`Tool` and `Capability` accept `permission=` in both frontends, carrying the
`Allow` / `Ask` / `Deny` markers generated from
`crates/machine/ais/src/permissions.rs` — the one place the three-value
vocabulary is defined. The request travels as
`CapabilityRequirement.requested_permission` on the FrontendGraph
(`crates/machine/program/src/frontend_graph.rs`), which is a typed source
intent in exactly ADR-0015 §6's sense; no operation was added and Rust still
selects AIS alone.

This does not reverse ADR-0015 §2's exclusion of grants from author source,
because a request is not a grant. Resolution happens above the program, through
a closed layer stack whose precedence is fixed in the machine:
`PermissionLayer` orders `Code` (10), `Package` (20), and `Deployment` (30),
and `PermissionResolution::resolve` applies layers in the enum's own order
rather than the caller's, so whoever assembles a stack cannot reorder it. A
layer above the code layer may only tighten: widening a held decision and
naming a capability the program never requested are both hard failures that
leave nothing resolved. The shipped arrangement is
`resolve_code_over_package`, the stack's single constructor with producers
today; `Deployment` has no producer in this tree, and its absence is documented
there as *no decision stated*, never as a deployment allowing what the layers
below decided.

The resolution reaches execution as admission data, not as program data:
`AdmittedCapabilityPermission` on `ExecutionAdmission.capability_permissions`
(`crates/runtime/kernel/src/admission.rs`). The driver records the decision as
a `CapabilityAttemptRecorded` fact before attempting the effect, then refuses
anything short of an outright allow before materializing an argument
(`crates/runtime/execution/src/driver.rs`). A package build refuses in the same
direction: `agent sync` will not emit an executable handler for a capability
the resolution denies, and derives `requires_approval` from the resolved
decision rather than from an authored field
(`crates/tools/cli/src/commands/agent.rs`).

That is the whole point of admitting `permission=` into source. Authority stays
a property of the machine — the layer order, the tighten-only rule, and the
admission the composition root states — and the program text is only the widest
thing that may be asked for.

### An unsatisfiable Capability reference is refused, not passed through

A reference that names nothing now fails at three points rather than resolving
to nothing at a registry.

At compile time, `check_capability_references_are_granted`
(`crates/tools/cli/src/commands/compile_service_canonical.rs`) holds every
`capability.invoke` the compiled program names against
`granted_capability_ids`, and names the two places a reference may come from in
its own diagnostic.

At artifact build, `reconcile_requirements_with_air`
(`crates/machine/program/src/artifact.rs`) holds the graph's declared
requirements against the operands the lowered AIR names, in both directions. An
artifact whose requirement set and effect set disagree has no single answer to
what the program needs, so it is not built.

At admission, `CapabilityGrantSet`
(`crates/runtime/execution/src/driver.rs`) is the finite set of references one
invocation may resolve, stated by the composition root from a source that is
not the program. The reference an admission carries is owned by the grant set
rather than copied from the AIR operand it is later compared against, which is
what turns the driver's authored-versus-admitted check into a real comparison;
`local_capability_invocation_admissions`
(`crates/tools/cli/src/commands/canonical_execute.rs`) mints admissions that
way. A reference that *is* granted but that policy refuses is admitted and
carries its refusing decision, so a refusal stays a decision in the evidence
instead of degrading into a registry miss.

Lowering-shape conformance corpora keep opaque references they never dispatch,
and their exemption is declared rather than inferred: `CapabilityGrantOrigin`
distinguishes `RegisteredImplementations` from `ConformanceCorpus`, and a
corpus takes the exemption by calling
`CapabilityGrantSet::for_conformance_corpus` at the one call site that needs it
(`crates/runtime/execution/tests/end_to_end.rs`). Nothing reads the shape of a
reference to decide.

### The package format is contract-bound, and the capability definition is gone

The authored package format now has a schema and executed vectors,
`contracts/schemas/apxm.agent.json` and `contracts/vectors/apxm.agent.json`,
which it did not have when ADR-0016 was written. Each vector is executed rather
than merely validated: `agent lint` runs over the vector materialized as a real
package on disk (`crates/tools/cli/src/commands/agent.rs`). `agent.toml` is the
only authored manifest in the package, and its `[permissions]` table is the
only authored permission surface in it.

The grantable set is the builtin catalogue plus the handlers the package ships:
`granted_capability_ids` is `BUILTINS` chained with
`shipped_capability_handler_ids`, which is the set of directories under
`capabilities/` containing a `handler.ts`
(`crates/tools/cli/src/commands/agent.rs`). The handler's existence is the
declaration. The per-capability `capability.toml` and `permission.toml`, and
the `capabilities.toml` / `permissions.toml` aggregates that restated them,
were deleted from every package in commit `51903765`, and the scaffold test
asserts a fresh package does not resurrect any of them.

ADR-0016 §3's sentence is therefore restated. Capability identity is the
grantable id — a builtin catalogue entry or a shipped handler's directory name.
Approval is derived from the resolved permission, not authored beside the
handler. Grants and resource authority remain exactly where ADR-0016 put them,
in admission and the exact bound adapter. Read-only classification survives
only for builtins, as `RuntimeCapability.read_only`
(`crates/runtime/capability-iface/src/metadata.rs`), set by the builtin that
declares it; no surviving authored surface states it for a package-shipped
handler, so the generated tool manifest carries none even though
`apxm.handler-manifest` still permits the field
(`examples/agents/coder/capabilities/handlers/tools.json`). Reinstating it
would mean inventing a second declaration beside the handler, which is the
shape this collapse removed.

## What this does not change

ADR-0015 §1's everyday surface is still exactly five concepts and §2's advanced
surface exactly four; `contracts/vectors/apxm.frontend-surface.json` lists
`Agent`, `agent`, `Context`, `Tool`, `Model` and `Capability`, `Event`, `Hook`,
`TaskGroup`, unchanged. No sixth marker exists. The generated catalogues are
not markers and are not exported from either package root.

ADR-0015's position that a reference is exact and a mutable display name is not
one is unchanged and still enforced at the marker, in both languages. The
`permission` argument is a closed option in ADR-0015 §4's own sense, and the
surface manifest already records it as one.

ADR-0016's separation of roles is unchanged and, in the permission direction,
strengthened. An Agent Program source file still imports nothing outside the
surface manifest, which `tools/scripts/check_frontend_surface.py` holds the
authoring samples to. `apxm.handler-manifest` is still the sole serialized
handler representation and is still build output, written only by `agent sync`
(`crates/tools/cli/src/commands/agent.rs`). And a handler still cannot
self-authorize: it is now refused emission outright when the resolution denies
its capability, which is a tightening of ADR-0016 §3 rather than a retreat from
it.

## Consequences

- The `Tool` and `Capability` rows of ADR-0015 §4 and the first row of
  ADR-0016 §1 read `Tool[I, O](capability_ref, permission=...)` and
  `Capability[I, O](ref, permission=...)`, with the TypeScript projections
  taking the same option in an options object.
- ADR-0016 §3's "capability definition" resolves through this ADR to the
  package's grantable surface and the runtime's builtin metadata. No future
  code, schema, or doc may reintroduce a per-capability metadata file as the
  home for identity, approval, or authority.
- A capability id has one source of truth,
  `crates/machine/ais/src/capabilities.rs`, and one permission vocabulary,
  `crates/machine/ais/src/permissions.rs`; both
  reach the frontends only by codegen under a drift gate, per ADR-0006's
  explicit-bridge boundary.

## Open

Skills are not declarable from either frontend. There is no `Skill` marker in
the surface manifest or in either package, so a program cannot author a skill,
and nothing in this ADR changes that.

Two of the three gaps this section recorded have since closed, outside this
ADR. The skill discovery ids are back in
`crates/machine/ais/src/capabilities.rs` as `list_skills`, `search_skills`, and
`read_skill`, this time behind handlers in
`crates/runtime/capability/src/builtins/skills.rs` — the ordering the earlier
state inverted. And the skill contracts have a reader:
`crates/machine/program/src/skill.rs` decodes and verifies
`apxm.skill-package`, `apxm.package-local-skill`, and
`apxm.skill-discovery-root`, held to their vectors by
`crates/machine/program/tests/skill_conformance.rs`.

The package folder contract still refuses a `skills/` path outright, which
`agent_package_rejects_local_skill_resources`
(`crates/tools/cli/src/commands/agent.rs`) pins. `apxm.package-local-skill`
describes what such a directory would contain, but the allowlist stays closed
until something authors one — recognizing the path first would repeat, at the
folder level, the allowlist-without-implementation mistake the capability ids
just came back from.

The frontend surface is not yet a checked interface.
`tools/scripts/check_frontend_surface.py` compares sets of exported identifiers
against the manifest and scans authoring samples for non-manifest imports; it
does not check that a marker's arguments match the `arguments` the manifest
records for it. ADR-0015 §7 assigned the manifest that content and the manifest
carries it, including this ADR's `permission` argument, but nothing yet holds
the frontends to it. Until something does, an argument may be added to a marker
in one language and not the other without the gate noticing.

`PermissionLayer::Deployment` has no producer. `PermissionResolution::resolve`
applies the layer the moment a caller supplies one and the tighten-only rule
already covers it, but nothing in this tree states a deployment-profile
decision, so the shipped stack stops at two layers
(`crates/machine/ais/src/permissions.rs`).

## Rejected alternatives

**Edit ADR-0015 §4 and ADR-0016 §1 and §3 in place.** Rejected, on ADR-0020's
reasoning: both are accepted decisions with recorded `amends` provenance, and
rewriting their decision text erases what was decided when. The signature and
the capability definition are exactly the kind of pinned detail a later record
is supposed to restate.

**Emit every `pub const` in the capability source of truth as a frontend
symbol.** Rejected. The constants declare more than an author may bind, so the
catalogue would mint importable symbols whose only effect is to fail the grant
check on use. Projecting binding admissibility is what keeps the two agreeable.

**Make the marker parameter the generated `CapabilityId` type.** Rejected for
now. A package's own handlers are grantable and are in no generated catalogue,
so a closed parameter type would reject a legitimate reference at the marker
and force a second, untyped spelling beside it. The grant check already refuses
what the catalogue cannot cover, and it refuses both halves of the grantable
set by the same rule.

**Treat a declared permission as a grant and skip the layer stack.** Rejected.
It would make authority a property of the program text, which is precisely what
ADR-0015 §2 excludes and what admission exists to prevent. A declaration is the
widest thing that may be asked for; every narrowing below it is the machine's.
