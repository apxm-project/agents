# Collapsing the org package onto one authored manifest

**Status:** proposal, not implemented. The drafted contract and vectors live in
`docs/ideas/org-package-collapse/` and are deliberately *not* under `contracts/`
— a published vector with no reader fails
`crates/machine/program/tests/contract_vector_conformance.rs`.

## Why this exists

The agent package was collapsed from eleven TOML files onto one authored
manifest. `agent.toml` now carries `[hierarchy]` and `[permissions]` inline, is
modelled by `AgentToml` with `#[serde(deny_unknown_fields)]`, is validated by
the published `contracts/schemas/apxm.agent.json`, and its folder contract is
derived from that schema's `PackageFiles.patternProperties` rather than from a
hand-written Rust `match`.

The **org** package never moved. `crates/tools/cli/src/commands/org.rs` still
reads and writes the exact files the agent collapse retired:

| path | written by | read by |
|---|---|---|
| `capabilities/capabilities.toml` | `org_new` | `load_org`, `load_org_global_capabilities` |
| `capabilities/permissions.toml` | `org_new` | `load_org`, `load_org_global_capabilities` |
| `agents/members.toml` | `org_new` | `load_org` |
| `topology.toml` | `org_new` | `load_org` |

So `apxm org new` scaffolds a four-manifest tree whose shape the rest of the
repository has abandoned, and `apxm agent lint --org` joins two files to
recover a capability set that one table could state directly.

## What the collapse is

One authored file, `org.toml`:

```toml
org_id = "acme"
schema_version = "apxm.org"
display_name = "Acme"

[policy]
approval_default = "manual"
autonomy_ceiling = "supervised"

# The org's GLOBAL capability set. A capability id with a decision IS the
# declaration — there is no second inventory for it to agree with.
[permissions]
"read" = "allow"
"write" = { decision = "ask", reason = "This org reviews host writes." }

[[members]]
id = "root-agent"
package = "root-pkg"
version = "0.1.0"
[members.hierarchy]
permitted_children = ["child-agent"]

[[members]]
id = "child-agent"
package = "child-pkg"
version = "^0.2.0"
[members.capability_mask]
deny = ["write"]
[members.hierarchy]
parent = "root-agent"

[topology.tree]
root = "root-agent"
edges = [{ parent = "root-agent", child = "child-agent" }]
```

**The key move mirrors the agent collapse: the declared-vs-permitted join is
deleted, not relocated.** An agent package deleted its capability inventory
because a shipped handler's existence is the declaration. An org has no handler
to derive from, so it must name ids — but there is no reason to name them
*twice*. One `[permissions]` table, keyed by capability id, is simultaneously
the declaration and the policy, which makes `check_global_capability_join`
unreachable by construction rather than merely passing.

The one thing lost outright is `CapabilityEntry.description`. Recover it through
`PermissionDecision`'s existing `reason` field rather than reintroducing a
second declaration.

## Drafted artifacts

Both are complete and were validated before being parked:

- `org-package-collapse/apxm.org.schema.draft.json` — the published contract.
  `PackageFiles.patternProperties` admits exactly `org.toml`, `README.md`,
  `prompts/*.md`, `tests/*`. Every retired path is refused *by absence*, which
  is what makes "no space for old things" enforceable rather than aspirational.
- `org-package-collapse/apxm.org.vectors.draft.json` — 13 vectors: 2 positive,
  11 negative. **All 13 were checked against the drafted schema with
  `jsonschema` and matched their `expected_valid` exactly (0 mismatches).**
  The negatives pin the four retired paths, a retired `[[capability]]` array, a
  decision outside the closed vocabulary, a bad `schema_version`, a member
  hierarchy with an undeclared key, a member with no version requirement, and an
  unrecognized top-level file.

To land them, move both into `contracts/schemas/` and `contracts/vectors/` under
their real names and complete the census in "Wiring" below.

## Implementation plan

### 1. Share the folder-contract machinery instead of copying it

In `crates/tools/cli/src/commands/agent.rs`, extract two helpers (this was
prototyped and compiles):

