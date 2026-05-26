# The APXM skill-library model

## Why this doc exists

APXM ships compiled, hash-pinned skill artifacts. A reader asking *what
is a skill library, in APXM terms?* needs one canonical answer that
holds together the producer side (the compiler, the wire format, the
manifest), the transport side (the pack, the hash chain, the registry),
and the consumer side (the loader, the resolver, the call surface).
This doc is that answer.

The frame that makes the design legible is **classical software
libraries**. A programmer writes `.h`/`.c`/`.py`, the toolchain
produces `.o`/`.a`/`.so`, and a linker plus a loader makes those
objects callable by name from other code. APXM has most of the same
pieces but with different names, and the gaps are exactly where the
analogy is incomplete. Reading the rest of this doc as the APXM
column of that table is the fastest way to get oriented.

## What plays the role of each classical artifact

| Classical                | APXM equivalent                                | Notes                                                                                                                                                                                                                                  |
|--------------------------|------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `.h` (header)            | `skill.toml` + `SKILL.md` frontmatter          | `apxm_skill::SkillManifest` declares `skill_id`, `version`, `entry_flow`, `inputs`, `outputs`, `required_capabilities`, `allowed_tools`, and the three-layer hash chain.                                                               |
| `.c` / `.cpp`            | `skill.air` (canonical AIR JSON)               | Compiled by `apxm_driver::Compiler` through the standard pass pipeline.                                                                                                                                                                |
| `.py` (source)           | `apxm` Python frontend (decorator DSL → AIR)   | Frontend emits AIR. Consumer-side import surface lives at `apxm.libs.load(skill_id)`; the producer surface is the decorator DSL.                                                                                                       |
| `.o` (object)            | `skill.apxmobj` (wire format)                  | `APXM` magic + version 1 + BLAKE3-of-payload + bincode `ArtifactPayload { metadata, dags: Vec<WireDag>, sections }`. Multi-DAG capable: `is_entry: true` marks the export.                                                              |
| ELF `.note` / debug info | `ArtifactSection { kind, data }` typed sections | `section_kinds::SKILL_MANIFEST_V1 = "apxm.skill_manifest.v1"` is the canonical kind for the embedded manifest. `apxm-server` validates the embedded manifest against the on-disk `skill.toml`.                                          |
| `.a` (archive)           | `packs/<pack-id>/skills/<skill-id>/`           | v1 = one skill per pack. The wire format already permits multiple flows per artifact; the pack layout already permits multiple skills per pack — the singleton is a v1 convention, not a format limit.                                |
| `ld` (linker)            | Cross-skill name resolution                    | Lazy: resolution happens at execution time against the live `SkillLibrary`. There is no separate static-link archive step. `apxm_driver::Linker` is the compile+execute orchestrator, not the cross-artifact name resolver.            |
| `ld.so` (loader)         | `apxm-server` `SkillLibrary.scan()` + `find()` | `find(requested_id)` accepts `<id>@<version>`; `find_executable()` returns an `ExecutableSkill` that a runtime handler invokes.                                                                                                        |
| Symbol export            | `ExecutionDag.is_entry = true` (one per skill) | The loader returns the entry DAG; non-entry DAGs are reachable today only by flow name within the same artifact. v1 keeps one entry per artifact (see the singleton convention below).                                                  |
| Cross-artifact call      | `FLOW_CALL` (in-session) + `call_skill` (across artifacts) | `FLOW_CALL` (`MAX_FLOW_CALL_DEPTH = 100`) dispatches by agent+flow inside a session. Cross-artifact dispatch by manifest identity is the `call_skill` AIS op; resolved-`(skill_id, version, artifact_hash)` is recorded in provenance. |
| `python -c "import foo"` | `from apxm.libs import load`                    | A `load(skill_id)` call resolves through `apxm-server` so capability admission and provenance records stay centralized. `SkillHandle.invoke(**kwargs)` posts to `/v1/skills/{id}/execute` and unpacks the session result.              |
| `sha256sum`              | Three-layer hash chain (BLAKE3)                | Wire-internal payload hash (`Artifact::write_to`/`read_from`); per-skill `skill.toml.artifact_hash`; per-pack `pack.toml.pack_hash`. Each layer has a single canonical writer and is verified independently at install and at load.    |

## Resolved decisions

The decisions below are the load-bearing v1 commitments. Each one is
the result of evaluating alternatives that the table above could have
admitted; document new evidence before reopening them.

### Lazy linking

Cross-artifact dispatch resolves at execution time. The runtime
handler for the cross-skill call op asks the live `SkillLibrary` for
the artifact whose manifest identity matches the requested
`skill_id` (or `skill_id@version`), invokes it, and records the
resolved `(skill_id, version, artifact_hash)` triple in the parent
session's provenance. There is no separate "link" artifact step
and no on-disk symbol table that pins names to artifact hashes at
build time.

