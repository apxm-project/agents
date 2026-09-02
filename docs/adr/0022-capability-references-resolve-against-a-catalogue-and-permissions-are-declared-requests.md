---
status: accepted
date: 2026-08-14
owner: APXM agents
requires: accepted Agents ADR-0006, ADR-0009, ADR-0013
amends: ADR-0015, ADR-0016
amended_by: ADR-0025
---

# Capability references resolve against a catalogue, and permissions are declared requests

## Status and authority

Accepted for the Agents repository. It is subordinate to ADR-0013 (core
semantics are closed and implementations enter through exact Port Bindings)
and to ADR-0009 (AIR has five public semantic operations); it adds no
operation. It amends ADR-0015 and ADR-0016 by restating what a Capability
reference is, what an author may say about one, where the metadata that used to
sit beside one now lives, how the cross-language declaration matrix is held, and
which half of ADR-0016's Python deferral survives. It does not reopen ADR-0015's
five-concept everyday surface, its representation stack, or its
FrontendGraph-owns-intent boundary, and it does not reopen ADR-0016's separation
of Tool declaration from admitted execution. Following ADR-0020's precedent, the
amended records are left as they were written; this is the later record that
resolves them.

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

None of the three is a change of position about who holds authority. It is the
opposite: the reason a permission may now be written in source is that writing
one grants nothing.

Two further sentences in the same two records had the opposite problem: they
were never held to anything. ADR-0015 §4 froze the declaration matrix as a table
in a document, and §7 assigned
`contracts/vectors/apxm.frontend-surface.json` the machine-readable version of
it, but the gate over that manifest compared sets of exported identifier *names*.
A name-set comparison cannot see a declaration one language projects and the
other does not, nor two projections sharing a name while accepting different
arguments. ADR-0016 §4 had meanwhile deferred "Python package-local handlers" as one thing,
so it covered Python's ability to *declare* a shipped Capability as well as its
ability to run one — a conflation the same gate left invisible.

But ADR-0015 and ADR-0016 are accepted decisions carrying `amends` provenance to
ADRs 0006, 0007, 0010, 0014, and 0015, and this repository
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

The resolution reaches execution as admission data, not as program data. On the
shipping path it rides the per-node `CapabilityInvocationAdmission.permission`
minted through `InvocationAdmission`
(`crates/runtime/execution/src/driver.rs`). `ExecutionAdmission` carries
`AdmittedCapabilityPermission` on `capability_permissions`
(`crates/runtime/kernel/src/admission.rs`) as the signed, nonce-bound envelope's
own carrier; nothing in this tree mints an `ExecutionAdmission`, so that field
is verified but unproduced, exactly as the rest of that envelope is. The driver
records the decision as
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
`capabilities/` containing a `handler.py` or a `handler.ts`
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

### The declaration matrix is checked, not asserted

ADR-0015 §4's matrix was a table in a document, and the gate over ADR-0015 §7's
manifest compared sets of exported identifier names, so nothing read the matrix
itself.

`contracts/vectors/apxm.frontend-surface.json` now registers the languages that
implement the surface — `python` and `typescript` — and, per declaration, states
each language's projection: the module, the symbol, and, for every argument, the
form that language projects it in. `dekk agents check-frontend-surface`
(`tools/scripts/check_frontend_surface.py`) extracts the real argument shape from
each language's own source and holds it to that statement. A registered language
with no projection for a declaration is
`FrontendSurfaceIncomplete{language, declaration}`; a generated module that has
drifted from the manifest is `FrontendSurfaceUnsynced{artifact}`. Adding a third
language is one `languages` entry, one projection per declaration, and one
extraction layer in the gate.

The gate compares shape rather than spelling — `capabilityRef` and
`capability_ref` fold to one argument — which is what lets a single declaration
say that `permission` is a Python keyword argument and a TypeScript option field
without either counting as drift.

