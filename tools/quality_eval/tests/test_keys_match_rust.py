"""Drift test: assert `tools/quality_eval/_keys.py` mirrors the Rust source
of truth at `crates/core/apxm-core/src/constants.rs::session`.

If a Rust constant in `session::files`, `session::results_keys`, or
`session::metrics_keys` changes value (or is added/removed), this test fails
and the Python mirror must be updated in lockstep. Catches the silent-drift
class of bug where Python harness keeps reading `"final_output"` while Rust
quietly serializes `"final_value"`.

Only the three modules above are mirrored — `session::node` and
`session::files::NODES_DIR` ARE included (NODES_DIR is mirrored), but
`session::node` is intentionally not yet mirrored because the harness does
not descend into per-node files.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from quality_eval._keys import (  # noqa: E402
    MetricsKeys,
    ResultsKeys,
    SessionFiles,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
RUST_CONSTANTS = (
    REPO_ROOT / "crates" / "core" / "apxm-core" / "src" / "constants.rs"
)


def _extract_module_consts(source: str, module_path: list[str]) -> dict[str, str]:
    """Return {const_name: value} for `pub const NAME: &str = "..."` lines
    nested inside the given module path.

    `module_path` is the chain of `pub mod` names to descend, e.g.
    `["session", "results_keys"]`.
    """
    # Walk the source line-by-line tracking module nesting via `pub mod X {`
    # / `}` braces. Cheap and good-enough — constants.rs has no string
    # literals containing `}` at module scope.
    consts: dict[str, str] = {}
    stack: list[str] = []
    depth_of_module: dict[int, str] = {}
    depth = 0
    mod_open_re = re.compile(r"^\s*pub mod (\w+)\s*\{")
    const_re = re.compile(r'^\s*pub const (\w+)\s*:\s*&str\s*=\s*"([^"]*)"\s*;')

    for line in source.splitlines():
        m = mod_open_re.match(line)
        if m:
            depth += 1
            stack.append(m.group(1))
            depth_of_module[depth] = m.group(1)
            # `pub mod X { ... }` on a single line is not used in this file.
            continue

        # Count brace deltas for non-mod lines so we pop on close.
        opens = line.count("{")
        closes = line.count("}")
        # `pub mod X {` was already counted above; non-mod lines only.
        if opens and not mod_open_re.match(line):
            depth += opens
        if closes:
            depth -= closes
            while stack and depth < len(stack):
                stack.pop()

        if stack[: len(module_path)] == module_path and len(stack) == len(module_path):
            cm = const_re.match(line)
            if cm:
                consts[cm.group(1)] = cm.group(2)

    return consts


def _python_class_consts(cls: type) -> dict[str, str]:
    return {
        name: getattr(cls, name)
        for name in vars(cls)
        if name.isupper() and isinstance(getattr(cls, name), str)
    }


def test_session_files_match_rust():
    rust = _extract_module_consts(RUST_CONSTANTS.read_text(), ["session", "files"])
    py = _python_class_consts(SessionFiles)
    assert rust, "failed to parse session::files from Rust constants.rs"
    # Every Python entry must exist in Rust with identical value. Rust may
    # carry extras the harness doesn't yet need (e.g. future per-node files);
    # asserting subset keeps the mirror minimal without failing on growth.
    for name, value in py.items():
        assert name in rust, f"SessionFiles.{name} not present in Rust session::files"
        assert rust[name] == value, (
            f"SessionFiles.{name} drift: python={value!r} rust={rust[name]!r}"
        )


def test_results_keys_match_rust():
    rust = _extract_module_consts(
        RUST_CONSTANTS.read_text(), ["session", "results_keys"]
    )
    py = _python_class_consts(ResultsKeys)
    assert rust, "failed to parse session::results_keys from Rust constants.rs"
    # results_keys is small and stable — assert exact set parity so adding
    # a Rust key without mirroring it (or vice versa) is a hard failure.
    assert set(py.keys()) == set(rust.keys()), (
        f"ResultsKeys drift: python_only={set(py) - set(rust)}, "
        f"rust_only={set(rust) - set(py)}"
    )
    for name, value in py.items():
        assert rust[name] == value, (
            f"ResultsKeys.{name} drift: python={value!r} rust={rust[name]!r}"
        )


def test_metrics_keys_match_rust():
    rust = _extract_module_consts(
        RUST_CONSTANTS.read_text(), ["session", "metrics_keys"]
    )
    py = _python_class_consts(MetricsKeys)
    assert rust, "failed to parse session::metrics_keys from Rust constants.rs"
    assert set(py.keys()) == set(rust.keys()), (
        f"MetricsKeys drift: python_only={set(py) - set(rust)}, "
        f"rust_only={set(rust) - set(py)}"
    )
    for name, value in py.items():
        assert rust[name] == value, (
            f"MetricsKeys.{name} drift: python={value!r} rust={rust[name]!r}"
        )
