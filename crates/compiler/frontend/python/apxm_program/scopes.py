"""Public import path for the generated Hook scope vocabulary.

``from apxm_program.scopes import CAPABILITY, MODEL`` — the scopes themselves are
generated from the ``apxm.frontend-graph`` contract by ``apxm codegen
frontend-vocabulary``. This module only gives them a stable, non-private path; it
is deliberately not re-exported from the package root, which carries exactly the
authoring surface manifest.
"""

from __future__ import annotations

from ._generated.scopes import *  # noqa: F401,F403
from ._generated.scopes import __all__ as __all__