Holding both languages to one statement forced the asymmetries out, and two rows
of the matrix moved. "Bundled Tool handler" and "Bundled Capability handler" are
one concept — a Capability a package ships — declared by `Tool.define` in
`crates/tools/cli/agent-packaging` and by `capability(...)` in
`crates/compiler/frontend/python/apxm_program/handlers.py`, not by overloading
the `Tool` and `Capability` markers that *reference* a Capability. Both return
the Capability id they implement, so the reference and the implementation are one
object. And an Agent's input and output are the declared types themselves in both
languages rather than strings naming a type in one: Python refuses a string
annotation at the marker
(`crates/compiler/frontend/python/apxm_program/_agent.py`), as TypeScript already
did through its type arguments.

### Python declares a shipped Capability, and now executes one

ADR-0016 §4 deferred the whole Python handler surface, authoring included,
because no Python bundler or worker adapter existed. That conflated two
questions. Whether a language *can declare* a shipped Capability is a question
about the authoring surface, which the manifest above states and the gate above
proves for every registered language; whether one *can run* is a question about
the packaging and execution path, which is what ADR-0016 §4 was actually
protecting. Deferring the first to protect the second left Python unable to state
something true about a package it ships, and left the gap invisible, because a
missing projection is not something a name-set comparison can see.

`apxm_program.handlers.capability(...)` now declares a shipped Capability in the
same shape `Tool.define` uses, and both return the Capability id they implement
rather than a descriptor that has to agree with a string written elsewhere
(`crates/compiler/frontend/python/apxm_program/handlers.py`).

The execution half of ADR-0016 §4 is now closed rather than deferred.
`HandlerLanguage` admits `python` and `typescript`
(`crates/machine/contracts/src/types/handler_manifest.rs`);
A Python declaration is executable. `shipped_capability_handler_ids` recognizes
`capabilities/<id>/handler.py` alongside `handler.ts`, so shipping either file
is the whole grant and a program naming it passes the grant check
(`crates/tools/cli/src/commands/agent.rs`). Both languages emit the same
`apxm.handler-manifest` descriptors and register through the same executor
chokepoint, so the permission decision, argument schema, timeout and evidence
are language-independent; only discovery, bundling and the worker process differ.

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
- ADR-0015 §4's matrix is no longer where the cross-language surface is
  decided. `contracts/vectors/apxm.frontend-surface.json` is, and a change to
  either frontend's marker arguments is a change to that manifest or a gate
  failure.
- ADR-0015 §4's "Bundled Tool handler" and "Bundled Capability handler" rows
  are one row: a Capability a package ships, declared by `Tool.define` or by
  `apxm_program.handlers.capability(...)`, never by the `Tool` or `Capability`
  markers.
- ADR-0016 §4's Python deferral is fully closed. Declaring a shipped Capability
  from Python is supported, and running one is too — `HandlerLanguage` admits
  `python`, and `capabilities/<id>/handler.py` is discovered, bundled,
  registered, and executed like its TypeScript counterpart.

## Open

Skills are declarable from both frontends, and the three gaps this section
recorded have since closed, outside this ADR.

