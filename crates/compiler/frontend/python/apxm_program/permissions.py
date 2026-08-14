"""Public import path for the generated permission decision vocabulary.

``from apxm_program.permissions import Allow, Ask`` — the decisions themselves
are generated from the AIS permission source of truth by ``apxm codegen
permissions``. This module only gives them a stable, non-private path; it is
deliberately not re-exported from the package root, which carries exactly the
authoring surface manifest.
"""

from __future__ import annotations

from ._generated.permissions import *  # noqa: F401,F403
from ._generated.permissions import __all__ as __all__
