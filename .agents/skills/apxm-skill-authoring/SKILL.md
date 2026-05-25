---
name: apxm-skill-authoring
description: Use when adding or editing an APXM agent skill under .agents/skills/. Enforces SSOT pattern, _shared/ pointers (never inlined text), ≤100 lines per SKILL.md, and dual Claude+Codex compatibility.
user-invocable: true
---

# APXM Skill Authoring

Load `_shared/apxm-development-rules.md` before broad work.

## SSOT pattern

- `.agents/project.md` is the SSOT for project-wide guidance.
- `.agents/skills/_shared/*.md` are the shared rules.
- `.agents/skills/<name>/SKILL.md` is each skill — **a thin
  orchestrator that points at `_shared/` rules**, not a body of
  duplicated content.
- `AGENTS.md` / `CLAUDE.md` / `.agents.json` / `.cursorrules` /
  `.github/copilot-instructions.md` are **generated** from
  `.agents/project.md` by `dekk apxm skills generate`. Never edit
  them by hand.

## Authoring rules

1. **Frontmatter** (required):
   ```yaml
   ---
   name: <kebab-case skill name>
   description: <one-sentence agent-discoverable summary>
   user-invocable: true   # optional — surfaces as a slash command
   ---
   ```
2. **Body ≤100 lines.** If it grows past, move details to `docs/` or
   `.agents/domains/<area>/README.md`.
3. **Open with `Load _shared/<rule>.md before broad work.`** for every
   relevant `_shared` rule. Do not inline rule text.
4. **No referential content.** No "for plan04", "added by user
   request", "see ticket #N". The skill is for future agents, not a
   record of past work.
5. **List authority commands**, not raw `cargo`/`docker`/`srun`.
6. **List anti-patterns** — the project's hard-won lessons.
7. **Dual-compatibility**: the skill must work for Claude (via the
   plugin or skills/ directory) **and** for Codex (after
   `dekk apxm skills sync` syncs to `~/.codex/skills/`). Avoid
   Claude-only idioms in the body.

## Skill template

```markdown
---
name: apxm-<name>
description: <summary>
user-invocable: true
---

# APXM <Display Name>

Load `_shared/<rule>.md` before broad work.

## What this skill does
<bullet steps>

## Rules
<bullets>

## Anti-patterns
<bullets>

## See also
- <other skill names>
```

## Adding a new skill — workflow

1. Read 2–3 existing skills for style.
2. Create `.agents/skills/<name>/SKILL.md` from the template.
3. Run `dekk apxm skills status` — confirm registration.
4. Run `dekk apxm skills generate --target all` — regenerate
   `AGENTS.md`, `CLAUDE.md`, `.agents.json`, `.cursorrules`,
   `.github/copilot-instructions.md`.
5. Commit `.agents/skills/<name>/SKILL.md` + the regenerated outputs
   in one commit.

## When a skill should NOT exist

- One-off tasks that won't recur.
- "Just" wrapping a single command (the command alone is fine).
- Anything that duplicates `_shared/` content.
- Anything that duplicates an existing skill — extend the existing
  one instead.

## Anti-patterns

- Inlining `_shared/` text into a skill body.
- Skills longer than ~100 lines.
- Skills that reference other agents by name ("ask Claude to…").
- Skipping `dekk apxm skills generate` after editing — `AGENTS.md`
  and `CLAUDE.md` drift silently.
