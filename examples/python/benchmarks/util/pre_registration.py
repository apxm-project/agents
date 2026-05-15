"""Pre-registration file gate for benchmark harnesses.

The pre-registration discipline (see `docs/plans/00-evaluation-methodology.md`
§5) requires that hypotheses, SLOs, and metric tiers be committed in writing
*before* a measurement run starts. This module provides the file gate that
benchmark harnesses use to enforce that.

Public surface:
    enforce(*, pre_registration_arg, require, output_path) -> str | None
        Validate, copy, and return the absolute sidecar path. Exits the
        process with `EXIT_PRE_REGISTRATION_REQUIRED` when require=True
        and no valid file is provided.
    TEMPLATE_PATH                Path to the canonical template.
    SIDECAR_SUFFIX               Suffix appended to the output path.
    EXIT_PRE_REGISTRATION_REQUIRED  Process exit code on failure.
"""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

TEMPLATE_PATH = (
    Path(__file__).resolve().parent.parent / "templates" / "pre-registration.template.md"
)
SIDECAR_SUFFIX = ".pre-registration.md"
EXIT_PRE_REGISTRATION_REQUIRED = 2


def enforce(
    *,
    pre_registration_arg: Path | None,
    require: bool,
    output_path: Path,
) -> str | None:
    """Validate, copy, and return the resolved pre-registration sidecar path.

    Returns the absolute path string of the copied sidecar when a pre-reg
    file is supplied, None when none is supplied (and not required), and
    exits the process when required but missing.
    """
    if pre_registration_arg is None:
        if require:
            sys.stderr.write(
                "error: --require-pre-registration was set but --pre-registration "
                "was not provided.\n"
                f"Author the file from the template at {TEMPLATE_PATH} "
                "and pass it via --pre-registration <PATH>.\n"
            )
            raise SystemExit(EXIT_PRE_REGISTRATION_REQUIRED)
        return None
    src = pre_registration_arg.resolve()
    if not src.is_file():
        sys.stderr.write(f"error: pre-registration file not found: {src}\n")
        raise SystemExit(EXIT_PRE_REGISTRATION_REQUIRED)
    sidecar = output_path.with_suffix(output_path.suffix + SIDECAR_SUFFIX)
    sidecar.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(src, sidecar)
    return str(sidecar)
