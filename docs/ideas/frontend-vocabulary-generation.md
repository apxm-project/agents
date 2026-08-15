# Generating the source-first frontends from their contract

**Status: partly landed, mostly design.** The vocabulary projection described in
§3 is shipped and drift-gated; §4–§6 specify work that is *not* wired and must
not be read as describing current behaviour.

## 1. The problem this addresses

Both source-first authoring frontends implement `apxm.frontend-graph` by hand,
and neither reads it. `crates/compiler/frontend/python/apxm_program/` and
`crates/compiler/frontend/typescript/src/` contain no reference to any file
under `contracts/`; the only contract text either package carries is inlined at
codegen time by Rust, in `crates/tools/cli/src/frontend/codegen.rs` and
`crates/tools/cli/src/frontend/codegen_ts.rs`, which `include_str!` the
runtime-evidence schema.

The consequence is that every closed set the contract states is re-typed as bare
string literals in two languages. Before the change in §3, the Hook scope set
existed three times — in `contracts/schemas/apxm.frontend-graph.json`
(`$defs/HookBinding.properties.scope.enum`), in Python at
`apxm_program/_advanced.py`, and in TypeScript at `src/capture.ts` — with the
Rust consumer enum `HookScope` in
`crates/machine/program/src/frontend_graph.rs:35` as a fourth. The FrontendGraph
value vocabulary survived in Python only as trailing comments on
`_bound_tree.py`'s dataclass fields, which no tool can check.

That drift fails late and unhelpfully: `FrontendGraph` decodes in Rust with
`deny_unknown_fields` (`crates/machine/program/src/frontend_graph.rs:11`), so a
member one frontend invents is a compile-time rejection of the author's program
with no line pointing at the frontend that invented it.

## 2. Where each vocabulary's single source of truth lives

Two decisions, both now expressed in code:

**The FrontendGraph closed sets are owned by the published schema**,
`contracts/schemas/apxm.frontend-graph.json`, not by the Rust consumer enums.
Rust is one consumer of that contract among three, and
`crates/machine/program/tests/semantic_conformance.rs` already pins Rust enums
against schema `enum` members using the `schema_enum` helper in
`crates/machine/program/tests/common/mod.rs:103` — the same access path the
generator uses. Sourcing the frontends from the schema makes schema, Rust,
Python, and TypeScript resolve one set rather than four.

**The authoring diagnostic codes are owned by the frontend-surface manifest**,
`contracts/vectors/apxm.frontend-surface.json`, whose
`declarations[].diagnostics` name the rejection reasons each public marker can
raise. ADR-0015 §7 (`docs/adr/0015-source-first-agent-frontend-vocabulary.md:186`)
freezes that manifest as the one machine-readable statement of the authoring
surface, "the diagnostics it can raise" included; the ADR names the schema
(`contracts/schemas/apxm.frontend-surface.json`, which types `diagnostics` as an
array of strings) and `tools/scripts/check_frontend_surface.py` reads the
instance under `contracts/vectors/` and holds each frontend's generated
diagnostic module to it.

A Rust enum was considered for the diagnostic codes and rejected: no Rust code
raises them. They are source-capture rejections raised in Python and TypeScript,
and they are a different vocabulary from
`apxm_program::diagnostic::DiagnosticCode`
(`crates/machine/program/src/diagnostic.rs`), which is the graph/AIR/source-map
*verification* set and is raised only by Rust. Minting a Rust enum would have
been a third spelling that nothing reads.

## 3. What landed

Two generators, both following `crates/tools/cli/src/frontend/codegen_capabilities.rs`
in shape: a render function per language, a `CodegenAction` arm that writes or
drift-checks both halves, and in-crate tests that measure the generator rather
than the checked-in file.

### 3.1 `codegen frontend-vocabulary`

`crates/tools/cli/src/frontend/codegen_frontend_vocabulary.rs` parses the
schema at render time and projects every closed string set it states:

