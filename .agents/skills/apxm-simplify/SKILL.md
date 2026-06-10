---
name: apxm-simplify
description: Pre-finish review pass — remove copied _shared text, weak abstractions, referential comments, and over-large skill bodies before claiming completion. Mandatory before apxm-finish and any commit.
user-invocable: true
---

# APXM Simplify

A focused review pass before claiming completion. The ironbear pattern:
agents tend to leave behind copied rule text, defensive scaffolding,
referential comments, and overdrawn abstractions. Strip them now.

## What this skill checks

### 1. No copied `_shared/` text

Skills must point at `_shared/<rule>.md`, not inline the rule's text.
Replace any inlined paragraph with:

```
Load `_shared/<rule>.md` before broad work.
```

### 2. Prefer one global skill + thin local pointer

If two skills describe the same workflow, extract the common body to
a shared skill (or `_shared/` rule). Don't duplicate workflows.

### 3. Keep skill bodies concise

Target ≤100 lines per `SKILL.md`. Move details to `docs/`,
`.agents/domains/<area>/README.md`, or a `_shared/` rule.

### 4. Dekk is the public interface

In changed code, the public path is `dekk apxm <command>`. If new
code surfaces a non-Dekk command for normal use, fold it into
`.dekk.toml`.

### 5. Challenge abstractions

For every new trait, interface, generic, or wrapper: does it protect
a real boundary, or is it speculative? Three similar lines beat a
premature abstraction.

### 6. Can this be split?

If the change spans subsystems (compiler + runtime + zoo), ask
whether two smaller PRs would be more reviewable.

### 7. No referential comments

Per `feedback_no_referential_comments`: no `// for plan04`,
`// from issue #123`, `// per the user's request`, `// added by
apxm-execute-plan`, or `// see X.md`. The commit message owns "why
now" and "by whom".

### 8. Comment discipline

Default to no comments. Only justify *why*, never *what*. If the
identifier name is good, a comment usually isn't needed.

### 9. `.agents.json` contract

If the change adds or changes any skill, run
`dekk apxm skills status` to confirm generated agent surfaces resolve.

### 10. Generated artifact placement

Confirm any new artifact path is under `.apxm/`. See
`_shared/apxm-evaluation-rules.md`.

## Output

Short summary listing each simplification applied, plus any deferred
items kept as follow-up tasks.

## Next step

Hand off to `apxm-finish`.