The `Skill` marker exists in the surface manifest and in both packages.
`Skill(skill_id, entry=...)` and `Skill(skill_id, text=...)` are Python
(`crates/compiler/frontend/python/apxm_program/_markers.py`, exported from that
package's `__init__.py`); `Skill(skillId, { entry })` and
`Skill(skillId, { text })` are TypeScript
(`crates/compiler/frontend/typescript/src/markers.ts`).
`contracts/vectors/apxm.frontend-surface.json` carries `Skill` in its
`advanced` tier and binds both projections to the `SkillDecl` node that
`contracts/schemas/apxm.frontend-surface.json` admits, so a change to either
marker's arguments is a change to that manifest or a gate failure. A
declaration lowers to `SkillRequirement` on the FrontendGraph and travels into
the artifact as `skill_requirements`
(`crates/machine/program/src/frontend_graph.rs`,
`crates/machine/program/src/artifact.rs`). Loading is not a construct of its
own: `await skill.load()` records a `capability.invoke` on `read_skill`, which
`crates/tools/cli/tests/canonical_skill_execute.rs` drives end to end.

The skill discovery ids are back in
`crates/machine/ais/src/capabilities.rs` as `list_skills`, `search_skills`, and
`read_skill`, this time behind handlers in
`crates/runtime/capability/src/builtins/skills.rs` — the ordering the earlier
state inverted. And the skill contracts have a reader:
`crates/machine/program/src/skill.rs` decodes and verifies
`apxm.skill-package`, `apxm.package-local-skill`, and
`apxm.skill-discovery-root`, held to their vectors by
`crates/machine/program/tests/skill_conformance.rs`.

The package folder contract recognizes a `skills/` path now that a producer
exists, which `agent_package_recognizes_local_skill_resources`
(`crates/tools/cli/src/commands/agent.rs`) pins: a skill's instruction document
and its resources are recognized, while arbitrary package content parked under
`skills/` is not. The allowlist stayed closed until something could author one,
so the thing that consumes the path landed before the path was recognized —
the same ordering the capability ids came back from.

`agent lint` still starts its code layer from the widest thing a program could
have asked for. It reads a package as authored and compiles nothing, so it has
no authored request to state and `resolve_permission_layers`
(`crates/tools/cli/src/commands/agent.rs`) leaves every grantable id at the
`allow` floor. Nothing can widen what is already widest, so the tighten-only
arm of the stack is unreachable from lint by construction; only the
unrequested-override arm fires there.

The two callers that do hold the compiled program read the authored request.
AIR carries it as `AirModule::capability_permission_requests`, lowered from
`CapabilityRequirement.requested_permission` (`crates/machine/program/src/air.rs`,
`crates/machine/program/src/lower.rs`) and reconciled against the source bundle
the artifact digest binds (`crates/machine/program/src/artifact.rs`).
`local_capability_permissions` states it as the code layer for canonical local
execution (`crates/tools/cli/src/commands/canonical_execute.rs`), and
`compile-service-canonical` — the one caller holding both the lowered program
and the package that ships it — refuses to emit AIR when `agent.toml` widens it
(`crates/tools/cli/src/commands/compile_service_canonical.rs`).

A Python-declared shipped Capability is executable, which closes the half of
ADR-0016 §4 this record left open. The worker is a subprocess speaking the same
line protocol the TypeScript one does rather than an embedded interpreter, so a
handler body cannot hold the runtime's scheduler while it runs.

`PermissionLayer::Deployment` has no producer. `PermissionResolution::resolve`
applies the layer the moment a caller supplies one and the tighten-only rule
already covers it, but nothing in this tree states a deployment-profile
decision, so the shipped stack stops at two layers
(`crates/machine/ais/src/permissions.rs`).

## Rejected alternatives

**Edit ADR-0015 §4 and §7 and ADR-0016 §1, §3, and §4 in place, or append an
amendment section to each.** Rejected, on ADR-0020's reasoning: both are
accepted decisions with recorded `amends` provenance, and rewriting their
decision text erases what was decided when. Appending to them is the same
erasure with a heading on it, and it puts a second amendment mechanism beside
the `amends` front matter this repository already uses. The signature, the
capability definition, the frozen matrix, and the language deferral are exactly
the kind of pinned detail a later record is supposed to restate.

**Hold the cross-language surface with the prose matrix and the name-set gate.**
Rejected. A table nothing reads and a comparison of exported identifier names
cannot see the two failures that matter — a declaration one language projects
and the other does not, and two projections sharing a name while accepting
different arguments. Both had already happened when the gate was replaced.

**Keep deferring Python's authoring surface until Python can execute a
handler.** Rejected. It ties a statement about what a package contains to a
capability of the packaging toolchain, which is what left the asymmetry
unstated. The execution boundary is enforced by the contract types and the grant
check, not by withholding a declaration.

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
