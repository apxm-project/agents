---
name: simplify
group: Lifecycle
description: Pre-finish review pass — remove copied _shared text, weak abstractions, referential comments, and over-large skill bodies before claiming completion. Mandatory before finish and any commit.
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

In changed code, the public path is `dekk agents <command>`. If new
code surfaces a non-Dekk command for normal use, fold it into
`.dekk.toml`.

### 5. Challenge abstractions

For every new trait, interface, generic, or wrapper: does it protect
a real boundary, or is it speculative? Three similar lines beat a
premature abstraction.

### 6. Can this be split?

If the change spans subsystems (compiler + runtime + zoo), ask
whether two smaller PRs would be more reviewable.

### 7. Comments follow the comment rule

Comments must match `_shared/apxm-comment-rules.md`: default to none,
justify *why* not *what*, and no referential comments (no `// for
plan04`, `// from issue #123`, `// see X.md`, ticket refs). Strip any
that crept in — the commit message owns "why now" and "by whom".

### 8. `.agents.json` contract

If the change adds or changes any skill, run
`dekk agents check-agent-skills` to confirm every skill directory validates.

### 9. Generated artifact placement

Confirm any new artifact path is under `.apxm/`. See
`_shared/apxm-storage-layout-rules.md`.

## Output

Short summary listing each simplification applied, plus any deferred
items kept as follow-up tasks.

## Next step

Hand off to `finish`.
