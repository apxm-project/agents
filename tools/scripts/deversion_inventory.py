#!/usr/bin/env python3
"""Classify every versioned APXM identifier as Agents-owned or foreign.

The de-versioning sweep drops the `.vN` suffix from every id Agents owns. It must
NOT touch ids owned by another party: renaming those breaks a cross-owner contract
and violates the product-neutrality boundary in `AGENTS.md` §1.

This module is the single source of truth for that split. The sweep reads it, and
the consistency gate reads it, so "which ids are ours" cannot drift between the
change and the test that guards the change.

Two modes:

* default    — human-readable inventory of owned ids, foreign ids, file renames,
               and source lines that break when a renamed file moves.
* ``--check`` — print only violations and exit 1 if any Agents-owned versioned id
               or filename survives. This is what makes the module a gate rather
               than a report.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]

# Prefix-agnostic on purpose. The original pattern was anchored to a literal
# `apxm.` prefix, which made every versioned Agents-owned identifier that does not
# carry that namespace invisible to the sweep — example-program Model/Capability/
# Event ids (`parity.model.vN`, `review.model.vN`, `search.web.capability.vN`),
# typed port slots (`model.target.vN`), and generated-artifact ids
# (`op-spec.vectors.vN`). Ownership is decided by FOREIGN_IDS and
# PRESERVED_PREFIXES below, never by the leading namespace.
#
# Shape: a dotted identifier of at least three segments ending in `.vN`. The
# multi-segment tail also covers the port-contract document form
# (`apxm.model-inference.port-contract.vN`), which appears only as a filename —
# `is_schema_id` deliberately rejects multi-segment ids, see grammar.rs:77.
# Underscores are admitted as well as hyphens: `apxm.handler_manifest.vN` was
# invisible to a `[a-z0-9-]`-only class and survived a full sweep undetected.
#
# Examples here are written with a literal `vN` rather than a digit: this file
# is scanned by its own gate, and a real suffix in a comment is a false positive.
VERSIONED_ID = re.compile(r"\b[a-z][a-z0-9_-]*(?:\.[a-z0-9_-]+)+\.v[0-9]+\b")

# Source IDENTIFIERS carrying the generation marker: `ConformanceReport::schema_vN`,
# `REDACTION_POLICY_SUMMARY_HASH = "summary_hash_vN"`. A dotted-id pattern cannot
# see these — no Rust, TypeScript, or Python name is dotted — so five of them were
# swept by hand in one commit with nothing left behind to catch the sixth. Scoped
# to source files, and to a marker at the end of the name, so `schema_vN_path`
# (a name that merely mentions a generation) is not a rename this gate demands.
VERSIONED_IDENTIFIER = re.compile(r"\b[A-Za-z][A-Za-z0-9]*(?:_[A-Za-z0-9]+)*_[vV][0-9]+\b")
IDENTIFIER_SUFFIXES = (".rs", ".ts", ".py")

# Versioned FILENAMES carry the generation marker in any of four separator forms:
# `op-spec.v1.json`, `conversational-python.v2.json`, `event_vN.py`, `event-v1.ts`.
# Matching only the dotted-id form missed the `_vN` and `-vN` spellings entirely.
VERSIONED_FILENAME = re.compile(r"(?:^|(?<=[._-]))v[0-9]+(?=[._-]|$)")

# Ids owned by the private Contracts repo. `crates/machine/program/tests/fixtures/
# contracts/PROVENANCE.md` pins each to an upstream revision AND a content SHA-256
# and calls them "immutable inputs copied from the private Contracts owner; they
# are not Agents-owned production schemas".
CONTRACTS_OWNER_IDS = frozenset(
    {
        "apxm.contract-common.v1",
        "apxm.port-requirement.v1",
    }
)

# Ids owned by the vLLM project. The vLLM `apxm` branch is the live receiver.
# Agents does not keep a conformance-join catalog.
VLLM_OWNER_IDS = frozenset(
    {
        "apxm.vllm-inference.v1",
        "apxm.vllm-inference-request.v1",
        "apxm.vllm-inference-result.v1",
        "apxm.vllm-inference-failure.v1",
        "apxm.vllm-inference-stream-chunk.v1",
        "apxm.vllm-native-serving-binding.v1",
    }
)

# A `$ref` target with no definition anywhere in the tree — a pre-existing dangling
# reference, not something this sweep introduced. Renaming it would disguise the bug
# as a rename. Left suffixed and reported instead.
DANGLING_IDS = frozenset({"apxm.runtime-common.v1"})

# NOT a contract identity: a storage-format generation marker encoded in on-disk
# FILENAMES. `crates/runtime/commit-local/src/filesystem.rs` distinguishes the current
# store (`execution-commit-local.v2.json`) from a legacy one
# (`execution-commit-local.v1.json`) by filename alone, and `load_or_init` reports
# `SchemaMismatch` when only the legacy file is present. De-versioning collapses
# `store_path()` and `legacy_store_path()` onto the same path, which makes the legacy
# branch unreachable and feeds a v1 store into the v2 reader — durable-state corruption
# in the P-018 owner-local commit path, from a change advertised as a pure rename.
# Retiring the v1 generation is a migration with its own design, not part of this sweep.
STORAGE_FORMAT_IDS = frozenset(
    {
        "apxm.execution-commit-local.v1",
        "apxm.execution-commit-local.v2",
    }
)

# NOT ids at all: arbitrary stable strings fed to `digest_text()` at
# `canonical_execute.rs:1575-1576`, whose SHA-256 becomes `sandbox_digest` and
# `policy_digest`. Rewriting the seed rewrites the digest, which invalidates every
# checked-in admission fixture. They only look like ids.
DIGEST_SEED_STRINGS = frozenset(
    {
        "apxm.canonical.sandbox.v1",
        "apxm.canonical.policy.v1",
    }
)

# A grammar negative test asserting `is_schema_id` rejects a multi-segment id
# (`grammar.rs:77`). The suffix is the thing under test.
GRAMMAR_NEGATIVE_IDS = frozenset({"apxm.a.b.v1"})

# Source identifiers whose generation marker belongs to somebody else's API.
# `new_v4`/`new_v7`/`now_v7` are the `uuid` crate's constructors and name a UUID
# version, not a schema generation. `RUNTIME_DISPATCH_IR_V1` and its value
# `dispatch_ir_v1` are vLLM's own name — `constants.rs:626` says renaming it
# breaks the contract. `needs_v1` decides whether an OpenAI-style base URL still
# needs its `/v1` path segment, which is that vendor's route, not our id.
FOREIGN_IDENTIFIERS = frozenset(
    {
        "new_v4",
        "new_v7",
        "now_v7",
        "RUNTIME_DISPATCH_IR_V1",
        "dispatch_ir_v1",
        "needs_v1",
    }
)

# Canonical APXM wire/schema identities deliberately carry their generation.
# These are compatibility identities, not Agents source identifiers to sweep:
# changing them would change the contract vocabulary (and the v2 runtime
# request/handler dispatch names) while leaving existing peers on the old
# wire shape. Keep this list exact so the de-version gate remains strict for
# any new versioned identifier or filename.
CANONICAL_VERSIONED_IDS = frozenset(
    {
        "apxm.agents-local-release-artifact.v1",
        "apxm.agents-owner-descriptor.v1",
        "apxm.agents-service-release-manifest.v1",
        "apxm.agents-source-revision.v1",
        "apxm.agents.local-release-artifact.v1",
        "apxm.agents.owner-command-evidence.v1",
        "apxm.agents.owner-command-execution.v1",
        "apxm.agents.owner-handoff.v1",
        "apxm.agents.owner-phase-result.v1",
        "apxm.agents.owner-qualification-failure.v1",
        "apxm.agents.owner-qualification.v1",
        "apxm.compile-diagnostics.v1",
        "apxm.owner-command-output.v1",
        "apxm.agents.release-consumer-verification.v1",
        "apxm.agents.release-qualification.v1",
        "apxm.continuation-integrity.v1",
        "apxm.execution-lineage.v1",
        "apxm.execution-observation.v1",
        "apxm.execution-read.v1",
        "apxm.node-execution-inspection.v1",
        "apxm.session-output-ref.v1",
        "apxm.runtime-service.metadata.v1",
        "handle_v2",
        "request_v2",
    }
)

# The source-revision manifest filename is a published release input, while
# its basename does not repeat the full schema id. Preserve this exact path so
# the gate cannot infer a rename that would make the container build consume a
# different release manifest.
CANONICAL_VERSIONED_PATHS = frozenset({"deploy/services/source-revision.v1.json"})

FOREIGN_IDS = (
    CONTRACTS_OWNER_IDS
    | VLLM_OWNER_IDS
    | DANGLING_IDS
    | STORAGE_FORMAT_IDS
    | DIGEST_SEED_STRINGS
    | GRAMMAR_NEGATIVE_IDS
    | FOREIGN_IDENTIFIERS
)

# Paths whose `.vN` occurrences are *rejection* fixtures: the suffixed string is the
# thing under test, so it must survive. After the sweep `is_schema_id` rejects any
# Agents-owned suffixed id, which keeps every one of these a valid negative case.
#
# Only non-vector files are listed here. A whole-file exemption is blunt — it also
# stops the gate seeing a genuinely unswept id elsewhere in the same file — so the
# contract vectors, which are uniformly a JSON list of `{name, input, expected_valid}`
# cases, are exempted per-case by `negative_case_ids` instead. That keeps a positive
# vector in a file that also holds negative ones fully in scope.
REJECTION_FIXTURE_PATHS = frozenset({"tools/tests/test_no_active_program_contract_v1.py"})

# Preserved paper history (CLAUDE.md §8). Never swept.
PRESERVED_PREFIXES = ("docs/pxm/",)


def tracked_files() -> list[str]:
    # cwd= rather than `git -C`: the deployment host runs git 1.8.3.1, which has no
    # -C flag. Output order is git's own sort, so the scan is deterministic.
    git_env = os.environ.copy()
    if sys.platform == "darwin":
        # Dekk's conda environment ships libiconv, while the host Git links
        # against Homebrew's ABI. Do not let the conda loader path interpose
        # on Git, or the inventory gate aborts before it can scan the tree.
        git_env.pop("DYLD_LIBRARY_PATH", None)
        git_env.pop("DYLD_FALLBACK_LIBRARY_PATH", None)
    out = subprocess.run(
        ["git", "ls-files"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
        env=git_env,
    )
    return [line for line in out.stdout.splitlines() if line]


def is_preserved(path: str) -> bool:
    return any(path.startswith(prefix) for prefix in PRESERVED_PREFIXES)


def deversion(schema_id: str) -> str:
    """Drop the trailing generation marker from an Agents-owned id or identifier."""
    return re.sub(r"[._][vV][0-9]+$", "", schema_id)


def deversion_filename(name: str) -> str:
    """Drop the generation marker from a versioned filename.

    Handles every separator form: `op-spec.v1.json` -> `op-spec.json`,
    `event_vN.py` -> `event.py`, `event-v1.ts` -> `event.ts`.
    """
    return re.sub(r"[._-]v[0-9]+(?=[._-]|$)", "", name)


def negative_case_ids(rel: str, text: str) -> set[str]:
    """Ids that appear only inside `expected_valid: false` cases of a contract vector.

    A rejection case must carry the retired coordinate verbatim — that string is
    what the reader has to refuse — so those occurrences are not unswept ids. This
    is narrower than exempting the file: an id that also appears in a positive case
    is not returned here, and so stays a violation.
    """
    if not rel.startswith("contracts/vectors/") or not rel.endswith(".json"):
        return set()
    try:
        cases = json.loads(text)
    except json.JSONDecodeError:
        return set()
    if not isinstance(cases, list):
        return set()
    negative: set[str] = set()
    positive: set[str] = set()
    for case in cases:
        if not isinstance(case, dict):
            continue
        side = negative if case.get("expected_valid") is False else positive
        side.update(VERSIONED_ID.findall(json.dumps(case)))
    return negative - positive


def fixture_stems() -> set[str]:
    """Identifier-shaped names that are only ever a rejection fixture's own filename.

    `test_no_active_program_contract_v1.py` keeps its suffix because the suffix is
    what it tests, so the stem naming it — here, and in this module's own listing
    of it — is not an identifier awaiting a sweep.
    """
    return {Path(rel).name.split(".")[0] for rel in REJECTION_FIXTURE_PATHS}


def scan() -> dict[str, dict[str, int]]:
    """Map each id to {path: occurrence_count}."""
    found: dict[str, dict[str, int]] = {}
    stems = fixture_stems()
    for rel in tracked_files():
        if is_preserved(rel):
            continue
        path = REPO_ROOT / rel
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, FileNotFoundError, IsADirectoryError, OSError):
            continue
        under_test = negative_case_ids(rel, text)
        names = list(VERSIONED_ID.findall(text))
        if rel.endswith(IDENTIFIER_SUFFIXES):
            names += [
                name
                for name in VERSIONED_IDENTIFIER.findall(text)
                if name not in stems
            ]
        for schema_id in names:
            if schema_id in under_test:
                continue
            found.setdefault(schema_id, {}).setdefault(rel, 0)
            found[schema_id][rel] += 1
    return found


def owned_ids(found: dict[str, dict[str, int]] | None = None) -> set[str]:
    found = scan() if found is None else found
    return {
        schema_id
        for schema_id in found
        if schema_id not in FOREIGN_IDS and schema_id not in CANONICAL_VERSIONED_IDS
    }


def renames() -> tuple[list[tuple[str, str]], list[tuple[str, str]]]:
    """Split versioned filenames into (to-move, to-keep-with-reason)."""
    move: list[tuple[str, str]] = []
    keep: list[tuple[str, str]] = []
    for rel in tracked_files():
        if is_preserved(rel):
            continue
        name = Path(rel).name
        if not VERSIONED_FILENAME.search(name):
            continue
        if rel in REJECTION_FIXTURE_PATHS:
            keep.append((rel, "rejection fixture — the suffix is under test"))
        elif rel in CANONICAL_VERSIONED_PATHS:
            keep.append((rel, "canonical wire/schema filename"))
        elif any(
            schema_id in name
            for schema_id in FOREIGN_IDS | CANONICAL_VERSIONED_IDS
        ):
            reason = (
                "canonical wire/schema identity"
                if any(schema_id in name for schema_id in CANONICAL_VERSIONED_IDS)
                else "foreign owner"
            )
            keep.append((rel, reason))
        else:
            move.append((rel, str(Path(rel).parent / deversion_filename(name))))
    return move, keep


def stale_path_references(moving: set[str]) -> list[tuple[str, int, str]]:
    """Find source lines naming a file this sweep renames.

    `include_str!`/`include_bytes!` paths and test fixture paths break silently at
    the moment a contract file moves, so they must move in the same change.
    """
    basenames = {Path(rel).name for rel in moving}
    hits: list[tuple[str, int, str]] = []
    for rel in tracked_files():
        if is_preserved(rel) or rel in moving:
            continue
        path = REPO_ROOT / rel
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except (UnicodeDecodeError, FileNotFoundError, IsADirectoryError, OSError):
            continue
        for line_no, text in enumerate(lines, start=1):
            if any(name in text for name in basenames):
                hits.append((rel, line_no, text))
    return hits


def violations(
    found: dict[str, dict[str, int]] | None = None,
) -> tuple[dict[str, dict[str, int]], list[tuple[str, str]]]:
    """Agents-owned versioned ids and filenames that survived the sweep.

    An id whose only occurrences are inside REJECTION_FIXTURE_PATHS is not a
    violation: there the suffix is the thing under test.
    """
    found = scan() if found is None else found
    id_hits: dict[str, dict[str, int]] = {}
    for schema_id in owned_ids(found):
        paths = {
            rel: count
            for rel, count in found[schema_id].items()
            if rel not in REJECTION_FIXTURE_PATHS
        }
        if paths:
            id_hits[schema_id] = paths
    move, _ = renames()
    return id_hits, move


def check() -> int:
    id_hits, move = violations()
    if not id_hits and not move:
        print("deversion gate: clean — no Agents-owned versioned ids or filenames.")
        return 0

    total = sum(sum(paths.values()) for paths in id_hits.values())
    print(
        f"deversion gate: FAILED — {len(id_hits)} Agents-owned versioned id(s) "
        f"in {total} occurrence(s), {len(move)} versioned filename(s)."
    )
    if id_hits:
        print("\nVERSIONED IDS THAT MUST BE SWEPT:")
        for schema_id in sorted(id_hits):
            print(f"  {schema_id}  ->  {deversion(schema_id)}")
            for rel in sorted(id_hits[schema_id]):
                print(f"      {id_hits[schema_id][rel]:4d}  {rel}")
    if move:
        print("\nVERSIONED FILENAMES THAT MUST BE RENAMED:")
        for rel, dest in sorted(move):
            print(f"  {rel}\n   -> {dest}")
    print(
        "\nEither sweep the suffix, or add the id to a FOREIGN_IDS set (or the path "
        "to PRESERVED_PREFIXES / REJECTION_FIXTURE_PATHS) with a comment saying why."
    )
    return 1


def report() -> int:
    found = scan()
    owned = sorted(owned_ids(found))
    foreign = sorted(schema_id for schema_id in found if schema_id in FOREIGN_IDS)
    canonical = sorted(
        schema_id for schema_id in found if schema_id in CANONICAL_VERSIONED_IDS
    )

    def total(schema_id: str) -> int:
        return sum(found[schema_id].values())

    print(f"distinct ids: {len(found)}    occurrences: {sum(map(total, found))}")
    print(f"  owned:   {len(owned):3d} ids, {sum(map(total, owned)):5d} occurrences")
    print(f"  foreign: {len(foreign):3d} ids, {sum(map(total, foreign)):5d} occurrences")
    print()
    print("FOREIGN (never renamed):")
    for schema_id in foreign:
        print(f"  {total(schema_id):5d}  {schema_id}")
    print()
    print("CANONICAL VERSIONED (wire/schema compatibility identities):")
    for schema_id in canonical:
        print(f"  {total(schema_id):5d}  {schema_id}")
    print()
    print("REJECTION FIXTURES (suffix is the thing under test, keep suffixed):")
    for path in sorted(REJECTION_FIXTURE_PATHS):
        print(f"         {path}")
    print()
    print("OWNED (swept):")
    for schema_id in owned:
        print(f"  {total(schema_id):5d}  {schema_id}  ->  {deversion(schema_id)}")
    print()
    move, keep = renames()
    print(f"FILE RENAMES ({len(move)} git mv, {len(keep)} kept):")
    for rel, dest in sorted(move):
        print(f"  mv  {rel}\n   -> {dest}")
    for rel, why in sorted(keep):
        print(f"  KEEP {rel}   ({why})")
    print()
    stale = stale_path_references({rel for rel, _ in move})
    print(f"HARDCODED PATHS THAT BREAK ON RENAME ({len(stale)}):")
    for rel, line_no, text in stale:
        print(f"  {rel}:{line_no}  {text.strip()[:100]}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check",
        action="store_true",
        help="print only violations and exit 1 if any Agents-owned versioned id "
        "or filename remains",
    )
    args = parser.parse_args(argv)
    return check() if args.check else report()


if __name__ == "__main__":
    sys.exit(main())
