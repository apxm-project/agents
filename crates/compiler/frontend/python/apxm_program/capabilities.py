"""Public import path for the generated capability id catalogue.

``from apxm_program.capabilities import SEARCH_WEB`` — the symbols themselves are
generated from the AIS capability source of truth by ``apxm codegen
capabilities``. This module only gives them a stable, non-private path; it is
deliberately not re-exported from the package root, which carries exactly the
authoring surface manifest.
"""

from __future__ import annotations

from ._generated.capabilities import *  # noqa: F401,F403
from ._generated.capabilities import __all__ as __all__