- `pub(super) fn package_file_patterns(schema_json: &str, schema_id: &str) -> Vec<Regex>`
  — compiles a contract's `PackageFiles.patternProperties` keys. `agent.rs`'s
  `recognized_relpath_patterns` becomes a one-line call.
- `pub(super) fn unrecognized_files_under(root: &Path, patterns: &[Regex]) -> Result<Vec<String>>`
  — the existing walk, parameterized by predicate. `find_unrecognized_files`
  becomes a thin agent-scoped wrapper so its three call sites are untouched.

`org.rs` then gets its own `OnceLock<Vec<Regex>>` over `ORG_SCHEMA_JSON`. Copying
the body instead would be exactly the duplication the `simplify` gate exists to
catch.

### 2. Rewrite the org manifest layer

Delete `CapabilitiesToml`, `CapabilityEntry`, `PermissionsToml`,
`PermissionEntry`, `MembersToml`, and `check_global_capability_join`.

`OrgToml` gains `#[serde(deny_unknown_fields)]`, scalars before tables (so
`toml::to_string_pretty` round-trips), plus `schema_version`,
`permissions: BTreeMap<String, PermissionDecision>` (the owned enum from
`apxm_ais::permissions`), `members: Vec<MemberEntry>`, and
`topology: Option<TopologyToml>`. `MemberEntry`, `CapabilityMaskToml`,
`TopologyToml`, `TreeToml`, `EdgeToml`, `RelationToml` all gain
`deny_unknown_fields`.

`LoadedOrg` reduces to `{ root, org }`; `load_org` reads one file;
`load_org_global_capabilities` becomes
`Ok(load_org(org_root)?.org.permissions.into_keys().collect())`.

**Behavior change to note deliberately:** a missing `org.toml` now errors where
it previously yielded an empty capability set. That is correct — a directory
with no `org.toml` is not an org package — but see risk 2.

### 3. Rewrite the checks and `org new`

- `org_lint_at`: add the `schema_version` check and the unrecognized-file loop
  (mirroring `agent_lint`); drop the join check; retarget the duplicate-member
  message from `agents/members.toml` to `org.toml`.
- `check_member_resolution` and `check_hierarchy_consistency` take
  `&[MemberEntry]`; `check_capability_mask_validity` takes
  `(&[MemberEntry], &BTreeSet<String>)`.
- `check_hierarchy_consistency`'s error text cites `organization-packages.md`,
  **a document that does not exist in this repository**. Replace with
  manifest-relative phrasing. Its surrounding text is asserted by
  `lint_catches_hierarchy_contradicting_topology`, so both change together.
- `org_new` writes `org.toml`, `prompts/persona.md`, `tests/README.md` — three
  files, down from seven.

### 4. Tests

Rewrite `new_scaffolds_schema_valid_tree` and `build_valid_org` (four writes
become one). **Do not port the substring-surgery mutations**:
`lint_catches_unresolvable_member`, `_unsatisfied_version_requirement`,
`_cyclic_tree`, `_hierarchy_contradicting_topology`, and
`_invalid_capability_mask` all `read_to_string` + `replace` on files that no
longer exist in isolation, and inside one manifest the replaced substrings stop
being unique. Replace `build_valid_org` with a helper that writes a whole
`org.toml` from parameters and have each test write its own variant.

Add, mirroring the agent template: `org_vectors_match_the_lint_path` +
`lint_admits_vector`, `the_recognized_folder_contract_is_the_published_one`
(with `retired` = the four collapsed paths), and
`the_schema_version_constant_is_read_from_the_published_contract`.

### 5. Wiring outside `org.rs` — mandatory, or CI fails

1. `crates/machine/program/tests/contract_vector_conformance.rs`,
   `VECTORS_READ_ELSEWHERE`: add
   `("apxm.org.json", "crates/tools/cli/src/commands/org.rs")`. Without it,
   `every_published_vector_file_is_claimed_by_a_reader` fails.
2. `crates/machine/program/tests/semantic_conformance.rs`,
   `SCHEMAS_SPELLING_THE_DECISION_VOCABULARY`: add `"apxm.org.json"`. That test
   walks every file in `contracts/schemas/` and asserts an exact set.
