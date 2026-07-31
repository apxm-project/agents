"""Every gate the owner lane plan names resolves to a declared `dekk agents` command.

`dekk agents` is the only authority surface for this repository: a raw `cargo`
or `docker` invocation is not a gate. A lane that cites a gate which does not
exist as a command cannot claim to be proved, and today the only thing standing
between the plan and that state is someone reading both files carefully. This
gate makes the correspondence executable in both directions.
"""

from __future__ import annotations

import re
import tomllib
import unittest
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
DEKK_MANIFEST = REPOSITORY_ROOT / ".dekk.toml"
OWNER_LANE_PLAN = (
    REPOSITORY_ROOT / "docs/plans/agents-event-runtime-owner-lane-plan.md"
)

# `dekk agents <name>` citations in prose. A citation is a claim that the gate
# exists, so every one of them is checked.
GATE_CITATION = re.compile(r"dekk agents ([a-z0-9][a-z0-9-]*)")

# Commands that exist for operators rather than as lane evidence. They are
# allowed to go uncited by the plan without failing the reverse direction.
NON_GATE_COMMANDS = frozenset(
    {
        "clean",
        "clean-processes",
        "scrub-rustc-cache",
        "stop",
        "tokenize",
    }
)


def declared_commands() -> dict[str, str]:
    """Map every declared command name to its `run` string.

    A dekk command is any table carrying a `run` key, at the top level or
    nested in a command group.
    """

    manifest = tomllib.loads(DEKK_MANIFEST.read_text(encoding="utf-8"))
    found: dict[str, str] = {}

    def walk(table: dict[str, object]) -> None:
        for key, value in table.items():
            if not isinstance(value, dict):
                continue
            run = value.get("run")
            if isinstance(run, str):
                # A nested group may reuse a bare name such as `test`; the
                # top-level declaration is the one `dekk agents <name>`
                # resolves, so it is never overwritten by a nested one.
                found.setdefault(key, run)
            else:
                walk(value)

    walk(manifest)
    return found


def cited_gates() -> set[str]:
    return set(GATE_CITATION.findall(OWNER_LANE_PLAN.read_text(encoding="utf-8")))


class OwnerGateCommandTests(unittest.TestCase):
    def test_every_cited_gate_is_a_declared_command(self) -> None:
        declared = declared_commands()
        missing = sorted(gate for gate in cited_gates() if gate not in declared)
        self.assertEqual(
            missing,
            [],
            "the owner lane plan cites gates that `dekk agents` does not declare, "
            f"so those lanes cannot be proved: {missing}",
        )

    def test_the_plan_cites_at_least_one_gate(self) -> None:
        # A plan that cites nothing would pass the check above vacuously.
        self.assertGreater(
            len(cited_gates()),
            0,
            "the owner lane plan names no gate, so nothing above is being checked",
        )

    def test_no_cited_gate_is_an_empty_command(self) -> None:
        declared = declared_commands()
        empty = sorted(
            gate
            for gate in cited_gates()
            if gate in declared and not declared[gate].strip()
        )
        self.assertEqual(
            empty, [], f"a gate declared with no command proves nothing: {empty}"
        )

    def test_build_and_test_commands_are_reachable_through_dekk(self) -> None:
        # The ceiling is that `dekk agents` is the only authority surface. A
        # command declared outside a group is still reachable; this asserts the
        # manifest parses into named commands at all rather than silently
        # yielding an empty set, which would make every check above vacuous.
        declared = declared_commands()
        self.assertIn("clippy", declared)
        self.assertIn("owner-descriptor", declared)

    def test_gate_names_are_not_stale_aliases(self) -> None:
        # Guard the reverse direction for the suites the lanes actually run:
        # renaming a test command without updating the plan silently strands a
        # lane's evidence.
        declared = declared_commands()
        lane_suites = {
            name
            for name in declared
            if name.startswith("test-") and name not in NON_GATE_COMMANDS
        }
        cited = cited_gates()
        # Not every suite must be cited, but the core runtime suites this
        # repository's lanes depend on must be.
        required = {"test-kernel", "test-execution", "test-runtime-seams"}
        self.assertTrue(
            required <= lane_suites,
            f"a core runtime suite is no longer declared: {sorted(required - lane_suites)}",
        )
        self.assertTrue(
            required <= cited,
            f"a core runtime suite is no longer cited by the plan: {sorted(required - cited)}",
        )


if __name__ == "__main__":
    unittest.main()
