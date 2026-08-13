# Shared rule — APXM storage layout

Load before changing build paths or local `.apxm` configuration.

`/home` is shared storage. Dekk owns the Cargo target directory and the
compiler environment; use `dekk agents doctor` to inspect the resolved paths.

Generated compiler, execution, session, and diagnostic artifacts belong under
the repo-local `.apxm/` directory, which is ignored by Git. Do not place them
under examples, docs, or the repository root.

The scoped APXM configuration resolver is first-wins, not a merge. Read both
the project-local and user configuration before changing either one.
