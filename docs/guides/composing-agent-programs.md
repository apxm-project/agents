# Compose Agent Programs

- Architectural status: canonical target guide
- Frontend syntax status: `.new(...)` and `.invoke(...)` are captured by both
  frontends (`crates/compiler/frontend/typescript/src/capture.ts`,
  `crates/compiler/frontend/python/apxm_program/_capture.py`) and lower to
  `program.new` / `program.invoke`, per
  [ADR-0008](../adr/0008-agent-programs-compose-through-new-and-invoke.md)
- Normative contract: [composition and AIR](../agents/agent-program-composition-and-air-contract.md)

## 1. Three explicit calls

- `Agent.invoke(input)` runs a one-shot or independently admitted child.
- `Agent.new(context=...)` creates a stateful Program Instance.
- `instance.invoke(input)` invokes that instance.

These are the only Agent Program composition concepts. “Spawn,” “handoff,”
“delegate,” “workflow,” and runtime Agent routing are not alternate semantics.

## 2. One-shot specialist

```python
review = await SecurityReviewer.invoke(ReviewRequest(diff=diff))
```

```typescript
const review = await SecurityReviewer.invoke({ diff });
```

`SecurityReviewer` is a statically imported typed `ProgramRef`. The host admits
the exact child artifact and Agent Identity. The parent receives the declared
plain result, not the child's private Context or authority.

## 3. Stateful specialist

```python
specialist = AccountSpecialist.new(
    context=AccountContext(account_id=request.account_id),
)
result = await specialist.invoke(request)
```

```typescript
const specialist = AccountSpecialist.new({
  context: { accountId: request.accountId },
});
const result = await specialist.invoke(request);
```

The opaque `ProgramInstanceRef` may be retained only according to its typed
Context contract. The instance is single-flight and fail-busy. A yield keeps
its continuation; return completes it.

## 4. Choose among known programs

```python
choice = classify_request(request)
match choice:
    case Specialist.SECURITY:
        return await SecurityReviewer.invoke(request)
    case Specialist.FINANCE:
        return await FinanceReviewer.invoke(request)
```

The closed enum may come from pure logic, a model call or an admitted
Capability. It cannot contain an arbitrary program name. Source owns the
branch, so the compiler and an authorized host can explain it.

## 5. Hierarchy, skills and authority

A parent/child Program relationship is execution composition, and the three
calls in §1 are the only things that invoke. It is a different thing from the
`[hierarchy]` table a package declares: `parent` and `permitted_children` in
`agent.toml` are packaging topology, checked for consistency against an org's
`topology.toml` by `apxm org lint`
(`crates/tools/cli/src/commands/org.rs`). No membership, edge, or declared
parent causes an invocation.

Skills are not something an Agent Program declares, associates, or inherits.
There is no `Skill` marker in either frontend — the surface manifest carries no
skill declaration, so a program cannot author one. What a program can do is
*read* one through an admitted Capability: `list_skills`, `search_skills`, and
`read_skill` are builtin ids (`crates/machine/ais/src/capabilities.rs`)
implemented in `crates/runtime/capability/src/builtins/skills.rs`, resolving
against discovery roots the composition root configures. That root list is empty
by default, so nothing scans whatever happens to sit near the process.

Skill access is therefore an admitted grant to one invocation, not an attribute
travelling down a parent/child edge. A child that can read skills can do so
because its own admission says so; nothing copies from the parent, and the
program decides explicitly which discovered content reaches a model. The child
also has its own Agent Identity and receives an explicitly attenuated grant. The
parent may pass typed input and explicitly projected context, but cannot
transfer secrets, grants, prompt history or filesystem access automatically.

## 6. Failure and evidence

Parent and child have distinct Program Invocations linked by causal evidence.
Cancellation propagates only according to the structured task contract.
Idempotency, retries and outcome-unknown handling remain per exact effect.
An authorized host can traverse parent → child → node while preserving each identity,
authority, budget, usage and Session Output folder.