| Emitted type | Schema path |
| --- | --- |
| `SourceLanguage` | `properties.source_language.enum` |
| `DeclKind` | `$defs.Declaration.properties.decl_kind.enum` |
| `ParameterRole` | `$defs.Parameter.properties.role.enum` |
| `ValueOrigin` | `$defs.Value.properties.origin.enum` |
| `ValueExpressionKind` | `$defs.ValueExpression.oneOf[*].properties.kind.const` |
| `RegionRole` | `$defs.Region.properties.region_role.enum` |
| `IntentKind` | `$defs.CallIntent.properties.intent_kind.enum` |
| `ReceiverKind` | `$defs.CallIntent.properties.receiver_kind.enum` |
| `ControlKind` | `$defs.ControlIntent.properties.control_kind.enum` |
| `PredicateComparator` | `$defs.ControlPredicate.oneOf[*].properties.comparator.const` |
| `PredicateScalarType` | `$defs.PredicateLiteral.oneOf[*].properties.scalar_type.const` |
| `HookScope` | `$defs.HookBinding.properties.scope.enum` |
| `HookPhase` | `$defs.HookBinding.properties.phase.enum` |
| `HookReturnMode` | `$defs.HookBinding.properties.return_mode.enum` |

The table lives in the `FAMILIES` constant; adding a row is the whole cost of
projecting a set the contract gains.

Outputs are `apxm_program/_generated/frontend_graph.py` and
`typescript/src/generated/frontend-graph.ts`. Per family the shape is a type
alias over the literal union, one prefixed constant per member, and a tuple or
`as const` array naming every member — `HOOK_SCOPE_NODE`, `HOOK_SCOPES`,
`HookScope`. Constants are prefixed because the sets overlap: `context` is both
a `DeclKind` and a `ParameterRole`, and an unprefixed constant would resolve to
whichever vocabulary was emitted last.

`PermissionDecision` is stated by the same schema and deliberately *not*
emitted: `crates/tools/cli/src/frontend/codegen_permissions.rs` already owns it,
and a test asserts it stays absent here.

### 3.2 `codegen diagnostics`

`crates/tools/cli/src/frontend/codegen_diagnostics.rs` unions
`declarations[].diagnostics` from the manifest in manifest order with repeats
dropped, and emits `DiagnosticCode`, one `SCREAMING_SNAKE` constant per code,
and `DIAGNOSTIC_CODES`, into `apxm_program/_generated/diagnostics.py`,
`typescript/src/generated/diagnostics.ts`, and — because the manifest projects
TypeScript's shipped-handler declaration into the packaging package —
`crates/tools/cli/agent-packaging/diagnostics.mjs`. The codes the manifest
declares — `AgentBodyNotAsync`, `HookTargetUnresolved`, and the rest — existed
nowhere but that manifest before this, and
`tools/scripts/check_frontend_surface.py` now proves each one reaches a
`raise`/`throw` in every registered language rather than only existing.

### 3.3 Where these modules sit

Neither is authoring surface. Unlike `apxm_program/capabilities.py` and
`typescript/src/capabilities.ts`, which exist to give authors a stable import
path, these are frontend internals: no public shim, no `package.json` export
entry, and no binding in either package root. `tools/scripts/check_frontend_surface.py`
holds the Python root to exactly the manifest's names, so a root-bound
re-export here would fail that gate rather than pass unnoticed.

### 3.4 Drift gates

- `codegen frontend-vocabulary --check` and `codegen diagnostics --check` are in
  `.dekk.toml`'s `check` and `check-frontend-codegen` chains alongside the
  existing arms, plus their own `codegen-frontend-vocabulary` and
  `codegen-diagnostics` entries.