3. Do **not** add `apxm.org` to `SCHEMAS_UNDER_TEST` — that table requires a
   *top-level* `properties.schema_version.const`. `apxm.agent` nests
   `schema_version` under `agent` and is deliberately excluded; `apxm.org` must
   be too.
4. `crates/tools/cli/src/commands/cli.rs`: the `--org` help text still describes
   "capabilities/{capabilities, permissions}.toml"; `OrgAction` still says
   `(apxm.org-package)`.
5. `docs/guides/composing-agent-programs.md` and
   `docs/guides/agent-package-format.md` cite `topology.toml` / `members.toml`.
6. `.dekk.toml` has **no `[commands.org]` group at all** — `apxm org` is not
   reachable through Dekk. Adding one is strictly additive.

### 6. Fixture

Add `examples/orgs/demo/` — `org.toml` + `prompts/persona.md`, **zero members**.
There is no checked-in org package anywhere today, so the contract is proved
only against tempdirs. A zero-member fixture lints clean against any
`APXM_HOME`, so it needs no installed-agent setup.

## Risks

1. **Vector execution vs. member resolution — the one real design decision.**
   `lint_admits_vector` runs the *whole* real lint. Agent vectors have no
   external dependency; org's `check_member_resolution` requires each member's
   package to be installed under `APXM_HOME`. Either have the org vector runner
   install a fixture per distinct `(package, version)` into a tempdir home
   (recommended — holds the full verdict, as agent does), or split lint into a
   home-free half and run only that (cheaper, but lets a vector claim a verdict
   the real command never produces).
2. **`agent lint --org` has zero test coverage and its contract changes.** All
   13 `agent_lint` call sites pass `None`, so `load_org_global_capabilities` is
   entirely uncovered. Add a test before changing its missing-file behavior.
3. **`toml::to_string` over a JSON array-of-tables is unproven here.**
   `lint_admits_vector` serializes the vector's manifest through toml 0.9. Org's
   `members` is an array of tables, which no existing vector exercises. Verify
   the round-trip before writing 15 vectors against it; if it fails, have the
   runner build the manifest from typed structs instead of raw JSON.
4. **Dotted capability ids must be quoted TOML keys.** `[permissions]` keyed by
   `org.http_get` becomes a nested `[permissions.org]` table unless written
   `"org.http_get" = "allow"`. Scaffolded output and every doc example must
   quote.
5. **Org has no integrity chain.** There is no `org build`, no `org verify`, no
   `integrity.toml`; `org install` is an unverified `copy_dir_recursive`. So the
   drafted schema publishes a `files` digest map that nothing computes. That
   asymmetry is accepted to keep this scoped — but adding `org build`/`org
   verify` mirroring the agent commands is the natural follow-up, and would then
   justify an org entry in `tools/scripts/agent_packages.py` and a CI step.

## Related unfinished work

**The S6 capability-resolution gate now runs for both frontends**, but proving
it *fires* for TypeScript still has no dedicated test. The layered defenses in
front of it — the typed capability catalogue in the frontend markers, and `tsc`
itself — reject the obvious bad cases before the gate is reached, so a negative
test needs a graph that passes the frontend while naming an id the package does
not supply. Reachability is currently established only indirectly, by the fact
that a successful compile must traverse it.

**Inline skills are not servable from a package root.** `Skill(id, text=...)`
produces a body that exists only in `SourceBundle`, which is digest-collapsed
before execution, and `AirModule` carries nothing skill-shaped. A package root
therefore serves `entry` skills only, and a host must still materialize inline
ones. Fixing that properly means adding `skill_requirements` to
`ExecutableArtifact` (currently `deny_unknown_fields`) and giving
`execute-canonical` an artifact input — a change to `apxm.executable-artifact`,
separate work.

**Cross-root skill id ambiguity.** `read_skill` refuses an id that resolves in
more than one discovery root, and the frontend's `Skill.load()` lowering emits
`{"skill_id": ...}` with no `root_id`, so the caller cannot disambiguate. Adding
`root_id` to the lowering is a frontend-surface and conformance-corpus change.