The cost of lazy resolution is one hash lookup per call against an
in-memory map. The benefit is that the producer toolchain stays
simple: there is no static-link format to design, no archive layout
to version, no separate verifier for cross-artifact symbol tables.
Reproducibility is preserved at the provenance layer instead of at
the artifact layer — replaying a parent session against the same
library state produces the same resolved hashes.

Static linking is a candidate future optimization if profiling shows
that resolution cost matters. It is not in v1.

### O2 canonical opt level

Every shipped pack is compiled at `OptimizationLevel::O2`. O2 is
the proven-deterministic level: the compiler emits byte-stable
artifacts across repeated invocations on the same input. Determinism
is the precondition for tamper detection — a pack's
`skill.toml.artifact_hash` must byte-match the `.apxmobj` on every
consumer install, and the install transport's tamper detection
depends on that match.

O3 is faster on hot paths but is not deterministic-checked and
disables the verifier in places. A per-pack
`pack.toml [compile].opt_level = "O3"` override remains available
for packs whose author has profiled the win and is willing to ship
per-architecture artifacts or to accept rebuild-on-install. The
override is not used by any v1 pack.

### Embedded manifest section `apxm.skill_manifest.v1`

The compiled `.apxmobj` carries its own manifest in a typed
artifact section, kind `apxm.skill_manifest.v1`. `apxm-server`
validates the embedded manifest field-for-field against the
on-disk `skill.toml` at load time and refuses to load on any
mismatch. The embed mechanism reuses the same section API the
compiler already uses for sidecar payloads such as Python tool
bundles — no new wire path was introduced.

The result is that the `.apxmobj` is self-describing: a tampered
install whose `skill.toml` has been edited to match the tampered
artifact is still caught at load, because the embedded section
carries the manifest the artifact was compiled against, and that
copy is integrity-protected by the wire-internal BLAKE3.

### Three-layer hash chain with canonical writers

Three hashes cover three different scopes:

| Hash                          | Covers                                  | Writer                                  | Verifier                                  |
|-------------------------------|-----------------------------------------|-----------------------------------------|-------------------------------------------|
| Wire-internal BLAKE3          | `ArtifactPayload` bytes                 | `Artifact::write_to`                    | `Artifact::read_from` on every load       |
| `skill.toml.artifact_hash`    | `.apxmobj` file bytes                   | The pack-compile workflow (canonical)   | `apxm-libs` install + verify path         |
| `pack.toml.pack_hash`         | The shipped tarball bytes               | The pack-compile workflow (canonical)   | `apxm-libs` install: pre-unpack gate      |

The contract is that the pack-compile workflow is the **single
canonical writer** for `artifact_hash` and `pack_hash`. The
maintainer never types or copy-pastes either value. The install
path verifies the pack hash before unpacking, the artifact hash
after unpacking, and the wire-internal hash on every load. A
mismatch at any layer fails clean with
`(pack_id, skill_id, expected, actual)` — never a panic, never a
silent skip.

### No static-link archive format in v1

A static-link archive — a single shipping unit that bundles a
parent skill together with the artifact hashes of every child it
calls — is plausible but unnecessary at v1. Lazy resolution plus
provenance recording gives reproducibility. A static archive only
makes sense if profiling shows resolution cost dominates or if a
distribution channel needs an offline-self-contained bundle. Either
is a future-phase question.

### v1 singleton convention

A pack ships **one skill**; an artifact ships **one entry DAG**.
The wire format already supports multiple flows per artifact
(`ArtifactPayload.dags: Vec<WireDag>` with `is_entry` selecting
the export) and the pack layout already supports multiple skills
per pack. v1 keeps the singleton for two reasons: it makes the
manifest-versus-artifact correspondence one-to-one (simpler hash
chain, simpler `find()` semantics), and it keeps the maintainer
mental model close to the classical-library analogy (one `.c`
file becomes one `.o`).

A v3 multi-skill or multi-entry pack format is not blocked by the
wire format; it is blocked by a deliberate choice. Toolchain
assumptions baked in at v1 must not foreclose that future. The
canonical writer for the hash chain, the embedded-manifest section,
and the loader's `find()` should all stay structured so that
adding `Vec<SkillManifest>` and `Vec<is_entry>` later is additive,
not breaking.

## What this doc does not cover

The runtime mechanics of the cross-skill call op (attribute list,
admission rules, depth limit, failure modes) live in the op's own
reference page when that op lands. The pack format on disk
(directory layout, required files, schemas) lives in the
`apxm-libs` repo's `pack-format.md` when that repo ships its docs
tree. The producer's authoring guide and the consumer's usage
guide also live in `apxm-libs/docs/`. This doc is the bridge
between the runtime backlog and those downstream docs — it names
the pieces and pins down the v1 decisions; the downstream docs
implement them.