- `apxm_program/_generated/` gained the stray-file rejection the TypeScript
  directory already had. `GENERATED_PYTHON_FRONTEND_FILES` in
  `crates/tools/cli/src/frontend/codegen.rs` names every module the directory
  may hold; `check_generated_named_files` in
  `crates/tools/cli/src/commands/codegen.rs` rejects any other `.py` file there.
  The same function serves both directories, taking the extension as a
  parameter.
- `_generated/__init__.py` is itself generated from that owned-file list, so it
  re-exports every generated sibling. It previously named only
  `runtime_evidence`, which is how `capabilities` and `permissions` came to be
  reachable only by their private module path.
- Both generators hold their output against the manifest's own `non_public`
  list, read from the manifest rather than restated, so tightening the manifest
  tightens the generators. Membership is tested token-wise, not by substring:
  `REGION_ROLE_LOOP_BODY` contains the characters of `OP_` without naming an
  operation.

### 3.5 Call sites converted

`_advanced.py` and `advanced.ts` build Hook declarations from the generated
scope, phase, and return-mode constants; the hand-kept five-member scope sets in
`_advanced.py` and `src/capture.ts` are replaced by `HOOK_SCOPES`. The hook
scope comparisons in `_capture.py`'s `_hook_target` and `capture.ts`'s
`resolveHookTarget` bind the constants too, and `resolveHookTarget` now takes
`HookScope` rather than `string`.

## 4. Design: generating the record types

**Landed.** The contract records are generated into
`_generated/frontend_records.py` and `generated/frontend-records.ts` by
`codegen_frontend_records.rs`, and `capture.ts`'s inline `CallRecord` /
`ControlRecord` types are gone. `_bound_tree.py`'s `BoundCall` and
`BoundControl` compose a generated record with their authoring-time extras;
the other bound-tree records stay hand-written, as item 2 describes.

The bound tree is not a one-to-one projection of the graph: `BoundCall` carries
a `span` and `operands` with slot identity, while `$defs/CallIntent` carries
`operand_values` and leaves slot identity to `data_edges`. Generating the bound
tree wholesale would therefore either lose the frontend's authoring-time extras
or push them into the contract.

The split that resolves it:

1. **Generate the contract records.** For each `$defs` object with `type:
   object`, `required`, `properties`, and `additionalProperties: false`, emit
   one record type named for the `$defs` key. Field typing:
   - `$ref` to `apxm.contract-common.v1#/$defs/Identifier` or `.../Digest` →
     `str` / `string`.
   - `type: integer` → `int` / `number`; `type: boolean` → `bool` / `boolean`.
   - `type: array` of `Identifier` → `tuple[str, ...]` / `readonly string[]`.
   - a property with an `enum` or a discriminating `const` → the §3.1 type alias
     for that schema path, resolved through the same `FAMILIES` table.
   - `$ref` to a local `#/$defs/Y` → the generated record for `Y`.
   - a `oneOf` discriminated by a `const` property → a union of one record per
     branch.
   - a property in `required` → a positional field; otherwise `Optional[T] =
     None` / `readonly f?: T`.
2. **Keep the authoring extras hand-written.** `Span` and `BoundOperand` have no
   contract counterpart and stay as they are. `BoundCall` and `BoundControl`
   become hand-written records that carry a generated contract record plus their
   `span` and `operands`.
3. **Introduce the missing TypeScript layer.** TypeScript has no bound tree at
   all today — `capture.ts` builds wire-shaped objects inline. `src/bound-tree.ts`
   must mirror `_bound_tree.py` before the generated records can serve both
   languages, and that mirroring is what makes the two frontends' parity
   structural rather than reviewed.

## 5. Generating the emitter

Most of the old `_emit.py` was one rule applied once per record: build a dict
with every required key, then add each optional key if and only if its source
value is not `None`. `_declaration`, `_value`, `_region`, `_hook`, and
`_capability_requirement` were five hand-written instances of it. That rule is
fully determined by the schema's `required` and `properties`, so
`crates/tools/cli/src/frontend/codegen_frontend_serializers.rs` emits a
`serialize_<record>` function per record type beside the §4 record types, into
`apxm_program/_generated/frontend_serializers.py` and
`typescript/src/generated/frontend-serializers.ts`. Keys land required-first in
`required` order, then the rest — which pins the ordering to the contract rather
than to whichever language's dict happened to iterate first.

