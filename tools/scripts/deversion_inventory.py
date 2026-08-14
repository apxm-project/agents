#!/usr/bin/env python3
"""Classify every versioned APXM identifier as Agents-owned or foreign.

The de-versioning sweep drops the `.vN` suffix from every id Agents owns. It must
NOT touch ids owned by another party: renaming those breaks a cross-owner contract
and violates the product-neutrality boundary in `.agents/project.md` §1.

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

# Versioned FILENAMES carry the generation marker in any of four separator forms:
# `op-spec.v1.json`, `conversational-python.v2.json`, `event_v1.py`, `event-v1.ts`.
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

FOREIGN_IDS = (
    CONTRACTS_OWNER_IDS
    | VLLM_OWNER_IDS
    | DANGLING_IDS
    | STORAGE_FORMAT_IDS
    | DIGEST_SEED_STRINGS
    | GRAMMAR_NEGATIVE_IDS
)

# Paths whose `.vN` occurrences are *rejection* fixtures: the suffixed string is the
# thing under test, so it must survive. After the sweep `is_schema_id` rejects any
# Agents-owned suffixed id, which keeps every one of these a valid negative case.
#
# The two standalone `contracts/vectors/apxm.*.v1-rejection.json` files this set used
# to name were folded into the de-versioned vector files as named negative cases
# (`"name": "incompatible-air-v1-schema-version-rejected"`). The entries move with
# them rather than being dropped: the retired AIR and FrontendGraph coordinates are
# still under test, just from a new home, and dropping the entries would make --check
# report them as unswept.
REJECTION_FIXTURE_PATHS = frozenset(
    {
        "tools/tests/test_no_active_program_contract_v1.py",
        "contracts/vectors/apxm.air.json",
        "contracts/vectors/apxm.frontend-graph.json",
    }
)

# Preserved paper history (CLAUDE.md §8). Never swept.
PRESERVED_PREFIXES = ("docs/pxm/",)


def tracked_files() -> list[str]:
    # cwd= rather than `git -C`: the deployment host runs git 1.8.3.1, which has no
    # -C flag. Output order is git's own sort, so the scan is deterministic.
    out = subprocess.run(
        ["git", "ls-files"], cwd=REPO_ROOT, capture_output=True, text=True, check=True
    )
    return [line for line in out.stdout.splitlines() if line]


def is_preserved(path: str) -> bool:
    return any(path.startswith(prefix) for prefix in PRESERVED_PREFIXES)


def deversion(schema_id: str) -> str:
    """Drop the trailing `.vN` from an Agents-owned id."""
    return re.sub(r"\.v[0-9]+$", "", schema_id)


def deversion_filename(name: str) -> str:
    """Drop the generation marker from a versioned filename.

    Handles every separator form: `op-spec.v1.json` -> `op-spec.json`,
    `event_v1.py` -> `event.py`, `event-v1.ts` -> `event.ts`.
    """
    return re.sub(r"[._-]v[0-9]+(?=[._-]|$)", "", name)


def scan() -> dict[str, dict[str, int]]:
    """Map each id to {path: occurrence_count}."""
    found: dict[str, dict[str, int]] = {}
    for rel in tracked_files():
        if is_preserved(rel):
            continue
        path = REPO_ROOT / rel
        try:
            text = path.read_text(encoding="utf-8")
        except (UnicodeDecodeError, FileNotFoundError, IsADirectoryError, OSError):
            continue
        for schema_id in VERSIONED_ID.findall(text):
            found.setdefault(schema_id, {}).setdefault(rel, 0)
            found[schema_id][rel] += 1
    return found


def owned_ids(found: dict[str, dict[str, int]] | None = None) -> set[str]:
    found = scan() if found is None else found
    return {schema_id for schema_id in found if schema_id not in FOREIGN_IDS}


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
        elif any(schema_id in name for schema_id in FOREIGN_IDS):
            keep.append((rel, "foreign owner"))
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
