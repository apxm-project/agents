---
name: apxm-design-docs
group: Domain
description: Use when editing conceptual docs under docs/design/. Gates two shipped failure modes — overclaim (present-tense prose about unwired behaviour) and citation drift (claims with no anchor to shipped code).
user-invocable: true
---

# APXM Design Docs

Load `_shared/apxm-development-rules.md` before broad work.

Use this skill when editing any document under `docs/design/`. Its job is to
keep conceptual docs honest: every factual claim about runtime/compiler/server
behaviour cites a specific file or commit, and aspirational/in-flight sections
are labelled as such rather than written in the present tense.

## What this skill does (deterministic — no LLM call)

1. **Diff parse** — read the proposed edit against the file on disk; identify
   added/modified sentences and section headers.
2. **Tense + status check** — every present-tense factual claim about
   runtime/compiler/server behaviour must carry a citation. Sections describing
   aspirational or in-flight work must carry an explicit `Status: design`
   label; otherwise flag them as overclaim candidates.
3. **Citation resolution** — for each cited path/commit, confirm it exists on
   the current branch. Stale citations are flagged.
4. **Cross-doc consistency** — if the edit changes a claim other `docs/design/`
   files repeat, surface those neighbours so the correction fans out.

## Rules

- Every factual claim about shipped behaviour cites a path or commit.
- Aspirational sections are labelled explicitly, not implicitly.
- The skill **gates** the operator's edit and surfaces failure modes; it does
  **not** write the doc edit.

## Anti-patterns

- Present-tense prose for behaviour that isn't wired yet (overclaim).
- A factual claim with no anchor to shipped code.
- Citing a path/commit that no longer exists on the branch.

## See also

- `apxm-mcp-server`, `apxm-mlir-pass-development` — the subsystems most often
  over-described in design docs.