A `$defs` entry stated as a `oneOf` gets a dispatcher instead: it reads the
discriminant the contract states for that union — resolved through the same
`FAMILIES` table §3.1 projects — and delegates to the branch serializer. Python
reads a field through a `_field` helper that accepts a generated record or the
equivalent wire mapping, because the bound tree still spells a value expression
and a predicate literal as the plain mapping the contract states (§4 item 2).

Two parts of the emitter are frontend policy, not contract projection, and stay
hand-written in both languages:

- `_blocks()` / `blocks()`, which places a yielded resume value in its enclosing
  region's block so the continuation boundary is explicit to Rust lowering.
- the `region_annotations` derivation, which marks the first body region of a
  `loop` control intent `structural_loop`.

`capture.ts` split to follow: the AST walk and marker ergonomics stay in
`src/capture.ts`, which now builds the `src/bound-tree.ts` `BoundProgram` that
§4 item 3 introduced, and the fold moved to `src/emit.ts` mirroring `_emit.py`
function for function. Both languages' folds are now calls into the generated
serializers plus those two policy functions.

One ordering changed with the split, in TypeScript only: `data_edges` were
emitted in AST-walk order, interleaving call and control edges, and are now
grouped call-edges-then-control-edges as `_emit.py` has always grouped them. The
set of edges is unchanged, and `test_paired_corpus_covers_every_public_operation_and_structural_family`
already compared the two languages' complete AIR while those orders differed, so
the lowering does not read it. The one visible consequence is
`AgentHandle.artifactDigest`, which is `stableDigest(JSON.stringify(graph))` and
so moves whenever key order does; it is language-local, and the parity
projections exclude it for that reason.

## 6. Design: wiring the diagnostic codes

**Not implemented.** The vocabulary from §3.2 exists; no raise site binds it
yet. Python's `CaptureError` (`_capture.py:45`) and TypeScript's
(`capture.ts:136`) are still untyped, and their 39 `raise CaptureError` and 32
`throw new CaptureError` sites carry free text only.

The wiring:

1. `CaptureError` takes a required `code: DiagnosticCode` as its first argument
   in both languages. There is no default — a defaulted code is an open set
   wearing a closed set's name, which is exactly what
   `crates/machine/program/src/diagnostic.rs` refuses for the verification
   vocabulary.
2. Each raise site names its code. Messages are unchanged, so the codes are
   additive to what a reader already sees.
3. Sites whose reason the manifest does not yet name — the value-expression
   subset rejections, the safe-integer domain check, the spread rejections — add
   a code to `declarations[].diagnostics` for the marker that raises them.
   `contracts/schemas/apxm.frontend-surface.json` already types `diagnostics` as
   an array of strings, so no schema change is needed; the reviewer sees which
   marker gained a rejection reason, and regeneration mints the symbol.
4. `tools/scripts/check_frontend_surface.py` gains a check that no
   `raise CaptureError(` or `throw new CaptureError(` in either package lacks a
   code, so the migration cannot half-land.

## 7. Where this leaves the measurement

Counted with `wc -l` over the two packages after §3:

| Package | Generated | Total | Share |
| --- | --- | --- | --- |
| `apxm_program` | 768 | 2922 | 26.3% |
| `@apxm/frontend` `src` | 550 | 2869 | 19.2% |

Before §3 the same counts were 273 of 2412 (11.3%) and 207 of 2502 (8.3%).
§4–§6 target the two largest hand-written files, `_capture.py` (1168 lines) and
`capture.ts` (1774 lines), of which the emit halves and record types are the
generable part; the AST walk and marker ergonomics are not, and are not meant to
be.
