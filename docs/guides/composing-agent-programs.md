# Compose Agent Programs

- Architectural status: canonical target guide
- Frontend syntax status: design proposal aligned with
  [Author an Agent](creating-an-agent-program.md)
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

`SecurityReviewer` is a statically imported typed `ProgramRef`. Server admits
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
branch, so compiler and Studio can explain it.

## 5. Hierarchy, Skills and authority

A parent/child Program relationship is execution composition, not Company
hierarchy. An Area, Department or Group membership never causes invocation.

A specialist has its own Skill associations even when its parent does not.
Those Skills remain discovery-only. The child also has its own Agent Identity
and receives an explicitly attenuated grant. The parent may pass typed input
and explicitly projected context, but cannot transfer all Skills, secrets,
grants, prompt history or filesystem access automatically.

## 6. Failure and evidence

Parent and child have distinct Program Invocations linked by causal evidence.
Cancellation propagates only according to the structured task contract.
Idempotency, retries and outcome-unknown handling remain per exact effect.
Studio can traverse parent → child → node while preserving each identity,
authority, budget, usage and Session Output folder.
